//! # audio-chat-box: speech for programs that talk with people
//!
//! A program that talks with people reads its replies aloud and hears what they say. This
//! program does both on the computer it runs on, with sherpa-onnx on the CPU, and answers its
//! client over its standard input and output (`docs/protocol.md`, the Chess Node Protocol's
//! voice domain):
//!
//! - [`speak`]: Kokoro v1.0 reads text aloud, in any of its voices.
//! - [`hear`]: Parakeet TDT 0.6B v2 turns speech into text, with the client's hotwords
//!   favoured.
//! - [`listen`]: Silero VAD finds speech in audio as it comes, for a client that keeps its
//!   microphone open (cjus/chessgui#134): the node hears each utterance once the speaker
//!   pauses.
//! - [`models`]: the models, downloaded once, checked and unpacked.
//! - [`node`]: the node. Requests come in, and answers and the models' progress go out.
//!
//! It never touches a microphone or a speaker: the client records and plays. The client
//! starts it, hands it the folder its models live in, and asks it for speech and transcripts.
//! It knows nothing of the client's domain: the client says which words to expect.
//!
//! ## Why a program of its own
//!
//! sherpa-onnx's library includes eSpeak NG (GPL-3.0-or-later), which Kokoro uses to sound out
//! words its lexicon doesn't have. Linked into a closed-source program, it would make that
//! program GPL. As a separate program that the client starts and talks to through a public
//! protocol, it doesn't (cjus/chessgui#71). Our own code here is `MIT OR Apache-2.0`; the
//! program as built is GPL-3.0-or-later.
//!
//! ## Origins
//!
//! It began as `solrac-voice`, the voice of Solrac, a chess board (`cjus/chessgui`): its code
//! came from Solrac's `chessgui-speech` and `chessgui-agent`'s download (cjus/chessgui#59,
//! #69, #92), in step 6 of M1b (cjus/chessgui#98). The comments cite those issues as the
//! history of each choice. It became audio-chat-box once the chess words it favoured moved
//! to the client (cjus/chessgui#226), so that any program can use it.
//!
//! sherpa-onnx (Apache-2.0): <https://github.com/k2-fsa/sherpa-onnx>. Kokoro (Apache-2.0):
//! <https://huggingface.co/hexgrad/Kokoro-82M>. Parakeet TDT 0.6B v2 (CC-BY-4.0):
//! <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2>. Silero VAD (MIT):
//! <https://github.com/snakers4/silero-vad>. eSpeak NG (GPL-3.0-or-later):
//! <https://github.com/espeak-ng/espeak-ng>.

#![forbid(unsafe_code)]

pub mod hear;
pub mod listen;
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
