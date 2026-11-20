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
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::config;

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
/// not a JSON string; an empty string is a *valid* transcript
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

/// Pure helper: how long one transcription request may take for the
/// given utterance bound.
///
/// The longest audio the runner can send is `max_utterance_ms`, and the
/// documented CPU model runs at most
/// [`config::LOCAL_WHISPER_CPU_SPEED_FACTOR`] times realtime. The
/// margin covers the WAV upload, a cold model, and response formatting.
/// A 15 s bound therefore allows 105 s and the 60 s maximum allows
/// 330 s, instead of reqwest's 30 s default.
pub fn transcription_timeout_for_utterance(max_utterance_ms: u64) -> Duration {
    Duration::from_millis(max_utterance_ms) * config::LOCAL_WHISPER_CPU_SPEED_FACTOR
        + Duration::from_secs(config::LOCAL_WHISPER_TIMEOUT_MARGIN_SECS)
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
/// All fields are `pub` so the CLI can print them as diagnostics
/// without going through getters; the constructor
/// [`LocalWhisperClient::new`] is the recommended entry point and takes
/// the [`crate::config::LOCAL_WHISPER_*`] values plus the caller's
/// utterance bound.
pub struct LocalWhisperClient {
    /// Base URL of the local server, e.g. `http://localhost:8000`.
    /// Must **not** include the `/v1/audio/transcriptions` suffix;
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
    /// Longest utterance the caller can send, in milliseconds. Kept
    /// alongside the timeout so the timeout error can state what the
    /// deadline covers.
    pub max_utterance_ms: u64,
    /// Per-request deadline, derived from `max_utterance_ms` by
    /// [`transcription_timeout_for_utterance`].
    pub timeout: Duration,
}

impl LocalWhisperClient {
    /// Construct from `(base_url, model, language, max_utterance_ms)`.
    /// The `Option` dimension lets callers skip the language form field
    /// for auto-detection when desired. The utterance bound only
    /// decides the request timeout.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        language: Option<String>,
        max_utterance_ms: u64,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            language,
            max_utterance_ms,
            timeout: transcription_timeout_for_utterance(max_utterance_ms),
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
    /// * Request slower than the derived timeout → error naming the
    ///   deadline and the utterance bound it covers
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
            .timeout(self.timeout)
            .build()
            .context("building reqwest blocking client")?;

        let response = client.post(url).multipart(form).send().map_err(|err| {
            if err.is_timeout() {
                // A timeout is not the same failure as an unreachable
                // container: name the deadline and the bound it covers
                // instead of pointing at Docker.
                anyhow!(
                    "local faster-whisper server at {} did not answer within {} s; \
                     the deadline covers a {} ms utterance at the documented {}x \
                     worst-case CPU speed plus a {} s margin",
                    url,
                    self.timeout.as_secs(),
                    self.max_utterance_ms,
                    config::LOCAL_WHISPER_CPU_SPEED_FACTOR,
                    config::LOCAL_WHISPER_TIMEOUT_MARGIN_SECS,
                )
            } else {
                anyhow::Error::new(err).context(format!(
                    "sending POST request to local faster-whisper server at {} \
                     — is the Docker container running? \
                     (try: docker compose -f docker/faster-whisper/docker-compose.yml up)",
                    url
                ))
            }
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

    // ---- transcription_timeout_for_utterance --------------------------

    #[test]
    fn timeout_covers_the_default_utterance_bound() {
        // 15 s of audio at the documented worst case (5x realtime) plus
        // the margin, well past reqwest's 30 s default.
        assert_eq!(
            transcription_timeout_for_utterance(config::DEFAULT_MAX_UTTERANCE_MS),
            Duration::from_secs(15 * 5 + 30)
        );
    }

    #[test]
    fn timeout_covers_the_largest_accepted_utterance() {
        // MAX_MAX_UTTERANCE_MS is the 60 s cap the config accepts.
        assert_eq!(
            transcription_timeout_for_utterance(crate::app::config::MAX_MAX_UTTERANCE_MS),
            Duration::from_secs(60 * 5 + 30)
        );
    }

    #[test]
    fn timeout_scales_linearly_with_the_bound() {
        assert_eq!(
            transcription_timeout_for_utterance(2_000) - transcription_timeout_for_utterance(1_000),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn constructor_derives_the_timeout_from_the_bound() {
        let client = LocalWhisperClient::new("http://localhost:8000", "base", None, 2_000);
        assert_eq!(client.max_utterance_ms, 2_000);
        assert_eq!(client.timeout, transcription_timeout_for_utterance(2_000));
    }

    // ---- delayed local server ----------------------------------------

    #[test]
    fn a_stalled_local_server_hits_the_request_timeout() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::thread;
        use std::time::Instant;

        // A loopback listener that accepts the request, reads part of it,
        // and then never answers. No external network is involved.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let address = listener.local_addr().expect("listener address");
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0u8; 1024];
                        let _ = stream.read(&mut buffer);
                        thread::sleep(Duration::from_millis(400));
                        return;
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        });

        let wav_path =
            std::env::temp_dir().join(format!("tardis_whisper_timeout_{}.wav", std::process::id()));
        std::fs::write(&wav_path, b"RIFF").expect("write stub WAV");
        let wav_arg = wav_path.to_string_lossy().into_owned();

        let mut client = LocalWhisperClient::new(
            format!("http://{address}"),
            "base",
            None,
            config::DEFAULT_MAX_UTTERANCE_MS,
        );
        // The real derivation is covered above; shorten it so this test
        // proves enforcement without waiting two minutes.
        client.timeout = Duration::from_millis(100);

        let started = Instant::now();
        let err = client
            .transcribe_wav_file(&wav_arg)
            .expect_err("a stalled server must produce a timeout error");
        let elapsed = started.elapsed();
        // The client owns no handle on the file after the request returns.
        let _ = std::fs::remove_file(&wav_path);

        assert!(
            elapsed < Duration::from_secs(2),
            "timeout should fire promptly, took {elapsed:?}"
        );
        assert!(
            err.to_string().contains("did not answer within"),
            "error should name the timeout, got: {err}"
        );

        server.join().expect("server thread");
    }
}
