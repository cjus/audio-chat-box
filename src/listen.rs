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
//!   it started (two windows, 64 ms).
//! - The session adds the listen's `leadInMs` ([`LEAD_IN`] unless it says) more of the sound
//!   from before the start ([`Event::Ended`]'s
//!   `lead_in`), which Parakeet hears with the utterance, so a short first word isn't lost
//!   (below).
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
//! - **A short first word can fall before the start.** The run of `minSpeechMs` that starts
//!   an utterance must be unbroken. A word said fast, then a gap shorter than a pause, as in
//!   "Rook · takes bishop" or "see · four" in a speed game, never makes the run alone: it
//!   starts again at the next word, and the detector's 64 ms reach back miss the first. With
//!   250 ms, a 0.2 s "Rook" and a 150 ms gap came out "takes bishop." (cjus/chessgui#240).
//!   Hence [`LEAD_IN`], and Solrac's shorter `minSpeechMs` (150 ms): of twelve such clips,
//!   4 were heard whole before, all 12 with both. The lead-in never reaches back into the
//!   previous utterance, nor before the listen's first sample.
//! - **The detector doesn't promise that each start has an end**: at the end of the audio,
//!   or when it cuts a long utterance, a start may be left over. The node pairs them itself
//!   (`node.rs`), as the protocol asks.
//!
//! Silero VAD v5 (MIT), by the Silero Team: <https://github.com/snakers4/silero-vad>.
//! sherpa-onnx's voice activity detector:
//! <https://k2-fsa.github.io/sherpa/onnx/vad/index.html>.

use std::collections::VecDeque;
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

/// The shortest speech when the listen doesn't say: a click is shorter. Short enough that a
/// word said fast starts an utterance (cjus/chessgui#240); Solrac's piece and clock sounds
/// started none even at 100 ms.
pub const DEFAULT_MIN_SPEECH: Duration = Duration::from_millis(150);

/// The longest utterance when the listen doesn't say: as long as a recording made with Talk.
pub const DEFAULT_MAX_SPEECH: Duration = Duration::from_secs(30);

/// How much of the sound from before an utterance's start is heard with it when the listen
/// doesn't say (`leadInMs`), so that a short first word the detector's start leaves out is
/// heard (see the [module docs](self)). It is shorter than any pause that ends an utterance
/// Solrac asks for (800 ms), and the lead-in stops at the previous utterance's end anyway.
pub const LEAD_IN: Duration = Duration::from_millis(500);

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
    /// How much of the sound from before an utterance's start it is heard with: none at 0.
    pub lead_in: Duration,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            end_silence: DEFAULT_END_SILENCE,
            min_speech: DEFAULT_MIN_SPEECH,
            max_speech: DEFAULT_MAX_SPEECH,
            lead_in: LEAD_IN,
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
        lead_in: Option<u64>,
    ) -> Result<Settings, String> {
        let defaults = Settings::default();
        let pick = |ms: Option<u64>, default: Duration| ms.map_or(default, Duration::from_millis);
        let settings = Settings {
            end_silence: pick(end_silence, defaults.end_silence),
            min_speech: pick(min_speech, defaults.min_speech),
            max_speech: pick(max_speech, defaults.max_speech),
            lead_in: pick(lead_in, defaults.lead_in),
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
        // No lead-in is allowed: 0 is none.
        if settings.lead_in > MAX_SETTING {
            return Err(format!(
                "leadInMs is from 0 to {} milliseconds",
                MAX_SETTING.as_millis()
            ));
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
        /// Up to the listen's lead-in of the sound just before `start`, to be heard with it: part of
        /// the speech, but not part of where it is said to start.
        lead_in: Vec<f32>,
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
            speaking: false,
            min_speech: samples_in(settings.min_speech),
            recent: Recent::new(samples_in(
                settings.max_speech + BUFFER_MARGIN + settings.lead_in,
            )),
            lead_in: samples_in(settings.lead_in),
            heard_to: 0,
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
    /// The sound heard lately, for each utterance's lead-in.
    recent: Recent,
    /// Where the last utterance ended: no lead-in reaches back past it.
    heard_to: usize,
    /// The listen's lead-in, in samples.
    lead_in: usize,
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
            let lead_in = self.recent.before(start, self.heard_to, self.lead_in);
            self.heard_to = start + samples.len();
            events.push(Event::Ended {
                start,
                samples,
                lead_in,
            });
            self.speaking = false;
        }
        if self.vad.detected() && !self.speaking {
            self.speaking = true;
            let at = self.recent.received().saturating_sub(self.min_speech);
            events.push(Event::Started { at });
        }
        events
    }
}

