# Parrot Service: a browser speech demo in Docker

The **Parrot Service** records speech, transcribes it and reads the words back.

This demo packages the real `audio-chat-box` executable and a separate, small Rust HTTP
server. Open one page, record a short English utterance, see the transcript, then hear
Kokoro read it back. Choose a voice from the engine's current voice list, then replay the
same transcript in another voice without recording again. You can also type/edit text and
hear it spoken. Voice language labels describe the voices; they do not translate the words. The server communicates
with the executable over its existing standard-input/output protocol: it does not link the
speech engine into the web server.

**This is a speech round-trip, not an AI conversation.** The repository provides speech
recognition and synthesis, not a conversational language model. No API key, paid service,
browser speech-recognition service, or hosted LLM is used. An eventual conversational demo
would need a separately designed and explicitly disclosed language-model integration.

## Try it locally

Requires Docker Engine/Desktop with Compose v2 and enough disk for the build, native
libraries, and expanded models. This first version targets **Linux amd64 only**. Apple
Silicon uses Docker's amd64 emulation and may be substantially slower; native arm64 is not
claimed supported by this image.

```sh
docker compose up --build
```

Visit **http://localhost:8080** on the same machine. Allow microphone permission when you
press Record. A browser audio playback control remains available if autoplay is blocked.
Stop recording before speaking again. The page's Cancel control stops the recording/request;
audio is not saved to the model volume.

The first startup downloads the default models from this repository's `models-v1` release,
using the executable's existing pinned archive lengths and SHA-256 checks. Kokoro and
Parakeet total approximately **832 MB of compressed downloads**; expanded files, inference
memory, Docker build layers, and temporary downloads require additional space. The demo
does not need Silero VAD because recording is explicitly started and stopped. Wait for the
page's model status to report ready. A first-start download can take several minutes.

Models persist in a named Docker volume; future starts reuse them. Once populated, forbid
further model downloads with:

```sh
AUDIO_CHAT_BOX_OFFLINE=1 docker compose up
```

This flag prevents model downloads, not general network access at the container level. The
application does not upload recordings. Browser microphone data goes to this local demo's
server for inference, then is discarded rather than written to disk. Treat the host/container
as trusted: in-memory audio and text are still accessible to software with host privileges.

```sh
docker compose down       # stops the demo; preserves downloaded models
# Optional and destructive to the demo's model cache only:
docker compose down -v    # removes the models volume; next run downloads again
```

## Limits and deployment challenges

- **Microphones require a secure context.** Browsers generally allow `http://localhost`.
  An ordinary `http://192.168...` page on a LAN does not qualify. Remote access needs HTTPS,
  an explicitly configured `DEMO_ORIGIN`, and a separate authentication/access-control
  design. Do not just publish the port on all interfaces. The supplied Compose file binds
  only `127.0.0.1`; it is a single-user local demo, not an internet service.
- Browser recording is converted to mono 16 kHz PCM WAV before transcription. Sending a
  browser's arbitrary WebM/Opus recording straight to the engine would not work: the engine
  accepts WAV or raw PCM, not WebM. Browser permission denial, missing input devices,
  playback restrictions, and cancellation must remain visible rather than appearing hung.
- Input is bounded to 30 seconds of recording and 1,000 characters of text. The server
  accepts at most four HTTP connections and one inference at a time (other requests receive
  a busy error), limits request reads to 10 seconds and inference waits to 90 seconds.
  Cancel stops the microphone/browser request immediately. An in-flight native inference
  retains the single worker until it finishes, with its response discarded if disconnected;
  a protocol cancellation acknowledgment would not prove the native work had stopped.
  At 90 seconds the speech process is killed and the demo fails closed until container restart.
  Request IDs prevent an abandoned reply being delivered to a later request.
  Generated float WAV is converted to PCM16 WAV for broader browser playback support.
- Input is bounded to short recordings and text. The page/server enforce limits; this is
  not an upload endpoint for arbitrary large files. Do not use confidential audio on an
  untrusted Docker host or forward this unauthenticated local service to other users.
- The example allocates up to **4 CPUs and 6 GiB RAM**, with a PID limit. These are protective
  demo defaults, **not measured minimums or performance guarantees**. Startup/loading and
  inference may fail under insufficient Docker Desktop memory. CPU latency and emulated
  amd64 performance need measurement on the target machine; no GPU setup is included.
- The Docker build needs network access to Rust crates, the pinned protocol source,
  sherpa-onnx, the base image registry, Debian packages, and licence source files.
  Build-time native archive integrity is checked with the same checksum as repository CI.
  The compiler version is pinned; the base-image tags and apt package versions are not
  digest/version-pinned, so this is not a bit-for-bit reproducible supply-chain release.
- No images are pushed or services deployed by these files. `docker compose build` builds
  locally; it does not publish anything.

## Licences and redistribution

The final image contains the project's licences, credits, native dependency licence files,
compiled dependency notices generated by `tools/notices.sh`, separate demo dependency
notices, and a snapshot of the project's source/build files under
`/usr/share/audio-chat-box/`. The engine is a separate process and remains GPL-3.0-or-later
as a combined binary because it links eSpeak NG. The project code's dual MIT/Apache-2.0
licensing does not remove that obligation. Model licences remain distinct: Kokoro
Apache-2.0; Parakeet CC-BY-4.0, attributed to NVIDIA. Their downloads carry model notices.

**Publishing a prebuilt image requires a separate release/compliance review.** The included
project-source archive is not the full corresponding source of all statically linked native
libraries. Before redistribution, arrange appropriate corresponding source and build
information for the exact binary and its GPL components, verify all required notices and
model attribution, and preserve those alongside any distributed image. This PR does not
publish an image or assert that a repository link alone meets those requirements.

## Verification

Run the engine's existing `CLAUDE.md` checks, then the demo crate's tests/lints and browser
checks. A complete release check should build the image, start it with an empty model volume,
observe verified download/readiness, transcribe a synthetic/public-domain fixture, synthesize
and play the response, cancel/repeat recording, deny microphone permission, disconnect the
page, restart with the cached volume offline, and check non-local origins are rejected.

The PR describes which of these checks actually ran. A successful source-level or unit test
is not evidence that the real-model Docker/audio path was exercised.

### Developer checks for the demo

```sh
cargo fmt --manifest-path demo-web/Cargo.toml --check
cargo clippy --manifest-path demo-web/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path demo-web/Cargo.toml --locked
cargo build --manifest-path demo-web/Cargo.toml --locked
python3 demo-web/tests/smoke.py
node --test demo-web/static/*.test.mjs
cargo deny --manifest-path demo-web/Cargo.toml check
# Optional: Playwright and Chromium must already be installed.
# Uses a synthetic device, never your real microphone.
node demo-web/tests/browser.cjs
```

The HTTP and browser harnesses bind localhost port 8080; stop another demo first.
HTTP smoke uses a fake speech child and synthetic WAV, not real model inference.
The browser harness accepts `CHROMIUM_PATH` for another Chromium executable.
