//! # solrac-voice: Solrac's voice, as a program of its own
//!
//! Solrac's chat hears the player and reads its replies aloud. This program does both on the
//! computer it runs on, with sherpa-onnx on the CPU, and answers the Chess Node Protocol's
//! voice domain (CNP, spec section 8) over its standard input and output:
//!
//! - [`speak`]: Kokoro v1.0 reads text aloud, in any of its voices.
//! - [`hear`]: Parakeet TDT 0.6B v2 turns speech into text, with chess words favoured.
//! - [`models`]: the models, downloaded once from the PNX Studios CDN, checked and unpacked.
//! - [`node`]: the CNP node. Requests come in, and answers and the models' progress go out.
//!
//! It never touches a microphone or a speaker: the client records and plays. Solrac starts it,
//! hands it the folder its models live in, and asks it for speech and transcripts.
//!
//! ## Why a program of its own
//!
//! sherpa-onnx's library includes eSpeak NG (GPL-3.0-or-later), which Kokoro uses to sound out
//! words its lexicon doesn't have. Linked into Solrac, it would make Solrac GPL. As a separate
//! program that Solrac starts and talks to through a public protocol, it doesn't
//! (cjus/chessgui#71; the design is `docs/protocol.md` there). Our own code here is
//! `MIT OR Apache-2.0`; the program as built is GPL-3.0-or-later.
//!
//! The code came from Solrac's `chessgui-speech` and `chessgui-agent`'s download
//! (cjus/chessgui#59, #69, #92), in step 6 of M1b (cjus/chessgui#98).
//!
//! sherpa-onnx (Apache-2.0): <https://github.com/k2-fsa/sherpa-onnx>. Kokoro (Apache-2.0):
//! <https://huggingface.co/hexgrad/Kokoro-82M>. Parakeet TDT 0.6B v2 (CC-BY-4.0):
//! <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2>. eSpeak NG (GPL-3.0-or-later):
//! <https://github.com/espeak-ng/espeak-ng>.

#![forbid(unsafe_code)]

pub mod hear;
pub mod models;
pub mod node;
pub mod speak;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Speech: mono samples from -1 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Samples {
    /// Samples per second.
    pub rate: u32,
    /// The samples.
    pub samples: Vec<f32>,
}

impl Samples {
    /// How long the speech lasts.
    pub fn duration(&self) -> Duration {
        if self.rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.rate))
    }
}

/// Why speech couldn't be made or heard. The messages are for people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechError {
    /// A file of the model is missing.
    Missing(PathBuf),
    /// sherpa-onnx couldn't load the model.
    WontLoad,
    /// sherpa-onnx made no sound.
    Failed,
    /// It was stopped.
    Stopped,
    /// The audio can't be read.
    Unreadable,
    /// sherpa-onnx gave no transcript.
    Unheard,
}

impl fmt::Display for SpeechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpeechError::Missing(path) => write!(f, "{} is missing", path.display()),
            SpeechError::WontLoad => write!(f, "the model wouldn't load"),
            SpeechError::Failed => write!(f, "the speech model made no sound"),
            SpeechError::Stopped => write!(f, "stopped"),
            SpeechError::Unreadable => write!(f, "the audio can't be read"),
            SpeechError::Unheard => write!(f, "the transcription model gave no transcript"),
        }
    }
}

impl std::error::Error for SpeechError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_know_how_long_they_last() {
        let samples = Samples {
            rate: 24_000,
            samples: vec![0.0; 36_000],
        };
        assert_eq!(samples.duration(), Duration::from_millis(1500));
        let none = Samples {
            rate: 0,
            samples: vec![0.0; 10],
        };
        assert_eq!(none.duration(), Duration::ZERO);
    }
}
