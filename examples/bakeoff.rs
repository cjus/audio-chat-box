//! The transcription bake-off (#59): how well a speech recogniser hears what a player says,
//! and how fast, so that models can be compared before one is chosen. Parakeet was chosen
//! with it (`docs/chat.md`, "Local transcription"), and Carlos asked that it stay, to try
//! future models the same way: transcription quality comes first.
//!
//! ```text
//! tools/bakeoff/make-clips.sh /tmp/clips
//! cargo run --release --example bakeoff -- \
//!     <family> <model folder> /tmp/clips [--threads N] [--snr DB] [--beam]
//!     [--hotwords SCORE] [--precision int8|fp16|fp32] [--misses]
//! ```
//!
//! The model folder is one of sherpa-onnx's archives, unpacked
//! (<https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models>). The family says how
//! to run it:
//!
//! - `solrac`: Solrac's own `Transcriber` on a Parakeet folder, exactly as the chat hears.
//! - `parakeet`: a NeMo transducer (`encoder`, `decoder`, `joiner`), such as Parakeet TDT.
//!   `--beam` decodes with beam search, and `--hotwords SCORE` gives Solrac's chess words
//!   that bonus (it needs `--beam`, and turns it on).
//! - `whisper`: a Whisper encoder and decoder (`<name>-encoder…`), in English.
//! - `moonshine`: Moonshine v2 (`encoder_model.ort`, `decoder_model_merged.ort`).
//! - `qwen3`: Qwen3-ASR; `--hotwords` puts the chess words in its prompt (the score is
//!   ignored).
//! - `cohere`: Cohere Transcribe, in English.
//!
//! ## What it measures
//!
//! The clips are `make-clips.sh`'s: each phrase of `tools/bakeoff/phrases.txt` in nine
//! synthetic voices, at the window's 16 kHz. Each is heard once (after one untimed clip, to
//! warm the model up), and scored against what should be heard:
//!
//! - **Word errors**: the words substituted, left out or added, as a share of the words
//!   that should be heard (the edit distance), on the chess phrases and on the everyday
//!   ones apart. Before comparing, text is lower-cased, punctuation dropped, number words
//!   made digits ("five" is "5"), and a file letter followed by a rank made one square
//!   ("f 3" is "f3"), so that "Knight to F3." and "knight to f three" both count as right.
//! - **Squares**: of the clips whose phrase names squares, those in which the same squares
//!   were heard in the same order, notation included ("Nf6" names f6). A misheard square is
//!   a wrong move, so this matters most.
//! - **Exact**: the clips heard word for word.
//! - **Time**: the mean time to hear a move and a question, and how many seconds of speech
//!   it hears in a second.
//!
//! `--snr DB` makes a room of each clip first ([`room`]): two echoes and pink noise `DB`
//! decibels below the speech. 25 is a quiet room, 15 a noisy one.
//!
//! ## Pitfalls
//!
//! - **Synthetic voices are clean.** A person at a laptop's microphone is harder to hear,
//!   so the bake-off compares models; it doesn't say how well one will hear Carlos.
//! - **Time depends on the machine's load.** Run models one after another, not side by
//!   side, and compare times from the same session.
//! - **Compare models on one set of clips.** Kokoro's speech isn't the same twice (its
//!   lengths vary by a few samples), so a new set scores a little differently from another:
//!   Parakeet's 4.4% word errors in `docs/chat.md` were 4.3% on a second set.
//! - **Whisper pads every clip to 30 s**, so its time hardly depends on the clip's length.
//!
//! sherpa-onnx's Rust examples, which show each family's files:
//! <https://github.com/k2-fsa/sherpa-onnx/tree/master/rust-api-examples/examples>. Pink
//! noise: Paul Kellet's filter, <https://www.firstpr.com.au/dsp/pink-noise/>.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use sherpa_onnx::{
    OfflineCohereTranscribeModelConfig, OfflineMoonshineModelConfig, OfflineQwen3ASRModelConfig,
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
    OfflineWhisperModelConfig,
};
use solrac_voice::Samples;
use solrac_voice::hear::{HOTWORDS, Transcriber, samples_of_wav, vocabulary};
use solrac_voice::models::PARAKEET;
use solrac_voice::speak::THREADS;

/// The language the clips are in, for the models that are told.
const LANGUAGE: &str = "en";

/// How many transcripts beam search keeps (sherpa-onnx's default, as Solrac's `BEAM`).
const BEAM: i32 = 4;

