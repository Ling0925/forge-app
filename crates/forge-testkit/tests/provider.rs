use forge_protocol::{Content, MessageRole, ModelSelection};
use forge_provider::{
    ErrorKind, Provider, ProviderError, ProviderMessage, ProviderRequest, RetryAfter, StreamEvent,
};
use forge_testkit::provider::{Script, ScriptedProvider, Step};
use forge_tools::clock::FakeClock;
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
    time::{Duration, SystemTime},
};

fn request(text: &str) -> ProviderRequest {
    ProviderRequest {
        run: "run".into(),
        model: ModelSelection {
            provider: "script".into(),
            model: "fixture".into(),
        },
        system: None,
        messages: vec![ProviderMessage {
            role: MessageRole::User,
            content: Content {
                text: text.into(),
                attachments: vec![],
            },
            tool_call_id: None,
            tool_calls: vec![],
            is_error: false,
        }],
        tools: vec![],
    }
}

#[tokio::test]
async fn records_each_request_and_preserves_http_errors_and_retry_after() {
    let mut errors = Vec::new();
    for status in [401, 429, 503] {
        for retry_after in [
            None,
            Some(RetryAfter::Delay(Duration::from_secs(120))),
            Some(RetryAfter::At(
                SystemTime::UNIX_EPOCH + Duration::from_secs(10),
            )),
        ] {
            errors.push(ProviderError {
                code: None,
                kind: ErrorKind::Http {
                    status,
                    retry_after,
                },
                message: format!("HTTP {status}"),
            });
        }
    }
    let provider = ScriptedProvider::new(
        Arc::new(FakeClock::default()),
        errors.iter().cloned().map(Script::Error),
    );
    let mut requests = Vec::new();
    for (index, expected) in errors.iter().enumerate() {
        let request = request(&format!("input {index}"));
        requests.push(request.clone());
        let error = provider.request(request).await.err().unwrap();
        assert_eq!(&error, expected);
        assert_eq!(error.retry_after(), expected.retry_after());
        assert_eq!(provider.requests(), requests);
    }
}

#[tokio::test]
async fn streams_fragments_delays_and_disconnects() {
    let clock = Arc::new(FakeClock::default());
    let provider = ScriptedProvider::new(
        clock.clone(),
        [Script::Stream(vec![
            Step::Event(StreamEvent::TextDelta("first".into())),
            Step::Delay(Duration::from_secs(2)),
            Step::Event(StreamEvent::ReasoningDelta("reason".into())),
            Step::Disconnect,
        ])],
    );
    let mut stream = provider.request(request("input")).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        StreamEvent::TextDelta("first".into())
    );
    let mut delayed = stream.next();
    assert!(matches!(
        delayed
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    clock.advance(Duration::from_secs(2));
    assert_eq!(
        delayed.await.unwrap().unwrap(),
        StreamEvent::ReasoningDelta("reason".into())
    );
    assert_eq!(
        stream.next().await.unwrap().unwrap_err().kind,
        ErrorKind::Disconnected
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn stream_error_ends_stream_and_exhaustion_is_not_retryable() {
    let error = ProviderError {
        code: None,
        kind: ErrorKind::Http {
            status: 503,
            retry_after: Some(RetryAfter::Delay(Duration::from_secs(3))),
        },
        message: "unavailable".into(),
    };
    let provider = ScriptedProvider::new(
        Arc::new(FakeClock::default()),
        [Script::Stream(vec![
            Step::Error(error.clone()),
            Step::Event(StreamEvent::Finished),
        ])],
    );
    let mut stream = provider.request(request("input")).await.unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap_err(), error);
    assert!(stream.next().await.is_none());
    let error = provider.request(request("next")).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Configuration);
    assert!(!error.retryable());
}
