//! Internal capability-free OpenAI-compatible HTTP client
//! (`project/instrukt/adr/0002-openai-compatible-direct-http-client`).

use std::fmt;
use std::time::{Duration, Instant};

use instrukt_core::config::{Finding, LlmAdmission, LlmEndpointConfig};
use instrukt_outside::http::{
    BodyReadError, ClientConfig, Judgment, build_client, judge_reqwest_error, judge_response,
    read_bounded_body, redact_url,
};
use instrukt_outside::{Attempted, CallerWait, Gate, Key, Settings, Tried, Wait};
use serde::{Deserialize, Serialize};

/// Default completion token bound for capability-free calls.
pub const DEFAULT_MAX_TOKENS: u32 = 1024;

/// Minimum allowed completion token bound.
pub const MIN_MAX_TOKENS: u32 = 1;

/// Maximum allowed completion token bound.
pub const MAX_MAX_TOKENS: u32 = 4096;

/// Maximum `200 OK` response body size buffered while streaming.
pub const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

/// TCP/TLS connect timeout for each HTTP try.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a capability-free call or screening was refused before any HTTP request
/// was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryRefusal {
    /// Global `llm.screening` is absent from `~/.instrukt/instrukt.yaml`.
    Unconfigured,
    /// Global `llm` / `llm.screening` has configuration or secret findings.
    InvalidEndpoint(Vec<Finding>),
    /// Prompt or input text is empty or whitespace-only (zero words).
    EmptyInput,
    /// Requested `max_tokens` fell outside [`MIN_MAX_TOKENS`]..=[`MAX_MAX_TOKENS`].
    InvalidMaxTokens { given: u32, min: u32, max: u32 },
}

impl EntryRefusal {
    pub(crate) fn kind_label(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::InvalidEndpoint(_) => "invalid_endpoint",
            Self::EmptyInput => "empty_input",
            Self::InvalidMaxTokens { .. } => "invalid_max_tokens",
        }
    }
}

impl fmt::Display for EntryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unconfigured => write!(
                f,
                "llm.screening is not configured; set it with `instrukt config patch --global`"
            ),
            Self::InvalidEndpoint(findings) => {
                if findings.is_empty() {
                    return write!(
                        f,
                        "llm.screening configuration is invalid; run `instrukt config validate --global`"
                    );
                }
                let details: Vec<String> = findings
                    .iter()
                    .map(|finding| {
                        if finding.key.is_empty() {
                            finding.message.clone()
                        } else {
                            format!("{}: {}", finding.key, finding.message)
                        }
                    })
                    .collect();
                let joined = details.join("; ");
                let needs_validate_door = findings
                    .iter()
                    .any(|finding| !finding.message.contains("instrukt secrets set"));
                if needs_validate_door {
                    write!(
                        f,
                        "llm.screening is withheld ({joined}); run `instrukt config validate --global`"
                    )
                } else {
                    write!(f, "llm.screening is withheld ({joined})")
                }
            }
            Self::EmptyInput => write!(f, "input text is empty or whitespace-only"),
            Self::InvalidMaxTokens { given, min, max } => write!(
                f,
                "max_tokens {given} is outside the allowed range {min}..={max}"
            ),
        }
    }
}

/// Non-defect refusal returned by a capability-free call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallRefusal {
    /// Prerequisite or input check failed before any HTTP call.
    Entry(EntryRefusal),
    /// The model stopped for a reason other than `"stop"` (e.g. `"length"`).
    CutOff { finish_reason: Option<String> },
    /// Endpoint returned HTTP `400`, `413`, or `422` (request cannot be served).
    EndpointRejected { status: u16 },
    /// `200 OK` response exceeded [`MAX_RESPONSE_BODY_BYTES`], was invalid JSON, or lacked non-empty content.
    MalformedAnswer { detail: &'static str },
}

impl CallRefusal {
    fn kind_label(&self) -> &'static str {
        match self {
            Self::Entry(entry) => entry.kind_label(),
            Self::CutOff { .. } => "cut_off",
            Self::EndpointRejected { .. } => "endpoint_rejected",
            Self::MalformedAnswer { .. } => "malformed_answer",
        }
    }
}

