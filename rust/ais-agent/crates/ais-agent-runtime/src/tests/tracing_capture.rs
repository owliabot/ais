use std::{
    io,
    sync::{Arc, Mutex, OnceLock},
};

use tracing_subscriber::fmt::MakeWriter;

pub fn capture_tracing_output<T>(f: impl FnOnce() -> T) -> (String, T) {
    capture_tracing_output_at_level(tracing::Level::DEBUG, f)
}

pub fn capture_tracing_output_at_level<T>(
    level: tracing::Level,
    f: impl FnOnce() -> T,
) -> (String, T) {
    let _guard = tracing_capture_lock().lock().expect("capture lock");
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(level)
        .without_time()
        .with_writer(SharedBuffer(buffer.clone()))
        .finish();

    let result = tracing::subscriber::with_default(subscriber, f);
    let output = String::from_utf8(buffer.lock().expect("buffer lock").clone()).expect("utf8");
    (output, result)
}

fn tracing_capture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Clone)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBufferGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedBufferGuard(self.0.clone())
    }
}

struct SharedBufferGuard(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBufferGuard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
