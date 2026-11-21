# Local faster-whisper Docker transcription server

This directory runs a self-hosted, **OpenAI-compatible** HTTP transcription
service based on [`fedirz/faster-whisper-server`]. The Rust CLI calls this
server from the `local-transcribe-file` command. The port binds to
loopback, so audio stays on the machine.

[`fedirz/faster-whisper-server`]: https://github.com/fedirz/faster-whisper-server

## 1. What this service is for

* **Real local speech-to-text**: the first non-mock transcription
  provider. OpenAI-API-compatible:
  `POST http://localhost:8000/v1/audio/transcriptions`, the same shape
  `whisper.cpp --openai-api` and OpenAI's own cloud API expose.
* **File-based only for now**: pass a WAV path on disk, get text
  back. Future providers (`whisper.cpp` binary, cloud APIs) implement
  the same `transcription::LocalTranscriptionProvider` trait so the
  CLI doesn't change.
* **CPU-first by default**: the pinned `:0.5.0-cpu` image works on any
  machine with Docker; GPU is an opt-in tag swap.

## 2. How to start it

From the repo root:

```bash
docker compose -f docker/faster-whisper/docker-compose.yml up
```

The first run pulls the image and downloads the chosen model
(default `base`, ~150 MB). Subsequent runs reuse the cached model in
`./hf_cache`.

Detached mode (returns immediately, logs streamed with `docker compose logs`):

```bash
docker compose -f docker/faster-whisper/docker-compose.yml up -d
```

## 3. How to stop it

```bash
docker compose -f docker/faster-whisper/docker-compose.yml down
```

`down` removes the container but preserves the `./hf_cache` volume, so
the model is not re-downloaded next time. Add `-v` to also wipe the
model cache.

## 4. How to check health

The image starts accepting connections as soon as port `8000` is open,
but the model is still loading for the first ~10–30 s on CPU. Probe
with:

```bash
# Port reachability: the OpenAI list-models endpoint responds faster.
curl -fsS http://localhost:8000/v1/models

# Health endpoint (older image tags may not expose it).
curl -fsS http://localhost:8000/health
```

If both fail after ~1 minute, inspect the container:

```bash
docker compose -f docker/faster-whisper/docker-compose.yml logs -f faster-whisper
```

## 5. Example curl

Generate a 1-second WAV chunk on disk first:

```bash
cargo run -- save-chunks-test
# writes output/chunks/chunk_001.wav ... chunk_010.wav
```

Then call the server directly:

```bash
curl http://localhost:8000/v1/audio/transcriptions \
     -F file=@output/chunks/chunk_001.wav \
     -F model=base \
     -F language=en
```

Response shape:

```json
{ "text": "hello this is a test" }
```

## 6. First-run model download

| Model    | Approx. size |
|----------|-------------:|
| `tiny`   | ~75 MB       |
| `base`   | ~150 MB (default) |
| `small`  | ~500 MB      |
| `medium` | ~1.5 GB      |
| `large-v2`, `large-v3` | ~3 GB |

The model is cached under `./docker/faster-whisper/hf_cache/` on the
host. To pre-download or change the default model, edit `WHISPER_MODEL`
in [`docker-compose.yml`](./docker-compose.yml).

> **Note:** `WHISPER_MODEL` in `docker-compose.yml` is read at
> container start and determines which model is loaded into memory.
> The `model` form field the Rust client sends is informational only
> on this image: the **server-side env wins**. If you
> change the compose env to `small`, also update
> `LOCAL_WHISPER_MODEL` in [`src/config.rs`](../../src/config.rs)
> so they stay in sync, otherwise the README's model diagnostics
> will lie about what's actually running.

## 7. CPU vs GPU

The compose file ships the pinned `:0.5.0-cpu` image by default:

* **CPU**: works everywhere Docker runs. The `base` model is roughly
  **2–5× realtime** on a modern laptop CPU (i.e. transcribing 1 s of
  audio takes 2–5 s). Larger models scale linearly slower.
* **GPU**: swap `image` to the GPU tag you intend to run, pin its
  digest the same way, and uncomment the
  `deploy.resources.reservations.devices` block at the bottom of
  `docker-compose.yml`. Requires the **NVIDIA Container Toolkit**.
  Achieves roughly **real-time or faster** even for `large-v3`.

CPU is the right starting point for a developer laptop; GPU is for
production throughput. Both use the **same API contract**: swapping the
image is the only change needed; no client code changes.

## 8. Privacy

* Audio is uploaded to `http://localhost:8000`, bound to the loopback
  interface of the host (`127.0.0.1:8000` in `docker-compose.yml`).
  Nothing leaves your machine unless you change the port mapping.
* The `tardisv1` Rust CLI only ever connects to `http://localhost:8000`
  (see `LOCAL_WHISPER_BASE_URL` in [`src/config.rs`](../../src/config.rs)).
* The container has **no network egress to any cloud**: the model
  weights are downloaded once into `./hf_cache` and stay local.

## 9. Post-install sanity check

After `docker compose up`, from the repo root run:

```bash
cargo run -- local-transcribe-file output/chunks/chunk_001.wav
```

A silence-only chunk returns an empty `text`; a chunk with real speech
prints the recognized transcript. See the top-level
[`README.md`](../../README.md#local-faster-whisper-docker-transcription)
for the expected CLI output shape.

## 10. Dependency pinning

The compose file pins the image tag and its multi-arch manifest digest
(`0.5.0-cpu@sha256:...`), so a rebuild uses the same server even if
Docker Hub moves the tag. Change both parts together on the `image`
line in [`docker-compose.yml`](./docker-compose.yml).

Review the pin once a quarter, and sooner when an upstream security
advisory names the image. To move to a newer release:

1. Pick a release tag from the image's tag list and confirm it supports
   the platforms you use.
2. Read the new digest with
   `docker buildx imagetools inspect fedirz/faster-whisper-server:<tag>`.
3. Update both the tag and the digest on the `image` line, then run
   `docker compose -f docker/faster-whisper/docker-compose.yml pull`.
4. Run the section 9 sanity check before committing the change.

A GPU setup follows the same rule: pin the GPU tag with its digest
instead of switching to a moving `latest` tag.
