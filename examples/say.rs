//! Reads text aloud with a downloaded Kokoro model, writes it to a WAV file, and says how
//! long each part took: for measuring local speech (#59) and listening to it.
//!
//! ```text
//! cargo run --release --example say -- \
//!     <model folder> <out.wav> "<text>" [voice] [threads]
//! ```
//!
//! The model folder is an unpacked archive, such as Solrac's
//! `…/models/kokoro-int8-multi-lang-v1_0`. The text is read a sentence at a time, as the chat
//! reads a reply, and the sentences are joined in the file.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use solrac_voice::models::MODELS;
use solrac_voice::speak::{Speaker, THREADS, voice_number};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [folder, out, text, rest @ ..] = args.as_slice() else {
        eprintln!("usage: say <model folder> <out.wav> \"<text>\" [voice] [threads]");
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
        let samples = match speaker.speak(sentence, number, 1.0, &stop) {
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
