# System audio capture

`--source system` captures audio that the host is playing (a call, a video,
a stream) instead of the microphone. The rest of the five-stage loop is
unchanged: endpointing, transcription, processing, synthesis and playback
run the same way.

The source is part of the session config:

```json
{ "audio_source": "mic" }
```

`mic` is the default, so a `runtime.json` written before this field existed
still loads. The CLI selects the source with `--source mic|system`; the
Tauri shell stores the same field in `runtime.json`.

## macOS (supported in this release)

macOS has no public loopback tap that does not need a driver, so Tardis
captures a virtual input device that the system output is routed into.
BlackHole 2ch is the tested driver.

1. Install BlackHole 2ch:

   ```sh
   brew install blackhole-2ch
   ```

2. Route the audio you want to transcribe into BlackHole. In
   **System Settings → Sound → Output**, select **BlackHole 2ch**. If you
   also want to hear the audio, create a **Multi-Output Device** in
   **Audio MIDI Setup** containing your speakers and BlackHole, and select
   that as the output.

3. Confirm that Tardis sees the loopback:

   ```sh
   cargo run -- devices --source system
   ```

   The listing should include `BlackHole 2ch` (or `BlackHole 16ch/64ch`)
   with its device id.

4. Start the live loop. With the default mock synthesizer the pipeline is
   silent, so the simplest end-to-end check is:

   ```sh
   cargo run -- live-local-transcribe --source system
   ```

   With a real voice, send the synthesized reply to the speakers, not to
   BlackHole:

   ```sh
   cargo run -- live-local-transcribe --source system --synthesizer piper \
     --device "MacBook Pro Speakers"
   ```

5. If more than one loopback is installed, pick the one that receives the
   audio. `devices --source system` prints each device id; pass it with
   `--input-device`, or set `input_device` in `runtime.json` for the shell:

   ```sh
   cargo run -- live-local-transcribe --source system \
     --input-device "<id printed by devices --source system>"
   ```

   `--source system` rejects a microphone id: the selected device must
   match a loopback name, and the session fails with the id in the error
   otherwise.

## Capture and playback must be distinct

The pipeline speaks its own translations. If the player writes into the
device that `--source system` is capturing, the next utterance is Tardis
transcribing itself. Keep the two apart:

- capture: the loopback input (`BlackHole 2ch`);
- playback: a real output (`--device "MacBook Pro Speakers"`), or the OS
  default when that default is not BlackHole.

Validation rejects an explicit `player_device` that names a loopback
device (`"BlackHole 2ch"`), and the live runner compares the two resolved
device labels before capture starts. That catches a substring (`"2ch"`) or
an OS default that landed on the loopback too. The half-duplex gate is a
second line of defence while a reply is playing, not a substitute for
distinct devices.

## Linux (documented, not shipped in this release)

PulseAudio and PipeWire expose a monitor source per sink; it carries
everything the sink plays.

```sh
pactl list short sources | grep monitor
```

The intended setup is to capture that monitor source through the same
`SystemInput` path. This release builds the macOS backend only, so
`--source system` returns:

```
system audio capture is not supported on this platform; see docs/system-audio.md
```

## Windows (documented, not shipped in this release)

WASAPI can record the default output endpoint directly, without a virtual
cable. The intended backend opens that endpoint as a loopback stream.
This release does not ship it, so `--source system` returns the same
not-supported error as Linux.

Until the backend lands, a virtual cable such as VB-CABLE can be used as
an ordinary input device with `input_device` in `runtime.json`; that is the
microphone path with a different device, not `--source system`.

## Failure modes

- **`No system-audio loopback input device found`**: the driver is not
  installed, or its input is disabled. Install BlackHole and rerun
  `devices --source system`.
- **`system audio capture is not supported on this platform`**: this build
  has no backend for the host OS. See the Linux/Windows sections.
- **Transcript contains the previous reply**: capture and playback share a
  device. Point `--device` at the speakers and keep BlackHole out of the
  playback path.
- **Silence**: BlackHole is installed but nothing is routed into it. Set
  the system output to BlackHole (or to a Multi-Output Device that includes
  it).
