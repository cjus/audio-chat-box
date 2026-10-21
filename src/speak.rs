//! Speaking: Kokoro v1.0 reads text aloud (`voice/speak`, spec 8.2).
//!
//! A [`Speaker`] holds Kokoro, loaded once, and reads any text in any of its voices
//! ([`VOICES`]) at any speed. [`voices`] lists them for `voice/voices`, Solrac's default
//! ([`DEFAULT_VOICE`]) first.
//!
//! ## How speaking works
//!
//! Kokoro is a text-to-speech model of 82 million parameters. sherpa-onnx turns the text into
//! phonemes, from a lexicon of American English and, for words that aren't in it, eSpeak NG's
//! pronunciation rules. Then it runs the model through ONNX Runtime and returns the sound:
//! mono, 32-bit float samples at 24 kHz. A voice is a style vector that the model is given
//! with the phonemes. `voices.bin` holds all 54 of them, and a voice is chosen by its number
//! in it ([`VOICES`]).
//!
//! Each archive is the model packaged by sherpa-onnx, copied unchanged to the PNX Studios CDN.
//! The int8 model ([`KOKORO_INT8`](crate::models::KOKORO_INT8)) is the same model with its
//! weights quantised to 8-bit integers: less than half the download, but on the M1 Max it
//! measured about three times slower than the full model ([`KOKORO`](crate::models::KOKORO)),
//! barely as fast as real time. So the full model is the one to use there; the int8 one is
//! kept for CPUs where it may be faster (the measurements are in Solrac's `docs/chat.md`).
//!
//! ## Pitfalls
//!
//! - **sherpa-onnx's Rust wrapper panics on a NUL in the text** (it unwraps
//!   `CString::new`). [`Speaker::speak`] replaces any NUL with a space first.
//! - **A missing file is only "won't load".** sherpa-onnx checks the files itself but
//!   reports only that the model wouldn't load, so [`Speaker::load`] and
//!   [`Transcriber::load`](crate::hear::Transcriber::load) check each one first and say which
//!   is missing.
//! - **Speech shares the CPU with the board.** ONNX Runtime uses [`THREADS`] threads, a
//!   measured balance between time to the first sound and Solrac's frame rate.
//! - **One piece at a time.** A [`Speaker`] reads one text at a time, and a second call waits
//!   for the first to finish. The first piece of a reply is heard soonest when it doesn't
//!   share the CPU with the rest.
//! - **Stopping waits for a sentence's end.** sherpa-onnx splits a text into sentences, and
//!   checks whether to go on only after each one. So a stopped piece of one sentence is
//!   still finished, then thrown away.
//!
//! Kokoro (Apache-2.0): <https://huggingface.co/hexgrad/Kokoro-82M>.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use cnp::voice::Voice;
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

use crate::models::LocalModel;
use crate::{Samples, SpeechError};

/// The voice a request without one is read in, the first that [`voices`] lists: Solrac's
/// (#39).
pub const DEFAULT_VOICE: &str = "am_michael";

/// The slowest and fastest a voice may be asked to speak, as a multiple of its own pace.
pub const SPEEDS: (f32, f32) = (0.5, 2.0);

/// Kokoro v1.0's voices, in the order of `voices.bin`: a voice's number is its place here.
/// The table is the model's own (`speaker2id` in its metadata). The first letter is the
/// language (`a` American English, `b` British English, then Spanish, French, Hindi,
/// Italian, Japanese, Portuguese and Chinese), and the second `f` or `m`.
pub const VOICES: [&str; 54] = [
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
    "ef_dora",
    "em_alex",
    "ff_siwis",
    "hf_alpha",
    "hf_beta",
    "hm_omega",
    "hm_psi",
    "if_sara",
    "im_nicola",
    "jf_alpha",
    "jf_gongitsune",
    "jf_nezumi",
    "jf_tebukuro",
    "jm_kumo",
    "pf_dora",
    "pm_alex",
    "pm_santa",
    "zf_xiaobei",
    "zf_xiaoni",
    "zf_xiaoxiao",
    "zf_xiaoyi",
    "zm_yunjian",
    "zm_yunxi",
    "zm_yunxia",
    "zm_yunyang",
    "em_santa",
];

