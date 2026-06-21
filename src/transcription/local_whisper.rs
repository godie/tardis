//! Local faster-whisper HTTP client + pure helpers.
//!
//! [`LocalWhisperClient`] POSTs a WAV file to the OpenAI-compatible
//! `/v1/audio/transcriptions` endpoint exposed by a self-hosted
//! [`fedirz/faster-whisper-server`] (or any same-shape HTTP server:
//! `whisper.cpp --openai-api`, OpenAI's own cloud API, etc.).
//!
//! This module is the **first real local transcription provider** in
//! the crate; future providers (`whisper.cpp` binary, cloud APIs,
//! etc.) implement [`crate::transcription::LocalTranscriptionProvider`]
//! and can be swapped in without changing CLI callers.
//!
//! [`fedirz/faster-whisper-server`]: https://github.com/fedirz/faster-whisper-server

use std::path::Path;

use anyhow::{Context, Result, anyhow};

/// OpenAI-compatible transcription endpoint suffix. Server base URL
/// + this string is what [`build_transcription_url`] joins.
const TRANSCRIPTION_PATH: &str = "/v1/audio/transcriptions";

/// Pure helper: build the canonical transcription URL from a server
/// base. Trims any number of trailing `/` so the result never
/// contains `//`, regardless of how the caller stored the base URL.
///
/// Used by [`LocalWhisperClient::transcribe_wav_file`] internally;
/// exposed publicly so future providers (or CLI diagnostics) can
/// reuse the same URL shape.
pub fn build_transcription_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}{}", trimmed, TRANSCRIPTION_PATH)
}

/// Pure helper: extract the `text` field from an OpenAI-style
/// transcription response. Returns an error if `text` is missing or
/// not a JSON string — an empty string is a *valid* transcript
/// (silence-only chunk), not an error.
pub fn extract_text_from_response_json(json: &serde_json::Value) -> Result<String> {
    match json.get("text") {
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(anyhow!(
            "response JSON has a 'text' field but it is not a string"
        )),
        None => Err(anyhow!(
            "response JSON is missing the required 'text' field"
        )),
    }
}

/// First local transcription provider in this crate.
///
/// Talks to a self-hosted faster-whisper (or compatible) HTTP server
/// exposing the OpenAI `/v1/audio/transcriptions` endpoint.
///
/// Implements [`crate::transcription::LocalTranscriptionProvider`] so
/// future providers (`whisper.cpp` binary, cloud APIs, etc.) can be
/// substituted at the call site without altering CLI code.
///
/// All three fields are `pub` so the CLI can print them as
/// diagnostics without going through getters; the constructor
/// [`LocalWhisperClient::new`] is the recommended entry point and
/// matches the field-for-field parameters of
/// [`crate::config::LOCAL_WHISPER_*`].
pub struct LocalWhisperClient {
    /// Base URL of the local server, e.g. `http://localhost:8000`.
    /// Must **not** include the `/v1/audio/transcriptions` suffix —
    /// [`build_transcription_url`] appends it.
    pub base_url: String,
    /// faster-whisper model id passed as the multipart `model`
    /// field. Valid IDs: `tiny`, `base`, `small`, `medium`,
    /// `large-v2`, `large-v3`. Must match the value the server is
    /// configured to load (see `WHISPER_MODEL` in
    /// `docker/faster-whisper/docker-compose.yml`).
    pub model: String,
    /// Optional BCP-47-ish language code (e.g. `en`, `es`). `Some`
    /// adds a `language` form field; `None` omits it so the server
    /// runs auto-detection.
    pub language: Option<String>,
}

