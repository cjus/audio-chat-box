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
use std::path::PathBuf;
use std::process::ExitCode;

use audio_chat_box::models::{self, LocalModel, MODELS};
use audio_chat_box::node::{self, Config};

/// What the program is told on its command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Serve CNP with this config (boxed: it is far larger than the other commands).
    Serve(Box<Config>),
    /// Print the usage.
    Help,
    /// Print the version.
    Version,
}

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

/// Reads the command line (without the program's name).
fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut config = Config {
        models_dir: None,
        speech: Some(models::KOKORO),
        transcription: Some(models::PARAKEET),
        listening: None,
        offline: false,
        models_url: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--version" => return Ok(Command::Version),
            "--models" => config.models_dir = Some(PathBuf::from(value("--models")?)),
            "--speak" => config.speech = model_for(&value("--speak")?, "--speak")?,
            "--hear" => config.transcription = model_for(&value("--hear")?, "--hear")?,
            "--listen" => config.listening = model_for(&value("--listen")?, "--listen")?,
            "--offline" => config.offline = true,
            "--models-url" => {
                let url = value("--models-url")?;
                // The archives' names go after it.
                let url = if url.ends_with('/') {
                    url
                } else {
                    format!("{url}/")
                };
                config.models_url = Some(url);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let purposes = [
        (config.speech, cnp::voice::ModelPurpose::Speak, "--speak"),
        (
            config.transcription,
            cnp::voice::ModelPurpose::Transcribe,
            "--hear",
        ),
        (
            config.listening,
            cnp::voice::ModelPurpose::Listen,
            "--listen",
        ),
    ];
    for (model, purpose, flag) in purposes {
        if let Some(model) = model.filter(|model| model.purpose != purpose) {
            return Err(format!("{flag} {} is the wrong kind of model", model.id));
        }
    }
    if config.listening.is_some() && config.transcription.is_none() {
        return Err("--listen needs a model to hear with (--hear)".to_string());
    }
    Ok(Command::Serve(Box::new(config)))
}

/// The model named `id` for `flag`, or none for "none".
fn model_for(id: &str, flag: &str) -> Result<Option<LocalModel>, String> {
    if id == "none" {
        return Ok(None);
    }
    models::model(id)
        .map(Some)
        .ok_or_else(|| format!("{flag}: there is no model {id}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        super::parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn by_default_it_speaks_with_kokoro_and_hears_with_parakeet() {
        let Ok(Command::Serve(config)) = parse(&[]) else {
            panic!("expected a config");
        };
        assert_eq!(config.speech, Some(models::KOKORO));
        assert_eq!(config.transcription, Some(models::PARAKEET));
        assert_eq!(config.listening, None, "listening is asked for");
        assert_eq!(config.models_dir, None);
        assert!(!config.offline);
    }

    #[test]
    fn the_models_and_their_folder_are_chosen() {
        let Ok(Command::Serve(config)) = parse(&[
            "--models",
            "/tmp/models",
            "--speak",
            "kokoro-int8",
            "--hear",
            "none",
            "--offline",
        ]) else {
            panic!("expected a config");
        };
        let Ok(Command::Serve(listening)) = parse(&["--listen", "silero-vad"]) else {
            panic!("expected a config");
        };
        assert_eq!(listening.listening, Some(models::SILERO_VAD));
        assert_eq!(config.models_dir, Some(PathBuf::from("/tmp/models")));
        assert_eq!(config.speech, Some(models::KOKORO_INT8));
        assert_eq!(config.transcription, None);
        assert!(config.offline);
    }

    #[test]
    fn mistakes_are_refused() {
        assert!(
            parse(&["--speak", "parakeet"]).is_err(),
            "not a speech model"
        );
        assert!(
            parse(&["--hear", "kokoro"]).is_err(),
            "not a transcription model"
        );
        assert!(parse(&["--speak", "whisper"]).is_err());
        assert!(
            parse(&["--listen", "parakeet"]).is_err(),
            "not a listening model"
        );
        assert!(
            parse(&["--hear", "none", "--listen", "silero-vad"]).is_err(),
            "listening hears with the transcription model"
        );
        assert!(parse(&["--models"]).is_err());
        assert!(parse(&["--cnp"]).is_err());
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
    }
}