/// The number of the voice called `name` ([`VOICES`]), if Kokoro has one.
pub fn voice_number(name: &str) -> Option<usize> {
    VOICES.iter().position(|voice| *voice == name)
}

/// The language of `voice` as a BCP 47 tag, and in words, from its first letter (see
/// [`VOICES`]).
fn language(voice: &str) -> (&'static str, &'static str) {
    match voice.as_bytes().first() {
        Some(b'a') => ("en-US", "American English"),
        Some(b'b') => ("en-GB", "British English"),
        Some(b'e') => ("es", "Spanish"),
        Some(b'f') => ("fr", "French"),
        Some(b'h') => ("hi", "Hindi"),
        Some(b'i') => ("it", "Italian"),
        Some(b'j') => ("ja", "Japanese"),
        Some(b'p') => ("pt-BR", "Brazilian Portuguese"),
        Some(b'z') => ("zh", "Chinese"),
        _ => ("und", "an unknown language"),
    }
}

/// `voice` as `voice/voices` lists it: `am_michael` is "Michael", American English, a man's
/// voice.
pub fn describe(voice: &str) -> Voice {
    let (tag, words) = language(voice);
    let given = voice.split_once('_').map_or(voice, |(_, name)| name);
    let mut letters = given.chars();
    let name = letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default();
    let who = match voice.as_bytes().get(1) {
        Some(b'f') => "a woman's voice",
        Some(b'm') => "a man's voice",
        _ => "a voice",
    };
    Voice {
        id: voice.to_string(),
        name,
        language: tag.to_string(),
        description: Some(format!("{words}, {who}")),
    }
}

/// Every voice, as `voice/voices` lists them: [`DEFAULT_VOICE`] first, then the rest in
/// [`VOICES`]' order.
pub fn voices() -> Vec<Voice> {
    std::iter::once(DEFAULT_VOICE)
        .chain(
            VOICES
                .iter()
                .copied()
                .filter(|voice| *voice != DEFAULT_VOICE),
        )
        .map(describe)
        .collect()
}

/// The threads ONNX Runtime uses for one piece of speech, or to hear one recording. More make
/// each piece sooner but take more of the CPU from Solrac's board (Solrac's `docs/chat.md`
/// has the measurements).
pub const THREADS: usize = 4;

/// The lexicon used before eSpeak NG's rules: American English, as the voices are.
const LEXICON: &str = "lexicon-us-en.txt";

/// The voices' style vectors.
const VOICES_FILE: &str = "voices.bin";

/// The model's phoneme symbols.
const TOKENS_FILE: &str = "tokens.txt";

/// eSpeak NG's pronunciation data.
const ESPEAK_DATA: &str = "espeak-ng-data";

/// Reads text aloud with Kokoro, in any of its voices.
pub struct Speaker {
    tts: OfflineTts,
    /// Held while reading, so one text is read at a time (module docs).
    reading: Mutex<()>,
}

impl Speaker {
    /// Loads `model` from `folder`, its unpacked archive, with [`THREADS`] threads. Takes
    /// about a second.
    pub fn load(model: &LocalModel, folder: &Path) -> Result<Speaker, SpeechError> {
        Speaker::load_with_threads(model, folder, THREADS)
    }

