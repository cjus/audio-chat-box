# solrac-voice

The voice of [Solrac](https://github.com/cjus/solrac-chess), as a program of its own. It reads
text aloud with [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M) and turns speech into text
with [Parakeet TDT 0.6B v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2), on the computer
it runs on, through [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx). It answers the
[Chess Node Protocol](https://github.com/cjus/chess-node-protocol)'s voice domain (CNP, spec
section 8) over its standard input and output. It never touches a microphone or a speaker:
the client records and plays.

Solrac starts it and talks to it. Any other program that speaks CNP can too.

## Running it

```sh
cargo build --release
target/release/solrac-voice --models <folder>
```

| Option | |
| --- | --- |
| `--models <folder>` | Where the models are, and are downloaded to the first time: Kokoro 350 MB, Parakeet 482 MB, from the PNX Studios CDN, each checked against its SHA-256. Solrac passes its own (`~/Library/Application Support/Solrac/models` on macOS) |
| `--speak <model>` | `kokoro` (the default), `kokoro-int8`, or `none` |
| `--hear <model>` | `parakeet` (the default), or `none` |
| `--offline` | Never download: a model that isn't in the folder fails |

It then reads CNP messages, one JSON line each, and answers:

```text
→ {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": "1.0", "client": {"name": "me", "version": "1"}}}
← {"jsonrpc": "2.0", "id": 1, "result": {"protocol": "1.0", "node": {"name": "solrac-voice", …}, "capabilities": {"voice": {"speak": true, "transcribe": true, "formats": ["wav", "pcmF32"]}}, …}}
← {"jsonrpc": "2.0", "method": "voice/modelState", "params": {"id": "kokoro", "state": {"stage": "ready"}}}
→ {"jsonrpc": "2.0", "id": 2, "method": "voice/speak", "params": {"text": "Knight to f3.", "voice": "am_michael"}}
← {"jsonrpc": "2.0", "id": 2, "result": {"format": "wav", "sampleRate": 24000, "audio": "UklGR…", "durationMs": 1150}}
```

`voice/voices` lists Kokoro's 54 voices, `voice/models` the models and how ready each is, and
`voice/transcribe` hears a WAV file or raw samples. On the M1 Max both models load in about a
second, and a sentence is read in under one.

`cargo run --release --example say`, `hear` and `bakeoff` read text aloud to a WAV file, hear
WAV files, and compare transcription models, for measuring. `tools/bakeoff/make-clips.sh` makes
the bake-off's recordings (macOS).

## Licence

Our own code is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as
above, without any additional terms or conditions.

The `solrac-voice` program as built links sherpa-onnx's prebuilt library, which includes
[eSpeak NG](https://github.com/espeak-ng/espeak-ng) (GPL-3.0-or-later): Kokoro uses it to sound
out words its lexicon doesn't have. So the program as a whole is
[GPL-3.0-or-later](LICENSE-GPL-3.0), and its source is this repository. The models have licences
of their own: Kokoro Apache-2.0, Parakeet CC-BY-4.0 (by NVIDIA).

Every copy of the program goes with the notices of what it is built with:
`tools/notices.sh` writes them to `target/licences/THIRD-PARTY-NOTICES.txt`. They are the
crates' licences (with cargo-about 0.9: `cargo install --locked cargo-about@0.9.2 --features
cli`), then those of the C and C++ libraries in sherpa-onnx's prebuilt library, which the
archive doesn't carry, so they are kept in [`licences/`](licences/README.md) with where each
came from.