/// Below this, a sample is silence, not speech, when the speech's loudness is measured.
const QUIET_SAMPLE: f32 = 0.01;

/// The echoes of [`room`]: their delay in milliseconds, and how loud they are.
const ECHOES: [(usize, f32); 2] = [(35, 0.3), (60, 0.2)];

/// Where [`room`]'s noise starts, before the clip's length is added: the noise the tables
/// in `docs/chat.md` were measured with.
const NOISE_SEED: u64 = 0x5eed;

/// What the bake-off is asked to do (see the [module docs](self)).
struct Options {
    family: String,
    folder: PathBuf,
    clips: PathBuf,
    threads: usize,
    snr: Option<f32>,
    beam: bool,
    hotwords: Option<f32>,
    precision: String,
    misses: bool,
}

/// What kind of thing a phrase is.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Move,
    Question,
    Everyday,
}

/// A line of `phrases.txt`: its kind, and what should be heard.
struct Phrase {
    kind: Kind,
    expected: String,
}

/// Hears a clip: a sherpa-onnx recogniser, or Solrac's transcriber.
enum Hearer {
    Sherpa {
        recognizer: OfflineRecognizer,
        /// The hotwords given with each clip, for a transducer.
        hotwords: Option<String>,
        /// Whether the clip's language is set on its stream (Cohere's way).
        language: bool,
    },
    Solrac(Transcriber),
}

impl Hearer {
    fn hear(&self, samples: &Samples) -> String {
        match self {
            Hearer::Solrac(transcriber) => transcriber.transcribe(samples).unwrap_or_default(),
            Hearer::Sherpa {
                recognizer,
                hotwords,
                language,
            } => {
                let stream = match hotwords {
                    Some(hotwords) => recognizer.create_stream_with_hotwords(hotwords),
                    None => recognizer.create_stream(),
                };
                if *language {
                    stream.set_option("language", LANGUAGE);
                }
                let rate = i32::try_from(samples.rate).unwrap_or(16_000);
                stream.accept_waveform(rate, &samples.samples);
                recognizer.decode(&stream);
                stream
                    .get_result()
                    .map(|result| result.text.trim().to_string())
                    .unwrap_or_default()
            }
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = options(std::env::args().skip(1).collect())?;
    let phrases = phrases(&options.clips.join("phrases.txt"))?;
    let clips = clips(&options.clips, phrases.len())?;
    let started = Instant::now();
    let hearer = hearer(&options)?;
    println!(
        "{} ({}), {} threads: loaded in {:.1} s",
        options.folder.display(),
        options.family,
        options.threads,
        started.elapsed().as_secs_f64()
    );

    let load = |path: &Path| -> Result<Samples, String> {
        let wav = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let samples =
            samples_of_wav(&wav).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(match options.snr {
            Some(snr) => room(&samples, snr),
            None => samples,
        })
    };
    if let Some((_, first)) = clips.first() {
        hearer.hear(&load(first)?);
    }

    let mut score = Score::default();
    for (phrase, path) in &clips {
        let phrase = &phrases[*phrase];
        let samples = load(path)?;
        let started = Instant::now();
        let heard = hearer.hear(&samples);
        let took = started.elapsed().as_secs_f64();
        let right = score.add(phrase, &heard, samples.duration().as_secs_f64(), took);
        if options.misses && !right {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            println!("  {name:24} {took:5.2} s  {heard:?}");
        }
    }
    let room = options
        .snr
        .map_or("clean".to_string(), |snr| format!("noise {snr} dB below"));
    println!("{room}: {}", score.summary());
    Ok(())
}

/// The options in `args` (see the [module docs](self)).
fn options(args: Vec<String>) -> Result<Options, String> {
    const USAGE: &str = "usage: bakeoff <solrac|parakeet|whisper|moonshine|qwen3|cohere> \
        <model folder> <clips folder> [--threads N] [--snr DB] [--beam] [--hotwords SCORE] \
        [--precision int8|fp16|fp32] [--misses]";
    let mut args = args.into_iter();
    let (Some(family), Some(folder), Some(clips)) = (args.next(), args.next(), args.next()) else {
        return Err(USAGE.to_string());
    };
    let mut options = Options {
        family,
        folder: PathBuf::from(folder),
        clips: PathBuf::from(clips),
        threads: THREADS,
        snr: None,
        beam: false,
        hotwords: None,
        precision: "int8".to_string(),
        misses: false,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        let number = |text: String| -> Result<f32, String> {
            text.parse()
                .map_err(|_| format!("{flag}: {text} isn't a number"))
        };
        match flag.as_str() {
            "--threads" => options.threads = number(value()?)?.max(1.0) as usize,
            "--snr" => options.snr = Some(number(value()?)?),
            "--beam" => options.beam = true,
            "--hotwords" => {
                options.hotwords = Some(number(value()?)?);
                options.beam = true;
            }
            "--precision" => options.precision = value()?,
            "--misses" => options.misses = true,
            _ => return Err(format!("{flag}?\n{USAGE}")),
        }
    }
    Ok(options)
}

/// The phrases in `path`, `phrases.txt`.
fn phrases(path: &Path) -> Result<Vec<Phrase>, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split('|');
            let kind = match parts.next() {
                Some("move") => Kind::Move,
                Some("question") => Kind::Question,
                Some("everyday") => Kind::Everyday,
                _ => return Err(format!("{}: no kind in {line:?}", path.display())),
            };
            let expected = parts
                .nth(1)
                .ok_or_else(|| format!("nothing expected in {line:?}"))?;
            Ok(Phrase {
                kind,
                expected: expected.to_string(),
            })
        })
        .collect()
}

