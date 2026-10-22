//! The CNP node: Solrac's voice behind the Chess Node Protocol's voice domain (spec, 8).
//!
//! [`run`] reads the client's messages, one a line, and answers them:
//!
//! - **The lifecycle** (spec, 5): `initialize`, which also starts getting the models ready,
//!   `ping`, `shutdown`, `exit`, and `$/cancelRequest`.
//! - **`voice/voices` and `voice/models`**, at once.
//! - **`voice/speak` and `voice/transcribe`.** Their params are checked at once, so wrong
//!   params are `InvalidParams` whether or not the models are ready (spec, 8). Then the work is
//!   done on a thread of its own, and the node keeps answering `ping` and cancels while Kokoro
//!   reads.
//! - **`voice/listen`** (spec, 8.5; cjus/chessgui#134): its params are checked as above, then
//!   a thread of its own hears its audio as `voice/audio` brings it. Silero VAD finds each
//!   utterance ([`listen`]), `voice/speechStarted` says it has begun, and once the speaker
//!   pauses Parakeet hears it and `voice/heard` says what was said. One listen at a time.
//! - **The models' progress:** `voice/modelState` at each new stage, and while downloading at
//!   most once each [`PROGRESS_INTERVAL`].
//!
//! ## The models
//!
//! Each model the node was started with ([`Config`]) gets ready on a thread of its own once
//! the client has initialized: downloaded into the models folder if it isn't there
//! ([`models`]), then loaded. A request that needs a model that isn't ready is
//! answered `NotReady`, saying how far the model has got. A model that failed stays failed,
//! and its state's message says why: the client does the work another way (Solrac asks
//! OpenRouter).
//!
//! ## Cancelling
//!
//! Every request is answered once. A cancel answers a running request at once with
//! `Cancelled`, and stops Kokoro at its next sentence's end; whatever the work finds after that
//! is dropped. Parakeet can't be stopped, so a cancelled transcription runs to its end (well
//! under a second) with no one waiting for it.
//!
//! A cancelled listen stops taking audio, and its thread ends with it, dropping the
//! utterance it was hearing: nothing it sends for that listen follows the cancel's answer,
//! since each notification is written only if the listen hasn't been answered, under the same
//! lock as the answer.
//!
//! ## Pitfalls
//!
//! - **Nothing but CNP on standard output** (spec, 3.1). sherpa-onnx and ONNX Runtime log to
//!   standard error, and so does nothing else here.
//! - **The output is shared** by the loop that reads requests, the work threads and the model
//!   threads, so each message is written whole under a lock.
//! - **A model is ready before it says so.** It is stored first, then `ready` is sent, so a
//!   client that acts on the notification finds it.
//! - **Audio never waits.** The loop that reads the client's messages only decodes a listen's
//!   audio and hands it to the listen's thread: it never transcribes. A loop that stopped
//!   reading while Parakeet worked would fill the pipe, and the client writing the next audio
//!   would wait for it.

use std::collections::HashMap;
use std::io::{self, BufRead, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use cnp::lifecycle::{
    CancelRequest, Capabilities, Exit, IncompatibleVersionData, Initialize, InitializeResult,
    Limits, NodeInfo, PROTOCOL_VERSION, Ping, Shutdown, negotiate,
};
use cnp::stdio::{ReadError, read_message};
use cnp::voice::{
    Audio, AudioFormat, AudioParams, Heard, HeardParams, Listen, ListenResult, Model, ModelState,
    ModelStateChanged, ModelStateParams, Models, ModelsResult, Speak, SpeakResult, SpeechStarted,
    SpeechStartedParams, Stage, Transcribe, TranscribeResult, VoiceCapabilities, Voices,
    VoicesResult,
};
use cnp::{Id, Message, Notification, Request, RequestMethod, Response, ResponseError, code};

use crate::hear::{self, Transcriber};
use crate::listen::{self, Detect, Detector, Event, Utterances};
use crate::models::{self, LocalModel};
use crate::speak::{self, DEFAULT_VOICE, SPEEDS, Speaker};
use crate::{Samples, SpeechError};

/// The node's name, as `initialize` gives it.
pub const NAME: &str = "solrac-voice";

/// The node's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Who wrote it.
pub const AUTHOR: &str = "Carlos Justiniano";

/// The program's licence as a whole: it links eSpeak NG (GPL-3.0-or-later) through
/// sherpa-onnx. Our own code is `MIT OR Apache-2.0`.
pub const LICENSE: &str = "GPL-3.0-or-later";

/// Where its source is.
pub const SOURCE: &str = "https://github.com/cjus/solrac-voice";

/// How often a download's progress is reported, at most (spec, 8.4: "about once a second").
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

/// What the node was started with: its command line (`main.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Where the models are, or are downloaded to. Without one, every model fails.
    pub models_dir: Option<PathBuf>,
    /// The model to speak with, if the node speaks.
    pub speech: Option<LocalModel>,
    /// The model to hear with, if the node hears.
    pub transcription: Option<LocalModel>,
    /// The model that finds speech, if the node listens (with the model it hears with).
    pub listening: Option<LocalModel>,
    /// Never download: a model that isn't in the models folder fails. For tests and
    /// computers without the network.
    pub offline: bool,
}

/// Reads text aloud: Kokoro's [`Speaker`], or a fake in tests.
pub trait Speech: Send + Sync {
    /// Reads `text` in the voice numbered `voice`, at `speed`, stopping at a sentence's end
    /// once `stop` is set.
    fn speak(
        &self,
        text: &str,
        voice: usize,
        speed: f32,
        stop: &Arc<AtomicBool>,
    ) -> Result<Samples, SpeechError>;
}

impl Speech for Speaker {
    fn speak(
        &self,
        text: &str,
        voice: usize,
        speed: f32,
        stop: &Arc<AtomicBool>,
    ) -> Result<Samples, SpeechError> {
        Speaker::speak(self, text, voice, speed, stop)
    }
}

/// Hears speech: Parakeet's [`Transcriber`], or a fake in tests.
pub trait Hearing: Send + Sync {
    /// What is said in `speech`: empty if nothing was heard.
    fn transcribe(&self, speech: &Samples) -> Result<String, SpeechError>;
}

impl Hearing for Transcriber {
    fn transcribe(&self, speech: &Samples) -> Result<String, SpeechError> {
        Transcriber::transcribe(self, speech)
    }
}

/// Where the node's messages go: shared by the request loop, the work threads and the model
/// threads.
type Output = Arc<Mutex<Box<dyn Write + Send>>>;

/// Writes one message, whole. If the client has gone there is no one left to tell, so errors
/// are ignored: the request loop sees the input end and stops.
fn send(output: &Output, message: Message) {
    let mut out = output.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = cnp::stdio::write_message(&mut *out, &message);
}

/// One model the node was started with, how far it has got, and the model once it is
/// loaded.
struct Slot<T: ?Sized> {
    model: LocalModel,
    state: Mutex<ModelState>,
    ready: OnceLock<Arc<T>>,
}

