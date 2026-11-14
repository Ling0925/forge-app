mod common;
use forge_protocol::*;
use forge_provider::*;
use forge_testkit::http::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::time::{Duration, UNIX_EPOCH};

const KEY: &str = "sentinel-test-key-never-persist";
const ANTHROPIC: &str = include_str!("fixtures/anthropic.sse");
const OPENAI: &str = include_str!("fixtures/openai.sse");
fn request(provider: &str) -> ProviderRequest {
    ProviderRequest {
        run: "run".into(),
        model: common::settings(provider).model,
        system: Some("Be helpful".into()),
        messages: vec![ProviderMessage {
            role: MessageRole::User,
            content: Content {
                text: "hello".into(),
                attachments: vec![],
            },
            tool_call_id: None,
        }],
        tools: vec![ToolDefinition {
            name: "read".into(),
            description: "read a file".into(),
            parameters_json: r#"{"type":"object"}"#.into(),
        }],
    }
}
fn provider(name: &str, endpoint: &str, key: Option<String>) -> std::sync::Arc<dyn Provider> {
    let mut settings = common::settings(name).providers.remove(0);
    settings.endpoint = Some(endpoint.into());
    HttpProviderFactory.create(&settings, key).unwrap()
}
async fn collect(provider: &dyn Provider, name: &str) -> Result<Vec<StreamEvent>, ProviderError> {
    let mut stream = provider.request(request(name)).await?;
    let mut events = vec![];
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}
#[tokio::test]
async fn golden_streams_and_byte_split_crlf_streams_match_and_requests_are_correct() {
    for (name, fixture) in [("anthropic", ANTHROPIC), ("openai", OPENAI)] {
        assert!(!fixture.contains(KEY));
        assert!(!fixture.to_lowercase().contains("authorization"));
        let mut baseline = None;
        for split in [false, true] {
            let body = fixture.replace('\n', "\r\n");
            let mut response = MockResponse::sse(&body);
            if split {
                response.chunks = body.bytes().map(|b| vec![b]).collect();
            }
            let server = MockServer::start(response).await;
            let events = collect(
                provider(name, &server.endpoint, Some(KEY.into())).as_ref(),
                name,
            )
            .await
            .unwrap();
            let received = server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
            assert_eq!(received.method, "POST");
            let body: Value = serde_json::from_slice(&received.body).unwrap();
            assert_eq!(body["model"], "fixture-model");
            assert_eq!(body["stream"], true);
            if name == "anthropic" {
                assert_eq!(received.path, "/v1/messages");
                assert_eq!(received.headers["x-api-key"], KEY);
                assert_eq!(received.headers["anthropic-version"], ANTHROPIC_VERSION);
                assert_eq!(body["max_tokens"], 128);
                assert_eq!(body["system"], "Be helpful");
                assert_eq!(body["messages"], json!([{"role":"user","content":"hello"}]));
                assert_eq!(body["tools"][0]["input_schema"], json!({"type":"object"}));
                assert!(events.contains(&StreamEvent::ReasoningDelta("Consider it.".into())));
            } else {
                assert_eq!(received.path, "/v1/chat/completions");
                assert_eq!(received.headers["authorization"], format!("Bearer {KEY}"));
                assert_eq!(body["stream_options"]["include_usage"], true);
                assert_eq!(
                    body["messages"],
                    json!([{"role":"system","content":"Be helpful"},{"role":"user","content":"hello"}])
                );
                assert_eq!(
                    body["tools"][0]["function"]["parameters"],
                    json!({"type":"object"})
                );
            }
            assert_eq!(
                events
                    .iter()
                    .filter_map(|e| if let StreamEvent::TextDelta(s) = e {
                        Some(s.as_str())
                    } else {
                        None
                    })
                    .collect::<String>(),
                "Hello 世界"
            );
            for (id, name, args) in [
                ("call_one", "read", r#"{"path":"file"}"#),
                ("call_two", "list", "{}"),
            ] {
                assert!(events.contains(&StreamEvent::ToolCallStarted {
                    id: id.into(),
                    name: name.into()
                }));
                assert_eq!(
                    events
                        .iter()
                        .filter_map(|e| match e {
                            StreamEvent::ToolArgumentsDelta { id: actual, json }
                                if actual.0 == id =>
                                Some(json.as_str()),
                            _ => None,
                        })
                        .collect::<String>(),
                    args
                );
            }
            assert_eq!(
                events
                    .iter()
                    .filter_map(|e| if let StreamEvent::Usage(u) = e {
                        Some(u)
                    } else {
                        None
                    })
                    .next_back(),
                Some(&Usage {
                    input_tokens: 12,
                    output_tokens: 9,
                    cached_tokens: 3,
                    requests: 1,
                    estimated_cost_usd: None
                })
            );
            assert_eq!(events.last(), Some(&StreamEvent::Finished));
            if let Some(baseline) = &baseline {
                assert_eq!(&events, baseline);
            } else {
                baseline = Some(events);
            }
        }
    }
}
#[tokio::test]
async fn http_errors_preserve_status_retry_after_and_redact_keys() {
    for name in ["anthropic", "openai"] {
        for (status, header, expected) in [
            (401, None, None),
            (
                429,
                Some("120"),
                Some(RetryAfter::Delay(Duration::from_secs(120))),
            ),
            (
                503,
                Some("Thu, 01 Jan 1970 00:02:00 GMT"),
                Some(RetryAfter::At(UNIX_EPOCH + Duration::from_secs(120))),
            ),
        ] {
            let mut response = MockResponse::sse(&format!("{{\"error\":\"bad {KEY}\"}}"));
            response.status = status;
            if let Some(header) = header {
                response.headers.push(("retry-after".into(), header.into()));
            }
            let server = MockServer::start(response).await;
            let error = collect(
                provider(name, &server.endpoint, Some(KEY.into())).as_ref(),
                name,
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.kind,
                ErrorKind::Http {
                    status,
                    retry_after: expected
                }
            );
            assert!(!error.to_string().contains(KEY));
            assert!(error.message.contains("[redacted]"));
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}
#[tokio::test]
async fn malformed_streams_and_disconnects_are_distinct_and_terminal_stops_reading() {
    for name in ["anthropic", "openai"] {
        for (body, finish, kind) in [
            ("data: {bad}\n\n", true, ErrorKind::InvalidResponse),
            ("data: {}\n\n", true, ErrorKind::InvalidResponse),
            (": comment\n\n", true, ErrorKind::Disconnected),
            (": comment\n\n", false, ErrorKind::Disconnected),
        ] {
            let mut response = MockResponse::sse(body);
            response.finish = finish;
            let server = MockServer::start(response).await;
            let error = collect(
                provider(name, &server.endpoint, Some(KEY.into())).as_ref(),
                name,
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind, kind);
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
        let body = if name == "anthropic" {
            format!("{ANTHROPIC}data: invalid\n\n")
        } else {
            format!("{OPENAI}data: invalid\n\n")
        };
        let server = MockServer::start(MockResponse::sse(&body)).await;
        assert!(
            collect(
                provider(name, &server.endpoint, Some(KEY.into())).as_ref(),
                name
            )
            .await
            .is_ok()
        );
        server.request.await.unwrap();
        server.task.await.unwrap().unwrap();
    }
}
#[tokio::test]
async fn missing_keys_invalid_endpoints_and_connection_failures() {
    for name in ["anthropic", "openai"] {
        let error = collect(provider(name, "http://localhost", None).as_ref(), name)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Configuration);
        let error = collect(provider(name, "", Some(KEY.into())).as_ref(), name)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Configuration);
        // Port zero cannot serve HTTP; this avoids a race with a reused listener port.
        let error = collect(
            provider(name, "http://127.0.0.1:0", Some(KEY.into())).as_ref(),
            name,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Network);
    }
}
#[tokio::test]
async fn streaming_error_event_is_redacted_and_classified() {
    let server=MockServer::start(MockResponse::sse(&format!("event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"overloaded_error\",\"message\":\"{KEY}\"}}}}\n\n"))).await;
    let error = collect(
        provider("anthropic", &server.endpoint, Some(KEY.into())).as_ref(),
        "anthropic",
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Api);
    assert!(!error.message.contains(KEY));
    assert!(error.retryable());
    server.request.await.unwrap();
    server.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn real_http_disconnect_after_text_preserves_delta_and_ends_with_disconnected() {
    for (name, body) in [
        (
            "anthropic",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
        ),
        (
            "openai",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\n",
        ),
    ] {
        for finish in [true, false] {
            let mut response = MockResponse::sse(body);
            response.finish = finish;
            let server = MockServer::start(response).await;
            let provider = provider(name, &server.endpoint, Some(KEY.into()));
            let mut stream = provider.request(request(name)).await.unwrap();
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                StreamEvent::TextDelta("partial".into())
            );
            assert_eq!(
                stream.next().await.unwrap().unwrap_err().kind,
                ErrorKind::Disconnected
            );
            assert!(stream.next().await.is_none());
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn generation_limits_come_from_settings_with_no_implicit_default() {
    let mut settings = common::settings("anthropic").providers.remove(0);
    settings.max_output_tokens = None;
    let provider = HttpProviderFactory
        .create(&settings, Some(KEY.into()))
        .unwrap();
    assert_eq!(
        collect(provider.as_ref(), "anthropic")
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Configuration
    );
    let server = MockServer::start(MockResponse::sse(OPENAI)).await;
    settings.id = "openai".into();
    settings.endpoint = Some(server.endpoint.clone());
    let provider = HttpProviderFactory
        .create(&settings, Some(KEY.into()))
        .unwrap();
    collect(provider.as_ref(), "openai").await.unwrap();
    let request = server.request.await.unwrap();
    server.task.await.unwrap().unwrap();
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("max_tokens").is_none());
}

#[tokio::test]
async fn http_and_stream_errors_extract_structured_codes_from_recorded_payloads() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/provider_errors.json")).unwrap();
    for fixture in fixtures {
        let name = fixture["provider"].as_str().unwrap();
        for status in [400, 200] {
            let payload = json!({"type":"error", "error":fixture["error"]}).to_string();
            let body = if status == 200 {
                format!("event: error\ndata: {payload}\n\n")
            } else {
                payload
            };
            let mut response = MockResponse::sse(&body);
            response.status = status;
            let server = MockServer::start(response).await;
            let error = collect(
                provider(name, &server.endpoint, Some(KEY.into())).as_ref(),
                name,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code.as_deref(), fixture["code"].as_str());
            assert_eq!(
                error.kind,
                if status == 200 {
                    ErrorKind::Api
                } else {
                    ErrorKind::Http {
                        status,
                        retry_after: None,
                    }
                }
            );
            assert_eq!(
                error.retryable(),
                status == 200 && fixture["retryable"].as_bool().unwrap()
            );
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn structured_error_codes_and_messages_both_redact_secrets() {
    for name in ["anthropic", "openai"] {
        for status in [400, 200] {
            let payload =
                json!({"type":"error", "error":{"code":KEY,"type":KEY,"message":KEY}}).to_string();
            let body = if status == 200 {
                format!("data: {payload}\n\n")
            } else {
                payload
            };
            let mut response = MockResponse::sse(&body);
            response.status = status;
            let server = MockServer::start(response).await;
            let error = collect(
                provider(name, &server.endpoint, Some(KEY.into())).as_ref(),
                name,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code.as_deref(), Some("[redacted]"));
            assert!(!format!("{error:?}").contains(KEY));
            assert_eq!(error.retryable(), status == 200);
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}
