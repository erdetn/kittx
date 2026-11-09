use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::Instant,
};

const KEY: &str = "test-secret-not-a-real-api-key";
const PROMPT: &str = "Analyze selected evidence only: speed=45 km/h at 1000 us; DTC P0123.";

fn config(service: AiService) -> RequestConfig {
    RequestConfig::from_environment(service, |name| {
        Ok((name == service.api_key_env()).then(|| KEY.into()))
    })
    .unwrap()
}

fn answer(service: AiService) -> Value {
    match service {
        AiService::OpenAi => json!({
            "status":"completed", "output":[
                {"type":"reasoning","summary":[{"text":"Hidden reasoning"}]},
                {"type":"message","role":"assistant","content":[
                    {"type":"output_text","text":"Review P0123 and its observed times."}
                ]}
            ]
        }),
        AiService::Anthropic => json!({
            "type":"message","role":"assistant","stop_reason":"end_turn", "content":[
                {"type":"thinking","thinking":"Hidden reasoning"},
                {"type":"text","text":"Review P0123 and its observed times."}
            ]
        }),
        AiService::OpenRouter => json!({"choices":[{
            "finish_reason":"stop", "message":{
                "role":"assistant","content":"Review P0123 and its observed times.",
                "reasoning":"Hidden reasoning"
            }
        }]}),
    }
}

#[derive(Debug)]
struct CapturedRequest {
    line: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

struct Server {
    url: String,
    request: Receiver<CapturedRequest>,
    worker: JoinHandle<()>,
}

impl Server {
    fn new(reply: impl FnOnce(TcpStream) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/analyze", listener.local_addr().unwrap());
        let (tx, request) = mpsc::channel();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "client did not connect");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            // macOS can inherit the listener's nonblocking mode on accept.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let end = loop {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0, "request ended before its headers");
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(offset) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    break offset + 4;
                }
                assert!(bytes.len() < 64 * 1024);
            };
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let mut lines = head.lines();
            let line = lines.next().unwrap().to_owned();
            let headers: BTreeMap<_, _> = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                .collect();
            let length: usize = headers["content-length"].parse().unwrap();
            assert!(length < 4 * 1024 * 1024);
            while bytes.len() < end + length {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0, "request ended before its body");
                bytes.extend_from_slice(&buffer[..n]);
            }
            tx.send(CapturedRequest {
                line,
                headers,
                body: serde_json::from_slice(&bytes[end..end + length]).unwrap(),
            })
            .unwrap();
            reply(socket);
        });
        Self {
            url,
            request,
            worker,
        }
    }

    fn json(value: Value) -> Self {
        Self::new(move |mut socket| {
            let body = serde_json::to_vec(&value).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            let _ = socket.write_all(&body);
        })
    }

    fn finish(self) -> CapturedRequest {
        self.worker.join().unwrap();
        self.request.recv_timeout(Duration::from_secs(1)).unwrap()
    }
}

#[test]
fn providers_send_their_own_headers_models_and_request_formats() {
    assert_eq!(
        AiService::ALL,
        [
            AiService::Anthropic,
            AiService::OpenAi,
            AiService::OpenRouter
        ]
    );
    let expected = [
        (
            AiService::Anthropic,
            "Anthropic",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_MODEL",
            "claude-sonnet-4-6",
            "https://api.anthropic.com/v1/messages",
        ),
        (
            AiService::OpenAi,
            "OpenAI",
            "OPENAI_API_KEY",
            "OPENAI_MODEL",
            "gpt-5-mini",
            "https://api.openai.com/v1/responses",
        ),
        (
            AiService::OpenRouter,
            "openrouter.ai",
            "OPENROUTER_API_KEY",
            "OPENROUTER_MODEL",
            "nvidia/nemotron-3-ultra-550b-a55b:free",
            "https://openrouter.ai/api/v1/chat/completions",
        ),
    ];
    for (service, label, key_var, model_var, model, endpoint) in expected {
        assert_eq!(service.label(), label);
        assert_eq!(service.api_key_env(), key_var);
        assert_eq!(service.model_env(), model_var);
        assert_eq!(service.default_model(), model);
        assert_eq!(service.endpoint(), endpoint);
        let config = config(service);
        let server = Server::json(answer(service));
        let report = send(&config, PROMPT, &server.url, Duration::from_secs(2)).unwrap();
        assert_eq!(
            report,
            AnalysisReport {
                text: "Review P0123 and its observed times.".into(),
                incomplete: false,
            }
        );
        let request = server.finish();
        assert_eq!(request.line, "POST /analyze HTTP/1.1");
        assert_eq!(request.headers["content-type"], "application/json");
        assert!(request.headers["user-agent"].starts_with("KittxAnalyzer/"));
        assert_eq!(request.body["model"], model);
        assert_eq!(request.body["stream"], false);
        if service == AiService::Anthropic {
            assert_eq!(request.headers["x-api-key"], KEY);
            assert_eq!(request.headers["anthropic-version"], "2023-06-01");
            assert!(!request.headers.contains_key("authorization"));
            assert!(config.headers["x-api-key"].is_sensitive());
        } else {
            assert_eq!(request.headers["authorization"], format!("Bearer {KEY}"));
            assert!(!request.headers.contains_key("x-api-key"));
            assert!(!request.headers.contains_key("anthropic-version"));
            assert!(config.headers["authorization"].is_sensitive());
        }
        let expected_body = match service {
            AiService::OpenAi => {
                json!({"model":model,"input":PROMPT,"max_output_tokens":8192,"store":false,"stream":false})
            }
            AiService::Anthropic => {
                json!({"model":model,"messages":[{"role":"user","content":PROMPT}],"max_tokens":8192,"stream":false})
            }
            AiService::OpenRouter => {
                json!({"model":model,"messages":[{"role":"user","content":PROMPT}],"max_tokens":8192,"provider":{"max_price":{"prompt":0,"completion":0,"request":0}},"stream":false})
            }
        };
        assert_eq!(request.body, expected_body);
    }
}