impl<T: ?Sized> Slot<T> {
    fn new(model: LocalModel) -> Slot<T> {
        Slot {
            model,
            state: Mutex::new(models::state(Stage::Waiting)),
            ready: OnceLock::new(),
        }
    }

    fn state(&self) -> ModelState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Records the model's new state and tells the client.
    fn set_state(&self, state: ModelState, output: &Output) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state.clone();
        let params = ModelStateParams {
            id: self.model.id.to_string(),
            state,
        };
        send(
            output,
            Notification::new::<ModelStateChanged>(&params).into(),
        );
    }

    /// The model, if it is ready; otherwise `NotReady`, saying how far it has got.
    fn ready(&self) -> Result<Arc<T>, ResponseError> {
        if let Some(ready) = self.ready.get() {
            return Ok(Arc::clone(ready));
        }
        let state = self.state();
        let name = self.model.name;
        let message = match (state.stage, state.bytes, state.of) {
            (Stage::Failed, ..) => format!(
                "{name} failed: {}",
                state.message.as_deref().unwrap_or("no reason given")
            ),
            (Stage::Downloading, Some(bytes), Some(of)) => format!(
                "{name} isn't ready: downloading, {} of {} MB",
                megabytes(bytes),
                megabytes(of)
            ),
            (stage, ..) => format!("{name} isn't ready: {}", stage_words(stage)),
        };
        Err(ResponseError::new(code::NOT_READY, message))
    }

    /// The model as `voice/models` lists it.
    fn listed(&self) -> Model {
        Model {
            id: self.model.id.to_string(),
            name: self.model.name.to_string(),
            purpose: self.model.purpose,
            license: Some(self.model.license.to_string()),
            state: self.state(),
        }
    }
}

/// `bytes` in whole megabytes, for people.
fn megabytes(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

/// A stage in words.
fn stage_words(stage: Stage) -> &'static str {
    match stage {
        Stage::Waiting => "waiting",
        Stage::Downloading => "downloading",
        Stage::Unpacking => "unpacking",
        Stage::Loading => "loading",
        Stage::Ready => "ready",
        Stage::Failed => "failed",
    }
}

/// A request being worked on.
#[derive(Default)]
struct Job {
    /// Tells Kokoro to stop.
    stop: Arc<AtomicBool>,
    /// Set by whoever answers it first: the work, or a cancel.
    answered: AtomicBool,
}

impl Job {
    /// Sends `response`, unless the request has been answered already.
    fn answer(&self, output: &Output, response: Response) {
        if !self.answered.swap(true, Ordering::AcqRel) {
            send(output, response.into());
        }
    }
}

/// The requests being worked on, by id.
type Jobs = Arc<Mutex<HashMap<Id, Arc<Job>>>>;

/// A piece of a listen's audio, for its thread.
enum Chunk {
    /// The next samples, at [`listen::SAMPLE_RATE`].
    Samples(Vec<f32>),
    /// The audio has ended.
    End,
}

/// The listens taking audio, by id: where each one's audio goes.
type Listens = Arc<Mutex<HashMap<Id, mpsc::Sender<Chunk>>>>;

/// Where the session is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Before `initialize`.
    Starting,
    /// Between `initialize` and `shutdown`.
    Running,
    /// After `shutdown`.
    ShutDown,
}

/// The state of one CNP session.
struct Node {
    output: Output,
    config: Config,
    state: State,
    speech: Option<Arc<Slot<dyn Speech>>>,
    hearing: Option<Arc<Slot<dyn Hearing>>>,
    listening: Option<Arc<Slot<dyn Detect>>>,
    jobs: Jobs,
    listens: Listens,
}

/// Runs a CNP session until `exit` or the end of the input, and says whether the client shut
/// it down first (the exit code: 0 if so, 1 if not; spec 5.3).
pub fn run(input: impl BufRead, output: Box<dyn Write + Send>, config: Config) -> io::Result<bool> {
    let mut input = input;
    let mut node = Node::new(Arc::new(Mutex::new(output)), config);
    let max_bytes = usize::try_from(Limits::default().message_bytes()).unwrap_or(usize::MAX);
    loop {
        match read_message(&mut input, max_bytes) {
            Ok(None) => break,
            Ok(Some(Message::Request(request))) => node.request(&request),
            Ok(Some(Message::Notification(notification))) => {
                if notification.is::<Exit>() {
                    break;
                }
                node.notification(&notification);
            }
            // The node sends no requests, so it expects no responses.
            Ok(Some(Message::Response(_))) => {}
            Err(ReadError::Io(error)) => return Err(error),
            Err(error) => {
                if let Some(response) = error.response() {
                    send(&node.output, response.into());
                }
            }
        }
    }
    node.cancel_all();
    Ok(node.state == State::ShutDown)
}

impl Node {
    fn new(output: Output, config: Config) -> Node {
        Node {
            output,
            speech: config.speech.map(|model| Arc::new(Slot::new(model))),
            hearing: config.transcription.map(|model| Arc::new(Slot::new(model))),
            listening: config.listening.map(|model| Arc::new(Slot::new(model))),
            config,
            state: State::Starting,
            jobs: Arc::default(),
            listens: Arc::default(),
        }
    }

    /// Answers a request (a speech or a transcript is answered by its work thread).
    fn request(&mut self, request: &Request) {
        let id = request.id.clone();
        let error = |code, message: &str| {
            Response::error(Some(id.clone()), ResponseError::new(code, message))
        };
        let response = match (self.state, request.method.as_str()) {
            (State::Starting, Initialize::NAME) => {
                let response = self.initialize(request);
                let started = self.state == State::Running;
                send(&self.output, response.into());
                // The models' notifications come after the answer.
                if started {
                    self.prepare_models();
                }
                return;
            }
            (State::Starting, _) => error(code::NOT_INITIALIZED, "initialize first"),
            (_, Initialize::NAME) => error(code::INVALID_REQUEST, "already initialized"),
            (State::ShutDown, _) => error(code::SHUTTING_DOWN, "shutting down"),
            (State::Running, Ping::NAME) => Response::ok::<Ping>(id, &()),
            (State::Running, Shutdown::NAME) => {
                self.cancel_all();
                self.state = State::ShutDown;
                Response::ok::<Shutdown>(id, &())
            }
            (State::Running, Voices::NAME) => {
                let voices = VoicesResult {
                    voices: speak::voices(),
                };
                Response::ok::<Voices>(id, &voices)
            }
            (State::Running, Models::NAME) => Response::ok::<Models>(id, &self.models()),
            (State::Running, Speak::NAME) => match self.speak(request) {
                Ok(()) => return,
                Err(error) => Response::error(Some(id), error),
            },
            (State::Running, Transcribe::NAME) => match self.transcribe(request) {
                Ok(()) => return,
                Err(error) => Response::error(Some(id), error),
            },
            (State::Running, Listen::NAME) => match self.listen(request) {
                Ok(()) => return,
                Err(error) => Response::error(Some(id), error),
            },
            (State::Running, method) => error(
                code::METHOD_NOT_FOUND,
                &format!("{NAME} has no method {method}"),
            ),
        };
        send(&self.output, response.into());
    }

