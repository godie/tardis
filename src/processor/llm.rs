//! Real LLM [`Processor`] over an OpenAI-compatible chat endpoint.
//!
//! Mirrors [`crate::transcription::local_whisper`]: one blocking HTTP client
//! plus pure helpers for the request body, response parsing, status mapping
//! and key redaction, so the parts that can break are unit-tested without
//! touching the network. `reqwest::blocking` is already a dependency and
//! AGENTS.md forbids adding an async runtime.
//!
//! One request is in flight at a time, from the capture thread. A cold 7B
//! model therefore stalls the loop for up to
//! [`crate::config::LLM_TIMEOUT_SECS`]; the streaming design that removes
//! that stall needs an explicit go-ahead (see `docs/core-pipeline-plan.md`
//! §4.2).
//!
//! The API key lives in [`Secret`]: it is read from `TARDIS_LLM_API_KEY`,
//! never serialized, and redacted from every error string before the error
//! reaches a log line.

use std::fmt;
use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::config;
use crate::processor::processor::{Output, OutputKind, ProcessMode, Processor};
use crate::processor::prompt;

/// OpenAI-compatible chat endpoint suffix. `base_url` (e.g. Ollama's
/// `http://localhost:11434/v1`) + this string is the request URL.
const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";

/// Largest response body accepted, in bytes.
///
/// A translation of one utterance is a few KB. The cap is what stops a
/// misconfigured endpoint (or a proxy error page) from buffering megabytes
/// in the capture thread and dumping them into a log; the request is
/// rejected instead of parsed.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// How much of a failed response body reaches diagnostics, in characters.
pub const MAX_ERROR_EXCERPT_CHARS: usize = 300;

/// Derived implementation of [`Processor`] that asks a model.
///
/// Deliberately non-deterministic: the trait's purity rule applies to the
/// mock. Tests never call [`LlmProcessor::process`], because that would
/// make `cargo test` depend on a running model.
#[derive(Debug)]
pub struct LlmProcessor {
    /// Base URL without the `/chat/completions` suffix.
    pub base_url: String,
    /// Model id sent as the `model` field.
    pub model: String,
    client: reqwest::blocking::Client,
    secret: Secret,
}

/// API key wrapper whose `Debug` / `Display` never reveal the value.
///
/// Empty is valid: a local Ollama needs no key. Constructing one from an
/// environment variable is the only intended use; nothing serializes it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// `true` when no key was configured, which is the normal local case.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The raw value, for the one call site that sets the auth header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Pure helper: remove the key from a message before it is logged.
///
/// HTTP libraries echo request headers in some errors. An empty key leaves
/// the message untouched (there is nothing to redact, and replacing `""`
/// would corrupt every string).
pub fn redact_secret(message: &str, secret: &Secret) -> String {
    if secret.is_empty() {
        message.to_string()
    } else {
        message.replace(secret.expose(), "***")
    }
}

/// Pure helper: join a server base URL and the chat path, without `//`.
pub fn build_chat_completions_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}{}", trimmed, CHAT_COMPLETIONS_PATH)
}

/// Pure helper: the actionable half of a non-2xx response.
///
/// Kept as a table so the CLI and the UI say the same thing for the same
/// status instead of each inventing its own sentence.
pub fn http_status_reason(status: u16) -> &'static str {
    match status {
        401 | 403 => "authentication failed: check TARDIS_LLM_API_KEY",
        404 => "endpoint or model not found: check the base URL and the model id",
        408 => "the LLM server timed out",
        429 => "rate limited: retry later or use a smaller/local model",
        500..=599 => "the LLM server failed the request",
        _ => "the LLM server rejected the request",
    }
}

/// Pure helper: shorten a body for a log line, on a character boundary.
///
/// Error bodies are attacker-adjacent input: a proxy page or a model
/// echoing the prompt can be arbitrarily long. Diagnostics keep a prefix and
/// the character count, never the whole body.
pub fn truncate_for_log(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let total = trimmed.chars().count();
    if total <= max_chars {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(max_chars).collect();
    format!("{prefix}… ({total} chars total)")
}

/// Pure helper: the OpenAI-compatible chat request body.
pub fn build_chat_request_body(
    model: &str,
    system_prompt: &str,
    user_text: &str,
    max_tokens: u32,
    temperature: f32,
) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_text },
        ],
        "max_tokens": max_tokens,
        "temperature": temperature,
    })
}

