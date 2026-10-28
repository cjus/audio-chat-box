//! Hearing: speech turned into text on this computer (`voice/transcribe`, spec 8.3;
//! cjus/chessgui#59, step 4).
//!
//! The player holds Talk in Solrac and speaks, and Solrac sends the recording, a WAV file. A
//! [`Transcriber`] hears it with Parakeet TDT 0.6B v2 ([`PARAKEET`](crate::models::PARAKEET)), run by sherpa-onnx on
//! the CPU. Carlos chose local transcription whether or not it is faster, to depend less on
//! remote services as computers get faster, with its quality first (Solrac's PRD,
//! "Decisions", 2026-10-21).
//!
//! ## How it works
//!
//! Parakeet TDT is NVIDIA's English speech recogniser of 600 million parameters: a
//! FastConformer encoder that turns the sound into one vector every 80 ms, and a
//! token-and-duration transducer (TDT) that reads those vectors and writes word pieces, with
//! punctuation and capitals. "Duration" means the decoder also predicts how many vectors
//! each piece covers, and skips them, which is what makes it fast. sherpa-onnx computes the
//! sound's 128 mel filter-bank features, runs the three ONNX models (encoder, decoder,
//! joiner) through ONNX Runtime, and joins the pieces (`tokens.txt`) into text.
//!
//! [`samples_of_wav`] reads a WAV file (Solrac's are mono, 16-bit, 16 kHz) into samples,
//! [`samples_of_pcm_f32`] reads CNP's raw `pcmF32` audio, and [`Transcriber::transcribe`]
//! hears them.
//!
//! ## Beam search and hotwords
//!
//! sherpa-onnx decodes greedily by default: at each step it keeps only the likeliest piece.
//! A [`Transcriber`] keeps the [`BEAM`] likeliest transcripts instead (modified beam
//! search), and gives a bonus of [`HOTWORD_SCORE`] for each piece of a hotword the client
//! sent ([`Hotwords`]) that a transcript spells out. So when the sound is between "night"
//! and "knight", or "pawn" and "on", the hotword wins; elsewhere the bonus is too small to
//! matter. The client knows its domain and the node needn't: Solrac sends a chess move's
//! words, another program its own. Until cjus/chessgui#226 the chess words were built in
//! here. This is sherpa-onnx's contextual biasing: a trie of the hotwords'
//! pieces with Aho-Corasick's failure links, walked along with each transcript (its docs
//! below).
//!
//! sherpa-onnx matches hotwords piece by piece, so it splits each into the model's pieces
//! with a SentencePiece vocabulary (`bpe.vocab`). sherpa-onnx's archive of Parakeet doesn't
//! include one, so [`Transcriber::load`] writes one into the model's folder from
//! `tokens.txt` ([`vocabulary`]).
//!
//! ## Why Parakeet, and these settings
//!
//! Parakeet was compared with the other models sherpa-onnx runs that Solrac may use (by
//! licence), on 180 recordings of 20 things a player says (moves, questions, "castle
//! kingside", "the Ruy Lopez", and five everyday sentences) in nine voices, clean and with a
//! room's echo and noise, on the M1 Max. Its greedy decoding already heard squares right
//! more often than Whisper Large V3 Turbo (the model Solrac used through OpenRouter), Cohere
//! Transcribe, Qwen3-ASR or Moonshine, in every condition measured; beam search with the
//! chess words as hotwords cut its word errors on chess phrases from 8.3% to 4.4%. Its full-precision model
//! was no better. Solrac's `docs/chat.md` has the tables. The recordings were made by speech
//! synthesisers (macOS's and Kokoro), not people, so Carlos's own voice is the real test.
//!
//! ## Pitfalls
//!
//! - **"The knight is young."** Hotwords pull ordinary words towards them when the sound
//!   allows both: in a chess game, the right way to be wrong. Raising [`HOTWORD_SCORE`] makes
//!   it worse: at 2.0 the bake-off's chess phrases got worse too.
//! - **"/" and ":" in a hotword.** sherpa-onnx splits hotwords at "/" and reads ":" as a
//!   score, so [`Hotwords`] turns both into spaces.
//! - **The vocabulary is made up.** A real SentencePiece vocabulary gives each piece a
//!   score; `tokens.txt` has none, so [`vocabulary`] gives every piece the same, and a
//!   hotword is split into the fewest pieces. The model may split a word differently, and
//!   then that word's bonus is lost. The bake-off says it works for these words.
//! - **It can't be stopped.** sherpa-onnx's decoding has no callback, so a recording that
//!   is no longer wanted is still heard to the end (well under a second), then thrown away.
//!   The node answers a cancelled one at once, without waiting for it.
//! - **One recording at a time.** A [`Transcriber`] hears one at a time, so a second
//!   doesn't take the first one's CPU.
//! - **A rate other than 16 kHz** is resampled by sherpa-onnx, which logs a warning to
//!   standard error. Solrac's recordings are already 16 kHz.
//! - **English only.** Parakeet TDT 0.6B v2 hears English; the node refuses another
//!   language.
//!
//! Parakeet TDT 0.6B v2 (CC-BY-4.0), by NVIDIA:
//! <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2>, converted to ONNX and quantised to
//! 8-bit by sherpa-onnx. TDT: Xu et al., "Efficient Sequence Transduction by Jointly
//! Predicting Tokens and Durations", 2023, <https://arxiv.org/abs/2304.06795>. sherpa-onnx's
//! hotwords: <https://k2-fsa.github.io/sherpa/onnx/hotwords/index.html>.

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};

