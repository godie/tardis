# Local Piper Docker text-to-speech server

This directory runs a self-hosted, **local** HTTP text-to-speech service
based on [`piper-tts[http]`](https://github.com/OHF-Voice/piper1-gpl).
The Rust CLI calls this server through the `piper` synthesizer provider
(`live-local-transcribe --synthesizer piper`, `say-text`). The port
binds to loopback, so audio never leaves the machine.

## 1. What this service is for

* **Real local speech synthesis**: the first non-mock synthesizer
  provider. `POST http://localhost:5002/synthesize` with a JSON body
  (`{"text": "...", "voice": "..."}`) returns a WAV file
  (16-bit PCM, 22050 Hz mono for the default voice).
* **Loopback by construction**: the client (`LocalHttpSynthesizer`)
  only ever talks to `http://localhost:5002` (see `TTS_BASE_URL` in
  [`src/config.rs`](../../src/config.rs)). There is no API key
  because nothing is exposed off-host.
* **CPU-first**: synthesis runs on ONNX Runtime on CPU; a one-second
  utterance renders in well under a second on a laptop.

## 2. How to start it

From the repo root:

```bash
docker compose -f docker/piper/docker-compose.yml up -d
```

The image is built from the local `Dockerfile` (`python:3.13-slim` +
`piper-tts[http]`). The first start downloads the voice (~60 MB) into
`./voices` on the host; later starts reuse it.

## 3. How to stop it

```bash
docker compose -f docker/piper/docker-compose.yml down
```

`down` removes the container but keeps `./voices`, so the voice is
not re-downloaded next time. Delete the directory to free the ~60 MB.

## 4. How to check health

The server needs a few seconds to load the ONNX model on first start.
Probe with:

```bash
curl -fsS http://localhost:5002/info
```

Expected response for the default voice:

```json
{"last":null,"voice":{"language":"es-419","name":"es_MX-ald-medium","num_speakers":1}}
```

If it fails after ~1 minute, inspect the container:

```bash
docker compose -f docker/piper/docker-compose.yml logs -f piper
```

## 5. Example curl

```bash
curl -X POST -H 'Content-Type: application/json' \
     -d '{"text": "hola desde tardis"}' \
     -o /tmp/piper-test.wav http://localhost:5002/synthesize
```

Then, from the repo root, the same round trip end to end (server +
real output device):

```bash
cargo run -- say-text "hola desde tardis"
```

`say-text` synthesizes first and opens the device after, so a server
failure aborts before any audio plays.

## 6. The voice

The default is `es_MX-ald-medium` (`PIPER_VOICE` in
[`docker-compose.yml`](./docker-compose.yml)), single-speaker Mexican
Spanish. The client must request the same id: `TTS_VOICE` in
[`src/config.rs`](../../src/config.rs) — if you change one, change
both, like `WHISPER_MODEL` / `LOCAL_WHISPER_MODEL` in the
faster-whisper setup.

## 7. Privacy

* Audio playback happens on the host; only the text to speak crosses
  the loopback interface inside the machine, and the port is bound to
  `127.0.0.1` so nothing outside the machine can reach the server.
* The voice file stays on the host in `./voices`. It is downloaded
  once from Hugging Face on the first start, and the compose file does
  not block the container's outbound connections — the loopback
  binding covers inbound traffic only. If that matters to you,
  download the voice into `./voices` yourself before the first start.
