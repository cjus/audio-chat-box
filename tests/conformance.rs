//! The Chess Node Protocol: every conformance session from the protocol's repository, played
//! against the real `solrac-voice` program.
//!
//! The sessions come with the `cnp-conformance` crate ([`cnp_conformance::SESSIONS`]), so a
//! new session there is checked here as soon as the protocol's pin is bumped. The node runs
//! `--offline` with an empty models folder: no session needs a model to be ready, and no test
//! downloads one. It listens (`--listen`), so the listening sessions apply. The engine's sessions are for engine nodes, and are left out.

use std::path::PathBuf;
use std::time::Duration;

use cnp_conformance::runner::{Outcome, capabilities, missing, run_session};
use cnp_conformance::{SESSIONS, session};

/// How long one step of a session may wait. Generous: the tests run unoptimised builds.
const STEP_TIMEOUT: Duration = Duration::from_secs(60);

/// The program, offline, listening, with a models folder that doesn't exist.
fn node() -> Vec<String> {
    let models =
        std::env::temp_dir().join(format!("solrac-voice-conformance-{}", std::process::id()));
    vec![
        env!("CARGO_BIN_EXE_solrac-voice").to_string(),
        "--offline".to_string(),
        "--models".to_string(),
        models.to_string_lossy().into_owned(),
        "--listen".to_string(),
        "silero-vad".to_string(),
    ]
}

#[test]
fn solrac_voice_passes_every_conformance_session_that_applies() {
    let command = node();
    let mut sessions = session::load(&[PathBuf::from(SESSIONS)]).expect("the sessions load");
    assert!(sessions.len() >= 40, "only {} sessions", sessions.len());
    for session in &mut sessions {
        session.timeout = session.timeout.max(STEP_TIMEOUT);
    }
    let offered = capabilities(&command, STEP_TIMEOUT).expect("solrac-voice answers initialize");
    let (mut passed, mut failed, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for session in &sessions {
        let needs = missing(session, &offered);
        if !needs.is_empty() {
            // A voice node: the engine's sessions are for engines.
            if !needs.iter().any(|need| need == "engine") {
                skipped.push(format!("{} (needs {})", session.name, needs.join(", ")));
            }
            continue;
        }
        match run_session(session, &command) {
            Outcome::Failed(why) => failed.push(format!("{}: {why}", session.path.display())),
            _ => passed.push(session.name.clone()),
        }
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n\n"));
    assert!(skipped.is_empty(), "skipped: {skipped:?}");
    // The voice's own sessions ran.
    for voice in [
        "voices",
        "models",
        "speak-unknown-voice",
        "transcribe-bad-audio",
        "listen-bad-params",
        "audio-for-no-listen",
    ] {
        assert!(
            passed.iter().any(|name| name == voice),
            "{voice} didn't run"
        );
    }
}