impl fmt::Display for CallRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry(entry) => write!(f, "{entry}"),
            Self::CutOff {
                finish_reason: Some(reason),
            } => write!(
                f,
                "model did not finish its answer (finish_reason={reason})"
            ),
            Self::CutOff {
                finish_reason: None,
            } => write!(f, "model answer omitted finish_reason"),
            Self::EndpointRejected { status } => {
                write!(f, "endpoint rejected request (HTTP {status})")
            }
            Self::MalformedAnswer { detail } => write!(f, "malformed endpoint answer: {detail}"),
        }
    }
}

/// Outcome of one capability-free call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    /// Complete bounded text answer (`finish_reason == "stop"`).
    Answered(String),
    /// Refused at entry or post-call without retrying.
    Refused(CallRefusal),
    /// Endpoint was unavailable beyond the caller's wait bound.
    Waiting(Wait),
    /// Cancelled via `stop` before completing.
    Stopped,
}

/// Non-transient client, protocol, or runtime defect (never retried).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallDefect {
    #[error("failed to build HTTP client: {0}")]
    HttpClient(String),
    #[error("HTTP transport error talking to {url}: {cause}")]
    Transport { url: String, cause: String },
    #[error("unexpected HTTP status {status} from {url}")]
    UnexpectedStatus { url: String, status: u16 },
    #[error("async runtime error: {0}")]
    Runtime(String),
    #[error("cryptographic random source unavailable")]
    RandomSource,
}

/// Internal wait budget supporting unbounded waits, untimed single-try, and a
/// shared monotonic deadline across multi-step operations (such as two-check
/// screening).
#[derive(Debug, Clone, Copy)]
pub(crate) enum WaitBudget {
    UntilReturns,
    SingleTry,
    Until(Option<Instant>),
}

/// Minimal capability-free chat-completions request: `tools` is structurally
/// unrepresentable and no earlier conversation or `response_format` is sent.
#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: [UserMessage<'a>; 1],
    max_completion_tokens: u32,
}

#[derive(Serialize)]
struct UserMessage<'a> {
    role: &'static str,
    content: &'a str,
}

/// Lenient OpenAI-compatible chat-completions response envelope.
#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    message: Option<ChoiceMessage>,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

