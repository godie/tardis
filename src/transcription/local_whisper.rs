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

/// Pure helper: how long one transcription request may take for an
/// utterance of `audio_ms` milliseconds.
///
/// The documented CPU model runs at most
/// [`config::LOCAL_WHISPER_CPU_SPEED_FACTOR`] times realtime. The
/// margin covers the WAV upload, a cold model, and response formatting.
/// A 15 s utterance therefore allows 105 s and a 60 s utterance allows
/// 330 s, instead of reqwest's 30 s default.
pub fn transcription_timeout_for_utterance(audio_ms: u64) -> Duration {
    Duration::from_millis(audio_ms) * config::LOCAL_WHISPER_CPU_SPEED_FACTOR
        + Duration::from_secs(config::LOCAL_WHISPER_TIMEOUT_MARGIN_SECS)
}

/// Read a WAV file's duration in milliseconds from its header.
///
/// The [`LocalWhisperClient`] derives each request's timeout from the
/// audio it is about to upload, so a long file gets a longer deadline
/// than a short live utterance.
fn wav_duration_ms(path: &Path) -> Result<u64> {
    let reader = hound::WavReader::open(path)
        .with_context(|| format!("opening WAV file {}", path.display()))?;
    let sample_rate = reader.spec().sample_rate;
    if sample_rate == 0 {
        return Err(anyhow!(
            "WAV file has a zero sample rate: {}",
            path.display()
        ));
    }
    Ok(u64::from(reader.duration()) * 1_000 / u64::from(sample_rate))
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
/// [`LocalWhisperClient::new`] is the recommended entry point and
/// matches the field-for-field parameters of
/// [`crate::config::LOCAL_WHISPER_*`].
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
    /// * Request slower than the derived timeout → error naming the
    ///   deadline and the audio length it covers
    /// * HTTP non-2xx → `server returned status <code>: <body>`
    /// * JSON missing/non-string `text` → see
    ///   [`extract_text_from_response_json`]
    pub fn transcribe_wav_file(&self, file_path: &str) -> Result<String> {
        let path = Path::new(file_path);
        if !path.is_file() {
            return Err(anyhow!("WAV file does not exist: {}", file_path));
        }

        let audio_ms = wav_duration_ms(path)?;
        let timeout = transcription_timeout_for_utterance(audio_ms);
        let url = build_transcription_url(&self.base_url);
        let json = self.post_multipart(&url, path, timeout, audio_ms)?;
        extract_text_from_response_json(&json)
            .with_context(|| format!("parsing 'text' from response of {}", url))
    }

    /// Build the multipart body and POST it. Factored out so
    /// [`transcribe_wav_file`] stays small and the multipart payload
    /// shape is documented in one place.
    fn post_multipart(
        &self,
        url: &str,
        path: &Path,
        timeout: Duration,
        audio_ms: u64,
    ) -> Result<serde_json::Value> {
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
            .timeout(timeout)
            .build()
            .context("building reqwest blocking client")?;

        let response = client.post(url).multipart(form).send().map_err(|err| {
            if err.is_timeout() {
                request_timeout_error(url, timeout, audio_ms)
            } else if err.is_connect() {
                // Docker is only the likely cause when the connection
                // itself failed. DNS, TLS and multipart errors get the
                // operation context without that guess.
                anyhow::Error::new(err).context(format!(
                    "sending POST request to local faster-whisper server at {} \
                     — is the Docker container running? \
                     (try: docker compose -f docker/faster-whisper/docker-compose.yml up)",
                    url
                ))
            } else {
                anyhow::Error::new(err)
                    .context(format!("sending POST request to local server at {url}"))
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            // Keep the status: it is the actionable fact even when the
            // body read times out or fails.
            let detail = match response.text() {
                Ok(body) => body,
                Err(err) if err.is_timeout() => {
                    format!("<{}>", request_timeout_error(url, timeout, audio_ms))
                }
                Err(err) => format!("<unreadable response body: {err}>"),
            };
            return Err(anyhow!(
                "local faster-whisper server at {} returned status {}: {}",
                url,
                status,
                detail
            ));
        }

        response.json::<serde_json::Value>().map_err(|err| {
            if err.is_timeout() {
                request_timeout_error(url, timeout, audio_ms)
            } else {
                anyhow::Error::new(err).context(format!("parsing JSON response from {url}"))
            }
        })
    }
}

/// One message for every timeout, whether the server stalled before
/// responding or partway through the body.
fn request_timeout_error(url: &str, timeout: Duration, audio_ms: u64) -> anyhow::Error {
    anyhow!(
        "local faster-whisper server at {} did not answer within {} s; \
         the deadline covers {} ms of audio at the documented {}x \
         worst-case CPU speed plus a {} s margin",
        url,
        timeout.as_secs(),
        audio_ms,
        config::LOCAL_WHISPER_CPU_SPEED_FACTOR,
        config::LOCAL_WHISPER_TIMEOUT_MARGIN_SECS,
    )
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

    // ---- wav_duration_ms / transcription_timeout_for_utterance --------

    fn stub_wav_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("tardis_whisper_{tag}_{}.wav", std::process::id()))
    }

    fn write_test_wav(path: &std::path::Path, samples: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).expect("create test WAV");
        for _ in 0..samples {
            writer.write_sample(0_i16).expect("write test sample");
        }
        writer.finalize().expect("finalize test WAV");
    }

    #[test]
    fn wav_duration_reads_milliseconds_from_the_header() {
        let path = stub_wav_path("duration");
        for (samples, expected_ms) in [(8_000_u32, 1_000_u64), (4_000, 500)] {
            write_test_wav(&path, samples);
            assert_eq!(wav_duration_ms(&path).unwrap(), expected_ms);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn timeout_covers_a_fifteen_second_utterance() {
        // 15 s of audio at the documented worst case (5x realtime) plus
        // the margin, well past reqwest's 30 s default.
        assert_eq!(
            transcription_timeout_for_utterance(15_000),
            Duration::from_secs(15 * 5 + 30)
        );
    }

    #[test]
    fn timeout_covers_a_sixty_second_file() {
        assert_eq!(
            transcription_timeout_for_utterance(60_000),
            Duration::from_secs(60 * 5 + 30)
        );
    }

    #[test]
    fn timeout_scales_linearly_with_the_audio_length() {
        assert_eq!(
            transcription_timeout_for_utterance(2_000) - transcription_timeout_for_utterance(1_000),
            Duration::from_secs(5)
        );
    }

    // ---- delayed local server ----------------------------------------

    fn spawn_stalling_server(
        response: Option<Vec<u8>>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let address = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0u8; 1024];
                        let _ = std::io::Read::read(&mut stream, &mut buffer);
                        if let Some(bytes) = response {
                            let _ = std::io::Write::write_all(&mut stream, &bytes);
                            let _ = std::io::Write::flush(&mut stream);
                        }
                        std::thread::sleep(Duration::from_millis(400));
                        return;
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        });
        (address, server)
    }

    fn stub_upload_file(tag: &str) -> std::path::PathBuf {
        let path = stub_wav_path(tag);
        // The stalled-server tests call `post_multipart` directly, so the
        // bytes only need to exist for the multipart file part.
        std::fs::write(&path, b"RIFF").expect("write stub upload");
        path
    }

    #[test]
    fn a_stalled_local_server_hits_the_request_timeout() {
        let (address, server) = spawn_stalling_server(None);
        let path = stub_upload_file("stalled_head");
        let client = LocalWhisperClient::new(format!("http://{address}"), "base", None);

        let started = std::time::Instant::now();
        let err = client
            .post_multipart(
                &build_transcription_url(&client.base_url),
                &path,
                Duration::from_millis(100),
                60_000,
            )
            .expect_err("a stalled server must produce a timeout error");
        let elapsed = started.elapsed();
        let _ = std::fs::remove_file(&path);

        assert!(
            elapsed < Duration::from_secs(2),
            "timeout should fire promptly, took {elapsed:?}"
        );
        let message = err.to_string();
        assert!(
            message.contains("did not answer within"),
            "error should name the timeout, got: {message}"
        );
        assert!(
            message.contains("60000 ms of audio"),
            "error should name the audio length, got: {message}"
        );
        server.join().expect("server thread");
    }

    #[test]
    fn a_stalled_response_body_hits_the_request_timeout() {
        // Headers arrive, then the body stops partway through. The
        // request timeout also covers the body read.
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"text\":".to_vec();
        let (address, server) = spawn_stalling_server(Some(response));
        let path = stub_upload_file("stalled_body");
        let client = LocalWhisperClient::new(format!("http://{address}"), "base", None);

        let err = client
            .post_multipart(
                &build_transcription_url(&client.base_url),
                &path,
                Duration::from_millis(100),
                5_000,
            )
            .expect_err("a stalled body must produce a timeout error");
        let _ = std::fs::remove_file(&path);

        assert!(
            err.to_string().contains("did not answer within"),
            "error should name the timeout, got: {err}"
        );
        server.join().expect("server thread");
    }

    #[test]
    fn connection_refused_keeps_the_docker_hint() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind free port");
        let address = listener.local_addr().expect("listener address");
        drop(listener);
        let path = stub_upload_file("refused");
        let client = LocalWhisperClient::new(format!("http://{address}"), "base", None);

        let err = client
            .post_multipart(
                &build_transcription_url(&client.base_url),
                &path,
                Duration::from_secs(2),
                1_000,
            )
            .expect_err("a refused connection must produce an error");
        let _ = std::fs::remove_file(&path);

        assert!(
            err.to_string().contains("Docker container running"),
            "connect failures should keep the Docker hint, got: {err}"
        );
    }
}
