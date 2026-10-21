# solrac-voice — instructions for Claude

`solrac-voice` (repo `cjus/solrac-voice`) is Solrac's voice as a program of its own: Kokoro
reads text aloud, Parakeet hears speech, through sherpa-onnx, behind the Chess Node Protocol's
voice domain over standard input and output. It is step 6 of Solrac's milestone M1b
(`cjus/chessgui`, tracked in cjus/chessgui#98; the design is `docs/protocol.md` there). Its client
is Solrac. Owner: Carlos.

## Rules

1. **It is a separate program, and stays one.** It links eSpeak NG (GPL-3.0-or-later) through
   sherpa-onnx, so Solrac must never link it: Solrac starts it and talks CNP to it. Nothing here
   may become a library Solrac depends on.
2. **Our code is `MIT OR Apache-2.0`**; every crate it depends on is permissive (`deny.toml`).
   The program as a whole is GPL-3.0-or-later (README, "Licence").
3. **Nothing but CNP on standard output** (spec, 3.1). Logs go to standard error.
4. **The protocol is pinned** to a commit (`rev`) of `cjus/chess-node-protocol` in `Cargo.toml`,
   for both `cnp` and `cnp-conformance`. `tests/conformance.rs` runs every session that applies
   to a voice node against the real program; a new session there is checked when the pin moves.
5. **Models are checked.** Each model's archive size and SHA-256 are in `src/models.rs`; a
   download that differs is deleted, never unpacked.
6. **Every public item has rustdoc** (`missing_docs = "deny"`). Module docs say what the part
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
`target/debug/solrac-voice --models "$HOME/Library/Application Support/Solrac/models"`, then
send it `initialize`, wait for both models' `ready`, and send `voice/speak` and
`voice/transcribe`.

## Items for Carlos go in GitHub issues

Anything Carlos needs to do, decide or answer is an issue assigned to `cjus` with the
`needs-carlos` label, in `cjus/chessgui`, saying why it matters, the exact steps, the expected
result, and "Done when".