    /// Acts on a notification: `$/cancelRequest`, and a listen's audio.
    fn notification(&mut self, notification: &Notification) {
        if self.state != State::Running {
            return;
        }
        if notification.is::<Audio>() {
            if let Ok(params) = notification.params::<Audio>() {
                self.audio(params);
            }
            return;
        }
        let Ok(params) = notification.params::<CancelRequest>() else {
            return;
        };
        // A listen's thread ends when its audio stops coming.
        self.listens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&params.id);
        let job = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&params.id);
        if let Some(job) = job {
            cancel(&job, params.id, &self.output);
        }
    }

    /// Hands a piece of audio to its listen's thread. Audio for a listen the node doesn't
    /// have is ignored, since the two can cross (spec, 8.5); audio that can't be read ends
    /// the listen with `InvalidParams`.
    fn audio(&self, params: AudioParams) {
        let mut listens = self.listens.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(listen) = listens.get(&params.id) else {
            return;
        };
        if let Some(audio) = &params.audio {
            let samples = BASE64
                .decode(audio.as_bytes())
                .ok()
                .and_then(|bytes| hear::samples_of_pcm_f32(&bytes, listen::SAMPLE_RATE).ok());
            let Some(samples) = samples else {
                listens.remove(&params.id);
                drop(listens);
                let job = self
                    .jobs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&params.id);
                if let Some(job) = job {
                    let why = invalid("the audio isn't whole pcmF32 samples in base64".to_string());
                    job.stop.store(true, Ordering::Relaxed);
                    job.answer(&self.output, Response::error(Some(params.id), why));
                }
                return;
            };
            // The thread has gone only once it has answered.
            let _ = listen.send(Chunk::Samples(samples.samples));
        }
        if params.end
            && let Some(listen) = listens.remove(&params.id)
        {
            let _ = listen.send(Chunk::End);
        }
    }

    /// Answers every request being worked on with `Cancelled` (a cancel, or `shutdown`).
    fn cancel_all(&mut self) {
        // Their audio stops coming, which ends the listens' threads.
        self.listens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        let jobs: Vec<_> = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .collect();
        for (id, job) in jobs {
            cancel(&job, id, &self.output);
        }
    }

    fn initialize(&mut self, request: &Request) -> Response {
        let id = request.id.clone();
        let params = match request.params::<Initialize>() {
            Ok(params) => params,
            Err(error) => return Response::error(Some(id), error),
        };
        let Some(protocol) = negotiate(params.protocol, PROTOCOL_VERSION) else {
            let data = IncompatibleVersionData {
                supported: vec![PROTOCOL_VERSION],
            };
            let message = format!("{NAME} speaks CNP {}.x", PROTOCOL_VERSION.major);
            return Response::error(
                Some(id),
                ResponseError::new(code::INCOMPATIBLE_VERSION, message).with_data(&data),
            );
        };
        self.state = State::Running;
        let result = InitializeResult {
            protocol,
            node: NodeInfo {
                name: NAME.to_string(),
                version: VERSION.to_string(),
                author: Some(AUTHOR.to_string()),
                license: LICENSE.to_string(),
                source: Some(SOURCE.to_string()),
            },
            capabilities: Capabilities {
                engine: None,
                voice: Some(VoiceCapabilities {
                    speak: self.speech.is_some(),
                    transcribe: self.hearing.is_some(),
                    listen: self.listening.is_some() && self.hearing.is_some(),
                    word_times: false,
                    formats: vec![AudioFormat::Wav, AudioFormat::PcmF32],
                }),
            },
            limits: Limits::default(),
        };
        Response::ok::<Initialize>(id, &result)
    }

    /// Starts getting each model ready, on a thread of its own (see the [module docs](self)).
    fn prepare_models(&self) {
        let (dir, offline) = (self.config.models_dir.clone(), self.config.offline);
        if let Some(slot) = &self.speech {
            let load = |model: &LocalModel, folder: &Path| -> Result<Arc<dyn Speech>, _> {
                Ok(Arc::new(Speaker::load(model, folder)?))
            };
            spawn_prepare(Arc::clone(slot), dir.clone(), offline, &self.output, load);
        }
        if let Some(slot) = &self.hearing {
            let load = |model: &LocalModel, folder: &Path| -> Result<Arc<dyn Hearing>, _> {
                Ok(Arc::new(Transcriber::load(model, folder)?))
            };
            spawn_prepare(Arc::clone(slot), dir.clone(), offline, &self.output, load);
        }
        if let Some(slot) = &self.listening {
            let load = |model: &LocalModel, folder: &Path| -> Result<Arc<dyn Detect>, _> {
                Ok(Arc::new(Detector::load(model, folder)?))
            };
            spawn_prepare(Arc::clone(slot), dir, offline, &self.output, load);
        }
    }

    fn models(&self) -> ModelsResult {
        let speech = self.speech.as_ref().map(|slot| slot.listed());
        let hearing = self.hearing.as_ref().map(|slot| slot.listed());
        let listening = self.listening.as_ref().map(|slot| slot.listed());
        ModelsResult {
            models: speech.into_iter().chain(hearing).chain(listening).collect(),
        }
    }

    /// Checks a `voice/speak` request and starts reading it (see the [module docs](self)).
    fn speak(&self, request: &Request) -> Result<(), ResponseError> {
        let params = request.params::<Speak>()?;
        let voice_id = params.voice.as_deref().unwrap_or(DEFAULT_VOICE);
        let voice = speak::voice_number(voice_id)
            .ok_or_else(|| invalid(format!("there is no voice \"{voice_id}\"")))?;
        let speed = params.speed.unwrap_or(1.0);
        if !(SPEEDS.0..=SPEEDS.1).contains(&speed) {
            let (slowest, fastest) = SPEEDS;
            return Err(invalid(format!("speed is from {slowest} to {fastest}")));
        }
        if params.text.trim().is_empty() {
            return Err(invalid("there is no text to read".to_string()));
        }
        let format = params.format.unwrap_or(AudioFormat::Wav);
        let speaker = self
            .speech
            .as_ref()
            .ok_or_else(|| unsupported("this node was started without a speech model"))?
            .ready()?;
        let (id, text) = (request.id.clone(), params.text);
        self.work(id.clone(), move |job| {
            match speaker.speak(&text, voice, speed, &job.stop) {
                Ok(samples) => Some(Response::ok::<Speak>(id, &spoken(&samples, format))),
                // A cancel stopped it, and has answered.
                Err(SpeechError::Stopped) => None,
                Err(error) => Some(Response::error(Some(id), internal(error.to_string()))),
            }
        });
        Ok(())
    }

    /// Checks a `voice/transcribe` request and starts hearing it (see the [module docs](self)).
    fn transcribe(&self, request: &Request) -> Result<(), ResponseError> {
        let params = request.params::<Transcribe>()?;
        let bytes = BASE64
            .decode(params.audio.as_bytes())
            .map_err(|_| invalid("the audio isn't base64".to_string()))?;
        let samples = match params.format {
            AudioFormat::Wav => hear::samples_of_wav(&bytes)
                .map_err(|_| invalid("the audio isn't a WAV file that can be read".to_string()))?,
            AudioFormat::PcmF32 => {
                let rate = params
                    .sample_rate
                    .ok_or_else(|| invalid("pcmF32 audio needs its sampleRate".to_string()))?;
                hear::samples_of_pcm_f32(&bytes, rate).map_err(|_| {
                    invalid("pcmF32 audio is whole 4-byte samples, at a rate above 0".to_string())
                })?
            }
        };
        if let Some(language) = &params.language {
            english_only(language)?;
        }
        let transcriber = self
            .hearing
            .as_ref()
            .ok_or_else(|| unsupported("this node was started without a transcription model"))?
            .ready()?;
        let id = request.id.clone();
        self.work(id.clone(), move |_job| {
            Some(match transcriber.transcribe(&samples) {
                Ok(text) => {
                    let heard = TranscribeResult {
                        text,
                        words: Vec::new(),
                    };
                    Response::ok::<Transcribe>(id, &heard)
                }
                Err(error) => Response::error(Some(id), internal(error.to_string())),
            })
        });
        Ok(())
    }

    /// Checks a `voice/listen` request and starts hearing its audio (see the
    /// [module docs](self)).
    fn listen(&self, request: &Request) -> Result<(), ResponseError> {
        let params = request.params::<Listen>()?;
        if params.format != AudioFormat::PcmF32 {
            return Err(invalid(
                "a listen's audio is pcmF32, the format that streams".to_string(),
            ));
        }
        if params.sample_rate != listen::SAMPLE_RATE {
            return Err(invalid(format!(
                "{NAME} listens to {} samples a second",
                listen::SAMPLE_RATE
            )));
        }
        let settings = listen::Settings::from_millis(
            params.end_silence_ms,
            params.min_speech_ms,
            params.max_speech_ms,
        )
        .map_err(invalid)?;
        if let Some(language) = &params.language {
            english_only(language)?;
        }
        if params.word_times {
            return Err(unsupported("this node can't tell when each word is said"));
        }
        let (Some(listening), Some(hearing)) = (&self.listening, &self.hearing) else {
            return Err(unsupported(
                "this node was started without a listening model",
            ));
        };
        let detector = listening.ready()?;
        let hearing = hearing.ready()?;
        let session = detector
            .start(&settings)
            .map_err(|error| internal(format!("the listening model failed: {error}")))?;
        let (audio, chunks) = mpsc::channel();
        {
            let mut listens = self.listens.lock().unwrap_or_else(PoisonError::into_inner);
            if !listens.is_empty() {
                return Err(ResponseError::new(
                    code::BUSY,
                    format!("{NAME} hears one listen at a time"),
                ));
            }
            listens.insert(request.id.clone(), audio);
        }
        let job = Arc::new(Job::default());
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(request.id.clone(), Arc::clone(&job));
        let listening = Listening {
            id: request.id.clone(),
            job,
            session,
            hearing,
            output: Arc::clone(&self.output),
            jobs: Arc::clone(&self.jobs),
            listens: Arc::clone(&self.listens),
            received: 0,
            started: None,
        };
        thread::spawn(move || listening.run(&chunks));
        Ok(())
    }

    /// Does `task` for the request `id` on a thread of its own, and answers with the response
    /// it gives: `None` when it was stopped, which a cancel has already answered.
    fn work(&self, id: Id, task: impl FnOnce(&Job) -> Option<Response> + Send + 'static) {
        let job = Arc::new(Job::default());
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), Arc::clone(&job));
        let (jobs, output) = (Arc::clone(&self.jobs), Arc::clone(&self.output));
        thread::spawn(move || {
            let response = task(&job);
            jobs.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            job.answer(&output, response.unwrap_or_else(|| cancelled(id)));
        });
    }
}

