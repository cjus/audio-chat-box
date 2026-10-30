//! Reads text aloud with a downloaded Kokoro model, writes it to a WAV file, and says how
//! long each part took: for measuring local speech (#59) and listening to it.
//!
//! ```text
//! cargo run --release --example say -- \
//!     <model folder> <out.wav> "<text>" [voice] [threads] [--speed SPEED]
//! ```
//!
//! The model folder is an unpacked archive, such as Solrac's
//! `…/models/kokoro-int8-multi-lang-v1_0`. The text is read a sentence at a time, as the chat
//! reads a reply, and the sentences are joined in the file. `--speed` reads it faster (1.4)
//! or slower (0.8) than Kokoro's normal pace, 1: for a transcription bake-off's fast clips.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use audio_chat_box::models::MODELS;
use audio_chat_box::speak::{Speaker, THREADS, voice_number};

/// Kokoro's normal pace.
const NORMAL_SPEED: f32 = 1.0;

fn main() -> ExitCode {
    const USAGE: &str =
        "usage: say <model folder> <out.wav> \"<text>\" [voice] [threads] [--speed SPEED]";
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut speed = NORMAL_SPEED;
    if let Some(at) = args.iter().position(|arg| arg == "--speed") {
        match args.get(at + 1).and_then(|value| value.parse().ok()) {
            Some(value) if value > 0.0 => speed = value,
            _ => {
                eprintln!("--speed needs a number above 0\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        args.drain(at..=at + 1);
    }
    let [folder, out, text, rest @ ..] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let voice = rest.first().map_or("am_michael", String::as_str);
    let threads = rest.get(1).and_then(|n| n.parse().ok()).unwrap_or(THREADS);
    let folder = PathBuf::from(folder);
    let Some(model) = MODELS.iter().find(|model| folder.ends_with(model.folder)) else {
        eprintln!("{} isn't one of the models' folders", folder.display());
        return ExitCode::FAILURE;
    };

    let started = Instant::now();
    let Some(number) = voice_number(voice) else {
        eprintln!("Kokoro has no voice called {voice:?}");
        return ExitCode::FAILURE;
    };
    let speaker = match Speaker::load_with_threads(model, &folder, threads) {
        Ok(speaker) => speaker,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "{}, {voice}, {threads} threads: loaded in {:.2} s",
        model.name,
        started.elapsed().as_secs_f64()
    );

    let stop = Arc::new(AtomicBool::new(false));
    let mut all = Vec::new();
    let mut rate = 0;
    let (mut working, mut speaking) = (0.0, 0.0);
    for sentence in text.split_inclusive(['.', '!', '?']).map(str::trim) {
        if sentence.is_empty() {
            continue;
        }
        let started = Instant::now();
        let samples = match speaker.speak(sentence, number, speed, &stop) {
            Ok(samples) => samples,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        let took = started.elapsed().as_secs_f64();
        let lasts = samples.duration().as_secs_f64();
        println!(
            "{took:5.2} s to make {lasts:5.2} s of speech ({:.2}× real time): {sentence}",
            lasts / took
        );
        working += took;
        speaking += lasts;
        rate = samples.rate;
        all.extend(samples.samples);
    }
    println!(
        "in all: {working:.2} s to make {speaking:.2} s of speech ({:.2}× real time)",
        speaking / working.max(f64::EPSILON)
    );

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let written = hound::WavWriter::create(out, spec).and_then(|mut wav| {
        all.iter()
            .try_for_each(|sample| wav.write_sample(*sample))?;
        wav.finalize()
    });
    if let Err(error) = written {
        eprintln!("{out}: {error}");
        return ExitCode::FAILURE;
    }
    println!("wrote {out}");
    ExitCode::SUCCESS
}
