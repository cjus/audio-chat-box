//! Local, bounded HTTP demo. The GPL speech executable remains a separate process.
//! One model pair serves one inference at a time; no audio or transcripts are logged/stored.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
const MAX_BODY: usize = 960_044;
const MAX_LINE: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(90);
struct Speech {
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Value>,
    next_id: u64,
    voices: Option<Value>,
    failed: bool,
}
impl Drop for Speech {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
struct State {
    ready: bool,
    detail: String,
    speak: bool,
    hear: bool,
}
fn start(state: Arc<Mutex<State>>) -> io::Result<Speech> {
    let mut command = Command::new(
        std::env::var("AUDIO_CHAT_BOX_BIN").unwrap_or_else(|_| "audio-chat-box".into()),
    );
    command.args([
        "--models",
        &std::env::var("AUDIO_CHAT_BOX_MODELS").unwrap_or_else(|_| "/models".into()),
    ]);
    if std::env::var("AUDIO_CHAT_BOX_OFFLINE").as_deref() == Ok("1") {
        command.arg("--offline");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(8);
    thread::spawn(move || {
        let mut output = BufReader::new(output);
        loop {
            let mut line = String::new();
            match output.by_ref().take(MAX_LINE + 1).read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line.len() as u64 > MAX_LINE => break,
                _ => {}
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                break;
            };
            if value["method"] == "voice/modelState" {
                let mut s = state.lock().unwrap();
                let stage = value["params"]["state"]["stage"]
                    .as_str()
                    .unwrap_or("unknown");
                match value["params"]["id"].as_str() {
                    Some("kokoro") => s.speak = stage == "ready",
                    Some("parakeet") => s.hear = stage == "ready",
                    _ => {}
                }
                s.ready = s.speak && s.hear;
                s.detail = if s.ready {
                    "Models ready".into()
                } else if stage == "downloading" {
                    // Only outside the Docker image, which carries its models.
                    "Models: downloading; the first start downloads approximately 832 MB".into()
                } else {
                    format!("Models: {stage}")
                };
            } else if value.get("id").is_some() && value["id"] != 0 && tx.try_send(value).is_err() {
                break;
            }
        }
        let mut s = state.lock().unwrap();
        s.ready = false;
        s.detail = "Speech process stopped; restart the container".into();
    });
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocol":"1.0","client":{"name":"browser-demo","version":"0.1.0"}}})
    )?;
    Ok(Speech {
        child,
        input,
        replies: rx,
        next_id: 1,
        voices: None,
        failed: false,
    })
}
impl Speech {
    fn voices(&mut self, socket: &TcpStream) -> Result<Value, &'static str> {
        if let Some(voices) = &self.voices {
            return Ok(voices.clone());
        }
        let voices = self.call("voice/voices", json!({}), socket)?;
        if !voices["voices"].is_array() {
            return Err("Invalid voice list");
        }
        self.voices = Some(voices.clone());
        Ok(voices)
    }
    fn call(
        &mut self,
        method: &str,
        params: Value,
        _socket: &TcpStream,
    ) -> Result<Value, &'static str> {
        if self.failed {
            return Err("Speech process stopped; restart the container");
        }
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .map_err(|_| "Speech process unavailable")?;
        let until = Instant::now() + TIMEOUT;
        loop {
            match self.replies.recv_timeout(Duration::from_millis(100)) {
                Ok(v) if v["id"] == id => {
                    break if v.get("error").is_some() {
                        Err("Speech request failed; check model readiness")
                    } else {
                        Ok(v["result"].clone())
                    };
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err("Speech process stopped"),
                _ => {}
            }
            // A protocol cancellation acknowledges intent, not the end of native inference.
            // Keep the single-worker lock even if the HTTP client disconnects: drain its reply
            // and discard delivery instead of admitting another uncancellable native job.
            if Instant::now() >= until {
                self.failed = true;
                let _ = self.child.kill();
                let _ = self.child.wait();
                break Err("Speech exceeded 90 seconds; restart the container");
            }
        }
    }
}
struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}
fn read_before(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), &'static str> {
    while !bytes.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Request timed out")?;
        if remaining.is_zero() {
            return Err("Request timed out");
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| "Connection failed")?;
        let count = stream.read(bytes).map_err(|_| "Incomplete request")?;
        if count == 0 {
            return Err("Incomplete request");
        }
        bytes = &mut bytes[count..];
    }
    Ok(())
}
fn request(stream: &mut TcpStream, origin: &str) -> Result<Request, &'static str> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .ok_or("Invalid server origin")?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err("Headers too large");
        }
        let mut byte = [0];
        read_before(stream, &mut byte, deadline)?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(|_| "Invalid header")?;
    let mut lines = header.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or("").split_whitespace().collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err("HTTP/1.1 required");
    }
    let mut length = None;
    let mut hosts = 0;
    let mut origins = 0;
    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line.split_once(':').ok_or("Invalid header")?;
        let value = value.trim();
        match key.to_ascii_lowercase().as_str() {
            "host" => {
                hosts += 1;
                if value != host {
                    return Err("Host not allowed");
                }
            }
            "origin" => {
                origins += 1;
                if value != origin {
                    return Err("Origin not allowed");
                }
            }
            "content-length" => {
                if length.is_some() {
                    return Err("Duplicate length");
                }
                length = Some(value.parse::<usize>().map_err(|_| "Invalid length")?);
            }
            "transfer-encoding" => return Err("Chunked encoding not supported"),
            _ => {}
        }
    }
    if hosts != 1 || origins > 1 || (parts[0] == "POST" && origins != 1) {
        return Err("Same-origin request required");
    }
    let length = length.unwrap_or(0);
    if length > MAX_BODY {
        return Err("Body exceeds 30 second limit");
    }
    let mut body = vec![0; length];
    read_before(stream, &mut body, deadline)?;
    Ok(Request {
        method: parts[0].into(),
        path: parts[1].into(),
        body,
    })
}
fn wav_valid(w: &[u8]) -> bool {
    w.len() >= 46
        && w.len() <= MAX_BODY
        && &w[..4] == b"RIFF"
        && &w[8..16] == b"WAVEfmt "
        && w[16..36]
            == [
                16, 0, 0, 0, 1, 0, 1, 0, 128, 62, 0, 0, 0, 125, 0, 0, 2, 0, 16, 0,
            ]
        && &w[36..40] == b"data"
        && u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize == w.len() - 44
        && u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize == w.len() - 8
        && w.len().is_multiple_of(2)
}
fn playback_wav(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    // The speech engine writes float WAVE_FORMAT_EXTENSIBLE; return broadly playable PCM16.
    let mut reader =
        hound::WavReader::new(io::Cursor::new(bytes)).map_err(|_| "Invalid engine WAV")?;
    let spec = reader.spec();
    if spec.channels == 1
        && spec.sample_format == hound::SampleFormat::Int
        && spec.bits_per_sample == 16
    {
        return Ok(bytes.to_vec());
    }
    if spec.channels != 1
        || spec.sample_format != hound::SampleFormat::Float
        || spec.bits_per_sample != 32
    {
        return Err("Unexpected engine audio format");
    }
    let mut output = io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut output,
        hound::WavSpec {
            channels: 1,
            sample_rate: spec.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|_| "Could not create playback")?;
    for sample in reader.samples::<f32>() {
        let sample = sample.map_err(|_| "Invalid engine sample")?;
        if !sample.is_finite() {
            return Err("Invalid engine sample");
        }
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|_| "Could not create playback")?;
    }
    writer.finalize().map_err(|_| "Could not finish playback")?;
    Ok(output.into_inner())
}
fn response(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; media-src 'self' blob:; worker-src 'self'; connect-src 'self'; frame-ancestors 'none'\r\nPermissions-Policy: microphone=(self)\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
    if status.starts_with('4') {
        // Deliver the rejection before closing an unread request body (otherwise TCP may
        // reset the connection and hide the HTTP error). Drain with a strict time/byte cap.
        let _ = stream.shutdown(Shutdown::Write);
        let deadline = Instant::now() + Duration::from_millis(100);
        let mut drained = 0;
        let mut buffer = [0; 8192];
        while drained <= MAX_BODY {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if left.is_zero() {
                break;
            }
            let _ = stream.set_read_timeout(Some(left));
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => drained += n,
            }
        }
    }
}
fn serve(mut stream: TcpStream, origin: &str, speech: &Mutex<Speech>, state: &Mutex<State>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let req = match request(&mut stream, origin) {
        Ok(r) => r,
        Err(e) => {
            response(&mut stream, "400 Bad Request", "text/plain", e.as_bytes());
            return;
        }
    };
    if req.method == "GET" {
        let asset = match req.path.as_str() {
            "/" => Some((
                "text/html; charset=utf-8",
                include_str!("../static/index.html"),
            )),
            "/app.js" => Some(("text/javascript", include_str!("../static/app.js"))),
            "/recorder.js" => Some(("text/javascript", include_str!("../static/recorder.js"))),
            "/style.css" => Some(("text/css", include_str!("../static/style.css"))),
            _ => None,
        };
        if let Some((mime, body)) = asset {
            response(&mut stream, "200 OK", mime, body.as_bytes());
            return;
        }
        if req.path == "/api/voices" {
            let Ok(mut speech) = speech.try_lock() else {
                response(
                    &mut stream,
                    "503 Service Unavailable",
                    "text/plain",
                    b"Demo busy",
                );
                return;
            };
            match speech.voices(&stream) {
                Ok(v) => response(
                    &mut stream,
                    "200 OK",
                    "application/json",
                    v.to_string().as_bytes(),
                ),
                Err(e) => response(&mut stream, "502 Bad Gateway", "text/plain", e.as_bytes()),
            }
            return;
        }
        if req.path == "/api/status" {
            let s = state.lock().unwrap();
            response(
                &mut stream,
                "200 OK",
                "application/json",
                json!({"ready":s.ready,"detail":s.detail})
                    .to_string()
                    .as_bytes(),
            );
            return;
        }
    }
    let (method, params) = match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/api/transcribe") if wav_valid(&req.body) => (
            "voice/transcribe",
            json!({"format":"wav","audio":STANDARD.encode(req.body)}),
        ),
        ("POST", "/api/speak") => {
            let body = serde_json::from_slice::<Value>(&req.body).unwrap_or(Value::Null);
            let text = body["text"].as_str().map(str::to_owned);
            let Some(text) = text.filter(|s| !s.trim().is_empty() && s.chars().count() <= 1000)
            else {
                response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Provide 1 to 1000 characters",
                );
                return;
            };
            (
                "voice/speak",
                json!({"text":text,"format":"wav","voice":body["voice"]}),
            )
        }
        _ => {
            response(
                &mut stream,
                "400 Bad Request",
                "text/plain",
                b"Unknown route or invalid mono 16kHz PCM16 WAV",
            );
            return;
        }
    };
    if !state.lock().unwrap().ready {
        response(
            &mut stream,
            "503 Service Unavailable",
            "text/plain",
            b"Models not ready",
        );
        return;
    }
    let Ok(mut speech) = speech.try_lock() else {
        response(
            &mut stream,
            "503 Service Unavailable",
            "text/plain",
            b"Demo busy; one inference at a time",
        );
        return;
    };
    if method == "voice/speak" {
        let Ok(voices) = speech.voices(&stream) else {
            response(
                &mut stream,
                "502 Bad Gateway",
                "text/plain",
                b"Voice list unavailable",
            );
            return;
        };
        if !params["voice"].is_null()
            && !voices["voices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == params["voice"] && params["voice"].is_string())
        {
            response(
                &mut stream,
                "400 Bad Request",
                "text/plain",
                b"Select a voice from the engine's current voice list",
            );
            return;
        }
    }
    match speech.call(method, params, &stream) {
        Ok(v) if method == "voice/speak" => {
            match STANDARD.decode(v["audio"].as_str().unwrap_or("")) {
                Ok(wav) => match playback_wav(&wav) {
                    Ok(wav) => response(&mut stream, "200 OK", "audio/wav", &wav),
                    Err(e) => response(&mut stream, "502 Bad Gateway", "text/plain", e.as_bytes()),
                },
                Err(_) => response(
                    &mut stream,
                    "502 Bad Gateway",
                    "text/plain",
                    b"Invalid engine audio",
                ),
            }
        }
        Ok(v) => response(
            &mut stream,
            "200 OK",
            "application/json",
            v.to_string().as_bytes(),
        ),
        Err(e) => response(&mut stream, "502 Bad Gateway", "text/plain", e.as_bytes()),
    }
}
fn main() -> io::Result<()> {
    let origin = std::env::var("DEMO_ORIGIN").unwrap_or_else(|_| "http://localhost:8080".into());
    let listener = TcpListener::bind("0.0.0.0:8080")?;
    let state = Arc::new(Mutex::new(State {
        detail: "Starting models".into(),
        ..Default::default()
    }));
    let speech = Arc::new(Mutex::new(start(state.clone())?));
    let connections = Arc::new(AtomicUsize::new(0));
    eprintln!("Local speech demo: {origin}; do not expose publicly");
    for stream in listener.incoming() {
        let mut stream = stream?;
        if connections.fetch_add(1, Ordering::SeqCst) >= 4 {
            connections.fetch_sub(1, Ordering::SeqCst);
            response(
                &mut stream,
                "503 Service Unavailable",
                "text/plain",
                b"Too many connections",
            );
            continue;
        }
        let (speech, state, connections, origin) = (
            speech.clone(),
            state.clone(),
            connections.clone(),
            origin.clone(),
        );
        thread::spawn(move || {
            serve(stream, &origin, &speech, &state);
            connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_engine_float_wav_to_pcm16() {
        let mut input = io::Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(
            &mut input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 24000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        writer.write_sample(0.5_f32).unwrap();
        writer.finalize().unwrap();
        let converted = playback_wav(&input.into_inner()).unwrap();
        let mut reader = hound::WavReader::new(io::Cursor::new(converted)).unwrap();
        assert_eq!(reader.spec().bits_per_sample, 16);
        assert_eq!(reader.spec().sample_rate, 24000);
        assert_eq!(reader.samples::<i16>().next().unwrap().unwrap(), 16383);
    }
    #[test]
    fn rejects_non_wav_and_unbounded_audio() {
        assert!(!wav_valid(b"WebM"));
        assert!(!wav_valid(&vec![0; MAX_BODY + 1]));
    }
}