/// One listen's thread: hears its audio as it comes, and says what it heard (see the
/// [module docs](self)).
struct Listening {
    id: Id,
    job: Arc<Job>,
    session: Box<dyn Utterances>,
    hearing: Arc<dyn Hearing>,
    output: Output,
    jobs: Jobs,
    listens: Listens,
    /// Samples heard so far.
    received: usize,
    /// Where the utterance being heard started, once `voice/speechStarted` has said so.
    started: Option<usize>,
}

impl Listening {
    /// Hears the audio from `chunks` until it ends (and answers the listen) or stops coming
    /// (a cancel, which has answered).
    fn run(mut self, chunks: &mpsc::Receiver<Chunk>) {
        loop {
            let (events, ended) = match chunks.recv() {
                Ok(Chunk::Samples(samples)) => {
                    self.received += samples.len();
                    (self.session.hear(&samples), false)
                }
                Ok(Chunk::End) => (self.session.end(), true),
                Err(_) => return self.forget(),
            };
            if let Err(error) = self.say(events) {
                self.forget();
                let why = internal(format!("hearing what was said failed: {error}"));
                return self
                    .job
                    .answer(&self.output, Response::error(Some(self.id), why));
            }
            if ended {
                // The detector may leave a start without its end (`listen`'s docs).
                if let Some(start) = self.started.take() {
                    self.heard(start, self.received, String::new());
                }
                self.forget();
                let result = ListenResult {
                    duration_ms: listen::millis_of(self.received),
                };
                return self
                    .job
                    .answer(&self.output, Response::ok::<Listen>(self.id, &result));
            }
        }
    }

    /// Says what the detector heard: each start once, and each utterance ended, heard.
    fn say(&mut self, events: Vec<Event>) -> Result<(), SpeechError> {
        for event in events {
            match event {
                Event::Started { at } => self.start(at),
                Event::Ended { start, samples } => {
                    self.start(start);
                    self.started = None;
                    let end = start + samples.len();
                    let speech = Samples {
                        rate: listen::SAMPLE_RATE,
                        samples,
                    };
                    let text = self.hearing.transcribe(&speech)?;
                    self.heard(start, end, text);
                }
            }
        }
        Ok(())
    }

    /// Says that speech started at sample `at`, unless it has said so for this utterance.
    fn start(&mut self, at: usize) {
        if self.started.is_none() {
            self.started = Some(at);
            let params = SpeechStartedParams {
                id: self.id.clone(),
                start_ms: listen::millis_of(at),
            };
            let message = Notification::new::<SpeechStarted>(&params).into();
            send_unless_answered(&self.output, &self.job, message);
        }
    }

    /// Says that `text` was said from sample `start` to `end`.
    fn heard(&self, start: usize, end: usize, text: String) {
        let params = HeardParams {
            id: self.id.clone(),
            text,
            start_ms: listen::millis_of(start),
            end_ms: listen::millis_of(end),
            words: Vec::new(),
        };
        send_unless_answered(
            &self.output,
            &self.job,
            Notification::new::<Heard>(&params).into(),
        );
    }

