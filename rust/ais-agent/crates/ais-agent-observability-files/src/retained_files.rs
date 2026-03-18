use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use time::{Date, Duration, Month, OffsetDateTime};

#[derive(Debug)]
pub struct DailyFileSink {
    dir: PathBuf,
    prefix: String,
    extension: &'static str,
    retention_days: u16,
    active_date: Option<Date>,
    active_file: Option<File>,
}

impl DailyFileSink {
    pub fn new(
        dir: PathBuf,
        prefix: impl Into<String>,
        extension: &'static str,
        retention_days: u16,
    ) -> io::Result<Self> {
        let mut sink = Self {
            dir,
            prefix: prefix.into(),
            extension,
            retention_days,
            active_date: None,
            active_file: None,
        };
        sink.ensure_ready_for(current_utc_date())?;
        Ok(sink)
    }

    pub fn append_line(&mut self, line: &str) -> io::Result<()> {
        self.append_bytes(line.as_bytes())?;
        self.append_bytes(b"\n")
    }

    pub fn append_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.append_bytes_at(current_utc_date(), bytes)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.active_file.as_mut() {
            file.flush()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn append_bytes_at(&mut self, date: Date, bytes: &[u8]) -> io::Result<()> {
        self.ensure_ready_for(date)?;
        let file = self
            .active_file
            .as_mut()
            .ok_or_else(|| io::Error::other("daily file sink has no active file"))?;
        file.write_all(bytes)?;
        Ok(())
    }

    #[cfg(not(test))]
    fn append_bytes_at(&mut self, date: Date, bytes: &[u8]) -> io::Result<()> {
        self.ensure_ready_for(date)?;
        let file = self
            .active_file
            .as_mut()
            .ok_or_else(|| io::Error::other("daily file sink has no active file"))?;
        file.write_all(bytes)?;
        Ok(())
    }

    fn ensure_ready_for(&mut self, date: Date) -> io::Result<()> {
        if self.active_date == Some(date) && self.active_file.is_some() {
            return Ok(());
        }

        fs::create_dir_all(&self.dir)?;
        self.prune_old_files(date)?;
        let path = self.path_for(date);
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        self.active_date = Some(date);
        self.active_file = Some(file);
        Ok(())
    }

    fn prune_old_files(&self, current_date: Date) -> io::Result<()> {
        let keep_window_days = i64::from(self.retention_days.saturating_sub(1));
        let cutoff = current_date - Duration::days(keep_window_days);
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let Some(date) = self.parse_date_from_path(&entry.path()) else {
                continue;
            };
            if date < cutoff {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    fn path_for(&self, date: Date) -> PathBuf {
        self.dir.join(format!(
            "{}-{}.{}",
            self.prefix,
            format_date(date),
            self.extension
        ))
    }

    fn parse_date_from_path(&self, path: &Path) -> Option<Date> {
        let name = path.file_name()?.to_str()?;
        let prefix = format!("{}-", self.prefix);
        let suffix = format!(".{}", self.extension);
        let date = name
            .strip_prefix(&prefix)?
            .strip_suffix(&suffix)?
            .trim()
            .to_owned();
        parse_date(&date)
    }
}

#[derive(Debug)]
pub struct JsonlCaptureFiles {
    inbound: DailyFileSink,
    outbound: DailyFileSink,
}

impl JsonlCaptureFiles {
    pub fn new(dir: PathBuf, retention_days: u16) -> io::Result<Self> {
        Ok(Self {
            inbound: DailyFileSink::new(dir.clone(), "inbound", "jsonl", retention_days)?,
            outbound: DailyFileSink::new(dir, "outbound", "jsonl", retention_days)?,
        })
    }

    pub fn record_inbound(&mut self, line: &str) -> io::Result<()> {
        self.inbound.append_line(line)
    }

    pub fn record_outbound(&mut self, line: &str) -> io::Result<()> {
        self.outbound.append_line(line)
    }
}

fn current_utc_date() -> Date {
    OffsetDateTime::now_utc().date()
}

fn format_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn parse_date(value: &str) -> Option<Date> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u8>().ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use time::{Date, Month};

    use super::{DailyFileSink, JsonlCaptureFiles};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("ais-agent-observability-files-{name}-{unique}"))
    }

    fn date(year: i32, month: Month, day: u8) -> Date {
        Date::from_calendar_date(year, month, day).expect("date")
    }

    #[test]
    fn rotates_daily_and_prunes_beyond_retention() {
        let dir = temp_dir("retention");
        let mut sink = DailyFileSink::new(dir.clone(), "ais-agent", "log", 2).expect("create sink");

        sink.append_bytes_at(date(2026, Month::March, 16), b"day-1\n")
            .expect("append");
        sink.append_bytes_at(date(2026, Month::March, 17), b"day-2\n")
            .expect("append");
        sink.append_bytes_at(date(2026, Month::March, 18), b"day-3\n")
            .expect("append");
        sink.flush().expect("flush");

        assert!(!dir.join("ais-agent-2026-03-16.log").exists());
        assert!(dir.join("ais-agent-2026-03-17.log").exists());
        assert!(dir.join("ais-agent-2026-03-18.log").exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jsonl_capture_writes_inbound_and_outbound_files() {
        let dir = temp_dir("capture");
        let mut capture = JsonlCaptureFiles::new(dir.clone(), 7).expect("capture");
        capture
            .record_inbound("{\"type\":\"command\"}")
            .expect("inbound");
        capture
            .record_outbound("{\"type\":\"response\"}")
            .expect("outbound");

        let files = fs::read_dir(&dir)
            .expect("read dir")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 2);

        let _ = fs::remove_dir_all(dir);
    }
}