#[test]
fn providers_send_large_measurement_prompts_without_truncation() {
    let samples: Vec<_> = (0..4097)
        .map(|index| json!({"elapsed_us": index * 1000, "utc_us": 123456789 + index * 1000, "value": index}))
        .collect();
    let prompt = format!(
        "Analyze the complete selected measurements.\n```json\n{}\n```",
        json!({"measurement_series": [{"label": "Engine RPM", "unit": "rpm", "samples": samples}]})
    );
    for service in AiService::ALL {
        let server = Server::json(answer(service));
        send(
            &config(service),
            &prompt,
            &server.url,
            Duration::from_secs(2),
        )
        .unwrap();
        let request = server.finish();
        let received = if service == AiService::OpenAi {
            &request.body["input"]
        } else {
            &request.body["messages"][0]["content"]
        };
        assert_eq!(received.as_str(), Some(prompt.as_str()));
    }
}

#[test]
fn environment_overrides_and_workspace_are_scoped_to_the_selected_provider() {
    for service in AiService::ALL {
        let model = if service == AiService::OpenRouter {
            "custom/model-version:free"
        } else {
            "custom/model-version"
        };
        let config = RequestConfig::from_environment(service, |name| {
            if name == service.api_key_env() {
                Ok(Some(format!("  {KEY}  ")))
            } else if name == service.model_env() {
                Ok(Some(format!("  {model}  ")))
            } else if name == "ANTHROPIC_WORKSPACE_ID" && service == AiService::Anthropic {
                Ok(Some("wrkspc_example".into()))
            } else {
                panic!("must not read another provider's environment: {name}")
            }
        })
        .unwrap();
        assert_eq!(config.body(PROMPT)["model"], model);
        assert_eq!(
            config.headers.contains_key("anthropic-workspace-id"),
            service == AiService::Anthropic
        );
        if service == AiService::Anthropic {
            assert_eq!(config.headers["anthropic-workspace-id"], "wrkspc_example");
        }
        let blank_model = RequestConfig::from_environment(service, |name| {
            Ok(Some(if name == service.api_key_env() {
                KEY.into()
            } else {
                "   ".into()
            }))
        })
        .unwrap();
        assert_eq!(blank_model.model, service.default_model());
        assert!(!blank_model.headers.contains_key("anthropic-workspace-id"));
    }
}

