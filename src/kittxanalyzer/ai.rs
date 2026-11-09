//! Non-streaming HTTPS requests to the selected AI service.
//!
//! [`analyze`] blocks, so the desktop UI invokes it in a background task. Keys
//! are read at invocation time and are never included in returned errors. There
//! are no automatic retries, redirects, tool execution, or conversation history.

use reqwest::{
    blocking::Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use std::{fmt, io::Read, time::Duration};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_OUTPUT_TOKENS: u32 = 8192;

/// Providers offered by the UI. Each uses its own endpoint and credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiService {
    Anthropic,
    OpenAi,
    OpenRouter,
}

impl AiService {
    /// Display order of the provider combobox; there is no default selection.
    pub const ALL: [Self; 3] = [Self::Anthropic, Self::OpenAi, Self::OpenRouter];

    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
            Self::OpenRouter => "openrouter.ai",
        }
    }

    pub fn api_key_env(self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::OpenAi => "OPENAI_API_KEY",
            Self::OpenRouter => "OPENROUTER_API_KEY",
        }
    }

    /// Optional environment variable overriding the provider's default model.
    /// OpenRouter overrides must identify a specific `:free` model.
    pub fn model_env(self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_MODEL",
            Self::OpenAi => "OPENAI_MODEL",
            Self::OpenRouter => "OPENROUTER_MODEL",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-sonnet-4-6",
            Self::OpenAi => "gpt-5-mini",
            Self::OpenRouter => "nvidia/nemotron-3-ultra-550b-a55b:free",
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/messages",
            Self::OpenAi => "https://api.openai.com/v1/responses",
            Self::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
        }
    }
}

/// Visible assistant text. Provider reasoning, wire JSON and the input prompt are
/// not part of the report. An interrupted/token-limited answer retains its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisReport {
    pub text: String,
    /// True when the service stopped before completing the answer.
    pub incomplete: bool,
}

/// Errors safe to display in the UI. Raw provider errors may echo credentials or
/// recording evidence, so neither their bodies nor HTTP client errors are exposed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisError {
    MissingApiKey(&'static str),
    InvalidEnvironment(&'static str),
    OpenRouterFreeModelRequired,
    EmptyPrompt,
    ClientSetup,
    Timeout,
    Connection,
    Http(u16),
    Provider,
    InvalidResponse,
    EmptyResponse,
    ResponseTooLarge,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey(name) => write!(
                f,
                "Set {name} in the environment used to launch KittxAnalyzer, then try again."
            ),
            Self::InvalidEnvironment(name) => write!(f, "{name} has an invalid value."),
            Self::OpenRouterFreeModelRequired => f.write_str("OPENROUTER_MODEL must name a model ending in :free. Unset it to use the default free model."),
            Self::EmptyPrompt => f.write_str("The selected evidence produced an empty prompt."),
            Self::ClientSetup => f.write_str("Cannot initialize the HTTPS client."),
            Self::Timeout => f.write_str("The AI request timed out. You can try Analyze again."),
            Self::Connection => f.write_str("Cannot communicate with the AI service. Check the network connection."),
            Self::Http(status) => {
                let advice = match status {
                    401 | 403 => "Check the selected service's API key and access permissions.",
                    402 => "Check the selected service's account credit.",
                    408 | 504 => "The service timed out. You can try Analyze again.",
                    413 => "Select fewer recording items and try again.",
                    429 => "The rate limit or account quota was reached. Check usage before trying again.",
                    500..=599 => "The service is unavailable. Try again later.",
                    300..=399 => "The endpoint redirected the request; it was not followed.",
                    _ => "Check model availability, account access and the size of the selected evidence.",
                };
                write!(f, "The AI service returned HTTP {status}. {advice}")
            }
            Self::Provider => f.write_str("The AI service returned an error. Check model availability and account usage."),
            Self::InvalidResponse => f.write_str("The AI service returned an unexpected response format."),
            Self::EmptyResponse => f.write_str("The AI service returned no report text. Check the model configuration or try again."),
            Self::ResponseTooLarge => f.write_str("The AI response exceeded the 2 MiB response limit."),
        }
    }
}

