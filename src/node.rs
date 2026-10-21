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
//! ## Pitfalls
//!
//! - **Nothing but CNP on standard output** (spec, 3.1). sherpa-onnx and ONNX Runtime log to
//!   standard error, and so does nothing else here.
//! - **The output is shared** by the loop that reads requests, the work threads and the model
//!   threads, so each message is written whole under a lock.
//! - **A model is ready before it says so.** It is stored first, then `ready` is sent, so a
//!   client that acts on the notification finds it.

use std::collections::HashMap;
use std::io::{self, BufRead, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    AudioFormat, Model, ModelState, ModelStateChanged, ModelStateParams, Models, ModelsResult,
    Speak, SpeakResult, Stage, Transcribe, TranscribeResult, VoiceCapabilities, Voices,
    VoicesResult,
};
use cnp::{Id, Message, Notification, Request, RequestMethod, Response, ResponseError, code};

use crate::hear::{self, Transcriber};
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
    jobs: Arc<Mutex<HashMap<Id, Arc<Job>>>>,
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
            config,
            state: State::Starting,
            jobs: Arc::default(),
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
            (State::Running, method) => error(
                code::METHOD_NOT_FOUND,
                &format!("{NAME} has no method {method}"),
            ),
        };
        send(&self.output, response.into());
    }

    /// Acts on a notification: only `$/cancelRequest` means anything here.
    fn notification(&mut self, notification: &Notification) {
        if self.state != State::Running || !notification.is::<CancelRequest>() {
            return;
        }
        let Ok(params) = notification.params::<CancelRequest>() else {
            return;
        };
        let job = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&params.id);
        if let Some(job) = job {
            cancel(&job, params.id, &self.output);
        }
    }

    /// Answers every request being worked on with `Cancelled` (a cancel, or `shutdown`).
    fn cancel_all(&mut self) {
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
            spawn_prepare(Arc::clone(slot), dir, offline, &self.output, load);
        }
    }

    fn models(&self) -> ModelsResult {
        let speech = self.speech.as_ref().map(|slot| slot.listed());
        let hearing = self.hearing.as_ref().map(|slot| slot.listed());
        ModelsResult {
            models: speech.into_iter().chain(hearing).collect(),
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
            let primary = language.split('-').next().unwrap_or_default();
            if !primary.eq_ignore_ascii_case(hear::LANGUAGE) {
                return Err(unsupported("Parakeet hears English only"));
            }
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

    /// A running node with both models ready (fakes), and what it writes.
    fn ready_node(pause: Duration) -> (Node, Written) {
        let written = Written::default();
        let mut node = Node::new(
            Arc::new(Mutex::new(Box::new(written.clone()))),
            Config {
                speech: Some(models::KOKORO),
                transcription: Some(models::PARAKEET),
                ..Config::default()
            },
        );
        node.state = State::Running;
        let speech: Arc<dyn Speech> = Arc::new(FakeSpeech { pause });
        let hearing: Arc<dyn Hearing> = Arc::new(FakeHearing);
        let _ = node.speech.as_ref().unwrap().ready.set(speech);
        let _ = node.hearing.as_ref().unwrap().ready.set(hearing);
        (node, written)
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
        while !(failed("kokoro") && failed("parakeet")) {
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