/// The clips in `folder`, in order: each phrase's number (from 0, below `phrases`) and file.
fn clips(folder: &Path, phrases: usize) -> Result<Vec<(usize, PathBuf)>, String> {
    let entries = fs::read_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let mut clips: Vec<(usize, PathBuf)> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            let number: usize = name.split('-').next()?.parse().ok()?;
            (1..=phrases)
                .contains(&number)
                .then_some((number - 1, path))
        })
        .collect();
    clips.sort();
    if clips.is_empty() {
        return Err(format!(
            "no clips in {}: run make-clips.sh",
            folder.display()
        ));
    }
    Ok(clips)
}

/// The model in `options.folder`, ready to hear, as its family runs it.
fn hearer(options: &Options) -> Result<Hearer, String> {
    let folder = &options.folder;
    if options.family == "solrac" {
        return Transcriber::load_with_threads(&PARAKEET, folder, options.threads)
            .map(Hearer::Solrac)
            .map_err(|error| error.to_string());
    }
    let file = |name: &str| -> Result<Option<String>, String> {
        let path = folder.join(name);
        match path.exists() {
            true => Ok(Some(path.to_string_lossy().into_owned())),
            false => Err(format!("{} is missing", path.display())),
        }
    };
    // `encoder.int8.onnx`, `encoder.fp16.onnx`, or `encoder.onnx` at full precision.
    let precise = |stem: &str| match options.precision.as_str() {
        "fp32" => format!("{stem}.onnx"),
        precision => format!("{stem}.{precision}.onnx"),
    };
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.num_threads = i32::try_from(options.threads).unwrap_or(1);
    let (mut hotwords, mut language) = (None, false);
    match options.family.as_str() {
        "parakeet" => {
            config.model_config.transducer = OfflineTransducerModelConfig {
                encoder: file(&precise("encoder"))?,
                decoder: file(&precise("decoder"))?,
                joiner: file(&precise("joiner"))?,
            };
            config.model_config.tokens = file("tokens.txt")?;
            config.model_config.model_type = Some("nemo_transducer".to_string());
            if options.beam {
                config.decoding_method = Some("modified_beam_search".to_string());
                config.max_active_paths = BEAM;
            }
            if let Some(score) = options.hotwords {
                // As Solrac's transcriber does: a vocabulary made from `tokens.txt`, here
                // in the temporary folder rather than the model's.
                let tokens = fs::read_to_string(folder.join("tokens.txt"))
                    .map_err(|error| error.to_string())?;
                let vocabulary_file = std::env::temp_dir().join("solrac-bakeoff-bpe.vocab");
                fs::write(&vocabulary_file, vocabulary(&tokens)).map_err(|e| e.to_string())?;
                config.model_config.modeling_unit = Some("bpe".to_string());
                config.model_config.bpe_vocab =
                    Some(vocabulary_file.to_string_lossy().into_owned());
                config.hotwords_score = score;
                hotwords = Some(HOTWORDS.join("/"));
            }
        }
        "whisper" => {
            // sherpa-onnx names a Whisper's files after it: `turbo-encoder.int8.onnx`.
            let ending = precise("-encoder");
            let encoder = fs::read_dir(folder)
                .map_err(|error| error.to_string())?
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .find(|name| name.ends_with(&ending))
                .ok_or_else(|| format!("no *{ending} in {}", folder.display()))?;
            let name = encoder.trim_end_matches(&ending);
            config.model_config.whisper = OfflineWhisperModelConfig {
                encoder: file(&encoder)?,
                decoder: file(&format!("{name}{}", precise("-decoder")))?,
                language: Some(LANGUAGE.to_string()),
                task: Some("transcribe".to_string()),
                tail_paddings: -1,
                ..Default::default()
            };
            config.model_config.tokens = file(&format!("{name}-tokens.txt"))?;
        }
        "moonshine" => {
            config.model_config.moonshine = OfflineMoonshineModelConfig {
                encoder: file("encoder_model.ort")?,
                merged_decoder: file("decoder_model_merged.ort")?,
                ..Default::default()
            };
            config.model_config.tokens = file("tokens.txt")?;
        }
        "qwen3" => {
            config.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
                conv_frontend: file("conv_frontend.onnx")?,
                encoder: file(&precise("encoder"))?,
                decoder: file(&precise("decoder"))?,
                tokenizer: file("tokenizer")?,
                hotwords: options.hotwords.map(|_| HOTWORDS.join(",")),
                ..Default::default()
            };
            // Qwen3-ASR reads its own tokenizer; sherpa-onnx wants this empty.
            config.model_config.tokens = Some(String::new());
        }
        "cohere" => {
            config.model_config.cohere_transcribe = OfflineCohereTranscribeModelConfig {
                encoder: file(&precise("encoder"))?,
                decoder: file(&precise("decoder"))?,
                use_punct: true,
                use_itn: true,
                ..Default::default()
            };
            config.model_config.tokens = file("tokens.txt")?;
            language = true;
        }
        family => return Err(format!("no family called {family}")),
    }
    let recognizer = OfflineRecognizer::create(&config)
        .ok_or_else(|| format!("sherpa-onnx couldn't load {}", folder.display()))?;
    Ok(Hearer::Sherpa {
        recognizer,
        hotwords,
        language,
    })
}