impl std::error::Error for AnalysisError {}

// No Debug implementation: this structure contains authentication headers.
struct RequestConfig {
    service: AiService,
    headers: HeaderMap,
    model: String,
}

impl RequestConfig {
    fn from_environment(
        service: AiService,
        read: impl Fn(&str) -> Result<Option<String>, ()>,
    ) -> Result<Self, AnalysisError> {
        let read_value = |name| read(name).map_err(|()| AnalysisError::InvalidEnvironment(name));
        let key_name = service.api_key_env();
        let key = read_value(key_name)?
            .filter(|key| !key.trim().is_empty())
            .ok_or(AnalysisError::MissingApiKey(key_name))?;
        let auth = match service {
            AiService::Anthropic => key.trim().to_owned(),
            _ => format!("Bearer {}", key.trim()),
        };
        let mut auth = HeaderValue::from_str(&auth)
            .map_err(|_| AnalysisError::InvalidEnvironment(key_name))?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        if service == AiService::Anthropic {
            headers.insert("x-api-key", auth);
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            if let Some(workspace) =
                read_value("ANTHROPIC_WORKSPACE_ID")?.filter(|value| !value.trim().is_empty())
            {
                let value = HeaderValue::from_str(workspace.trim())
                    .map_err(|_| AnalysisError::InvalidEnvironment("ANTHROPIC_WORKSPACE_ID"))?;
                headers.insert("anthropic-workspace-id", value);
            }
        } else {
            headers.insert(AUTHORIZATION, auth);
        }
        let model = read_value(service.model_env())?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| service.default_model().into());
        if model.len() > 200 || model.chars().any(char::is_control) {
            return Err(AnalysisError::InvalidEnvironment(service.model_env()));
        }
        let model = model.trim();
        if service == AiService::OpenRouter && !model.ends_with(":free") {
            return Err(AnalysisError::OpenRouterFreeModelRequired);
        }
        Ok(Self {
            service,
            headers,
            model: model.into(),
        })
    }

    fn body(&self, prompt: &str) -> Value {
        match self.service {
            AiService::OpenAi => json!({
                "model": self.model,
                "input": prompt,
                "max_output_tokens": MAX_OUTPUT_TOKENS,
                "store": false,
                "stream": false,
            }),
            AiService::Anthropic => json!({
                "model": self.model,
                "messages": [{"role": "user", "content": prompt}],
                "max_tokens": MAX_OUTPUT_TOKENS,
                "stream": false,
            }),
            AiService::OpenRouter => json!({
                "model": self.model,
                "messages": [{"role": "user", "content": prompt}],
                "max_tokens": MAX_OUTPUT_TOKENS,
                "provider": {"max_price": {"prompt": 0, "completion": 0, "request": 0}},
                "stream": false,
            }),
        }
    }
}

/// Send one prompt using the selected provider's environment configuration.
///
/// This blocking operation has a 15-second connection timeout and a 180-second
/// total request timeout (including reading the response). It performs one POST
/// without automatic retries. Only selected evidence should be passed as input.
/// Responses are bounded to 2 MiB and 8,192 output tokens are requested (which
/// can include reasoning tokens). Keys and prompts are never logged here.
/// OpenRouter accepts only `:free` model IDs and caps prompt, completion, and
/// per-request provider pricing at zero; unavailable free routes return an error.
pub fn analyze(service: AiService, prompt: &str) -> Result<AnalysisReport, AnalysisError> {
    if prompt.trim().is_empty() {
        return Err(AnalysisError::EmptyPrompt);
    }
    let config = RequestConfig::from_environment(service, |name| match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(()),
    })?;
    send(&config, prompt, service.endpoint(), REQUEST_TIMEOUT)
}

