//! Runtime event stream helpers.

use ais_agent_control::events::RunEventEnvelope;

use crate::runtime::ActiveRun;

#[derive(Debug, Clone)]
pub struct RuntimeEventSlice {
    pub events: Vec<RunEventEnvelope>,
    pub latest_event_seq: Option<u64>,
    pub next_after_event_seq: Option<u64>,
    pub truncated: bool,
}

#[derive(Debug, Default)]
pub struct RuntimeEventStream;

impl RuntimeEventStream {
    pub fn read(
        runtime: &ActiveRun,
        after_event_seq: Option<u64>,
        limit: Option<usize>,
    ) -> RuntimeEventSlice {
        let mut events = runtime
            .event_log
            .iter()
            .filter(|event| {
                after_event_seq
                    .map(|after| event.event_seq > after)
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        let truncated = limit.map(|limit| events.len() > limit).unwrap_or(false);
        if let Some(limit) = limit {
            events.truncate(limit);
        }
        let next_after_event_seq = events.last().map(|event| event.event_seq);

        RuntimeEventSlice {
            events,
            latest_event_seq: runtime.latest_event_seq(),
            next_after_event_seq,
            truncated,
        }
    }
}