/// The scores so far (see the [module docs](self)).
#[derive(Default)]
struct Score {
    /// Word errors and words expected, on chess phrases.
    chess: (usize, usize),
    /// The same, on everyday phrases.
    everyday: (usize, usize),
    /// Clips whose squares were all heard, and clips that name squares.
    squares: (usize, usize),
    /// Clips heard word for word, and clips.
    exact: (usize, usize),
    /// Seconds spent hearing moves, and how many.
    moves: (f64, usize),
    /// The same, for questions.
    questions: (f64, usize),
    /// Seconds of speech heard, and seconds spent hearing it.
    speech: (f64, f64),
}

impl Score {
    /// Scores `heard` for `phrase`, a clip `seconds` long heard in `took` seconds. Returns
    /// whether it was heard word for word.
    fn add(&mut self, phrase: &Phrase, heard: &str, seconds: f64, took: f64) -> bool {
        let (expected, got) = (words(&phrase.expected), words(heard));
        let errors = edit_distance(&expected, &got);
        let tally = match phrase.kind {
            Kind::Everyday => &mut self.everyday,
            Kind::Move | Kind::Question => &mut self.chess,
        };
        tally.0 += errors;
        tally.1 += expected.len();
        let wanted = squares(&phrase.expected);
        if !wanted.is_empty() {
            self.squares.0 += usize::from(squares(heard) == wanted);
            self.squares.1 += 1;
        }
        self.exact.0 += usize::from(errors == 0);
        self.exact.1 += 1;
        let time = match phrase.kind {
            Kind::Move => Some(&mut self.moves),
            Kind::Question => Some(&mut self.questions),
            Kind::Everyday => None,
        };
        if let Some(time) = time {
            time.0 += took;
            time.1 += 1;
        }
        self.speech.0 += seconds;
        self.speech.1 += took;
        errors == 0
    }

    /// One line: the scores, then the times.
    fn summary(&self) -> String {
        let share = |(part, whole): (usize, usize)| 100.0 * part as f64 / whole.max(1) as f64;
        let mean = |(sum, count): (f64, usize)| sum / count.max(1) as f64;
        format!(
            "chess phrases {:.1}% word errors, squares right in {}/{}; everyday {:.1}%; \
             {}/{} exact; {:.2} s a move, {:.2} s a question, {:.1}x real time",
            share(self.chess),
            self.squares.0,
            self.squares.1,
            share(self.everyday),
            self.exact.0,
            self.exact.1,
            mean(self.moves),
            mean(self.questions),
            self.speech.0 / self.speech.1.max(f64::EPSILON)
        )
    }
}

