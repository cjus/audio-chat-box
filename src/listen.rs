//! Listening: speech heard as it comes (`voice/listen`, spec 8.5; cjus/chessgui#134).
//!
//! Solrac's hands-free voice keeps the microphone open and streams what it hears. The node
//! has to say when the player starts speaking and, once they pause, what they said. A
//! [`Detector`] finds the speech, with Silero VAD ([`SILERO_VAD`](crate::models::SILERO_VAD))
//! run by sherpa-onnx; the node then hears each utterance with Parakeet, as it hears a
//! recording ([`hear`](crate::hear)).
//!
//! ## How it works
//!
//! Silero VAD is a small neural network (about 2 MB) that reads 32 ms of 16 kHz sound at a
//! time (a window of [`WINDOW`] samples) and gives the chance that it is speech. sherpa-onnx's
//! voice activity detector turns those chances into utterances:
//!
//! - Speech starts once the chance has stayed above [`THRESHOLD`] for the listen's
//!   `minSpeechMs`: a cough or a piece put down on the board is shorter, and is no utterance.
//! - It ends once the chance has stayed below the threshold for `endSilenceMs`, a pause. The
//!   detector then hands over the utterance's samples, with a little of the sound from before
//!   it started, so its first syllable isn't cut.
//! - An utterance that reaches `maxSpeechMs` is cut there, pause or not.
//!
//! A [`Session`] is one listen's detector: created with the listen's settings, it is fed
//! the samples as they come ([`Utterances::hear`]) and says what happened in them
//! ([`Event`]), and at the end of the audio it ends the utterance it is hearing
//! ([`Utterances::end`]).
//!
//! ## Pitfalls
//!
//! - **16 kHz only.** sherpa-onnx's detector reads 16 kHz, and Solrac sends it. The node
//!   refuses other rates rather than resample.
//! - **The pause is the cost.** Letting go of Talk ended a message at once; here a message
//!   ends only after `endSilenceMs` of silence. Shorter cuts "knight to… f3" in two, longer
//!   makes every reply slower (cjus/chessgui#134, phase 2).
//! - **"Started" comes before its samples do.** The detector says speech has started as soon
//!   as it is sure, but where exactly it started is known only with the utterance, so
//!   [`Event::Started`]'s place is about right, and [`Event::Ended`]'s exact.
//! - **The detector doesn't promise that each start has an end**: at the end of the audio,
//!   or when it cuts a long utterance, a start may be left over. The node pairs them itself
//!   (`node.rs`), as the protocol asks.
//!
//! Silero VAD v5 (MIT), by the Silero Team: <https://github.com/snakers4/silero-vad>.
//! sherpa-onnx's voice activity detector:
//! <https://k2-fsa.github.io/sherpa/onnx/vad/index.html>.

use std::path::Path;
use std::time::Duration;

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

use crate::SpeechError;
use crate::models::LocalModel;

/// The sample rate the detector reads, and the only one a listen takes.
pub const SAMPLE_RATE: u32 = 16_000;

/// How many samples Silero VAD v5 reads at a time at 16 kHz: 32 ms.
pub const WINDOW: i32 = 512;

/// The chance of speech above which a window counts as speech: Silero's own default.
pub const THRESHOLD: f32 = 0.5;

/// How long a pause ends an utterance when the listen doesn't say: Solrac's own choice too
/// (cjus/chessgui#134).
pub const DEFAULT_END_SILENCE: Duration = Duration::from_millis(800);

/// The shortest speech when the listen doesn't say: a cough or a click is shorter.
pub const DEFAULT_MIN_SPEECH: Duration = Duration::from_millis(250);

/// The longest utterance when the listen doesn't say: as long as a recording made with Talk.
pub const DEFAULT_MAX_SPEECH: Duration = Duration::from_secs(30);

/// The longest pause or shortest speech a listen may ask for: past this, nobody is talking
/// to the board.
pub const MAX_SETTING: Duration = Duration::from_secs(10);