/// Synchronous wrapper around [`call_async`] using the workspace-shared Tokio
/// runtime bridge.
pub fn call(
    admission: &LlmAdmission,
    prompt: &str,
    max_tokens: Option<u32>,
    gate: &Gate,
    caller_wait: CallerWait,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<CallOutcome, CallDefect> {
    instrukt_outside::runtime::block_on(call_async(
        admission,
        prompt,
        max_tokens,
        gate,
        caller_wait,
        stop,
    ))
    .map_err(|error| CallDefect::Runtime(error.to_string()))?
}

/// Sends one single-message, tool-less OpenAI-compatible chat-completions call
/// through `gate`, returning one bounded text answer, a refusal, a wait, or a
/// defect.
pub async fn call_async(
    admission: &LlmAdmission,
    prompt: &str,
    max_tokens: Option<u32>,
    gate: &Gate,
    caller_wait: CallerWait,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<CallOutcome, CallDefect> {
    let budget = match caller_wait {
        CallerWait::UntilReturns => WaitBudget::UntilReturns,
        CallerWait::Within(duration) if duration.is_zero() => WaitBudget::SingleTry,
        CallerWait::Within(duration) => WaitBudget::Until(gate.monotonic().checked_add(duration)),
    };
    call_with_budget(admission, prompt, max_tokens, gate, budget, stop).await
}

pub(crate) async fn call_with_budget(
    admission: &LlmAdmission,
    prompt: &str,
    max_tokens: Option<u32>,
    gate: &Gate,
    budget: WaitBudget,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<CallOutcome, CallDefect> {
    let started = Instant::now();
    let endpoint = match check_entry(admission, prompt, max_tokens) {
        Ok(endpoint) => endpoint,
        Err(refusal) => {
            tracing::warn!(
                target: "instrukt_screening",
                event = "capability_free_call_refused",
                kind = refusal.kind_label(),
                duration_ms = started.elapsed().as_millis() as u64,
            );
            return Ok(CallOutcome::Refused(CallRefusal::Entry(refusal)));
        }
    };
    let max_tokens = max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    if stop() {
        return Ok(CallOutcome::Stopped);
    }

    let request_url = completions_url(&endpoint.url);
    let safe_url = redact_url(&request_url);
    let client = build_client(&ClientConfig {
        connect_timeout: CONNECT_TIMEOUT,
        request_timeout: endpoint.timeout,
        follow_redirects: false,
        ..ClientConfig::default()
    })
    .map_err(|error| CallDefect::HttpClient(error.without_url().to_string()))?;

    let key = Key::new("screening", [host_port(&endpoint.url)]).item(&endpoint.model);
    let settings = Settings::default();
    let body = ChatCompletionRequest {
        model: &endpoint.model,
        messages: [UserMessage {
            role: "user",
            content: prompt,
        }],
        max_completion_tokens: max_tokens,
    };

    let try_once = || {
        let client = &client;
        let request_url = &request_url;
        let safe_url = &safe_url;
        let body = &body;
        let api_key = endpoint.api_key.as_ref();
        async move {
            let mut request = client.post(request_url.as_str()).json(body);
            if let Some(secret) = api_key {
                request = request.bearer_auth(secret.expose());
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    let scrubbed = error.without_url();
                    let defect = CallDefect::Transport {
                        url: safe_url.clone(),
                        cause: scrubbed.to_string(),
                    };
                    return match judge_reqwest_error(&scrubbed, &settings) {
                        Judgment::Unavailable(unavailable) => {
                            Tried::Unavailable(unavailable, defect)
                        }
                        Judgment::Defect | Judgment::Success => Tried::Defect(defect),
                    };
                }
            };
            // Stamp header receipt time and judge status/headers BEFORE reading
            // any response body (`project/instrukt/spec/domain/screening`).
            let received_at = gate.now();
            let status = response.status().as_u16();
            if status == 200 {
                let bytes = match read_bounded_body(response, MAX_RESPONSE_BODY_BYTES).await {
                    Ok(bytes) => bytes,
                    Err(BodyReadError::TooLarge { .. }) => {
                        return Tried::Done(CallOutcome::Refused(CallRefusal::MalformedAnswer {
                            detail: "response body exceeded maximum allowed size",
                        }));
                    }
                    Err(BodyReadError::Transport(error)) => {
                        let defect = CallDefect::Transport {
                            url: safe_url.clone(),
                            cause: error.to_string(),
                        };
                        return match judge_reqwest_error(&error, &settings) {
                            Judgment::Unavailable(unavailable) => {
                                Tried::Unavailable(unavailable, defect)
                            }
                            Judgment::Defect | Judgment::Success => Tried::Defect(defect),
                        };
                    }
                };
                return Tried::Done(parse_completion_body(&bytes));
            }
            if matches!(status, 400 | 413 | 422) {
                return Tried::Done(CallOutcome::Refused(CallRefusal::EndpointRejected {
                    status,
                }));
            }
            if status < 400 {
                return Tried::Defect(CallDefect::UnexpectedStatus {
                    url: safe_url.clone(),
                    status,
                });
            }
            let judgment = judge_response(status, response.headers(), received_at, None, &settings);
            let defect = CallDefect::UnexpectedStatus {
                url: safe_url.clone(),
                status,
            };
            match judgment {
                Judgment::Unavailable(unavailable) => Tried::Unavailable(unavailable, defect),
                Judgment::Defect | Judgment::Success => Tried::Defect(defect),
            }
        }
    };

    // Emits one privacy-safe boundary diagnostic if this external call is
    // interrupted (via future drop on SIGINT/SIGTERM or via `stop()`), while
    // emitting nothing extra on ordinary completion.
    struct CallCompletionGuard<'a> {
        safe_url: &'a str,
        model: &'a str,
        started: Instant,
        completed: bool,
    }

    impl Drop for CallCompletionGuard<'_> {
        fn drop(&mut self) {
            if !self.completed {
                tracing::warn!(
                    target: "instrukt_screening",
                    event = "capability_free_call_interrupted",
                    endpoint = %self.safe_url,
                    model = %self.model,
                    duration_ms = self.started.elapsed().as_millis() as u64,
                );
            }
        }
    }

    let mut guard = CallCompletionGuard {
        safe_url: &safe_url,
        model: &endpoint.model,
        started,
        completed: false,
    };

    let attempted = match budget {
        WaitBudget::UntilReturns => {
            gate.call_async(&key, &settings, CallerWait::UntilReturns, stop, try_once)
                .await
        }
        WaitBudget::SingleTry => {
            gate.call_async(
                &key,
                &settings,
                CallerWait::Within(Duration::ZERO),
                stop,
                try_once,
            )
            .await
        }
        WaitBudget::Until(deadline) => {
            gate.call_async_until(&key, &settings, deadline, stop, try_once)
                .await
        }
    };

    // Re-check `stop` before emitting `Answered` or `Refused` so cancellation
    // never yields a late result; dropping `guard` records the interruption.
    if stop() {
        return Ok(CallOutcome::Stopped);
    }

    guard.completed = true;
    let duration_ms = started.elapsed().as_millis() as u64;
    match attempted {
        Attempted::Done(CallOutcome::Answered(answer)) => {
            tracing::debug!(
                target: "instrukt_screening",
                event = "capability_free_call_completed",
                endpoint = %safe_url,
                model = %endpoint.model,
                duration_ms,
            );
            Ok(CallOutcome::Answered(answer))
        }
        Attempted::Done(CallOutcome::Refused(refusal)) => {
            tracing::warn!(
                target: "instrukt_screening",
                event = "capability_free_call_refused",
                endpoint = %safe_url,
                model = %endpoint.model,
                kind = refusal.kind_label(),
                duration_ms,
            );
            Ok(CallOutcome::Refused(refusal))
        }
        Attempted::Done(other) => Ok(other),
        Attempted::Wait { wait, .. } => Ok(CallOutcome::Waiting(wait)),
        Attempted::Defect(defect) => {
            tracing::error!(
                target: "instrukt_screening",
                event = "capability_free_call_failed",
                endpoint = %safe_url,
                model = %endpoint.model,
                error = %defect,
                duration_ms,
            );
            Err(defect)
        }
    }
}