impl LocalWhisperClient {
    /// Construct from `(base_url, model, language)`. The `Option`
    /// dimension lets callers skip the language form field for
    /// auto-detection when desired.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            language,
        }
    }

    /// POST a WAV file on disk to `/v1/audio/transcriptions` and
    /// return the plaintext transcript.
    ///
    /// Error surfaces:
    ///
    /// * File missing on disk → `WAV file does not exist: <path>`
    /// * Connection refused / DNS failure → wrapped error mentioning
    ///   the local URL and pointing at the compose file
    /// * HTTP non-2xx → `server returned status <code>: <body>`
    /// * JSON missing/non-string `text` → see
    ///   [`extract_text_from_response_json`]
    pub fn transcribe_wav_file(&self, file_path: &str) -> Result<String> {
        let path = Path::new(file_path);
        if !path.is_file() {
            return Err(anyhow!("WAV file does not exist: {}", file_path));
        }

        let url = build_transcription_url(&self.base_url);
        let json = self.post_multipart(&url, path)?;
        extract_text_from_response_json(&json)
            .with_context(|| format!("parsing 'text' from response of {}", url))
    }

    /// Build the multipart body and POST it. Factored out so
    /// [`transcribe_wav_file`] stays small and the multipart payload
    /// shape is documented in one place.
    fn post_multipart(&self, url: &str, path: &Path) -> Result<serde_json::Value> {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "audio.wav".to_string());

        let file_part = reqwest::blocking::multipart::Part::file(path)
            .with_context(|| format!("opening WAV file for upload: {}", path.display()))?
            .file_name(file_name)
            .mime_str("audio/wav")
            .context("setting audio/wav mime on multipart file part")?;

        let mut form = reqwest::blocking::multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone());

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        let client = reqwest::blocking::Client::builder()
            .build()
            .context("building reqwest blocking client")?;

        let response = client.post(url).multipart(form).send().with_context(|| {
            format!(
                "sending POST request to local faster-whisper server at {} \
                     — is the Docker container running? \
                     (try: docker compose -f docker/faster-whisper/docker-compose.yml up)",
                url
            )
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .unwrap_or_else(|_| "<unreadable response body>".to_string());
            return Err(anyhow!(
                "local faster-whisper server at {} returned status {}: {}",
                url,
                status,
                body
            ));
        }

        response
            .json::<serde_json::Value>()
            .with_context(|| format!("parsing JSON response from {}", url))
    }
}

// ===== Trait binding =====================================================

impl crate::transcription::LocalTranscriptionProvider for LocalWhisperClient {
    fn name(&self) -> &'static str {
        "local-whisper (faster-whisper Docker HTTP, OpenAI-compatible)"
    }

    fn transcribe(&self, file_path: &str) -> Result<String> {
        // Trait method delegates to the inherent public method so the
        // concrete spec-mandated name (`transcribe_wav_file`) remains
        // the canonical call-site contract; the trait only adds
        // interchangeability for future providers.
        self.transcribe_wav_file(file_path)
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- build_transcription_url --------------------------------------

    #[test]
    fn build_url_without_trailing_slash() {
        assert_eq!(
            build_transcription_url("http://localhost:8000"),
            "http://localhost:8000/v1/audio/transcriptions"
        );
    }

    #[test]
    fn build_url_with_single_trailing_slash() {
        assert_eq!(
            build_transcription_url("http://localhost:8000/"),
            "http://localhost:8000/v1/audio/transcriptions"
        );
    }

    #[test]
    fn build_url_strips_multiple_trailing_slashes() {
        // Defensive: even if a caller passes "http://localhost:8000///",
        // the result must not contain a doubled "//v1/..." path segment.
        assert_eq!(
            build_transcription_url("http://localhost:8000///"),
            "http://localhost:8000/v1/audio/transcriptions"
        );
    }

    // ---- extract_text_from_response_json ------------------------------

    #[test]
    fn extract_text_from_valid_json() {
        let v = json!({"text": "hello world"});
        assert_eq!(extract_text_from_response_json(&v).unwrap(), "hello world");
    }

    #[test]
    fn extract_text_empty_string_is_ok() {
        // An empty transcript is a *valid* transcript (silence-only
        // chunk); not an error.
        let v = json!({"text": ""});
        assert_eq!(extract_text_from_response_json(&v).unwrap(), "");
    }

    #[test]
    fn extract_text_missing_field_returns_error() {
        let v = json!({"other": "x"});
        let err = extract_text_from_response_json(&v).unwrap_err();
        assert!(
            err.to_string().contains("missing"),
            "error message should mention missing field, got: {}",
            err
        );
    }

    #[test]
    fn extract_text_non_string_returns_error() {
        let v = json!({"text": 42});
        let err = extract_text_from_response_json(&v).unwrap_err();
        assert!(
            err.to_string().contains("not a string"),
            "error message should mention non-string, got: {}",
            err
        );
    }

    #[test]
    fn extract_text_null_returns_error() {
        let v = json!({"text": null});
        let err = extract_text_from_response_json(&v).unwrap_err();
        assert!(
            err.to_string().contains("not a string"),
            "error message should mention non-string, got: {}",
            err
        );
    }
}