/// The longest utterance a listen may ask for. The detector keeps an utterance's samples
/// until it ends, so this bounds its memory: a minute of 16 kHz samples is under 4 MB.
pub const MAX_SPEECH: Duration = Duration::from_secs(60);

/// How much more than the longest utterance the detector's buffer holds: the sound kept from
/// before an utterance starts, and the pause after it.
const BUFFER_MARGIN: Duration = Duration::from_secs(MAX_SETTING.as_secs() + 2);

/// The threads ONNX Runtime uses for the detector: one window at a time is far less than a
/// millisecond's work.
const THREADS: i32 = 1;

/// How one listen hears: its settings, checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How long a pause ends an utterance.
    pub end_silence: Duration,
    /// Shorter sounds aren't speech.
    pub min_speech: Duration,
    /// The longest an utterance may be.
    pub max_speech: Duration,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            end_silence: DEFAULT_END_SILENCE,
            min_speech: DEFAULT_MIN_SPEECH,
            max_speech: DEFAULT_MAX_SPEECH,
        }
    }
}

impl Settings {
    /// A listen's settings in milliseconds, each `None` for the default, or why they can't be
    /// kept to (for an `InvalidParams` error).
    pub fn from_millis(
        end_silence: Option<u64>,
        min_speech: Option<u64>,
        max_speech: Option<u64>,
    ) -> Result<Settings, String> {
        let defaults = Settings::default();
        let pick = |ms: Option<u64>, default: Duration| ms.map_or(default, Duration::from_millis);
        let settings = Settings {
            end_silence: pick(end_silence, defaults.end_silence),
            min_speech: pick(min_speech, defaults.min_speech),
            max_speech: pick(max_speech, defaults.max_speech),
        };
        let checks = [
            ("endSilenceMs", settings.end_silence, MAX_SETTING),
            ("minSpeechMs", settings.min_speech, MAX_SETTING),
            ("maxSpeechMs", settings.max_speech, MAX_SPEECH),
        ];
        for (name, value, most) in checks {
            if value.is_zero() || value > most {
                return Err(format!(
                    "{name} is from 1 to {} milliseconds",
                    most.as_millis()
                ));
            }
        }
        if settings.max_speech <= settings.min_speech {
            return Err("maxSpeechMs must be longer than minSpeechMs".to_string());
        }
        Ok(settings)
    }
}

/// What a session heard in the samples it was given.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Someone has started speaking, at about this sample.
    Started {
        /// About where, in samples from the session's first.
        at: usize,
    },
    /// An utterance has ended: its samples, at [`SAMPLE_RATE`].
    Ended {
        /// Where it starts, in samples from the session's first.
        start: usize,
        /// Its samples.
        samples: Vec<f32>,
    },
}

/// Starts listens: the [`Detector`], or a fake in the node's tests.
pub trait Detect: Send + Sync {
    /// A session for one listen, hearing with `settings`.
    fn start(&self, settings: &Settings) -> Result<Box<dyn Utterances>, SpeechError>;
}

/// One listen's detector: fed samples, it says what it heard in them.
pub trait Utterances: Send {
    /// Hears the next `samples`, at [`SAMPLE_RATE`].
    fn hear(&mut self, samples: &[f32]) -> Vec<Event>;
    /// The audio has ended: ends the utterance being heard, if any.
    fn end(&mut self) -> Vec<Event>;
}

/// Finds speech with Silero VAD (see the [module docs](self)).
pub struct Detector {
    /// The model's file.
    model: String,
}

impl Detector {
    /// Loads `model` from `folder`, its unpacked archive: makes one session, so that a model
    /// that won't load fails now rather than at the first listen.
    pub fn load(model: &LocalModel, folder: &Path) -> Result<Detector, SpeechError> {
        let path = folder.join(model.model_file);
        if !path.exists() {
            return Err(SpeechError::Missing(path));
        }
        let detector = Detector {
            model: path.to_string_lossy().into_owned(),
        };
        detector.session(&Settings::default())?;
        Ok(detector)
    }