/// Pure helper: `choices[0].message.content` from a chat completion.
///
/// An empty string is a valid (if useless) reply and is returned as `""`;
/// the caller decides that there is nothing to say. Missing or non-string
/// `content` is an error, never a silent empty transcript.
pub fn parse_chat_completion(json: &Value) -> Result<String> {
    let content = json
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"));
    match content {
        Some(Value::String(text)) => Ok(text.trim().to_string()),
        Some(_) => Err(anyhow!(
            "chat completion has a 'content' field but it is not a string"
        )),
        None => Err(anyhow!(
            "chat completion is missing choices[0].message.content"
        )),
    }
}

impl LlmProcessor {
    /// Build the client for one endpoint. `base_url` must not include the
    /// `/chat/completions` suffix; [`build_chat_completions_url`] adds it.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        secret: Secret,
    ) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(config::LLM_TIMEOUT_SECS))
            .build()
            .context("building the LLM HTTP client")?;
        Ok(Self {
            base_url: base_url.into(),
            model: model.into(),
            client,
            secret,
        })
    }

    /// Defaults from [`crate::config::LLM_*`]; key from
    /// `TARDIS_LLM_API_KEY` (empty is valid).
    pub fn from_env() -> Result<Self> {
        let secret = Secret::new(std::env::var("TARDIS_LLM_API_KEY").unwrap_or_default());
        Self::new(config::LLM_BASE_URL, config::LLM_MODEL, secret)
    }

    /// One chat round-trip. Errors are already redacted.
    fn complete(&self, system_prompt: &str, user_text: &str) -> Result<String> {
        let url = build_chat_completions_url(&self.base_url);
        let body = build_chat_request_body(
            &self.model,
            system_prompt,
            user_text,
            config::LLM_MAX_TOKENS,
            config::LLM_TEMPERATURE,
        );
        let mut request = self.client.post(&url).json(&body);
        if !self.secret.is_empty() {
            request = request.bearer_auth(self.secret.expose());
        }
        let response = request.send().map_err(|e| {
            anyhow!(
                "{}",
                redact_secret(
                    &format!(
                        "POST {url} failed: {e}. Is the LLM server running at {}?",
                        self.base_url
                    ),
                    &self.secret
                )
            )
        })?;
        let status = response.status();
        let mut body = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| {
                anyhow!(
                    "{}",
                    redact_secret(
                        &format!("reading the response from {url} failed: {e}"),
                        &self.secret
                    )
                )
            })?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!(
                "response from {url} exceeded {} bytes; refusing to buffer it",
                MAX_RESPONSE_BYTES
            ));
        }
        let body_text = String::from_utf8_lossy(&body);
        if !status.is_success() {
            return Err(anyhow!(
                "{}",
                redact_secret(
                    &format!(
                        "{url} returned {}: {} ({})",
                        status.as_u16(),
                        truncate_for_log(&body_text, MAX_ERROR_EXCERPT_CHARS),
                        http_status_reason(status.as_u16())
                    ),
                    &self.secret
                )
            ));
        }
        let json: Value = serde_json::from_str(&body_text)
            .with_context(|| format!("parsing the chat completion from {url}"))?;
        parse_chat_completion(&json)
    }
}

