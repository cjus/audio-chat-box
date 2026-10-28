# audio-chat-box — instructions for Claude

`audio-chat-box` (repo `cjus/audio-chat-box`, formerly `cjus/solrac-voice`) is speech for
programs that talk with people, as a program of its own: Kokoro reads text aloud, Parakeet
hears speech, Silero VAD finds speech as it streams in, through sherpa-onnx, over standard
input and output (`docs/protocol.md`, the Chess Node Protocol's voice domain). It began as
Solrac's voice (`cjus/chessgui`, M1b step 6, cjus/chessgui#98), and Solrac is its first
client; it is meant for any product (README, "Origins"). Owner: Carlos.

## Rules

1. **It is a separate program, and stays one.** It links eSpeak NG (GPL-3.0-or-later) through
   sherpa-onnx, so a closed-source client such as Solrac must never link it: the client starts
   it and talks to it. Nothing here may become a library Solrac depends on.
2. **Nothing of a client's domain.** No chess (or any product's) words, phrases or tuning
   are built in: a client gives its own hotwords and listen settings in its requests. Its
   recordings and phrase lists stay in its own repository (Solrac's in
   `cjus/chessgui`, `tools/bakeoff/`).
3. **Our code is `MIT OR Apache-2.0`**; every crate it depends on is permissive (`deny.toml`).
   The program as a whole is GPL-3.0-or-later (README, "Licence"). Every copy of it goes with
   `THIRD-PARTY-NOTICES.txt` (`tools/notices.sh`): the crates' licences, and those of the C and
   C++ libraries in sherpa-onnx's prebuilt library, kept in `licences/`. Bumping the
   sherpa-onnx pin means looking at those libraries again (`licences/README.md`).
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
2. Work is on a `claude/<topic>` branch from the latest `main`, with a PR to `main` and green
   CI. Open PRs as drafts and mark them ready when their work is done: CI checks only PRs that
   are ready (Actions minutes are limited, cjus/chessgui#73).
3. **Claude may merge its PRs to `main`** (granted by Carlos 2026-10-21 for M1b's work) once CI
   (one check, `CI`) has passed on the latest commit (not a draft's skipped run), the PR merges
   cleanly and no review comment is unanswered. Use a merge commit, not a squash.

## Trying it with the real models

The tests never download a model (the conformance sessions run `--offline`). To hear and
speak for real, point it at a folder with the models, such as Solrac's:
`target/debug/audio-chat-box --models "$HOME/Library/Application Support/Solrac/models"`, then
send it `initialize`, wait for both models' `ready`, and send `voice/speak` and
`voice/transcribe`. To listen, add `--listen silero-vad` and stream audio to a `voice/listen`;
`cargo run --release --example listen -- <models folder> <file.wav>...` hears WAV files the same
way without the protocol.

## Items for Carlos go in GitHub issues

Anything Carlos needs to do, decide or answer is an issue assigned to `cjus` with the
`needs-carlos` label, in `cjus/chessgui`, saying why it matters, the exact steps, the expected
result, and "Done when".