/// The words of `text`, to compare (see the [module docs](self)): lower case, without
/// punctuation, number words as digits, and a file letter then a rank as one square.
fn words(text: &str) -> Vec<String> {
    const NUMBERS: [&str; 9] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
    ];
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut words: Vec<String> = Vec::new();
    for word in cleaned.split_whitespace() {
        let word = match NUMBERS.iter().position(|number| *number == word) {
            Some(digit) => digit.to_string(),
            None => word.to_string(),
        };
        let is_rank = word.len() == 1 && word.chars().all(|c| c.is_ascii_digit());
        let after_file = words
            .last()
            .is_some_and(|last| last.len() == 1 && last.chars().all(|c| ('a'..='h').contains(&c)));
        match (is_rank, after_file) {
            (true, true) => {
                let file = words.pop().unwrap_or_default();
                words.push(format!("{file}{word}"));
            }
            _ => words.push(word),
        }
    }
    words
}

/// The squares `text` names, in order, in notation too: `Nf6` names f6.
fn squares(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in words(text) {
        let chars: Vec<char> = word.chars().collect();
        for at in 0..chars.len().saturating_sub(1) {
            // A square starts a word, or follows a piece's letter or a capture's x.
            let starts = at == 0 || matches!(chars[at - 1], 'n' | 'b' | 'r' | 'q' | 'k' | 'x');
            let is_square = ('a'..='h').contains(&chars[at])
                && ('1'..='8').contains(&chars[at + 1])
                && chars.get(at + 2).is_none_or(|next| !next.is_ascii_digit());
            if starts && is_square {
                found.push(format!("{}{}", chars[at], chars[at + 1]));
            }
        }
    }
    found
}

/// How many words must be substituted, left out or added to make `got` into `expected`
/// (Levenshtein's distance, a row at a time).
fn edit_distance(expected: &[String], got: &[String]) -> usize {
    let mut row: Vec<usize> = (0..=got.len()).collect();
    for (i, want) in expected.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, word) in got.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(want != word));
            diagonal = above;
        }
    }
    row[got.len()]
}

/// `samples` as if heard in a room: two echoes ([`ECHOES`]), and pink noise `snr` dB below
/// the speech's loudness (the root mean square of its samples that aren't silence). The
/// same clip always gets the same noise.
fn room(samples: &Samples, snr: f32) -> Samples {
    let clean = &samples.samples;
    let mut heard = clean.clone();
    for (milliseconds, gain) in ECHOES {
        let delay = milliseconds * samples.rate as usize / 1000;
        for at in delay..clean.len() {
            heard[at] += gain * clean[at - delay];
        }
    }
    let rms = |values: &[f32]| {
        (values.iter().map(|v| v * v).sum::<f32>() / values.len().max(1) as f32).sqrt()
    };
    let speech: Vec<f32> = heard
        .iter()
        .copied()
        .filter(|s| s.abs() > QUIET_SAMPLE)
        .collect();
    let noise = pink_noise(heard.len(), NOISE_SEED + clean.len() as u64);
    let scale = rms(&speech) / 10f32.powf(snr / 20.0) / rms(&noise).max(f32::EPSILON);
    Samples {
        rate: samples.rate,
        samples: heard
            .iter()
            .zip(noise)
            .map(|(sample, noise)| (sample + noise * scale).clamp(-1.0, 1.0))
            .collect(),
    }
}

/// `count` samples of pink noise: white noise from a xorshift generator seeded with `seed`,
/// through Paul Kellet's three-pole filter ("economy" version, within 0.05 dB of 1/f above
/// 9 Hz at 44.1 kHz; near enough at 16 kHz for a background).
fn pink_noise(count: usize, seed: u64) -> Vec<f32> {
    let mut state = seed | 1;
    let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            // The top 24 bits, as a number from -1 to 1.
            let white = (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
            b0 = 0.99765 * b0 + white * 0.099_046;
            b1 = 0.963 * b1 + white * 0.296_516_4;
            b2 = 0.57 * b2 + white * 1.052_691_3;
            b0 + b1 + b2 + white * 0.1848
        })
        .collect()
}