impl Utterances for Session {
    fn hear(&mut self, samples: &[f32]) -> Vec<Event> {
        self.vad.accept_waveform(samples);
        self.recent.hear(samples);
        self.events()
    }

    fn end(&mut self) -> Vec<Event> {
        self.vad.flush();
        let events = self.events();
        self.speaking = false;
        events
    }
}

/// The last samples a session heard, as many as it keeps: an utterance's lead-in is taken
/// from them once the utterance ends, up to its length plus a pause after its start.
struct Recent {
    /// The samples kept, oldest first.
    samples: VecDeque<f32>,
    /// How many samples have been heard in all.
    received: usize,
    /// How many are kept.
    keep: usize,
}

impl Recent {
    /// Keeps the last `keep` samples heard.
    fn new(keep: usize) -> Recent {
        Recent {
            samples: VecDeque::new(),
            received: 0,
            keep,
        }
    }

    /// Hears the next `samples`, forgetting the oldest past the number kept.
    fn hear(&mut self, samples: &[f32]) {
        self.samples.extend(samples);
        self.received += samples.len();
        let over = self.samples.len().saturating_sub(self.keep);
        self.samples.drain(..over);
    }

    /// How many samples have been heard in all.
    fn received(&self) -> usize {
        self.received
    }

    /// Up to `most` samples just before sample `start`, none before sample `floor`, nor before
    /// the oldest kept.
    fn before(&self, start: usize, floor: usize, most: usize) -> Vec<f32> {
        let oldest = self.received - self.samples.len();
        let start = start.min(self.received);
        let from = start.saturating_sub(most).max(floor).max(oldest);
        if from >= start {
            return Vec::new();
        }
        self.samples
            .range(from - oldest..start - oldest)
            .copied()
            .collect()
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
            Settings::from_millis(None, None, None, None),
            Ok(Settings::default())
        );
        let chosen = Settings::from_millis(Some(600), Some(100), Some(20_000), Some(0)).unwrap();
        assert_eq!(chosen.end_silence, Duration::from_millis(600));
        assert_eq!(chosen.min_speech, Duration::from_millis(100));
        assert_eq!(chosen.max_speech, Duration::from_secs(20));
        // No lead-in at all.
        assert_eq!(chosen.lead_in, Duration::ZERO);
        for wrong in [
            (Some(0), None, None, None),
            (Some(10_001), None, None, None),
            (None, Some(0), None, None),
            (None, None, Some(60_001), None),
            (None, Some(500), Some(400), None),
            (None, None, None, Some(10_001)),
        ] {
            assert!(
                Settings::from_millis(wrong.0, wrong.1, wrong.2, wrong.3).is_err(),
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

    /// The samples 0, 1, 2… as many as `count`, as a session would hear them.
    fn counting(count: usize) -> Vec<f32> {
        (0..count).map(|n| n as f32).collect()
    }

    #[test]
    fn the_lead_in_is_the_sound_just_before_the_start() {
        let mut recent = Recent::new(100);
        recent.hear(&counting(50));
        assert_eq!(recent.before(30, 0, 5), counting(30)[25..].to_vec());
        assert_eq!(recent.received(), 50);
    }

    #[test]
    fn the_lead_in_stops_at_the_last_utterance_and_the_first_sample() {
        let mut recent = Recent::new(100);
        recent.hear(&counting(50));
        assert_eq!(recent.before(30, 28, 5), vec![28.0, 29.0]);
        assert_eq!(recent.before(3, 0, 5), vec![0.0, 1.0, 2.0]);
        assert_eq!(recent.before(0, 0, 5), Vec::<f32>::new());
        assert_eq!(recent.before(30, 30, 5), Vec::<f32>::new());
    }

    #[test]
    fn only_the_samples_kept_can_be_a_lead_in() {
        let mut recent = Recent::new(10);
        for piece in counting(40).chunks(7) {
            recent.hear(piece);
        }
        assert_eq!(recent.received(), 40);
        assert_eq!(recent.before(35, 0, 20), counting(35)[30..].to_vec());
    }

    #[test]
    fn a_model_that_isnt_there_is_named() {
        let folder = std::env::temp_dir().join("audio-chat-box-no-silero");
        let error = Detector::load(&crate::models::SILERO_VAD, &folder)
            .err()
            .unwrap();
        assert!(matches!(error, SpeechError::Missing(_)), "{error}");
    }
}