impl Processor for LlmProcessor {
    /// `Ok(None)` only for empty input (nothing to translate). Transport,
    /// auth, malformed and empty completions are all `Err`, so the runner
    /// can surface them instead of reporting them as silence.
    fn process(&self, text: &str, mode: &ProcessMode) -> Result<Option<Output>> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        let ProcessMode::Translate {
            source_language,
            target_language,
        } = mode;
        let system_prompt = prompt::build_translate_system_prompt(source_language, target_language);
        let user_text = prompt::wrap_transcript(text);
        let rendered = self.complete(&system_prompt, &user_text)?;
        if rendered.is_empty() {
            return Err(anyhow!(
                "model '{}' returned an empty reply (no translation produced)",
                self.model
            ));
        }
        Ok(Some(Output {
            kind: OutputKind::Translate,
            source_text: text.to_string(),
            text: rendered,
            source_language: source_language.clone(),
            target_language: target_language.clone(),
            is_final: true,
        }))
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_url_trims_trailing_slashes() {
        assert_eq!(
            build_chat_completions_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            build_chat_completions_url("http://localhost:11434/v1///"),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn request_body_carries_model_both_messages_and_limits() {
        let body = build_chat_request_body("m", "system here", "user here", 64, 0.1);
        assert_eq!(body["model"], "m");
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "system here");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "user here");
    }

    #[test]
    fn parse_chat_completion_returns_trimmed_content() {
        let json = json!({
            "choices": [{ "message": { "role": "assistant", "content": "  hola  " } }]
        });
        assert_eq!(parse_chat_completion(&json).unwrap(), "hola");
    }

    #[test]
    fn parse_chat_completion_empty_content_is_ok() {
        let json = json!({ "choices": [{ "message": { "content": "" } }] });
        assert_eq!(parse_chat_completion(&json).unwrap(), "");
    }

    #[test]
    fn parse_chat_completion_without_choices_is_an_error() {
        let err = parse_chat_completion(&json!({ "error": "nope" })).unwrap_err();
        assert!(err.to_string().contains("choices[0].message.content"));
    }

    #[test]
    fn parse_chat_completion_non_string_content_is_an_error() {
        let json = json!({ "choices": [{ "message": { "content": null } }] });
        let err = parse_chat_completion(&json).unwrap_err();
        assert!(err.to_string().contains("not a string"));
    }

    #[test]
    fn http_status_reasons_cover_the_actionable_cases() {
        assert!(http_status_reason(401).contains("TARDIS_LLM_API_KEY"));
        assert!(http_status_reason(403).contains("TARDIS_LLM_API_KEY"));
        assert!(http_status_reason(404).contains("base URL"));
        assert!(http_status_reason(429).contains("rate limited"));
        assert!(http_status_reason(500).contains("server failed"));
        assert!(http_status_reason(503).contains("server failed"));
        assert!(http_status_reason(418).contains("rejected"));
    }

    #[test]
    fn redact_secret_removes_the_key_from_a_message() {
        let secret = Secret::new("sk-live-123");
        let message = "POST /chat/completions failed: header authorization: Bearer sk-live-123";
        let redacted = redact_secret(message, &secret);
        assert!(!redacted.contains("sk-live-123"));
        assert!(redacted.contains("***"));
    }

    #[test]
    fn redact_secret_with_an_empty_key_leaves_the_message_intact() {
        let message = "connection refused";
        assert_eq!(redact_secret(message, &Secret::default()), message);
    }

    #[test]
    fn secret_debug_and_display_never_print_the_value() {
        let secret = Secret::new("sk-live-123");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert!(!format!("{secret:?}").contains("sk-live-123"));
    }

    #[test]
    fn secret_exposes_its_value_for_the_auth_header_only() {
        let secret = Secret::new("sk-live-123");
        assert_eq!(secret.expose(), "sk-live-123");
        assert!(!secret.is_empty());
        assert!(Secret::default().is_empty());
    }

    #[test]
    fn truncate_for_log_keeps_short_text_intact() {
        assert_eq!(truncate_for_log("  boom  ", 10), "boom");
    }

    #[test]
    fn truncate_for_log_cuts_long_text_and_reports_the_total() {
        let long = "x".repeat(MAX_ERROR_EXCERPT_CHARS + 50);
        let out = truncate_for_log(&long, MAX_ERROR_EXCERPT_CHARS);
        assert!(out.starts_with(&"x".repeat(MAX_ERROR_EXCERPT_CHARS)));
        assert!(out.ends_with("(350 chars total)"), "got: {out}");
    }

    #[test]
    fn truncate_for_log_never_splits_a_multibyte_character() {
        let out = truncate_for_log(&"á".repeat(10), 3);
        assert_eq!(out, "ááá… (10 chars total)");
    }
}
