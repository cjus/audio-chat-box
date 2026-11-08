//! Spike: Kokoro reads a sentence, Parakeet hears it back with the time of each word piece.
//! `cargo run --release --example wordtimes -- <kokoro folder> <parakeet folder> "<text>"`
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use audio_chat_box::hear::Transcriber;
use audio_chat_box::models::MODELS;
use audio_chat_box::speak::{Speaker, THREADS, voice_number};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (kfolder, pfolder, text) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]), &args[2]);
    let find = |f: &PathBuf| {
        MODELS
            .iter()
            .find(|m| f.ends_with(m.folder))
            .expect("model folder")
    };
    let speaker = Speaker::load_with_threads(find(&kfolder), &kfolder, THREADS).unwrap();
    let ears = Transcriber::load_with_threads(find(&pfolder), &pfolder, THREADS).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let voice = voice_number("am_michael").unwrap();
    for sentence in text
        .split_inclusive(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let t = Instant::now();
        let speech = speaker.speak(sentence, voice, 1.0, &stop).unwrap();
        let made = t.elapsed().as_secs_f32();
        let audio = speech.samples.len() as f32 / speech.rate as f32;
        let t = Instant::now();
        let pieces = ears.transcribe_pieces(&speech).unwrap();
        let aligned = t.elapsed().as_secs_f32();
        println!(
            "\n{sentence:?}\n  audio {audio:.2}s, made in {made:.2}s, aligned in {aligned:.3}s"
        );
        let said = audio_chat_box::align::align(sentence, &pieces, audio);
        for w in &said {
            print!("{}:{}-{}  ", w.text, w.start_ms, w.end_ms);
        }
        println!();
        let mut line = String::new();
        for p in pieces {
            let (p, s, d) = (p.text, p.start, p.duration);
            line.push_str(&format!("[{p:?} {s:.2}+{d:.2}] "));
        }
        println!("  {line}");
    }
}
