//! Hears WAV files with a downloaded transcription model and says what it heard and how long
//! each took: for measuring local transcription (#59) and trying it on recordings of your
//! own.
//!
//! ```text
//! cargo run --release --example hear -- \
//!     <model folder> <file.wav>... [--threads N] [--hotwords FILE]
//! ```
//!
//! The model folder is an unpacked archive, such as Solrac's
//! `…/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`. Each file is heard as
//! `voice/transcribe` hears it: with beam search, and the hotwords in `FILE` (one a line), if
//! given.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use audio_chat_box::hear::{Hotwords, Transcriber, samples_of_wav};
use audio_chat_box::models::MODELS;
use audio_chat_box::speak::THREADS;
use cnp::voice::ModelPurpose;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let threads = match args.iter().position(|arg| arg == "--threads") {
        Some(at) if at + 1 < args.len() => {
            let threads = args[at + 1].parse().unwrap_or(THREADS);
            args.drain(at..=at + 1);
            threads
        }
        _ => THREADS,
    };
    let hotwords = match args.iter().position(|arg| arg == "--hotwords") {
        Some(at) if at + 1 < args.len() => {
            let path = args[at + 1].clone();
            args.drain(at..=at + 1);
            let read = std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"));
            match read.and_then(|text| Hotwords::of(&Hotwords::lines(&text))) {
                Ok(hotwords) => hotwords,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => Hotwords::none(),
    };
    let [folder, files @ ..] = args.as_slice() else {
        eprintln!("usage: hear <model folder> <file.wav>... [--threads N] [--hotwords FILE]");
        return ExitCode::FAILURE;
    };
    let folder = PathBuf::from(folder);
    let Some(model) = MODELS
        .iter()
        .filter(|model| model.purpose == ModelPurpose::Transcribe)
        .find(|model| folder.ends_with(model.folder))
    else {
        eprintln!("{} isn't one of the models' folders", folder.display());
        return ExitCode::FAILURE;
    };

    let started = Instant::now();
    let transcriber = match Transcriber::load_with_threads(model, &folder, threads) {
        Ok(transcriber) => transcriber,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "{}, {threads} threads: loaded in {:.2} s",
        model.name,
        started.elapsed().as_secs_f64()
    );

    for file in files {
        let samples = match std::fs::read(file)
            .map_err(|error| error.to_string())
            .and_then(|wav| samples_of_wav(&wav).map_err(|error| error.to_string()))
        {
            Ok(samples) => samples,
            Err(error) => {
                eprintln!("{file}: {error}");
                continue;
            }
        };
        let started = Instant::now();
        match transcriber.transcribe(&samples, &hotwords) {
            Ok(text) => println!(
                "{file}: {:.1} s heard in {:.2} s: {text:?}",
                samples.duration().as_secs_f64(),
                started.elapsed().as_secs_f64()
            ),
            Err(error) => eprintln!("{file}: {error}"),
        }
    }
    ExitCode::SUCCESS
}