pub(crate) fn check_admission(
    admission: &LlmAdmission,
) -> Result<&LlmEndpointConfig, EntryRefusal> {
    match admission {
        LlmAdmission::Unconfigured => Err(EntryRefusal::Unconfigured),
        LlmAdmission::Withheld(findings) => Err(EntryRefusal::InvalidEndpoint(findings.clone())),
        LlmAdmission::Admitted(endpoint) => Ok(endpoint),
    }
}

fn check_entry<'a>(
    admission: &'a LlmAdmission,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<&'a LlmEndpointConfig, EntryRefusal> {
    if prompt.split_whitespace().next().is_none() {
        return Err(EntryRefusal::EmptyInput);
    }
    let endpoint = check_admission(admission)?;
    let requested = max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    if !(MIN_MAX_TOKENS..=MAX_MAX_TOKENS).contains(&requested) {
        return Err(EntryRefusal::InvalidMaxTokens {
            given: requested,
            min: MIN_MAX_TOKENS,
            max: MAX_MAX_TOKENS,
        });
    }
    Ok(endpoint)
}

fn completions_url(base: &url::Url) -> String {
    format!("{}/chat/completions", base.as_str().trim_end_matches('/'))
}

fn host_port(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

fn parse_completion_body(bytes: &[u8]) -> CallOutcome {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return CallOutcome::Refused(CallRefusal::MalformedAnswer {
            detail: "response body is not valid UTF-8",
        });
    };
    let Ok(parsed) = serde_json::from_str::<ChatCompletionResponse>(text) else {
        return CallOutcome::Refused(CallRefusal::MalformedAnswer {
            detail: "response body is not valid chat-completions JSON",
        });
    };
    let Some(choice) = parsed.choices.into_iter().next() else {
        return CallOutcome::Refused(CallRefusal::MalformedAnswer {
            detail: "response contained no choices",
        });
    };
    if choice.finish_reason.as_deref() != Some("stop") {
        return CallOutcome::Refused(CallRefusal::CutOff {
            finish_reason: choice.finish_reason,
        });
    }
    let Some(content) = choice.message.and_then(|msg| msg.content) else {
        return CallOutcome::Refused(CallRefusal::MalformedAnswer {
            detail: "response choice omitted message content",
        });
    };
    if content.trim().is_empty() {
        return CallOutcome::Refused(CallRefusal::MalformedAnswer {
            detail: "response choice contained empty content",
        });
    }
    CallOutcome::Answered(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::config::SecretValue;
    use instrukt_outside::Report;
    use jiff::Timestamp;
    use std::sync::{Arc, Mutex};

    fn test_gate() -> Gate {
        let now: Timestamp = "2026-11-28T10:00:00Z".parse().unwrap();
        let mono = Instant::now();
        Gate::new(
            Arc::new(move || now),
            Arc::new(move || mono),
            Report::to(|_| {}),
        )
    }

    fn admitted(port: u16, api_key: Option<&str>) -> LlmAdmission {
        LlmAdmission::Admitted(Box::new(LlmEndpointConfig {
            url: url::Url::parse(&format!("http://127.0.0.1:{port}/v1")).unwrap(),
            model: "plain-model".into(),
            api_key: api_key.map(|k| SecretValue::new(k.into())),
            timeout: Duration::from_secs(10),
        }))
    }

    type StubResponse = (u16, Vec<(&'static str, &'static str)>, Vec<u8>);
    type SeenRequest = (String, Option<String>, serde_json::Value);
    type SeenLog = Arc<Mutex<Vec<SeenRequest>>>;

    fn spawn_stub(responses: Vec<StubResponse>) -> (u16, SeenLog, std::thread::JoinHandle<()>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = Arc::clone(&seen);
        let handle = std::thread::spawn(move || {
            for (status, headers, body) in responses {
                let mut req = server.recv().unwrap();
                let url = req.url().to_owned();
                let auth = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.as_str().to_owned());
                let mut raw = String::new();
                req.as_reader().read_to_string(&mut raw).unwrap();
                let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
                seen_clone.lock().unwrap().push((url, auth, parsed));
                let mut resp = tiny_http::Response::from_data(body).with_status_code(status);
                for (name, value) in headers {
                    resp = resp.with_header(
                        tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap(),
                    );
                }
                req.respond(resp).unwrap();
            }
        });
        (port, seen, handle)
    }

    #[test]
    fn capability_free_call_omits_tools_sends_single_user_message_and_handles_stop() {
        let ok_body = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        })
        .to_string()
        .into_bytes();
        let (port, seen, handle) = spawn_stub(vec![(200, vec![], ok_body)]);
        let gate = test_gate();
        let out = crate::call(
            &admitted(port, Some("sk-test-secret")),
            "hello world",
            Some(256),
            &gate,
            CallerWait::Within(Duration::ZERO),
            &|| false,
        )
        .unwrap();
        handle.join().unwrap();
        assert!(matches!(out, CallOutcome::Answered(ref answer) if !answer.is_empty()));
        let reqs = seen.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].0, "/v1/chat/completions");
        assert_eq!(reqs[0].1.as_deref(), Some("Bearer sk-test-secret"));
        assert!(reqs[0].2.get("tools").is_none());
        assert!(reqs[0].2.get("response_format").is_none());
        assert_eq!(reqs[0].2["max_completion_tokens"], 256);
        assert_eq!(reqs[0].2["messages"].as_array().unwrap().len(), 1);
        assert_eq!(reqs[0].2["messages"][0]["role"], "user");
        assert_eq!(reqs[0].2["messages"][0]["content"], "hello world");
    }

    #[test]
    fn check_entry_refuses_unconfigured_admission() {
        assert_eq!(
            check_entry(&LlmAdmission::Unconfigured, "hello", None),
            Err(EntryRefusal::Unconfigured)
        );
    }

    #[test]
    fn check_entry_refuses_empty_prompt() {
        assert_eq!(
            check_entry(&admitted(1, None), "   \n\t ", None),
            Err(EntryRefusal::EmptyInput)
        );
    }

    #[test]
    fn check_entry_refuses_out_of_range_max_tokens() {
        let given = MAX_MAX_TOKENS + 1;
        assert_eq!(
            check_entry(&admitted(1, None), "hello", Some(given)),
            Err(EntryRefusal::InvalidMaxTokens {
                given,
                min: MIN_MAX_TOKENS,
                max: MAX_MAX_TOKENS,
            })
        );
    }
}