    /// A session hearing with `settings`.
    pub fn session(&self, settings: &Settings) -> Result<Session, SpeechError> {
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(self.model.clone()),
                threshold: THRESHOLD,
                min_silence_duration: settings.end_silence.as_secs_f32(),
                min_speech_duration: settings.min_speech.as_secs_f32(),
                window_size: WINDOW,
                max_speech_duration: settings.max_speech.as_secs_f32(),
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: THREADS,
            ..VadModelConfig::default()
        };
        let buffer = (settings.max_speech + BUFFER_MARGIN).as_secs_f32();
        let vad = VoiceActivityDetector::create(&config, buffer).ok_or(SpeechError::WontLoad)?;
        Ok(Session {
            vad,
            received: 0,
            speaking: false,
            min_speech: samples_in(settings.min_speech),
        })
    }
}

impl Detect for Detector {
    fn start(&self, settings: &Settings) -> Result<Box<dyn Utterances>, SpeechError> {
        Ok(Box::new(self.session(settings)?))
    }
}

/// One listen's detector (see the [module docs](self)).
pub struct Session {
    vad: VoiceActivityDetector,
    /// Samples heard so far.
    received: usize,
    /// Whether it has said that speech started, and not yet that it ended.
    speaking: bool,
    /// The listen's shortest speech, in samples: speech has gone on this long when the
    /// detector notices it.
    min_speech: usize,
}

impl Session {
    /// The utterances the detector has finished, then whether speech has started.
    fn events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Some(segment) = self.vad.front() {
            let start = usize::try_from(segment.start()).unwrap_or(0);
            let samples = segment.samples().to_vec();
            self.vad.pop();
            events.push(Event::Ended { start, samples });
            self.speaking = false;
        }
        if self.vad.detected() && !self.speaking {
            self.speaking = true;
            let at = self.received.saturating_sub(self.min_speech);
            events.push(Event::Started { at });
        }
        events
    }
}

impl Utterances for Session {
    fn hear(&mut self, samples: &[f32]) -> Vec<Event> {
        self.vad.accept_waveform(samples);
        self.received += samples.len();
        self.events()
    }

    fn end(&mut self) -> Vec<Event> {
        self.vad.flush();
        let events = self.events();
        self.speaking = false;
        events
    }
}

/// How many samples at [`SAMPLE_RATE`] last `duration`.
pub fn samples_in(duration: Duration) -> usize {
    (duration.as_secs_f64() * f64::from(SAMPLE_RATE)).round() as usize
}

/// How long `samples` samples at [`SAMPLE_RATE`] last, in whole milliseconds.
pub fn millis_of(samples: usize) -> u64 {
    samples as u64 * 1000 / u64::from(SAMPLE_RATE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_are_the_defaults_and_wrong_ones_are_refused() {
        assert_eq!(
            Settings::from_millis(None, None, None),
            Ok(Settings::default())
        );
        let chosen = Settings::from_millis(Some(600), Some(100), Some(20_000)).unwrap();
        assert_eq!(chosen.end_silence, Duration::from_millis(600));
        assert_eq!(chosen.min_speech, Duration::from_millis(100));
        assert_eq!(chosen.max_speech, Duration::from_secs(20));
        for wrong in [
            (Some(0), None, None),
            (Some(10_001), None, None),
            (None, Some(0), None),
            (None, None, Some(60_001)),
            (None, Some(500), Some(400)),
        ] {
            assert!(
                Settings::from_millis(wrong.0, wrong.1, wrong.2).is_err(),
                "{wrong:?}"
            );
        }
    }

    #[test]
    fn samples_and_milliseconds_at_16_khz() {
        assert_eq!(samples_in(Duration::from_millis(250)), 4000);
        assert_eq!(millis_of(16_000), 1000);
        assert_eq!(millis_of(8), 0);
    }

    #[test]
    fn a_model_that_isnt_there_is_named() {
        let folder = std::env::temp_dir().join("solrac-voice-no-silero");
        let error = Detector::load(&crate::models::SILERO_VAD, &folder)
            .err()
            .unwrap();
        assert!(matches!(error, SpeechError::Missing(_)), "{error}");
    }
}
