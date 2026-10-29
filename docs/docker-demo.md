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

## Run it from Docker Hub

Needs only Docker. The image is `pnxstudios/audio-chat-box-demo` on Docker Hub: about 880 MB
to download, 1.2 GB once pulled.

```sh
docker pull pnxstudios/audio-chat-box-demo
docker run --rm --platform linux/amd64 -p 127.0.0.1:8080:8080 pnxstudios/audio-chat-box-demo
```

Open **http://localhost:8080** on the same machine once the page's model status says ready
(a few seconds). Allow microphone permission when you press Record. A browser audio playback
control remains available if autoplay is blocked. Stop recording before speaking again. The
page's Cancel control stops the recording/request. Ctrl+C stops the container.

**The image carries its models**, Kokoro and Parakeet, so it needs no network, volume or
account to run, and never tries to download a model (`AUDIO_CHAT_BOX_OFFLINE=1`). It is
built for **Linux amd64 only**: on Apple Silicon, `--platform linux/amd64` runs it under
Docker's emulation, slower (on the Mac it was tested on, about 4 s to speak a phrase, against
under 1 s for the program built natively); native arm64 is not claimed supported.

To run it with fewer privileges and capped resources (the server needs none of what these
take away):

```sh
docker run --rm --platform linux/amd64 -p 127.0.0.1:8080:8080 \
  --read-only --tmpfs /tmp:size=64m,mode=1777 \
  --cap-drop ALL --security-opt no-new-privileges:true \
  --cpus 4 --memory 6g --pids-limit 128 \
  pnxstudios/audio-chat-box-demo
```

The server accepts only requests whose `Host` and `Origin` match `DEMO_ORIGIN`
(`http://localhost:8080`), so open the page at that address, not `127.0.0.1`. Another host
port needs a matching origin, for example
`-p 127.0.0.1:9000:8080 -e DEMO_ORIGIN=http://localhost:9000`.

The application does not upload recordings. Browser microphone data goes to this local
demo's server for inference, then is discarded rather than written to disk. Treat the
host/container as trusted: in-memory audio and text are still accessible to software with
host privileges.

## Build it yourself

From a clone of this repository, with about 10 GB of free disk for Docker's build cache:

```sh
docker build --platform linux/amd64 -t pnxstudios/audio-chat-box-demo .
```

then run it as above. The build downloads Kokoro and Parakeet once (about **832 MB**) from
this repository's `models-v1` release, each checked against the size and SHA-256 in
`src/models.rs` before it is unpacked (`tools/fetch-models.sh`), and keeps only the files
`audio-chat-box` reads. Docker's build cache keeps them, so later builds download nothing
unless `src/models.rs` or the script changes. The demo doesn't need Silero VAD, because
recording is started and stopped by hand.

## Limits and deployment challenges

- **Microphones require a secure context.** Browsers generally allow `http://localhost`.
  An ordinary `http://192.168...` page on a LAN does not qualify. Remote access needs HTTPS,
  an explicitly configured `DEMO_ORIGIN`, and a separate authentication/access-control
  design. Do not just publish the port on all interfaces: the commands above bind only
  `127.0.0.1`; it is a single-user local demo, not an internet service.
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
- The capped command allows up to **4 CPUs and 6 GiB RAM**, with a PID limit. These are protective
  demo defaults, **not measured minimums or performance guarantees**. Startup/loading and
  inference may fail under insufficient Docker Desktop memory. CPU latency and emulated
  amd64 performance need measurement on the target machine; no GPU setup is included.
- The Docker build needs network access to Rust crates, the pinned protocol source,
  sherpa-onnx, the `models-v1` release, the base image registry, Debian packages, and
  licence source files.
  Build-time native archive integrity is checked with the same checksum as repository CI.
  The compiler version is pinned; the base-image tags and apt package versions are not
  digest/version-pinned, so this is not a bit-for-bit reproducible supply-chain release.

## Licences and redistribution

The image holds works under three licences, and its `org.opencontainers.image.licenses`
label says so (`GPL-3.0-or-later AND Apache-2.0 AND CC-BY-4.0`):

