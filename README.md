# TARDIS v1

TARDIS v1 is a Rust CLI foundation for a future real-time transcription and translation desktop app. It captures microphone audio, splits it into chunks, runs a mock transcription pipeline, and runs a mock translation pipeline.

The name is inspired by the **Doctor Who** TARDIS translation concept — a tool that listens, transcodes, and rewrites what you hear in the language you want.

---

## Current Status

- Rust CLI project created
- Audio input/output devices can be listed
- Default microphone capture works
- 5-second microphone recording works
- WAV chunk recording works
- Real-time chunking test works
- Mock transcription pipeline works (live and file-based)
- Mock translation pipeline works
- Local faster-whisper Docker provider integrated (HTTP, OpenAI-compatible endpoint)
- Pure unit tests cover volume, chunking, mock transcription, mock translation, WAV sample conversion, audio-activity classifier, and the faster-whisper URL/JSON helpers
- No live streaming transcription yet (file-based only today)
- No Tauri UI yet

---

## Tech Stack

- **Rust** (edition 2024)
- **Cargo** for build, test, and run
- **CPAL 0.18** for audio device access and real-time microphone capture
- **hound** for WAV read/write
- **anyhow 1.0.102** for error handling

---

## CLI Commands

| Command | Description | Stops automatically? |
|---|---|---|
| `cargo run -- devices` | Lists input/output audio devices and default devices. | Yes |
| `cargo run -- mic-5s` | Captures the default microphone for 5 seconds and prints volume activity. | Yes |
| `cargo run -- mic` | Captures the default microphone continuously and prints volume activity. | No, press Ctrl+C |
| `cargo run -- record-5s` | Records 5 seconds from the default microphone to `output/mic_test.wav`. | Yes |
| `cargo run -- chunk-test` | Captures microphone audio, splits it into chunks, and prints chunk metadata. | Yes |
| `cargo run -- mock-transcribe` | Captures microphone audio, chunks it, and runs the mock transcription pipeline. | Yes |
| `cargo run -- save-chunks-test` | Captures microphone audio and saves each chunk as a WAV file under `output/chunks/`. | Yes |
| `cargo run -- mock-transcribe-file output/chunks/chunk_001.wav` | Reads a saved WAV chunk and runs mock transcription on it. | Yes |
| `cargo run -- mock-translate` | Captures microphone audio, runs mock transcription, then mock translation. | Yes |
| `cargo run -- local-transcribe-file [--provider <name>] <path>` | Sends a WAV file to a `LocalTranscriptionProvider` selected by `--provider` (default `local-whisper`) and prints the plaintext transcript. The faster-whisper provider requires the Docker container to be running — see [`## Local faster-whisper Docker transcription`](#local-faster-whisper-docker-transcription). Pass `--provider mock-local` to use the deterministic offline stub instead (no Docker required). | Yes |

---

## Sample Output

One representative line per command, captured on macOS. Volume readings
reflect a quiet office, so chunk lines come back as silence — that is
expected when no one is talking into the microphone.

### `cargo run -- devices`

```text
Default input device: Micrófono del MacBook Pro
```

### `cargo run -- mic-5s`

```text
Capturing for 5 seconds...
```

### `cargo run -- mic`

```text
Listening to microphone... Press Ctrl+C to stop.
```

### `cargo run -- record-5s`

```text
Saved WAV to output/mic_test.wav
```

### `cargo run -- chunk-test`

```text
[chunk #  1] samples= 48000 ~ 1000 ms vol=0.000
```

### `cargo run -- mock-transcribe`

```text
[chunk 1] silence detected, skipping...
```

### `cargo run -- save-chunks-test`

```text
[chunk 1] saved to output/chunks/chunk_001.wav | samples: 48000 | volume: 0.0000
```

### `cargo run -- mock-transcribe-file output/chunks/chunk_001.wav`

*Requires a WAV produced by a prior `save-chunks-test` run.*

```text
silence detected
```

### `cargo run -- mock-translate`

```text
[chunk 1] silence detected, skipping translation...
```

### `cargo run -- local-transcribe-file output/chunks/chunk_001.wav`

*Requires the local faster-whisper Docker container running on port 8000. See [`## Local faster-whisper Docker transcription`](#local-faster-whisper-docker-transcription) below for setup.*

Expected output (**illustrative**, not captured on this repo's hardware — a quiet laptop produces silence-only chunks, which return an empty `text` field; a chunk with real speech produces something like below):

```text
file:     output/chunks/chunk_001.wav
provider: local-whisper (faster-whisper Docker HTTP, OpenAI-compatible)
transcribing...

transcript:
hello this is a test
```

### `cargo run -- local-transcribe-file --provider mock-local output/chunks/chunk_001.wav`

*No Docker required. The mock-local stub echoes `"mock transcript for <basename>"` for whatever path is given — useful for offline development and unit-test wiring.*

Expected output:

```text
file:     output/chunks/chunk_001.wav
provider: mock-local
transcribing...

transcript:
mock transcript for chunk_001.wav
```

If the Docker container is not running, the CLI exits with a self-explaining error:

```text
Error: sending POST request to local faster-whisper server at http://localhost:8000/v1/audio/transcriptions — is the Docker container running?
       (try: docker compose -f docker/faster-whisper/docker-compose.yml up)
```

---

## Local faster-whisper Docker transcription

This section describes the **first real local transcription provider** in `tardisv1`: a self-hosted [`fedirz/faster-whisper-server`] Docker container exposing an OpenAI-compatible HTTP API. The Rust CLI calls it via `local-transcribe-file`. No audio leaves the machine.

[`fedirz/faster-whisper-server`]: https://github.com/fedirz/faster-whisper-server

### Start the Docker service

From the repo root:

```bash
docker compose -f docker/faster-whisper/docker-compose.yml up
```

The first run pulls the image **and** downloads the `base` Whisper model (~150 MB). Subsequent runs reuse the cached model under `docker/faster-whisper/hf_cache/`. To stop:

```bash
docker compose -f docker/faster-whisper/docker-compose.yml down
```

The image exposes `POST http://localhost:8000/v1/audio/transcriptions` (loopback only) — exactly the endpoint the Rust client targets. See `docker/faster-whisper/README.md` for full operator docs (health checks, model sizes, GPU opt-in, privacy).

### Generate a chunk to transcribe

```bash
cargo run -- save-chunks-test
# writes output/chunks/chunk_001.wav ... chunk_010.wav
```

### Transcribe a chunk against the local server

```bash
cargo run -- local-transcribe-file output/chunks/chunk_001.wav
```

This constructs `transcription::local_whisper::LocalWhisperClient` from `config::LOCAL_WHISPER_*` constants, POSTs the WAV to `/v1/audio/transcriptions`, and prints the plaintext `text` field of the OpenAI-style JSON response.

### Switch providers

The CLI accepts an optional `--provider <name>` flag that selects between implementations of the `transcription::LocalTranscriptionProvider` trait. Available values today:

| `--provider <name>` | Backend | Network? |
|---|---|---|
| `local-whisper` *(default)* | Docker faster-whisper HTTP server | Yes (loopback) |
| `mock-local` | Deterministic stub — echoes `"mock transcript for <basename>"` | No |

Examples:

```bash
# Default (the local Docker server):
cargo run -- local-transcribe-file output/chunks/chunk_001.wav

# Offline / no-Docker development:
cargo run -- local-transcribe-file --provider mock-local output/chunks/chunk_001.wav

# Equals form also works:
cargo run -- local-transcribe-file --provider=mock-local output/chunks/chunk_001.wav
```

Passing an unknown provider name exits with a self-explaining error:

```text
Error: unknown provider 'whisper-cpp'; valid values are: local-whisper, mock-local
```

### Notes

- **First real local provider.** Future providers (`whisper.cpp` binary, OpenAI Whisper local Python, optional cloud APIs) implement the same `transcription::LocalTranscriptionProvider` trait so the CLI surface stays unchanged. A future `--provider` flag will dispatch between providers at runtime.
- **CPU is the default.** The `:latest-cpu` image is used by `docker-compose.yml`. Switching to the GPU image (`latest`) and uncommenting the GPU `deploy` block enables CUDA. CPU on a laptop is roughly **2–5× realtime** for the `base` model — slower but works without NVIDIA drivers.
- **No cloud credentials.** The container has no network egress to any cloud; the model runs locally and binds only to `127.0.0.1`.
- **Audio stays local.** The CLI only ever connects to `http://localhost:8000` (see `LOCAL_WHISPER_BASE_URL` in `src/config.rs`).

---

## Testing

```bash
cargo test
```

- Unit tests cover **pure logic only** (volume, chunking, mock transcription, mock translation, WAV sample conversion, chunk recorder helpers).
- Hardware and audio behavior — CPAL streams, real microphones, OS device lists, file permissions — are **not** unit-tested.
- Hardware behavior is verified manually through the CLI commands above.

---

## Project Architecture

```
src/
├── main.rs           # dispatch on cargo run -- <mode>
├── audio/            # capture + record + chunk helpers
│   ├── devices          # device listing
│   ├── mic capture      # default microphone stream
│   ├── recording        # 5-second WAV recording
│   ├── chunking         # 1-second chunk boundaries
│   ├── chunk recording  # WAV-per-chunk writer
│   ├── volume helpers
│   └── activity (Silence/SpeechLike classifier)
├── transcription/    # transcriber trait + mocks + pipelines + first real local provider
│   ├── transcriber trait
│   ├── mock transcriber
│   ├── live mock transcription pipeline
│   ├── file-based mock transcription pipeline
│   ├── local-whisper (HTTP client; faster-whisper Docker, OpenAI-compatible)
│   └── LocalTranscriptionProvider trait (provider-agnostic, awaits a `--provider` flag)
└── translation/      # translator trait + mocks + pipeline
    ├── translator trait
    ├── mock translator
    └── mock end-to-end translation pipeline
```

---

## Current Mock Pipeline

```
Microphone
  → audio chunks
  → volume/activity check
  → MockTranscriber
  → MockTranslator
  → console output
```

The same conceptual flow is exercised three ways:

- **Live** — `mock-transcribe` and `mock-translate` capture from the mic in real time.
- **File-based** — `mock-transcribe-file output/chunks/chunk_001.wav` re-classifies a previously recorded chunk.
- **Recorded** — `save-chunks-test` persists every chunk to disk so the file-based flow has something to read.

---

## Development Principles

- Build small, testable CLI commands first.
- Add unit tests for every new piece of pure logic.
- Never test real hardware (CPAL streams, microphones, OS devices) in unit tests — exercise it through the CLI instead.
- Keep CPAL/audio capture separate from transcription and translation abstractions.
- Do not add Tauri until the CLI audio engine is stable.
- Do not add real transcription or translation APIs until the mock pipelines are validated.

---

## Roadmap

- Centralize constants / configuration (sample rate, buffer size, thresholds, language codes).
- Add a speech-activity helper module.
- Improve README examples with real sample output.
- Research real transcription providers (Whisper local, whisper.cpp, Deepgram, etc.).
- Add a first real transcription provider behind the `Transcriber` trait.
- Add a first real translation provider behind the `Translator` trait.
- Investigate system audio capture (loopback on macOS / Windows / Linux).
- Add a Tauri desktop UI on top of the stable CLI engine.

---

## Privacy Note

This project captures microphone audio. Future versions may also capture **system audio**. Audio may contain sensitive information, so visible recording indicators, explicit permissions, and clear user controls should be part of the final app.
