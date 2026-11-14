use forge_protocol::{Event, EventEnvelope, RunId};

#[derive(Default)]
pub struct EventRecorder {
    events: Vec<EventEnvelope>,
    drop_seq: Option<u64>,
}

impl EventRecorder {
    pub fn dropping_seq(seq: u64) -> Self {
        Self {
            events: Vec::new(),
            drop_seq: Some(seq),
        }
    }
    pub fn record(&mut self, envelope: EventEnvelope) -> Option<EventEnvelope> {
        if self.drop_seq == Some(envelope.seq) {
            return None;
        }
        self.events.push(envelope.clone());
        Some(envelope)
    }
    pub fn events(&self) -> &[EventEnvelope] {
        &self.events
    }

    pub fn assert_ordered(&self) {
        for pair in self.events.windows(2) {
            assert!(
                pair[0].seq < pair[1].seq,
                "non-increasing event sequence: {pair:?}"
            );
        }
    }

    pub fn assert_exactly_once(&self, predicate: impl Fn(&Event) -> bool) {
        assert_eq!(
            self.events
                .iter()
                .filter(|envelope| predicate(&envelope.event))
                .count(),
            1
        );
    }

    pub fn assert_run_finished_once(&self, run: &RunId) {
        self.assert_exactly_once(
            |event| matches!(event, Event::RunFinished { run: id, .. } if id == run),
        );
    }
}