fn send(
    config: &RequestConfig,
    prompt: &str,
    endpoint: &str,
    timeout: Duration,
) -> Result<AnalysisReport, AnalysisError> {
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT.min(timeout))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .user_agent(concat!("KittxAnalyzer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| AnalysisError::ClientSetup)?;
    let response = client
        .post(endpoint)
        .headers(config.headers.clone())
        // Also set the underlying async request's total deadline, including its
        // response body, rather than restarting a timeout on each body read.
        .timeout(timeout)
        .json(&config.body(prompt))
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                AnalysisError::Timeout
            } else {
                AnalysisError::Connection
            }
        })?;
    if !response.status().is_success() {
        return Err(AnalysisError::Http(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES)
    {
        return Err(AnalysisError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut
                || error
                    .get_ref()
                    .and_then(|error| error.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout)
            {
                AnalysisError::Timeout
            } else {
                AnalysisError::Connection
            }
        })?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(AnalysisError::ResponseTooLarge);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| AnalysisError::InvalidResponse)?;
    parse_report(config.service, &value)
}

fn parse_report(service: AiService, value: &Value) -> Result<AnalysisReport, AnalysisError> {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(AnalysisError::Provider);
    }
    let mut parts = Vec::new();
    let incomplete = match service {
        AiService::OpenAi => {
            let incomplete = match value["status"].as_str() {
                Some("completed") => false,
                Some("incomplete") => true,
                Some("failed" | "cancelled") => return Err(AnalysisError::Provider),
                _ => return Err(AnalysisError::InvalidResponse),
            };
            for item in value["output"]
                .as_array()
                .ok_or(AnalysisError::InvalidResponse)?
            {
                if item["type"] == "message" && item["role"] == "assistant" {
                    collect_blocks(&item["content"], "output_text", &mut parts)?;
                }
            }
            incomplete
        }
        AiService::Anthropic => {
            if value["type"] != "message" || value["role"] != "assistant" {
                return Err(AnalysisError::InvalidResponse);
            }
            collect_blocks(&value["content"], "text", &mut parts)?;
            match value["stop_reason"].as_str() {
                Some("end_turn" | "stop_sequence" | "refusal") => false,
                Some(
                    "max_tokens" | "model_context_window_exceeded" | "pause_turn" | "tool_use",
                ) => true,
                _ => return Err(AnalysisError::InvalidResponse),
            }
        }
        AiService::OpenRouter => {
            let choice = value["choices"]
                .as_array()
                .and_then(|choices| choices.first())
                .ok_or(AnalysisError::InvalidResponse)?;
            if choice.get("error").is_some_and(|error| !error.is_null()) {
                return Err(AnalysisError::Provider);
            }
            let message = &choice["message"];
            if message["role"] != "assistant" {
                return Err(AnalysisError::InvalidResponse);
            }
            if let Some(text) = message["content"].as_str() {
                parts.push(text);
            } else if message["content"].is_array() {
                collect_blocks(&message["content"], "text", &mut parts)?;
            }
            if let Some(refusal) = message["refusal"].as_str() {
                parts.push(refusal);
            }
            match choice["finish_reason"].as_str() {
                Some("stop") => false,
                Some("length" | "content_filter" | "tool_calls") => true,
                Some("error") => return Err(AnalysisError::Provider),
                _ => return Err(AnalysisError::InvalidResponse),
            }
        }
    };
    let text = parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() {
        return Err(AnalysisError::EmptyResponse);
    }
    Ok(AnalysisReport { text, incomplete })
}

fn collect_blocks<'a>(
    content: &'a Value,
    text_type: &str,
    parts: &mut Vec<&'a str>,
) -> Result<(), AnalysisError> {
    for block in content.as_array().ok_or(AnalysisError::InvalidResponse)? {
        let text = if block["type"] == text_type {
            Some(
                block["text"]
                    .as_str()
                    .ok_or(AnalysisError::InvalidResponse)?,
            )
        } else if block["type"] == "refusal" {
            Some(
                block["refusal"]
                    .as_str()
                    .ok_or(AnalysisError::InvalidResponse)?,
            )
        } else {
            None
        };
        if let Some(text) = text {
            parts.push(text);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