| In the image | Licence | What the licence asks of a copy, and where the image meets it |
| --- | --- | --- |
| `/usr/local/bin/audio-chat-box` | GPL-3.0-or-later, as a whole, because it links eSpeak NG (README, "Licence") | The licence text (`/usr/share/audio-chat-box/LICENSE-GPL-3.0`), every library's notices (`THIRD-PARTY-NOTICES.txt`, `native-licences/`) and the Corresponding Source (below; `SOURCES.md` says where) |
| `/usr/local/bin/audio-chat-box-demo` | MIT OR Apache-2.0, its crates permissive (`cargo deny`); a separate program that talks to the other over pipes, not linked to it | Its crates' notices (`DEMO-THIRD-PARTY-NOTICES.txt`) and the licence texts (`LICENSE-MIT`, `LICENSE-APACHE`) |
| `/models/kokoro-multi-lang-v1_0` | Kokoro-82M and sherpa-onnx's packaging: Apache-2.0 | The licence text, kept as `LICENSE` in that folder. Neither project ships an Apache NOTICE file, and no kept file is changed |
| `/models/kokoro-multi-lang-v1_0/espeak-ng-data` | eSpeak NG's compiled pronunciation data: GPL-3.0-or-later | The licence text and its source, as for the program above |
| `/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` | Parakeet TDT 0.6B v2 by NVIDIA: CC-BY-4.0 | Attribution: the creator, a link to the licence and to the model, a reference to the licence's disclaimer of warranties, and what was changed (converted to ONNX, quantised, packaged by sherpa-onnx; `bpe.vocab` added). All are in `/models/NOTICE.txt` |

`/models/NOTICE.txt` (`demo-web/MODELS-NOTICE.txt`, also copied to
`/usr/share/audio-chat-box/`) gives each model's source and licence. The image leaves out
the files of each archive that `audio-chat-box` doesn't read: the Chinese and British
English lexicons and rules, a test recording whose source sherpa-onnx doesn't give, and
CppJieba's dictionaries (MIT), so their notices aren't needed. Leaving files out is allowed
by every licence here; nothing kept is changed.

**A public image owes its users the Corresponding Source** of its GPL parts: this
repository at the image's revision (its `project-source.tar.gz`, and the commit its
`org.opencontainers.image.revision` label names), and the source of every library in
sherpa-onnx's prebuilt static library and of eSpeak NG's pronunciation data, with how each was
built. Those are this repository's
[`sources-v1` release](https://github.com/cjus/audio-chat-box/releases/tag/sources-v1), which
[`tools/sources.sh`](../tools/sources.sh) gathers; the image carries its README as
`/usr/share/audio-chat-box/SOURCES.md`. It was put together for sherpa-onnx 1.13.8 on Linux
x86-64, and says how the build of each library, and the eSpeak NG commit that compiled the
data, were found.

## Publishing the image

The image's OCI labels name this repository, its licences and, from the `REVISION` build
argument, the commit it was built from.

A public image gives a GPL-3.0-or-later program, and eSpeak NG's data, to everyone who pulls
it, so each must be able to get their Corresponding Source ("Licences and redistribution"):
publish the image only from a revision whose sherpa-onnx pin has its `sources-v<n>` release.
The models' notices are already in the image.

Build from a commit on `main` that anyone can find, tag it with its version and `latest`
(what `docker pull pnxstudios/audio-chat-box-demo` fetches), then push both:

```sh
docker login -u pnxstudios           # an access token as the password
git checkout main && git pull
docker build --platform linux/amd64 --build-arg REVISION=$(git rev-parse HEAD) \
  -t pnxstudios/audio-chat-box-demo:0.1.0 -t pnxstudios/audio-chat-box-demo:latest .
docker push pnxstudios/audio-chat-box-demo:0.1.0
docker push pnxstudios/audio-chat-box-demo:latest
```

## Verification

Run the engine's existing `CLAUDE.md` checks, then the demo crate's tests/lints and browser
checks. A complete release check should build the image, start it with no network
(`--network none`) and see both models ready, transcribe a synthetic/public-domain fixture,
synthesize and play the response, cancel/repeat recording, deny microphone permission,
disconnect the page, and check non-local origins are rejected.

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