    /// [`Speaker::load`] with `threads` threads instead of [`THREADS`], for measuring.
    pub fn load_with_threads(
        model: &LocalModel,
        folder: &Path,
        threads: usize,
    ) -> Result<Speaker, SpeechError> {
        let file = |name: &str| -> Result<String, SpeechError> {
            let path = folder.join(name);
            if !path.exists() {
                return Err(SpeechError::Missing(path));
            }
            Ok(path.to_string_lossy().into_owned())
        };
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro: OfflineTtsKokoroModelConfig {
                    model: Some(file(model.model_file)?),
                    voices: Some(file(VOICES_FILE)?),
                    tokens: Some(file(TOKENS_FILE)?),
                    data_dir: Some(file(ESPEAK_DATA)?),
                    lexicon: Some(file(LEXICON)?),
                    ..OfflineTtsKokoroModelConfig::default()
                },
                num_threads: i32::try_from(threads.max(1)).unwrap_or(1),
                ..OfflineTtsModelConfig::default()
            },
            ..OfflineTtsConfig::default()
        };
        let tts = OfflineTts::create(&config).ok_or(SpeechError::WontLoad)?;
        Ok(Speaker {
            tts,
            reading: Mutex::new(()),
        })
    }

    /// Reads `text` aloud in the voice numbered `voice` ([`voice_number`]), at `speed` times
    /// its own pace. Waits while another text is being read (module docs). Stops, with
    /// [`SpeechError::Stopped`], at the end of a sentence after `stop` is set.
    pub fn speak(
        &self,
        text: &str,
        voice: usize,
        speed: f32,
        stop: &Arc<AtomicBool>,
    ) -> Result<Samples, SpeechError> {
        let _reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
        if stop.load(Ordering::Relaxed) {
            return Err(SpeechError::Stopped);
        }
        let text = text.replace('\0', " ");
        let config = GenerationConfig {
            sid: i32::try_from(voice).unwrap_or(0),
            speed,
            ..GenerationConfig::default()
        };
        let go_on = {
            let stop = Arc::clone(stop);
            move |_: &[f32], _: f32| !stop.load(Ordering::Relaxed)
        };
        let audio = self
            .tts
            .generate_with_config(&text, &config, Some(go_on))
            .ok_or(SpeechError::Failed)?;
        if stop.load(Ordering::Relaxed) {
            return Err(SpeechError::Stopped);
        }
        let rate = u32::try_from(audio.sample_rate()).map_err(|_| SpeechError::Failed)?;
        let samples = audio.samples().to_vec();
        if rate == 0 || samples.is_empty() {
            return Err(SpeechError::Failed);
        }
        Ok(Samples { rate, samples })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::KOKORO_INT8;

    #[test]
    fn voices_are_numbered_as_in_the_model() {
        // From the model's own `speaker2id`.
        assert_eq!(voice_number("af_alloy"), Some(0));
        assert_eq!(voice_number("am_michael"), Some(16));
        assert_eq!(voice_number("bm_george"), Some(26));
        assert_eq!(voice_number("em_santa"), Some(53));
        assert_eq!(voice_number("Michael"), None);
        let mut sorted = VOICES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), VOICES.len(), "no voice twice");
    }

    #[test]
    fn the_default_voice_comes_first_and_every_voice_once() {
        let listed = voices();
        assert_eq!(listed.len(), VOICES.len());
        assert_eq!(listed[0].id, DEFAULT_VOICE);
        assert!(voice_number(DEFAULT_VOICE).is_some());
        assert_eq!(
            listed[0],
            Voice {
                id: "am_michael".into(),
                name: "Michael".into(),
                language: "en-US".into(),
                description: Some("American English, a man's voice".into()),
            }
        );
        assert_eq!(describe("bf_emma").language, "en-GB");
        assert_eq!(describe("jf_gongitsune").name, "Gongitsune");
        assert!(listed.iter().all(|voice| voice.language != "und"));
    }

    #[test]
    fn a_missing_file_is_named() {
        let folder = std::env::temp_dir().join("solrac-voice-no-such-model");
        match Speaker::load(&KOKORO_INT8, &folder) {
            Err(SpeechError::Missing(path)) => {
                assert_eq!(path, folder.join(KOKORO_INT8.model_file));
            }
            other => panic!("{:?}", other.err()),
        }
    }
}
