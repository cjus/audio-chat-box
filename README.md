# audio-chat-box

Speech for a program that talks with people, on the computer it runs on. audio-chat-box
reads text aloud with [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M), turns speech into
text with [Parakeet TDT 0.6B v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2), and can
listen: the client streams its microphone, [Silero VAD](https://github.com/snakers4/silero-vad)
finds each thing said, and Parakeet hears it once the speaker pauses. It all runs locally,
through [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), with nothing sent anywhere.

It is a program of its own that your program starts and talks to over standard input and
output, one JSON message a line ([`docs/protocol.md`](docs/protocol.md)). It never touches
a microphone or a speaker: your program records and plays. It knows nothing of your
product's domain: your program tells it which words to expect (hotwords), and it favours
them where the sound allows, "knight" over "night" for a chess board, your product's names
for yours.

## Running it

```sh
cargo build --release
target/release/audio-chat-box --models <folder>
```

| Option | |
| --- | --- |
| `--models <folder>` | Where the models are, and are downloaded to the first time: Kokoro 350 MB, Parakeet 482 MB, Silero VAD 2 MB, each checked against its SHA-256 |
| `--models-url <url>` | Where the models are downloaded from. The default is this repository's [`models-v1` release](https://github.com/cjus/audio-chat-box/releases/tag/models-v1); a product can name its own copy of the same files |
| `--speak <model>` | `kokoro` (the default), `kokoro-int8`, or `none` |
| `--hear <model>` | `parakeet` (the default), or `none` |
| `--listen <model>` | `silero-vad`, or `none` (the default): listen to speech as it comes (`voice/listen`), heard with the `--hear` model |
| `--offline` | Never download: a model that isn't in the folder fails |

It then reads messages, one JSON line each, and answers:

```text
→ {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": "1.0", "client": {"name": "me", "version": "1"}}}
← {"jsonrpc": "2.0", "id": 1, "result": {"protocol": "1.0", "node": {"name": "audio-chat-box", …}, "capabilities": {"voice": {"speak": true, "transcribe": true, "hotwords": true, "formats": ["wav", "pcmF32"]}}, …}}
← {"jsonrpc": "2.0", "method": "voice/modelState", "params": {"id": "kokoro", "state": {"stage": "ready"}}}
→ {"jsonrpc": "2.0", "id": 2, "method": "voice/speak", "params": {"text": "Your table is ready.", "voice": "am_michael"}}
← {"jsonrpc": "2.0", "id": 2, "result": {"format": "wav", "sampleRate": 24000, "audio": "UklGR…", "durationMs": 1150}}
```

`voice/voices` lists Kokoro's 54 voices, `voice/models` the models and how ready each is, and
`voice/transcribe` hears a WAV file or raw samples, with the hotwords given. On an M1 Max
both models load in about a second, and a sentence is read in under one.

`voice/listen` takes 16 kHz raw samples in `voice/audio` notifications, as they come from the
microphone, and says `voice/speechStarted` when someone starts speaking and `voice/heard` with
what they said once they pause (800 ms unless the listen says otherwise). One listen at a time.
[`docs/protocol.md`](docs/protocol.md) has every message.

`cargo run --release --example say`, `hear`, `listen` and `bakeoff` read text aloud to a WAV
file, hear WAV files, listen to WAV files as a stream, and compare transcription models on
recordings of your own, for measuring.

## The models

The models are downloaded once, the first time each is needed, and each archive is checked
against the size and SHA-256 built into the program (`src/models.rs`) before it is unpacked.
By default they come from this repository's
[`models-v1` release](https://github.com/cjus/audio-chat-box/releases/tag/models-v1): copies
of sherpa-onnx's archives, unchanged, and Silero VAD packed the same way, with a
`NOTICE.txt` of their licences. A product may host the same files itself and pass
`--models-url`; a file that differs from the one built in is refused. [CREDITS.md](CREDITS.md)
says where each came from.

## Origins

audio-chat-box began as `solrac-voice`, the voice of [Solrac](https://github.com/cjus/solrac-chess),
a 3D chess board that teaches, made in its repository `cjus/chessgui`. Solrac's chat reads its
replies aloud and hears its player, by Talk or hands-free, and this program grew out of that
work: its models were chosen in Solrac's bake-off of speech recognisers, its listening was
tuned on a player's moves said at speed-chess pace, and its protocol is the voice domain of
the [Chess Node Protocol](https://github.com/cjus/chess-node-protocol), which Solrac speaks to
all its modules. The comments still cite those projects' issues (`cjus/chessgui#…`), as the
history of why each choice was made. It was renamed in October 2026, once what was chess in
it, the words it favoured, had moved to the client, so that any product can use it.

## Licence

Our own code is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as
above, without any additional terms or conditions.

The `audio-chat-box` program as built links sherpa-onnx's prebuilt library, which includes
[eSpeak NG](https://github.com/espeak-ng/espeak-ng) (GPL-3.0-or-later): Kokoro uses it to sound
out words its lexicon doesn't have. So the program as a whole is
[GPL-3.0-or-later](LICENSE-GPL-3.0), and its source is this repository. The models have licences
of their own: Kokoro Apache-2.0, Parakeet CC-BY-4.0 (by NVIDIA), Silero VAD MIT.

Every copy of the program goes with the notices of what it is built with:
`tools/notices.sh` writes them to `target/licences/THIRD-PARTY-NOTICES.txt`. They are the
crates' licences (with cargo-about 0.9: `cargo install --locked cargo-about@0.9.2 --features
cli`), then those of the C and C++ libraries in sherpa-onnx's prebuilt library, which the
archive doesn't carry, so they are kept in [`licences/`](licences/README.md) with where each
came from.
