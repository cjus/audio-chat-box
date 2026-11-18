//! The `audio-chat-box` program: a CNP voice node on its standard input and output.
//!
//! ```text
//! audio-chat-box [--models <folder>] [--models-url <url>] [--speak <model|none>]
//!                [--hear <model|none>] [--listen <model|none>] [--offline]
//! ```
//!
//! Its client starts it with its models folder (`--models`), and with the models its
//! settings choose. Solrac, its first client, also names its own copy of the models
//! (`--models-url`). It listens (`voice/listen`) only when asked to with `--listen`: a client that
//! doesn't know the listening model's purpose would fail to read `voice/models`. The program writes nothing but CNP to its standard output, and exits with 0 after
//! `shutdown` and `exit`, 1 when its input ends without them (spec, 5.3), and 2 when its
//! command line is wrong.

use std::io::{self, BufReader};
use std::process::ExitCode;

use audio_chat_box::args::{Command, parse};
use audio_chat_box::models::{self, MODELS};
use audio_chat_box::node;

fn usage() -> String {
    let ids = |purpose| {
        MODELS
            .iter()
            .filter(|model| model.purpose == purpose)
            .map(|model| model.id)
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "usage: audio-chat-box [--models <folder>] [--models-url <url>] [--speak <model|none>] \
         [--hear <model|none>] [--listen <model|none>] [--offline]\n\n\
         A voice node on standard input and output (docs/protocol.md): Kokoro reads text \
         aloud, Parakeet hears speech, Silero VAD finds it as it comes.\n\n  \
         --models <folder>  where the models are, and are downloaded to the first time\n  \
         --models-url <url> where they are downloaded from (default {})\n  \
         --speak <model>    {} (default {}), or none\n  \
         --hear <model>     {} (default {}), or none\n  \
         --listen <model>   {}, or none (the default); needs --hear\n  \
         --offline          never download: a model that isn't in the folder fails\n  \
         --version          print the version\n",
        models::MODELS_URL,
        ids(cnp::voice::ModelPurpose::Speak),
        models::KOKORO.id,
        ids(cnp::voice::ModelPurpose::Transcribe),
        models::PARAKEET.id,
        ids(cnp::voice::ModelPurpose::Listen),
    )
}

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)) {
        Ok(Command::Serve(config)) => {
            let input = BufReader::new(io::stdin());
            match node::run(input, Box::new(io::stdout()), *config) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(error) => {
                    eprintln!("audio-chat-box: reading the input failed: {error}");
                    ExitCode::from(1)
                }
            }
        }
        Ok(Command::Help) => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("{} {}", node::NAME, node::VERSION);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("audio-chat-box: {error}\n\n{}", usage());
            ExitCode::from(2)
        }
    }
}
