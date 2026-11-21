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

/// First local transcription provider in this crate.
///
/// Talks to a self-hosted faster-whisper (or compatible) HTTP server
/// exposing the OpenAI `/v1/audio/transcriptions` endpoint.
///
/// Implements [`crate::transcription::LocalTranscriptionProvider`] so
/// future providers (`whisper.cpp` binary, cloud APIs, etc.) can be
/// substituted at the call site without altering CLI code.
///
/// The three configuration fields are `pub` so the CLI can print them
/// as diagnostics without going through getters; the constructor
/// [`LocalWhisperClient::new`] is the recommended entry point and
/// matches the field-for-field parameters of
/// [`crate::config::LOCAL_WHISPER_*`]. The HTTP client is private and
/// built once: a live session sends one request per utterance, and
/// rebuilding the client each time would throw away the connection
/// pool between them.
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
    /// Reused across [`LocalWhisperClient::transcribe_wav_file`] calls so
    /// a long session keeps its connections alive.
    client: reqwest::blocking::Client,
}

impl LocalWhisperClient {
    /// Construct from `(base_url, model, language)`. The `Option`
    /// dimension lets callers skip the language form field for
    /// auto-detection when desired.
    ///
    /// Fallible because the HTTP client is built here, once per
    /// provider. Nothing is sent over the network: a missing Docker
    /// container is still a `transcribe_wav_file` error, not a
    /// construction one.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        language: Option<String>,
    ) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .context("building the faster-whisper HTTP client")?;
        Ok(Self {
            base_url: base_url.into(),
            model: model.into(),
            language,
            client,
        })
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

        let response = self
            .client
            .post(url)
            .multipart(form)
            .send()
            .with_context(|| {
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
    use std::io::{Read, Write};

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

    // ---- new ----------------------------------------------------------

    #[test]
    fn new_builds_the_client_and_keeps_the_config() {
        // Building an HTTP client sends nothing: the Docker container is
        // only reached from `transcribe_wav_file`.
        let client = LocalWhisperClient::new("http://localhost:8000", "base", Some("es".into()))
            .expect("building the HTTP client must succeed");
        assert_eq!(client.base_url, "http://localhost:8000");
        assert_eq!(client.model, "base");
        assert_eq!(client.language.as_deref(), Some("es"));
    }

    // ---- client reuse --------------------------------------------------

    // A loopback stub, not Docker: two requests served over one connection.
    // The assertions below fail if a per-call client comes back, because
    // the second request would arrive on a second connection.

    /// One HTTP/1.1 response body the stub sends for every request.
    const STUB_BODY: &[u8] = br#"{"text": "stub transcript"}"#;

    /// Serve `requests` requests, reusing the connection whenever the
    /// client does, and return how many connections it took.
    fn serve_keep_alive(
        listener: std::net::TcpListener,
        requests: usize,
    ) -> std::io::Result<usize> {
        let mut connections = 0;
        let mut served = 0;
        while served < requests {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            connections += 1;
            while served < requests {
                if read_request(&mut stream)?.is_none() {
                    break;
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    STUB_BODY.len()
                )?;
                stream.write_all(STUB_BODY)?;
                served += 1;
            }
        }
        Ok(connections)
    }

    /// Consume one request (header block + declared body) so the next read
    /// starts at the next request. `Ok(None)` means the client closed.
    fn read_request(stream: &mut std::net::TcpStream) -> std::io::Result<Option<()>> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let body_start = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(None);
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let needed = body_start + content_length(&buf[..body_start])?;
        while buf.len() < needed {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(None);
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(Some(()))
    }

    /// `Content-Length` from a header block, `0` when absent.
    fn content_length(headers: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(headers);
        for line in text.lines() {
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                return value.trim().parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "bad content-length")
                });
            }
        }
        Ok(0)
    }

    /// A temp file the stub accepts as the upload; its bytes are ignored.
    fn temp_wav(test_name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after the epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tardis-local-whisper-{test_name}-{}-{nanos}.wav",
            std::process::id()
        ));
        std::fs::write(&path, b"RIFF").expect("write temp WAV");
        path
    }

    #[test]
    fn two_transcriptions_reuse_one_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback stub");
        let address = listener.local_addr().expect("stub address");
        let server = std::thread::spawn(move || serve_keep_alive(listener, 2));

        let wav = temp_wav("two_transcriptions_reuse_one_connection");
        let wav_arg = wav.to_string_lossy().to_string();
        let client = LocalWhisperClient::new(format!("http://{address}"), "base", None)
            .expect("build the HTTP client");

        assert_eq!(
            client.transcribe_wav_file(&wav_arg).expect("first request"),
            "stub transcript"
        );
        assert_eq!(
            client
                .transcribe_wav_file(&wav_arg)
                .expect("second request"),
            "stub transcript"
        );

        let connections = server.join().expect("stub thread").expect("stub IO");
        std::fs::remove_file(&wav).expect("remove temp WAV");
        assert_eq!(
            connections, 1,
            "both requests must share the provider's one HTTP client"
        );
    }
}