    /// Takes the listen out of the node's records, before it is answered: then a new listen
    /// may start. Only its own: once a cancel has answered it, the client may give its id to
    /// a new request (spec, 4), whose records this mustn't take.
    fn forget(&self) {
        let mut listens = self.listens.lock().unwrap_or_else(PoisonError::into_inner);
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        if jobs
            .get(&self.id)
            .is_some_and(|job| Arc::ptr_eq(job, &self.job))
        {
            jobs.remove(&self.id);
            listens.remove(&self.id);
        }
    }
}

/// Writes `message`, for `job`'s request, unless that request has been answered: nothing
/// follows a cancelled listen's answer (spec, 8.5). Checked under the output's lock, which
/// the answer also takes.
fn send_unless_answered(output: &Output, job: &Job, message: Message) {
    let mut out = output.lock().unwrap_or_else(PoisonError::into_inner);
    if !job.answered.load(Ordering::Acquire) {
        let _ = cnp::stdio::write_message(&mut *out, &message);
    }
}

/// Refuses a language Parakeet doesn't hear (it hears English, any region).
fn english_only(language: &str) -> Result<(), ResponseError> {
    let primary = language.split('-').next().unwrap_or_default();
    if primary.eq_ignore_ascii_case(hear::LANGUAGE) {
        Ok(())
    } else {
        Err(unsupported("Parakeet hears English only"))
    }
}

/// Answers the request `id` with `Cancelled`, unless it has been answered, and stops its work.
fn cancel(job: &Job, id: Id, output: &Output) {
    job.stop.store(true, Ordering::Relaxed);
    job.answer(output, cancelled(id));
}

fn cancelled(id: Id) -> Response {
    Response::error(Some(id), ResponseError::new(code::CANCELLED, "cancelled"))
}

fn invalid(message: String) -> ResponseError {
    ResponseError::new(code::INVALID_PARAMS, message)
}

fn unsupported(message: &str) -> ResponseError {
    ResponseError::new(code::UNSUPPORTED, message)
}

fn internal(message: String) -> ResponseError {
    ResponseError::new(code::INTERNAL_ERROR, message)
}

/// `voice/speak`'s answer for `samples`, in `format`.
fn spoken(samples: &Samples, format: AudioFormat) -> SpeakResult {
    let audio = match format {
        AudioFormat::Wav => wav(samples),
        AudioFormat::PcmF32 => samples
            .samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect(),
    };
    SpeakResult {
        format,
        sample_rate: samples.rate,
        audio: BASE64.encode(audio),
        duration_ms: u64::try_from(samples.duration().as_millis()).unwrap_or(u64::MAX),
        words: Vec::new(),
    }
}

/// `samples` as a WAV file: mono, 32-bit float.
fn wav(samples: &Samples) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: samples.rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut file = Cursor::new(Vec::new());
    // Writing to memory can't fail but for a file over 4 GB, hours of speech.
    if let Ok(mut writer) = hound::WavWriter::new(&mut file, spec) {
        for &sample in &samples.samples {
            let _ = writer.write_sample(sample);
        }
        let _ = writer.finalize();
    }
    file.into_inner()
}

/// Starts a thread that gets `slot`'s model ready: downloads it into `models_dir` unless it
/// is there (or `offline`), loads it with `load`, and reports each stage (see the
/// [module docs](self)).
fn spawn_prepare<T: ?Sized + Send + Sync + 'static>(
    slot: Arc<Slot<T>>,
    models_dir: Option<PathBuf>,
    offline: bool,
    output: &Output,
    load: impl FnOnce(&LocalModel, &Path) -> Result<Arc<T>, SpeechError> + Send + 'static,
) {
    let output = Arc::clone(output);
    thread::spawn(move || {
        let model = slot.model;
        let name = model.name;
        let prepared = (|| {
            let models_dir = models_dir.ok_or_else(|| {
                format!("there is no models folder: start {NAME} with --models <folder>")
            })?;
            let folder = if models::is_ready(&model, &models_dir) {
                models::folder(&model, &models_dir)
            } else if offline {
                return Err(format!(
                    "{name} isn't in {}, and {NAME} was started --offline",
                    models_dir.display()
                ));
            } else {
                download(&slot, &models_dir, &output)?
            };
            slot.set_state(models::state(Stage::Loading), &output);
            load(&model, &folder).map_err(|error| format!("{name} couldn't be loaded: {error}"))
        })();
        match prepared {
            Ok(loaded) => {
                let _ = slot.ready.set(loaded);
                slot.set_state(models::state(Stage::Ready), &output);
            }
            Err(why) => slot.set_state(models::failed(why), &output),
        }
    });
}