#[test]
fn openrouter_rejects_paid_or_automatic_overrides_and_keeps_free_overrides_at_zero_price() {
    for model in [
        "openai/gpt-5-mini",
        "nvidia/nemotron-3-ultra-550b-a55b",
        "openrouter/auto",
        "openrouter/free", // Random routing does not select a specific heavy model.
        "vendor/model:free:nitro",
    ] {
        let result = RequestConfig::from_environment(AiService::OpenRouter, |name| {
            Ok(Some(if name == "OPENROUTER_API_KEY" {
                KEY.into()
            } else {
                model.into()
            }))
        });
        assert!(matches!(
            result,
            Err(AnalysisError::OpenRouterFreeModelRequired)
        ));
    }
    let config = RequestConfig::from_environment(AiService::OpenRouter, |name| {
        Ok(Some(if name == "OPENROUTER_API_KEY" {
            KEY.into()
        } else {
            "  nvidia/nemotron-3-super-120b-a12b:free  ".into()
        }))
    })
    .unwrap();
    let server = Server::json(answer(AiService::OpenRouter));
    send(&config, PROMPT, &server.url, Duration::from_secs(2)).unwrap();
    let request = server.finish();
    assert_eq!(
        request.body["model"],
        "nvidia/nemotron-3-super-120b-a12b:free"
    );
    assert_eq!(
        request.body["provider"]["max_price"],
        json!({"prompt":0,"completion":0,"request":0})
    );
    assert!(request.body.get("models").is_none());
    assert!(request.body.get("route").is_none());
    assert!(
        AnalysisError::OpenRouterFreeModelRequired
            .to_string()
            .contains("OPENROUTER_MODEL")
    );
    assert!(
        AnalysisError::OpenRouterFreeModelRequired
            .to_string()
            .contains(":free")
    );
}

#[test]
fn configuration_rejects_missing_empty_nonunicode_and_invalid_values_without_exposing_them() {
    for service in AiService::ALL {
        for key in [None, Some(""), Some(" \t ")] {
            let result = RequestConfig::from_environment(service, |_| Ok(key.map(str::to_owned)));
            assert!(
                matches!(result, Err(AnalysisError::MissingApiKey(name)) if name == service.api_key_env())
            );
        }
        let result = RequestConfig::from_environment(service, |_| Err(()));
        assert!(
            matches!(result, Err(AnalysisError::InvalidEnvironment(name)) if name == service.api_key_env())
        );
        let result = RequestConfig::from_environment(service, |_| {
            Ok(Some(format!("{KEY}\r\ninjected: header")))
        });
        assert!(
            matches!(result, Err(AnalysisError::InvalidEnvironment(name)) if name == service.api_key_env())
        );
        for model in ["not\na-model".to_owned(), "x".repeat(201)] {
            let result = RequestConfig::from_environment(service, |name| {
                Ok((name == service.api_key_env())
                    .then(|| KEY.into())
                    .or_else(|| (name == service.model_env()).then(|| model.clone())))
            });
            assert!(
                matches!(result, Err(AnalysisError::InvalidEnvironment(name)) if name == service.model_env())
            );
        }
    }
    let result = RequestConfig::from_environment(AiService::Anthropic, |name| {
        Ok(Some(if name == "ANTHROPIC_WORKSPACE_ID" {
            "wrkspc_bad\r\ninjected".into()
        } else {
            KEY.into()
        }))
    });
    assert!(matches!(
        result,
        Err(AnalysisError::InvalidEnvironment("ANTHROPIC_WORKSPACE_ID"))
    ));
    assert_eq!(
        analyze(AiService::OpenAi, " \n\t "),
        Err(AnalysisError::EmptyPrompt)
    );
}