use crate::models::LocalModel;
use crate::speak::THREADS;
use crate::{Samples, SpeechError};

/// The language Parakeet hears, as a BCP 47 tag's primary subtag: English, any region.
pub const LANGUAGE: &str = "en";

/// The transducer's decoder, which predicts the next word piece from the ones before.
const DECODER_FILE: &str = "decoder.int8.onnx";

/// The transducer's joiner, which combines the encoder's and the decoder's outputs.
const JOINER_FILE: &str = "joiner.int8.onnx";

/// The word pieces the model writes.
const TOKENS_FILE: &str = "tokens.txt";

/// What sherpa-onnx calls a NeMo transducer, such as Parakeet TDT.
const MODEL_TYPE: &str = "nemo_transducer";

/// The vocabulary [`Transcriber::load`] writes into the model's folder ([`vocabulary`]).
const VOCABULARY_FILE: &str = "bpe.vocab";

/// sherpa-onnx's beam search for transducers, which hotwords need.
const DECODING_METHOD: &str = "modified_beam_search";

/// How many transcripts the beam search keeps at each step: sherpa-onnx's default. In the
/// bake-off, beam search alone cut word errors on chess phrases from 8.3% to 6.5%.
pub const BEAM: i32 = 4;

/// The most hotwords one request may give: far more than the words of every legal move in a
/// chess position. More is `InvalidParams`.
pub const MAX_HOTWORDS: usize = 1000;

/// Words and phrases a client expects to be said, as sherpa-onnx takes them (see the
/// [module docs](self)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hotwords(String);

impl Hotwords {
    /// No hotwords.
    pub fn none() -> Hotwords {
        Hotwords::default()
    }

    /// The hotwords in `words`, or why they can't be taken (for an `InvalidParams` error): one
    /// that is empty or holds a line break, or more than [`MAX_HOTWORDS`].
    pub fn of(words: &[String]) -> Result<Hotwords, String> {
        if words.len() > MAX_HOTWORDS {
            return Err(format!("at most {MAX_HOTWORDS} hotwords"));
        }
        let mut taken = Vec::with_capacity(words.len());
        for word in words {
            if word.contains(['\n', '\r']) {
                return Err(format!("the hotword {word:?} holds a line break"));
            }
            // sherpa-onnx splits at "/" and reads ":" as a score (module docs).
            let word = word.replace(['/', ':', '\0'], " ");
            let word = word.split_whitespace().collect::<Vec<_>>().join(" ");
            if word.is_empty() {
                return Err("a hotword is empty".to_string());
            }
            taken.push(word);
        }
        // sherpa-onnx reads one hotword a line, or with "/" between them.
        Ok(Hotwords(taken.join("/")))
    }

