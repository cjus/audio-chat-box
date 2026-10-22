//! Listens to WAV files as `voice/listen` would hear them streamed (cjus/chessgui#134): finds
//! each utterance with Silero VAD, hears it with Parakeet, and says where each started and
//! ended. For trying the end-of-turn settings on recordings, such as a move said with a pause
//! in it ("knight to… f3").
//!
//! ```text
//! cargo run --release --example listen -- \
//!     <models folder> <file.wav>... [--end-silence MS] [--min-speech MS]
//! ```
//!
//! The models folder holds the unpacked archives, such as Solrac's `…/Solrac/models`, with
//! `silero-vad-v5` and `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` in it. The files must be
//! 16 kHz. They are heard one after another, as one stream, a second of silence before each
//! and after the last, in pieces of 100 ms, as Solrac sends them.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use solrac_voice::Samples;
use solrac_voice::hear::{Transcriber, samples_of_wav};
use solrac_voice::listen::{self, Detector, Event, Settings, Utterances};
use solrac_voice::models::{PARAKEET, SILERO_VAD};

/// The silence put before each file and after the last.
const GAP: Duration = Duration::from_secs(1);

/// How much audio is heard at a time: as Solrac streams it.
const PIECE: Duration = Duration::from_millis(100);

/// Takes `--name MS` out of `args`, if it is there.
fn millis(args: &mut Vec<String>, name: &str) -> Result<Option<u64>, String> {
    let Some(at) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    let value = args
        .get(at + 1)
        .and_then(|value| value.parse().ok())
        .ok_or(format!("{name} needs a number of milliseconds"))?;
    args.drain(at..=at + 1);
    Ok(Some(value))
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let settings = millis(&mut args, "--end-silence").and_then(|end| {
        let min = millis(&mut args, "--min-speech")?;
        Settings::from_millis(end, min, None)
    });
    let settings = match settings {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let [folder, files @ ..] = args.as_slice() else {
        eprintln!(
            "usage: listen <models folder> <file.wav>... [--end-silence MS] [--min-speech MS]"
        );
        return ExitCode::FAILURE;
    };
    let folder = PathBuf::from(folder);
    let loaded = Detector::load(&SILERO_VAD, &folder.join(SILERO_VAD.folder)).and_then(|vad| {
        let transcriber = Transcriber::load(&PARAKEET, &folder.join(PARAKEET.folder))?;
        Ok((vad, transcriber))
    });
    let (vad, transcriber) = match loaded {
        Ok(models) => models,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let gap = vec![0.0; listen::samples_in(GAP)];
    let mut audio = Vec::new();
    for file in files {
        let samples = std::fs::read(file)
            .map_err(|error| error.to_string())
            .and_then(|wav| samples_of_wav(&wav).map_err(|error| error.to_string()));
        match samples {
            Ok(samples) if samples.rate == listen::SAMPLE_RATE => {
                audio.extend_from_slice(&gap);
                println!(
                    "{file}: from {:.2} s",
                    audio.len() as f64 / f64::from(listen::SAMPLE_RATE)
                );
                audio.extend(samples.samples);
            }
            Ok(samples) => eprintln!("{file}: {} Hz, not 16 kHz", samples.rate),
            Err(error) => eprintln!("{file}: {error}"),
        }
    }
    audio.extend_from_slice(&gap);
    println!(
        "pauses of {} ms end an utterance; speech is at least {} ms",
        settings.end_silence.as_millis(),
        settings.min_speech.as_millis()
    );

    let mut session = match vad.session(&settings) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let seconds = |samples: usize| samples as f64 / f64::from(listen::SAMPLE_RATE);
    let pieces = audio.chunks(listen::samples_in(PIECE));
    let mut heard_to = 0;
    for piece in pieces {
        heard_to += piece.len();
        let events = session.hear(piece);
        say(events, heard_to, &transcriber, seconds);
    }
    say(session.end(), heard_to, &transcriber, seconds);
    ExitCode::SUCCESS
}

/// Prints what the detector found by `heard_to` samples, each utterance heard.
fn say(
    events: Vec<Event>,
    heard_to: usize,
    transcriber: &Transcriber,
    seconds: impl Fn(usize) -> f64,
) {
    for event in events {
        match event {
            Event::Started { at } => println!(
                "{:6.2} s  speech started, about {:.2} s",
                seconds(heard_to),
                seconds(at)
            ),
            Event::Ended { start, samples } => {
                let end = start + samples.len();
                let speech = Samples {
                    rate: listen::SAMPLE_RATE,
                    samples,
                };
                let started = Instant::now();
                let text = transcriber.transcribe(&speech).unwrap_or_else(|e| e.to_string());
                println!(
                    "{:6.2} s  heard {:.2}–{:.2} s in {:.2} s: {text:?}",
                    seconds(heard_to),
                    seconds(start),
                    seconds(end),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }
}