#[test]
fn openai_collects_only_visible_assistant_text_and_preserves_incomplete_answers_and_refusals() {
    let mut value = answer(AiService::OpenAi);
    value["output"].as_array_mut().unwrap().insert(
        0,
        json!({
            "type":"message","role":"user","content":[{"type":"output_text","text":PROMPT}]
        }),
    );
    value["output"].as_array_mut().unwrap().push(json!({
        "type":"message","role":"assistant","content":[
            {"type":"output_text","text":"Second paragraph."},
            {"type":"refusal","refusal":"Cannot conclude the fault's cause from this evidence."}
        ]
    }));
    value["status"] = json!("incomplete");
    value["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let report = parse_report(AiService::OpenAi, &value).unwrap();
    assert!(report.incomplete);
    assert_eq!(
        report.text,
        "Review P0123 and its observed times.\n\nSecond paragraph.\n\nCannot conclude the fault's cause from this evidence."
    );
    assert!(!report.text.contains("Hidden reasoning") && !report.text.contains(PROMPT));
    value["output"] = json!([{"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"Declined."}]}]);
    value["status"] = json!("completed");
    assert_eq!(
        parse_report(AiService::OpenAi, &value).unwrap().text,
        "Declined."
    );
}

#[test]
fn anthropic_text_blocks_exclude_thinking_and_mark_interrupted_generations() {
    let mut value = answer(AiService::Anthropic);
    value["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"text","text":"Check the circuit before replacing parts."}));
    let report = parse_report(AiService::Anthropic, &value).unwrap();
    assert_eq!(
        report.text,
        "Review P0123 and its observed times.\n\nCheck the circuit before replacing parts."
    );
    assert!(!report.incomplete);
    for reason in [
        "max_tokens",
        "model_context_window_exceeded",
        "pause_turn",
        "tool_use",
    ] {
        value["stop_reason"] = json!(reason);
        assert!(
            parse_report(AiService::Anthropic, &value)
                .unwrap()
                .incomplete
        );
    }
    for reason in ["stop_sequence", "refusal"] {
        value["stop_reason"] = json!(reason);
        assert!(
            !parse_report(AiService::Anthropic, &value)
                .unwrap()
                .incomplete
        );
    }
}

#[test]
fn openrouter_handles_text_blocks_refusals_and_finish_reasons_without_showing_reasoning() {
    let mut value = answer(AiService::OpenRouter);
    for reason in ["length", "content_filter", "tool_calls"] {
        value["choices"][0]["finish_reason"] = json!(reason);
        let report = parse_report(AiService::OpenRouter, &value).unwrap();
        assert!(report.incomplete);
        assert_eq!(report.text, "Review P0123 and its observed times.");
    }
    value["choices"][0]["finish_reason"] = json!("stop");
    value["choices"][0]["message"]["content"] = json!([
        {"type":"text","text":"First."}, {"type":"image_url","image_url":{"url":"ignored"}},
        {"type":"text","text":"Second."}
    ]);
    assert_eq!(
        parse_report(AiService::OpenRouter, &value).unwrap().text,
        "First.\n\nSecond."
    );
    value["choices"][0]["message"]["content"] = Value::Null;
    value["choices"][0]["message"]["refusal"] = json!("Declined.");
    assert_eq!(
        parse_report(AiService::OpenRouter, &value).unwrap().text,
        "Declined."
    );
    for change in [
        json!({"finish_reason":"error"}),
        json!({"error":{"message":KEY}}),
    ] {
        let mut failed = answer(AiService::OpenRouter);
        for (key, value) in change.as_object().unwrap() {
            failed["choices"][0][key] = value.clone();
        }
        assert_eq!(
            parse_report(AiService::OpenRouter, &failed),
            Err(AnalysisError::Provider)
        );
    }
}

#[test]
fn malformed_failed_empty_and_unexpected_provider_responses_are_errors() {
    for service in AiService::ALL {
        for invalid in [Value::Null, json!({}), json!([]), json!("not an answer")] {
            assert_eq!(
                parse_report(service, &invalid),
                Err(AnalysisError::InvalidResponse)
            );
        }
        assert_eq!(
            parse_report(
                service,
                &json!({"error":{"message":format!("{KEY} {PROMPT}")}})
            ),
            Err(AnalysisError::Provider)
        );
        let mut empty = answer(service);
        match service {
            AiService::OpenAi => empty["output"] = json!([]),
            AiService::Anthropic => empty["content"] = json!([]),
            AiService::OpenRouter => empty["choices"][0]["message"]["content"] = json!(" \n "),
        }
        assert_eq!(
            parse_report(service, &empty),
            Err(AnalysisError::EmptyResponse)
        );
        let mut unexpected = answer(service);
        match service {
            AiService::OpenAi => unexpected["status"] = json!("in_progress"),
            AiService::Anthropic => unexpected["stop_reason"] = Value::Null,
            AiService::OpenRouter => unexpected["choices"][0]["finish_reason"] = Value::Null,
        }
        assert_eq!(
            parse_report(service, &unexpected),
            Err(AnalysisError::InvalidResponse)
        );
    }
    for status in ["failed", "cancelled"] {
        let mut value = answer(AiService::OpenAi);
        value["status"] = json!(status);
        assert_eq!(
            parse_report(AiService::OpenAi, &value),
            Err(AnalysisError::Provider)
        );
    }
    let mut bad_block = answer(AiService::Anthropic);
    bad_block["content"] = json!([{"type":"text","text":9}]);
    assert_eq!(
        parse_report(AiService::Anthropic, &bad_block),
        Err(AnalysisError::InvalidResponse)
    );
    bad_block["content"] = Value::Null;
    assert_eq!(
        parse_report(AiService::Anthropic, &bad_block),
        Err(AnalysisError::InvalidResponse)
    );
    for service in [AiService::Anthropic, AiService::OpenRouter] {
        let mut value = answer(service);
        if service == AiService::Anthropic {
            value["role"] = json!("user");
        } else {
            value["choices"][0]["message"]["role"] = json!("user");
        }
        assert_eq!(
            parse_report(service, &value),
            Err(AnalysisError::InvalidResponse)
        );
    }
}

#[test]
fn http_errors_do_not_expose_provider_bodies_or_retry_requests() {
    for status in [400, 401, 402, 403, 408, 413, 429, 500, 503, 504] {
        let server = Server::new(move |mut socket| {
            let body = format!("{{\"error\":{{\"message\":\"{KEY} {PROMPT}\"}}}}");
            let _ = write!(
                socket,
                "HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        let error = send(
            &config(AiService::OpenAi),
            PROMPT,
            &server.url,
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert_eq!(error, AnalysisError::Http(status));
        assert!(error.to_string().contains(&format!("HTTP {status}")));
        assert!(!error.to_string().contains(KEY) && !error.to_string().contains(PROMPT));
        server.finish();
    }
}

#[test]
fn redirects_are_not_followed_and_do_not_forward_credentials() {
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    let target = format!("http://{}/trap", trap.local_addr().unwrap());
    let server = Server::new(move |mut socket| {
        write!(socket, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    assert_eq!(
        send(
            &config(AiService::OpenRouter),
            PROMPT,
            &server.url,
            Duration::from_secs(2)
        ),
        Err(AnalysisError::Http(307))
    );
    server.finish();
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn malformed_bodies_provider_errors_and_response_limits_are_handled() {
    let bad_json = Server::new(|mut socket| {
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot-json!"
        )
        .unwrap();
    });
    assert_eq!(
        send(
            &config(AiService::OpenAi),
            PROMPT,
            &bad_json.url,
            Duration::from_secs(2)
        ),
        Err(AnalysisError::InvalidResponse)
    );
    bad_json.finish();
    let provider_error = Server::json(json!({"error":{"message":KEY}}));
    assert_eq!(
        send(
            &config(AiService::OpenAi),
            PROMPT,
            &provider_error.url,
            Duration::from_secs(2)
        ),
        Err(AnalysisError::Provider)
    );
    provider_error.finish();
    for with_length in [true, false] {
        let server = Server::new(move |mut socket| {
            let _ = write!(socket, "HTTP/1.1 200 OK\r\nConnection: close\r\n");
            if with_length {
                let _ = write!(socket, "Content-Length: {}\r\n", MAX_RESPONSE_BYTES + 1);
            }
            let _ = write!(socket, "\r\n");
            let _ = socket.write_all(&vec![b' '; MAX_RESPONSE_BYTES as usize + 1]);
        });
        let result = send(
            &config(AiService::OpenAi),
            PROMPT,
            &server.url,
            Duration::from_secs(2),
        );
        server.finish();
        assert_eq!(
            result,
            Err(AnalysisError::ResponseTooLarge),
            "with Content-Length: {with_length}"
        );
    }
    let truncated = Server::new(|mut socket| {
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
    });
    assert_eq!(
        send(
            &config(AiService::OpenAi),
            PROMPT,
            &truncated.url,
            Duration::from_secs(2)
        ),
        Err(AnalysisError::Connection)
    );
    truncated.finish();
}

#[test]
fn connection_failure_and_total_deadlines_cover_headers_and_slow_response_bodies() {
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    assert_eq!(
        send(
            &config(AiService::OpenAi),
            PROMPT,
            &format!("http://{address}/"),
            Duration::from_secs(1)
        ),
        Err(AnalysisError::Connection)
    );
    let timeout = Duration::from_millis(150);
    for slow_body in [false, true] {
        let server = Server::new(move |mut socket| {
            if slow_body {
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                for _ in 0..30 {
                    if socket.write_all(b" ").is_err() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            } else {
                thread::sleep(Duration::from_millis(350));
            }
        });
        let start = Instant::now();
        assert_eq!(
            send(&config(AiService::OpenAi), PROMPT, &server.url, timeout),
            Err(AnalysisError::Timeout)
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        server.finish();
    }
}

#[test]
fn errors_are_displayable_without_raw_credentials_requests_or_sources() {
    use std::error::Error;
    for error in [
        AnalysisError::MissingApiKey("OPENAI_API_KEY"),
        AnalysisError::InvalidEnvironment("OPENAI_MODEL"),
        AnalysisError::OpenRouterFreeModelRequired,
        AnalysisError::EmptyPrompt,
        AnalysisError::ClientSetup,
        AnalysisError::Timeout,
        AnalysisError::Connection,
        AnalysisError::Http(429),
        AnalysisError::Provider,
        AnalysisError::InvalidResponse,
        AnalysisError::EmptyResponse,
        AnalysisError::ResponseTooLarge,
    ] {
        assert!(!error.to_string().is_empty());
        assert!(!format!("{error:?}").contains(KEY) && !error.to_string().contains(PROMPT));
        assert!(error.source().is_none());
    }
    assert!(
        AnalysisError::MissingApiKey("OPENAI_API_KEY")
            .to_string()
            .contains("OPENAI_API_KEY")
    );
}