    /// The hotwords as sherpa-onnx takes them: "/" between them.
    pub fn joined(&self) -> &str {
        &self.0
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The hotwords in a file's `text`, one a line, blank lines left out: for the examples'
    /// `--hotwords FILE`.
    pub fn lines(text: &str) -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// The bonus for each piece of a hotword (see the [module docs](self)). Measured in the
/// bake-off: 1.0 was best on clean recordings, 1.5 on noisy ones, 1.25 close to the best on
/// both; 2.0 was worse on all.
pub const HOTWORD_SCORE: f32 = 1.25;

/// Turns speech into text with [`PARAKEET`](crate::models::PARAKEET).
pub struct Transcriber {
    recognizer: OfflineRecognizer,
    /// Held while hearing, so one recording is heard at a time (module docs).
    hearing: Mutex<()>,
}

impl Transcriber {
    /// Loads `model` from `folder`, its unpacked archive, with [`THREADS`] threads. Takes
    /// about a second.
    pub fn load(model: &LocalModel, folder: &Path) -> Result<Transcriber, SpeechError> {
        Transcriber::load_with_threads(model, folder, THREADS)
    }

    /// [`Transcriber::load`] with `threads` threads instead of [`THREADS`], for measuring.
    pub fn load_with_threads(
        model: &LocalModel,
        folder: &Path,
        threads: usize,
    ) -> Result<Transcriber, SpeechError> {
        let file = |name: &str| -> Result<String, SpeechError> {
            let path = folder.join(name);
            if !path.exists() {
                return Err(SpeechError::Missing(path));
            }
            Ok(path.to_string_lossy().into_owned())
        };
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(file(model.model_file)?),
            decoder: Some(file(DECODER_FILE)?),
            joiner: Some(file(JOINER_FILE)?),
        };
        let tokens = file(TOKENS_FILE)?;
        let vocabulary_file = write_vocabulary(folder, &tokens)?;
        config.model_config.tokens = Some(tokens);
        config.model_config.model_type = Some(MODEL_TYPE.to_string());
        config.model_config.num_threads = i32::try_from(threads.max(1)).unwrap_or(1);
        config.model_config.modeling_unit = Some("bpe".to_string());
        config.model_config.bpe_vocab = Some(vocabulary_file);
        config.decoding_method = Some(DECODING_METHOD.to_string());
        config.max_active_paths = BEAM;
        config.hotwords_score = HOTWORD_SCORE;
        let recognizer = OfflineRecognizer::create(&config).ok_or(SpeechError::WontLoad)?;
        Ok(Transcriber {
            recognizer,
            hearing: Mutex::new(()),
        })
    }

    /// What is said in `speech`, trimmed, with `hotwords` favoured: empty if nothing was
    /// heard. Waits while another recording is being heard (module docs).
    pub fn transcribe(&self, speech: &Samples, hotwords: &Hotwords) -> Result<String, SpeechError> {
        let _hearing = self.hearing.lock().unwrap_or_else(PoisonError::into_inner);
        let rate = i32::try_from(speech.rate).map_err(|_| SpeechError::Unreadable)?;
        if rate == 0 || speech.samples.is_empty() {
            return Ok(String::new());
        }
        let stream = if hotwords.is_empty() {
            self.recognizer.create_stream()
        } else {
            self.recognizer.create_stream_with_hotwords(&hotwords.0)
        };
        stream.accept_waveform(rate, &speech.samples);
        self.recognizer.decode(&stream);
        let heard = stream.get_result().ok_or(SpeechError::Unheard)?;
        Ok(heard.text.trim().to_string())
    }
}

/// A SentencePiece vocabulary of the pieces in `tokens`, the contents of a model's
/// `tokens.txt` (a piece and its number a line): each piece and a score, a line each, all
/// scores the same (see the [module docs](self)). Leaves out `<unk>` and `<blk>`, which no
/// word is spelled with.
pub fn vocabulary(tokens: &str) -> String {
    tokens
        .lines()
        .filter_map(|line| line.rsplit_once(' ').map(|(piece, _number)| piece))
        .filter(|piece| !matches!(*piece, "" | "<unk>" | "<blk>"))
        .map(|piece| format!("{piece}\t-1\n"))
        .collect()
}

/// Writes the [`vocabulary`] of `tokens_file` into `folder`, unless it is there already, and
/// returns its path. It is written under a name of its own (the process id) and then moved
/// into place, so another audio-chat-box never reads it half written.
fn write_vocabulary(folder: &Path, tokens_file: &str) -> Result<String, SpeechError> {
    let tokens = fs::read_to_string(tokens_file).map_err(|_| SpeechError::WontLoad)?;
    let vocabulary = vocabulary(&tokens);
    let path = folder.join(VOCABULARY_FILE);
    if fs::read_to_string(&path).ok().as_deref() != Some(vocabulary.as_str()) {
        let part = folder.join(format!("{VOCABULARY_FILE}.{}", std::process::id()));
        fs::write(&part, vocabulary)
            .and_then(|()| fs::rename(&part, &path))
            .map_err(|_| SpeechError::WontLoad)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// The samples of `wav`, a WAV file of integer or float samples, its channels averaged into
/// one.
pub fn samples_of_wav(wav: &[u8]) -> Result<Samples, SpeechError> {
    let reader = hound::WavReader::new(Cursor::new(wav)).map_err(|_| SpeechError::Unreadable)?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|_| SpeechError::Unreadable)?,
        hound::SampleFormat::Int => {
            // An integer sample of `bits` bits runs from -2^(bits-1) to 2^(bits-1) - 1.
            let full_scale = 2f32.powi(i32::from(spec.bits_per_sample.max(1)) - 1);
            reader
                .into_samples::<i32>()
                .map(|sample| sample.map(|sample| sample as f32 / full_scale))
                .collect::<Result<_, _>>()
                .map_err(|_| SpeechError::Unreadable)?
        }
    };
    let samples = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    Ok(Samples {
        rate: spec.sample_rate,
        samples,
    })
}

/// The samples of `bytes`, CNP's `pcmF32`: raw mono samples, 32-bit float, little-endian, at
/// `rate` samples a second.
pub fn samples_of_pcm_f32(bytes: &[u8], rate: u32) -> Result<Samples, SpeechError> {
    if rate == 0 || !bytes.len().is_multiple_of(4) {
        return Err(SpeechError::Unreadable);
    }
    let samples = bytes
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
        .collect();
    Ok(Samples { rate, samples })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PARAKEET;

    /// A WAV file of `samples` at 16 kHz, `channels` channels, 16-bit.
    fn wav(samples: &[i16], channels: u16) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut wav, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
        wav.into_inner()
    }

