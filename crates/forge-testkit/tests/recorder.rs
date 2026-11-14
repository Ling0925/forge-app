use forge_protocol::{Event, EventEnvelope, RunOutcome};
use forge_testkit::recorder::EventRecorder;

fn finished(seq: u64) -> EventEnvelope {
    EventEnvelope {
        seq,
        event: Event::RunFinished {
            run: "run".into(),
            outcome: RunOutcome::Succeeded,
            reason: None,
        },
    }
}

#[test]
fn records_sequence_and_one_terminal_event() {
    let mut recorder = EventRecorder::default();
    recorder.record(EventEnvelope {
        seq: 4,
        event: Event::TextDelta {
            run: "run".into(),
            text: "hello".into(),
        },
    });
    recorder.record(finished(5));
    recorder.assert_ordered();
    recorder.assert_exactly_once(|event| matches!(event, Event::TextDelta { .. }));
    recorder.assert_run_finished_once(&"run".into());
    assert_eq!(recorder.events().len(), 2);
}

#[test]
#[should_panic(expected = "non-increasing")]
fn rejects_out_of_order_events() {
    let mut recorder = EventRecorder::default();
    recorder.record(finished(2));
    recorder.record(finished(1));
    recorder.assert_ordered();
}

#[test]
#[should_panic]
fn rejects_duplicate_terminal_events() {
    let mut recorder = EventRecorder::default();
    recorder.record(finished(1));
    recorder.record(finished(2));
    recorder.assert_run_finished_once(&"run".into());
}
