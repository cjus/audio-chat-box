# audio-chat-box — instructions for Claude

`audio-chat-box` is speech for programs that talk with people, as a program of its own: Kokoro
reads text aloud, Parakeet hears speech, Silero VAD finds speech as it streams in, through
sherpa-onnx, over standard input and output (`docs/protocol.md`, the Chess Node Protocol's voice
domain). It began as the voice of Solrac and is meant for any product (README, "Origins").

A maintainer's own instructions, if any, are in `CLAUDE.local.md`, which is not committed.

## Rules

1. **It is a separate program, and stays one.** It links eSpeak NG (GPL-3.0-or-later) through
   sherpa-onnx, so a closed-source client must never link it: the client starts it and talks
   to it. Nothing here may become a library a client depends on.
2. **Nothing of a client's domain.** No chess (or any product's) words, phrases or tuning
   are built in: a client gives its own hotwords and listen settings in its requests. Its
   recordings and phrase lists stay in its own repository.
3. **Our code is `MIT OR Apache-2.0`**; every crate it depends on is permissive (`deny.toml`).
   The program as a whole is GPL-3.0-or-later (README, "Licence"). Every copy of it goes with
   `THIRD-PARTY-NOTICES.txt` (`tools/notices.sh`): the crates' licences, and those of the C and
   C++ libraries in sherpa-onnx's prebuilt library, kept in `licences/`, and can lead to their
   source, the `sources-v<n>` release (`tools/sources.sh`). Bumping the sherpa-onnx pin means
   looking at those libraries again and a new sources release (`licences/README.md`).
4. **Nothing but protocol messages on standard output** (spec, 3.1). Logs go to standard error.
5. **The protocol is pinned** to a commit (`rev`) of `cjus/chess-node-protocol` in `Cargo.toml`,
   for both `cnp` and `cnp-conformance`. `tests/conformance.rs` runs every session that applies
   to a voice node against the real program; a new session there is checked when the pin moves.
6. **Models are checked.** Each model's archive size and SHA-256 are in `src/models.rs`; a
   download that differs is deleted, never unpacked. The default source is this repository's
   `models-v1` release; a new or changed archive goes there, unchanged, before the program
   names it. `docs/protocol.md` changes with the protocol (CNP's voice domain), in the same
   PR as the pin.
7. **Every public item has rustdoc** (`missing_docs = "deny"`). Module docs say what the part
   does, why, and its pitfalls. No `unsafe` (`unsafe_code = "forbid"`). Named constants, not
   magic numbers.

## Definition of done for a change

1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
   `RUSTDOCFLAGS=-D\ warnings cargo doc --no-deps --document-private-items` and
   `cargo deny check` pass.
2. Work is on a branch from the latest `main`, with a PR to `main` and green CI (one check,
   `CI`). CI checks only PRs that are ready for review: open a PR as a draft and mark it ready
   when its work is done.

## Trying it with the real models

The tests never download a model (the conformance sessions run `--offline`). To hear and
speak for real, point it at a folder for the models (downloaded there the first time):
`target/debug/audio-chat-box --models <folder>`, then send it `initialize`, wait for both
models' `ready`, and send `voice/speak` and `voice/transcribe`. To listen, add
`--listen silero-vad` and stream audio to a `voice/listen`;
`cargo run --release --example listen -- <models folder> <file.wav>...` hears WAV files the same
way without the protocol.