/// Downloads `slot`'s model into `models_dir`, reporting its progress at most once each
/// [`PROGRESS_INTERVAL`] but always at a new stage and at the end, and returns its folder.
fn download<T: ?Sized>(
    slot: &Slot<T>,
    models_dir: &Path,
    output: &Output,
) -> Result<PathBuf, String> {
    let name = slot.model.name;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("{name} couldn't be downloaded: {error}"))?;
    let mut last: Option<(Stage, Instant)> = None;
    let report = |state: ModelState| {
        let finished = state.bytes.is_some() && state.bytes == state.of;
        let due = last
            .is_none_or(|(stage, at)| stage != state.stage || at.elapsed() >= PROGRESS_INTERVAL);
        if due || finished {
            last = Some((state.stage, Instant::now()));
            slot.set_state(state, output);
        }
    };
    runtime
        .block_on(models::download(&slot.model, models_dir, report))
        .map_err(|error| format!("{name} couldn't be downloaded: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the node wrote, shared with the test.
    #[derive(Clone, Default)]
    struct Written(Arc<Mutex<Vec<u8>>>);

    impl Write for Written {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Written {
        /// The messages written so far: the whole lines, as one may be half written.
        fn messages(&self) -> Vec<Message> {
            let bytes = self.0.lock().unwrap().clone();
            String::from_utf8_lossy(&bytes)
                .split_inclusive('\n')
                .filter(|line| line.ends_with('\n'))
                .map(|line| line.parse().unwrap())
                .collect()
        }

        /// The response to `id`, waiting for it up to a few seconds.
        fn response(&self, id: i64) -> Response {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let found = self
                    .messages()
                    .into_iter()
                    .find_map(|message| match message {
                        Message::Response(response) if response.id == Some(Id::from(id)) => {
                            Some(response)
                        }
                        _ => None,
                    });
                if let Some(response) = found {
                    return response;
                }
                assert!(Instant::now() < deadline, "no response to {id}");
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// A Kokoro that makes `samples`, after waiting for `pause` or until stopped.
    struct FakeSpeech {
        pause: Duration,
    }

    impl Speech for FakeSpeech {
        fn speak(
            &self,
            text: &str,
            voice: usize,
            _speed: f32,
            stop: &Arc<AtomicBool>,
        ) -> Result<Samples, SpeechError> {
            let deadline = Instant::now() + self.pause;
            while Instant::now() < deadline {
                if stop.load(Ordering::Relaxed) {
                    return Err(SpeechError::Stopped);
                }
                thread::sleep(Duration::from_millis(1));
            }
            // One sample for each letter, and the voice's number in the first.
            let mut samples = vec![0.25; text.len()];
            samples[0] = voice as f32 / 100.0;
            Ok(Samples {
                rate: 24_000,
                samples,
            })
        }
    }

    /// A Parakeet that hears how many samples there were.
    struct FakeHearing;

    impl Hearing for FakeHearing {
        fn transcribe(&self, speech: &Samples) -> Result<String, SpeechError> {
            Ok(format!(
                "{} samples at {} Hz",
                speech.samples.len(),
                speech.rate
            ))
        }
    }

    /// A Silero VAD for which a sample of 0.5 or more is speech, and anything else silence.
    struct FakeDetect;

    impl Detect for FakeDetect {
        fn start(&self, settings: &listen::Settings) -> Result<Box<dyn Utterances>, SpeechError> {
            Ok(Box::new(FakeSession {
                min_speech: listen::samples_in(settings.min_speech),
                end_silence: listen::samples_in(settings.end_silence),
                received: 0,
                speech: None,
                silence: 0,
            }))
        }
    }

    /// [`FakeDetect`]'s session.
    struct FakeSession {
        min_speech: usize,
        end_silence: usize,
        received: usize,
        /// The speech so far, and where it started.
        speech: Option<(usize, Vec<f32>)>,
        /// The silence since the last loud sample.
        silence: usize,
    }

    impl Utterances for FakeSession {
        fn hear(&mut self, samples: &[f32]) -> Vec<Event> {
            let mut events = Vec::new();
            for &sample in samples {
                let at = self.received;
                self.received += 1;
                let loud = sample >= 0.5;
                match &mut self.speech {
                    None if loud => self.speech = Some((at, vec![sample])),
                    None => {}
                    Some((_, speech)) if loud => {
                        speech.extend(std::iter::repeat_n(0.0, self.silence));
                        speech.push(sample);
                        self.silence = 0;
                    }
                    Some(_) => self.silence += 1,
                }
                if let Some((start, speech)) = &self.speech {
                    if speech.len() == self.min_speech && loud && self.silence == 0 {
                        events.push(Event::Started { at: *start });
                    }
                    if self.silence == self.end_silence {
                        if speech.len() >= self.min_speech {
                            let (start, samples) = (*start, speech.clone());
                            events.push(Event::Ended { start, samples });
                        }
                        self.speech = None;
                        self.silence = 0;
                    }
                }
            }
            events
        }

        fn end(&mut self) -> Vec<Event> {
            self.silence = 0;
            match self.speech.take() {
                Some((start, samples)) if samples.len() >= self.min_speech => {
                    vec![Event::Ended { start, samples }]
                }
                _ => Vec::new(),
            }
        }
    }

    /// A running node with every model ready (fakes), and what it writes.
    fn ready_node(pause: Duration) -> (Node, Written) {
        let written = Written::default();
        let mut node = Node::new(
            Arc::new(Mutex::new(Box::new(written.clone()))),
            Config {
                speech: Some(models::KOKORO),
                transcription: Some(models::PARAKEET),
                listening: Some(models::SILERO_VAD),
                ..Config::default()
            },
        );
        node.state = State::Running;
        let speech: Arc<dyn Speech> = Arc::new(FakeSpeech { pause });
        let hearing: Arc<dyn Hearing> = Arc::new(FakeHearing);
        let listening: Arc<dyn Detect> = Arc::new(FakeDetect);
        let _ = node.speech.as_ref().unwrap().ready.set(speech);
        let _ = node.hearing.as_ref().unwrap().ready.set(hearing);
        let _ = node.listening.as_ref().unwrap().ready.set(listening);
        (node, written)
    }

    /// `count` samples of `level`, as a listen's audio: base64 pcmF32.
    fn audio(level: f32, count: usize) -> String {
        BASE64.encode(level.to_le_bytes().repeat(count))
    }

    /// Sends listen `id` a piece of audio, or its end.
    fn send_audio(node: &mut Node, id: i64, audio: Option<String>, end: bool) {
        let params = AudioParams {
            id: Id::from(id),
            audio,
            end,
        };
        node.notification(&Notification::new::<Audio>(&params));
    }

    /// The notifications the node sent so far, by method, with their params.
    fn notifications(written: &Written) -> Vec<(String, serde_json::Value)> {
        written
            .messages()
            .into_iter()
            .filter_map(|message| match message {
                Message::Notification(n) if !n.is::<ModelStateChanged>() => {
                    Some((n.method, n.params.unwrap_or_default()))
                }
                _ => None,
            })
            .collect()
    }

    /// A listen that ends an utterance after 10 ms of silence, and takes 5 ms of sound as
    /// speech.
    fn quick_listen(id: i64) -> Request {
        let params =
            json!({"format": "pcmF32", "sampleRate": 16000, "endSilenceMs": 10, "minSpeechMs": 5});
        request(id, "voice/listen", params)
    }

    fn request(id: i64, method: &str, params: serde_json::Value) -> Request {
        Request {
            id: Id::from(id),
            method: method.to_string(),
            params: Some(params),
        }
    }

    use serde_json::json;

    #[test]
    fn speech_comes_back_as_wav_or_raw_samples() {
        let (mut node, written) = ready_node(Duration::ZERO);
        node.request(&request(1, "voice/speak", json!({"text": "Knight to f3."})));
        let wav = written.response(1).result::<Speak>().unwrap();
        assert_eq!(wav.format, AudioFormat::Wav);
        assert_eq!(wav.sample_rate, 24_000);
        let samples = hear::samples_of_wav(&BASE64.decode(&wav.audio).unwrap()).unwrap();
        assert_eq!(samples.samples.len(), "Knight to f3.".len());
        // The default voice, am_michael, is number 16.
        assert_eq!(samples.samples[0], 0.16);

        let params = json!({"text": "e4", "voice": "bf_emma", "format": "pcmF32"});
        node.request(&request(2, "voice/speak", params));
        let raw = written.response(2).result::<Speak>().unwrap();
        assert_eq!(raw.format, AudioFormat::PcmF32);
        let bytes = BASE64.decode(&raw.audio).unwrap();
        let samples = hear::samples_of_pcm_f32(&bytes, raw.sample_rate).unwrap();
        assert_eq!(samples.samples, [0.21, 0.25]);
        assert_eq!(raw.duration_ms, 0, "two samples at 24 kHz");
    }

    #[test]
    fn bad_params_are_refused_before_anything_else() {
        let (mut node, written) = ready_node(Duration::ZERO);
        let bad = [
            json!({"text": "Hello", "voice": "nobody"}),
            json!({"text": "Hello", "speed": 9.0}),
            json!({"text": "  "}),
            json!({"words": "Hello"}),
        ];
        for (n, params) in bad.into_iter().enumerate() {
            let id = n as i64 + 1;
            node.request(&request(id, "voice/speak", params));
            let error = written.response(id).outcome.unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{}", error.message);
        }
        let bad_audio = [
            json!({"audio": "!!!", "format": "wav"}),
            json!({"audio": BASE64.encode("not a wav file"), "format": "wav"}),
            json!({"audio": BASE64.encode([0u8; 8]), "format": "pcmF32"}),
            json!({"audio": BASE64.encode([0u8; 7]), "format": "pcmF32", "sampleRate": 16000}),
        ];
        for (n, params) in bad_audio.into_iter().enumerate() {
            let id = n as i64 + 10;
            node.request(&request(id, "voice/transcribe", params));
            let error = written.response(id).outcome.unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{}", error.message);
        }
    }

    #[test]
    fn audio_is_heard_and_other_languages_refused() {
        let (mut node, written) = ready_node(Duration::ZERO);
        let audio = BASE64.encode([0u8; 16]);
        let params = json!({"audio": audio, "format": "pcmF32", "sampleRate": 16000});
        node.request(&request(1, "voice/transcribe", params));
        let heard = written.response(1).result::<Transcribe>().unwrap();
        assert_eq!(heard.text, "4 samples at 16000 Hz");

        let french =
            json!({"audio": audio, "format": "pcmF32", "sampleRate": 16000, "language": "fr"});
        node.request(&request(2, "voice/transcribe", french));
        assert_eq!(
            written.response(2).outcome.unwrap_err().code,
            code::UNSUPPORTED
        );
        let british =
            json!({"audio": audio, "format": "pcmF32", "sampleRate": 16000, "language": "en-GB"});
        node.request(&request(3, "voice/transcribe", british));
        assert!(written.response(3).result::<Transcribe>().is_ok());
    }

    #[test]
    fn a_cancel_answers_at_once_and_only_once() {
        let (mut node, written) = ready_node(Duration::from_secs(30));
        node.request(&request(7, "voice/speak", json!({"text": "A long reply."})));
        let cancel = Notification {
            method: "$/cancelRequest".to_string(),
            params: Some(json!({"id": 7})),
        };
        node.notification(&cancel);
        let error = written.response(7).outcome.unwrap_err();
        assert_eq!(error.code, code::CANCELLED);
        // The work stops, and doesn't answer again.
        thread::sleep(Duration::from_millis(100));
        let answers = written
            .messages()
            .into_iter()
            .filter(|m| matches!(m, Message::Response(r) if r.id == Some(Id::from(7))))
            .count();
        assert_eq!(answers, 1);
        assert!(node.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn a_listen_says_when_speech_starts_and_what_was_said_after_a_pause() {
        let (mut node, written) = ready_node(Duration::ZERO);
        node.request(&quick_listen(1));
        // 10 ms of silence, 20 ms of speech, 20 ms of silence, then 10 ms more speech.
        send_audio(&mut node, 1, Some(audio(0.0, 160)), false);
        send_audio(&mut node, 1, Some(audio(0.9, 320)), false);
        send_audio(&mut node, 1, Some(audio(0.0, 320)), false);
        send_audio(&mut node, 1, Some(audio(0.9, 160)), true);
        let result = written.response(1).result::<Listen>().unwrap();
        assert_eq!(result.duration_ms, 60);
        let said = notifications(&written);
        let methods: Vec<&str> = said.iter().map(|(method, _)| method.as_str()).collect();
        assert_eq!(
            methods,
            [
                "voice/speechStarted",
                "voice/heard",
                "voice/speechStarted",
                "voice/heard"
            ]
        );
        assert_eq!(said[0].1, json!({"id": 1, "startMs": 10}));
        assert_eq!(
            said[1].1,
            json!({"id": 1, "text": "320 samples at 16000 Hz", "startMs": 10, "endMs": 30})
        );
        // The audio ended mid-speech: what was said so far is heard.
        assert_eq!(said[3].1["text"], "160 samples at 16000 Hz");
        assert!(node.listens.lock().unwrap().is_empty());
        assert!(node.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn a_cancelled_listen_drops_what_it_was_hearing_and_says_nothing_more() {
        let (mut node, written) = ready_node(Duration::ZERO);
        node.request(&quick_listen(2));
        send_audio(&mut node, 2, Some(audio(0.9, 320)), false);
        let deadline = Instant::now() + Duration::from_secs(10);
        while notifications(&written).is_empty() {
            assert!(Instant::now() < deadline, "speech didn't start");
            thread::sleep(Duration::from_millis(5));
        }
        let cancel = Notification {
            method: "$/cancelRequest".to_string(),
            params: Some(json!({"id": 2})),
        };
        node.notification(&cancel);
        assert_eq!(
            written.response(2).outcome.unwrap_err().code,
            code::CANCELLED
        );
        // Audio after the cancel is ignored: the listen has gone.
        send_audio(&mut node, 2, Some(audio(0.0, 320)), true);
        thread::sleep(Duration::from_millis(100));
        let said = notifications(&written);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0].0, "voice/speechStarted");
        // And another listen may start.
        node.request(&quick_listen(3));
        send_audio(&mut node, 3, None, true);
        assert!(written.response(3).result::<Listen>().is_ok());
    }

    /// A Parakeet that takes `pause` to hear anything.
    struct SlowHearing {
        pause: Duration,
    }

    impl Hearing for SlowHearing {
        fn transcribe(&self, _speech: &Samples) -> Result<String, SpeechError> {
            thread::sleep(self.pause);
            Ok("Knight to f3.".to_string())
        }
    }

    #[test]
    fn a_cancelled_listen_leaves_a_new_listen_with_its_id_alone() {
        let (mut node, written) = ready_node(Duration::ZERO);
        let slow: Arc<dyn Hearing> = Arc::new(SlowHearing {
            pause: Duration::from_millis(300),
        });
        node.hearing = Some(Arc::new(Slot::new(models::PARAKEET)));
        let _ = node.hearing.as_ref().unwrap().ready.set(slow);
        node.request(&quick_listen(2));
        // An utterance that ends, which the listen's thread is still hearing when cancelled.
        send_audio(&mut node, 2, Some(audio(0.9, 320)), false);
        send_audio(&mut node, 2, Some(audio(0.0, 320)), false);
        thread::sleep(Duration::from_millis(50));
        let cancel = Notification {
            method: "$/cancelRequest".to_string(),
            params: Some(json!({"id": 2})),
        };
        node.notification(&cancel);
        assert_eq!(
            written.response(2).outcome.unwrap_err().code,
            code::CANCELLED
        );
        // The id is free once answered (spec, 4): a new listen takes it.
        node.request(&quick_listen(2));
        // The first listen's thread finishes hearing, and ends, after the new one started.
        thread::sleep(Duration::from_millis(500));
        send_audio(&mut node, 2, None, true);
        let deadline = Instant::now() + Duration::from_secs(10);
        let answered = || {
            written.messages().into_iter().any(|m| {
                matches!(m, Message::Response(r)
                    if r.id == Some(Id::from(2)) && r.outcome.is_ok())
            })
        };
        while !answered() {
            assert!(Instant::now() < deadline, "the new listen was forgotten");
            thread::sleep(Duration::from_millis(5));
        }
        // And nothing heard by the cancelled listen was said.
        assert!(
            !notifications(&written)
                .iter()
                .any(|(m, _)| m == "voice/heard")
        );
    }

    #[test]
    fn a_listen_is_checked_before_it_hears() {
        let (mut node, written) = ready_node(Duration::ZERO);
        let invalid_params = [
            json!({"format": "wav", "sampleRate": 16000}),
            json!({"format": "pcmF32", "sampleRate": 44100}),
            json!({"format": "pcmF32", "sampleRate": 0}),
            json!({"format": "pcmF32", "sampleRate": 16000, "endSilenceMs": 0}),
            json!({"format": "pcmF32", "sampleRate": 16000, "maxSpeechMs": 600000}),
            json!({"format": "pcmF32"}),
        ];
        for (n, params) in invalid_params.into_iter().enumerate() {
            let id = n as i64 + 1;
            node.request(&request(id, "voice/listen", params));
            let error = written.response(id).outcome.unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{}", error.message);
        }
        let unsupported = [
            json!({"format": "pcmF32", "sampleRate": 16000, "language": "fr"}),
            json!({"format": "pcmF32", "sampleRate": 16000, "wordTimes": true}),
        ];
        for (n, params) in unsupported.into_iter().enumerate() {
            let id = n as i64 + 10;
            node.request(&request(id, "voice/listen", params));
            let error = written.response(id).outcome.unwrap_err();
            assert_eq!(error.code, code::UNSUPPORTED, "{}", error.message);
        }
        // One listen at a time.
        node.request(&quick_listen(20));
        node.request(&quick_listen(21));
        assert_eq!(written.response(21).outcome.unwrap_err().code, code::BUSY);
        // Audio that isn't whole samples ends the listen.
        send_audio(&mut node, 20, Some(BASE64.encode([0u8; 6])), false);
        let error = written.response(20).outcome.unwrap_err();
        assert_eq!(error.code, code::INVALID_PARAMS, "{}", error.message);
        assert!(node.listens.lock().unwrap().is_empty());
    }

    #[test]
    fn a_listen_needs_its_models_ready_and_a_node_started_to_listen() {
        let written = Written::default();
        let mut node = Node::new(
            Arc::new(Mutex::new(Box::new(written.clone()))),
            Config {
                transcription: Some(models::PARAKEET),
                listening: Some(models::SILERO_VAD),
                ..Config::default()
            },
        );
        node.state = State::Running;
        node.request(&quick_listen(1));
        let error = written.response(1).outcome.unwrap_err();
        assert_eq!(error.code, code::NOT_READY);
        assert_eq!(error.message, "Silero VAD v5 isn't ready: waiting");

        let (mut deaf, written) = {
            let written = Written::default();
            let node = Node::new(
                Arc::new(Mutex::new(Box::new(written.clone()))),
                Config {
                    transcription: Some(models::PARAKEET),
                    ..Config::default()
                },
            );
            (node, written)
        };
        deaf.state = State::Running;
        deaf.request(&quick_listen(2));
        assert_eq!(
            written.response(2).outcome.unwrap_err().code,
            code::UNSUPPORTED
        );
    }

    #[test]
    fn a_model_that_isnt_ready_says_how_far_it_has_got() {
        let written = Written::default();
        let mut node = Node::new(
            Arc::new(Mutex::new(Box::new(written.clone()))),
            Config {
                speech: Some(models::KOKORO),
                ..Config::default()
            },
        );
        node.state = State::Running;
        let slot = node.speech.as_ref().unwrap();
        slot.set_state(models::downloading(52_000_000, 350_000_000), &node.output);
        node.request(&request(1, "voice/speak", json!({"text": "Hello."})));
        let error = written.response(1).outcome.unwrap_err();
        assert_eq!(error.code, code::NOT_READY);
        assert_eq!(
            error.message,
            "Kokoro 82M isn't ready: downloading, 52 of 350 MB"
        );
        // The client was told of the new state.
        assert!(written.messages().iter().any(|m| matches!(m,
            Message::Notification(n) if n.is::<ModelStateChanged>())));
        // No transcription model: unsupported, not "not ready".
        let audio =
            json!({"audio": BASE64.encode([0u8; 4]), "format": "pcmF32", "sampleRate": 16000});
        node.request(&request(2, "voice/transcribe", audio));
        assert_eq!(
            written.response(2).outcome.unwrap_err().code,
            code::UNSUPPORTED
        );
        // The models it has, with their state.
        node.request(&request(3, "voice/models", json!({})));
        let listed = written.response(3).result::<Models>().unwrap().models;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "kokoro");
        assert_eq!(listed[0].state.stage, Stage::Downloading);
    }

    #[test]
    fn offline_a_missing_model_fails_and_says_why() {
        let dir = std::env::temp_dir().join(format!("solrac-voice-offline-{}", std::process::id()));
        let written = Written::default();
        let mut node = Node::new(
            Arc::new(Mutex::new(Box::new(written.clone()))),
            Config {
                models_dir: Some(dir.clone()),
                speech: Some(models::KOKORO),
                transcription: Some(models::PARAKEET),
                listening: Some(models::SILERO_VAD),
                offline: true,
            },
        );
        let init = json!({"protocol": "1.0", "client": {"name": "test", "version": "1"}});
        node.request(&request(1, "initialize", init));
        assert!(written.response(1).result::<Initialize>().is_ok());
        let deadline = Instant::now() + Duration::from_secs(10);
        let failed = |id: &str| {
            written.messages().into_iter().any(|m| match m {
                Message::Notification(n) => n.params::<ModelStateChanged>().is_ok_and(|p| {
                    p.id == id
                        && p.state.stage == Stage::Failed
                        && p.state
                            .message
                            .as_deref()
                            .is_some_and(|m| m.contains("--offline"))
                }),
                _ => false,
            })
        };
        while !(failed("kokoro") && failed("parakeet") && failed("silero-vad")) {
            assert!(Instant::now() < deadline, "the models didn't fail");
            thread::sleep(Duration::from_millis(5));
        }
        // The answer to initialize came first.
        assert!(matches!(written.messages()[0], Message::Response(_)));
        node.request(&request(2, "voice/speak", json!({"text": "Hello."})));
        let error = written.response(2).outcome.unwrap_err();
        assert_eq!(error.code, code::NOT_READY);
        assert!(
            error.message.starts_with("Kokoro 82M failed:"),
            "{}",
            error.message
        );
        assert!(!dir.exists(), "offline, nothing is written");
    }
}