    #[test]
    fn raw_samples_are_read_four_bytes_each() {
        let bytes: Vec<u8> = [0.5f32, -1.0, 0.25]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let samples = samples_of_pcm_f32(&bytes, 16_000).unwrap();
        assert_eq!(samples.rate, 16_000);
        assert_eq!(samples.samples, [0.5, -1.0, 0.25]);
        assert_eq!(
            samples_of_pcm_f32(&bytes[..5], 16_000),
            Err(SpeechError::Unreadable)
        );
        assert_eq!(samples_of_pcm_f32(&bytes, 0), Err(SpeechError::Unreadable));
    }

    #[test]
    fn a_wav_file_is_read_into_samples_from_minus_one_to_one() {
        let samples = samples_of_wav(&wav(&[0, 16_384, -32_768, 32_767], 1)).unwrap();
        assert_eq!(samples.rate, 16_000);
        assert_eq!(samples.samples, [0.0, 0.5, -1.0, 32_767.0 / 32_768.0]);

        // Two channels are averaged into one.
        let stereo = samples_of_wav(&wav(&[16_384, 0, -16_384, -16_384], 2)).unwrap();
        assert_eq!(stereo.samples, [0.25, -0.5]);

        for not_wav in [&b""[..], b"RIFF", b"{\"error\": \"not audio\"}"] {
            assert_eq!(samples_of_wav(not_wav), Err(SpeechError::Unreadable));
        }
    }

    #[test]
    fn the_vocabulary_lists_every_piece_a_word_is_spelled_with() {
        let tokens = "<unk> 0\n\u{2581}t 1\n\u{2581}kn 119\night 412\n\u{3cc} 1023\n<blk> 1024\n";
        assert_eq!(
            vocabulary(tokens),
            "\u{2581}t\t-1\n\u{2581}kn\t-1\night\t-1\n\u{3cc}\t-1\n"
        );
        assert_eq!(vocabulary(""), "");
    }

    #[test]
    fn hotwords_are_taken_as_sherpa_onnx_reads_them() {
        let words = |words: &[&str]| {
            words
                .iter()
                .map(|word| word.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            Hotwords::of(&words(&["knight", " Ruy  Lopez "])),
            Ok(Hotwords("knight/Ruy Lopez".to_string()))
        );
        // sherpa-onnx would split at "/" and read ":" as a score.
        assert_eq!(
            Hotwords::of(&words(&["e4/e5", "ratio:2"])),
            Ok(Hotwords("e4 e5/ratio 2".to_string()))
        );
        assert!(Hotwords::of(&[]).unwrap().is_empty());
        assert!(Hotwords::of(&words(&[""])).is_err());
        assert!(Hotwords::of(&words(&[" / "])).is_err());
        assert!(Hotwords::of(&words(&["two\nwords"])).is_err());
        assert!(Hotwords::of(&vec!["word".to_string(); MAX_HOTWORDS + 1]).is_err());
    }

    #[test]
    fn a_missing_file_is_named() {
        let folder = std::env::temp_dir().join("audio-chat-box-no-such-transcription-model");
        match Transcriber::load(&PARAKEET, &folder) {
            Err(SpeechError::Missing(path)) => {
                assert_eq!(path, folder.join(PARAKEET.model_file));
            }
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("loaded from nothing"),
        }
    }
}
