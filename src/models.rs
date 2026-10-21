//! The models: Kokoro to speak, Parakeet to hear, downloaded once from the PNX Studios CDN,
//! checked, and unpacked into the models folder Solrac names (`--models`).
//!
//! A [`LocalModel`] is one of sherpa-onnx's archives, copied unchanged to the CDN at
//! [`MODELS_URL`]. [`download`] makes sure it is unpacked in the models folder, and returns
//! its folder:
//!
//! 1. A model's folder appears in the models folder only when it is whole (step 4), so if it
//!    is there ([`is_ready`]), there is nothing to do.
//! 2. Otherwise the archive is downloaded into the models folder, and its SHA-256 is worked
//!    out as it arrives.
//! 3. Its size and SHA-256 must be the ones built into this program ([`MODELS`]). A file that
//!    differs, whether cut short or changed on the CDN or on its way, is deleted and never
//!    unpacked ([`check`]).
//! 4. It is unpacked into a folder of its own, which is then moved into place in one step
//!    ([`unpack`]), and the archive is deleted.
//!
//! On the way it reports how far it has got, as CNP's model states (spec, 8.4): downloading,
//! with the bytes so far, at most once each [`PROGRESS_STEP`], then unpacking. The node passes
//! them on as `voice/modelState`, and Solrac's title screen shows them (cjus/chessgui#92).
//!
//! ## Pitfalls
//!
//! - **A download that stops starts again from the beginning** the next time. There is no
//!   resuming. The part left behind by a download that was killed is deleted when the next
//!   one starts, once it has been left alone for [`ABANDONED`].
//! - **A stalled download fails** after [`STALL`] without data, rather than waiting forever.
//! - **Two at once.** Each download unpacks under names of its own (its process id), so they
//!   never write into each other's files. If both finish, the second finds the folder already
//!   in place and keeps it.
//! - **Paths in the archive.** tar's `unpack` refuses paths with `..` and checks symlinks
//!   (tar's "Security" docs), and only an archive with the right checksum is unpacked.
//! - **Unpacking takes a while** (bzip2, about 15 s for the full Kokoro on the M1 Max), so it
//!   runs on Tokio's blocking threads.
//!
//! sha2 (MIT or Apache-2.0), tar and bzip2 (MIT or Apache-2.0); bzip2 links libbzip2
//! (bzip2-1.0.6 licence, BSD-like). Kokoro v1.0 (Apache-2.0) and Parakeet TDT 0.6B v2
//! (CC-BY-4.0), as sherpa-onnx packages them: <https://github.com/k2-fsa/sherpa-onnx/releases>.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cnp::voice::{ModelPurpose, ModelState, Stage};
use sha2::{Digest, Sha256};

/// Where the models are on the PNX Studios CDN.
pub const MODELS_URL: &str = "https://cdn.pnxstudios.com/solrac/models/";

/// How long the download may go without data before it fails.
pub const STALL: Duration = Duration::from_secs(30);

/// How long connecting to the CDN may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How often a download reports how far it has got: each time this many more bytes have
/// come. Solrac's title screen counts in megabytes, so a finer step would show nothing new.
pub const PROGRESS_STEP: u64 = 1_000_000;

/// How long a part of a download must have been left alone before another download deletes
/// it as abandoned: twice [`STALL`], after which the download that wrote it has given up.
pub const ABANDONED: Duration = Duration::from_secs(60);

/// A model this program can download, to speak or to hear with: one of sherpa-onnx's
/// archives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalModel {
    /// Its id in CNP's `voice/models`, and on the command line (`--speak`, `--hear`).
    pub id: &'static str,
    /// Its name for people.
    pub name: &'static str,
    /// Whether it speaks or hears.
    pub purpose: ModelPurpose,
    /// The model's licence, an SPDX expression.
    pub license: &'static str,
    /// The archive's file name, the same on the CDN as in sherpa-onnx's release.
    pub archive: &'static str,
    /// The archive's size, in bytes.
    pub archive_bytes: u64,
    /// The archive's SHA-256, in lowercase hex.
    pub sha256: &'static str,
    /// The folder the archive unpacks into.
    pub folder: &'static str,
    /// The model's file in that folder (for a transcription model, its encoder): when it is
    /// there, the archive was unpacked whole.
    pub model_file: &'static str,
}

/// Kokoro v1.0 at full precision: a 350 MB download. The default: on the M1 Max it measured
/// about three times faster than the 8-bit model.
pub const KOKORO: LocalModel = LocalModel {
    id: "kokoro",
    name: "Kokoro 82M",
    purpose: ModelPurpose::Speak,
    license: "Apache-2.0",
    archive: "kokoro-multi-lang-v1_0.tar.bz2",
    archive_bytes: 349_906_910,
    sha256: "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298",
    folder: "kokoro-multi-lang-v1_0",
    model_file: "model.onnx",
};

/// Kokoro v1.0 with its weights quantised to 8-bit integers: a 132 MB download, kept for
/// CPUs where it may be faster.
pub const KOKORO_INT8: LocalModel = LocalModel {
    id: "kokoro-int8",
    name: "Kokoro 82M, 8-bit",
    purpose: ModelPurpose::Speak,
    license: "Apache-2.0",
    archive: "kokoro-int8-multi-lang-v1_0.tar.bz2",
    archive_bytes: 132_303_094,
    sha256: "4c3052abaa60943a341f193888cf6abd68787dae6ab8ae5c925a706caa247e4e",
    folder: "kokoro-int8-multi-lang-v1_0",
    model_file: "model.int8.onnx",
};

/// Parakeet TDT 0.6B v2, English, with its weights quantised to 8-bit integers: a 482 MB
/// download.
pub const PARAKEET: LocalModel = LocalModel {
    id: "parakeet",
    name: "Parakeet TDT 0.6B v2, 8-bit",
    purpose: ModelPurpose::Transcribe,
    license: "CC-BY-4.0",
    archive: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
    archive_bytes: 482_468_385,
    sha256: "157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad",
    folder: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
    model_file: "encoder.int8.onnx",
};

/// Every model this program knows.
pub const MODELS: [LocalModel; 3] = [KOKORO, KOKORO_INT8, PARAKEET];

/// The model whose [`id`](LocalModel::id) is `id`, if there is one.
pub fn model(id: &str) -> Option<LocalModel> {
    MODELS.into_iter().find(|model| model.id == id)
}

/// A model's state at `stage`, with nothing more to say.
pub fn state(stage: Stage) -> ModelState {
    ModelState {
        stage,
        bytes: None,
        of: None,
        message: None,
    }
}

/// A model's state while `bytes` of its `of` have been downloaded.
pub fn downloading(bytes: u64, of: u64) -> ModelState {
    ModelState {
        bytes: Some(bytes),
        of: Some(of),
        ..state(Stage::Downloading)
    }
}

/// A model's state when it has failed, and why.
pub fn failed(message: impl Into<String>) -> ModelState {
    ModelState {
        message: Some(message.into()),
        ..state(Stage::Failed)
    }
}

/// Where `model`'s folder is in `models_dir`, whether or not it is there yet.
pub fn folder(model: &LocalModel, models_dir: &Path) -> PathBuf {
    models_dir.join(model.folder)
}

/// Whether `model` is unpacked in `models_dir`, ready to load.
pub fn is_ready(model: &LocalModel, models_dir: &Path) -> bool {
    folder(model, models_dir).join(model.model_file).is_file()
}

/// Makes sure `model` is unpacked in `models_dir`, downloading it if it isn't there (see the
/// [module docs](self)), and returns its folder. Tells `progress` how far it has got; a model
/// already there reports nothing.
pub async fn download(
    model: &LocalModel,
    models_dir: &Path,
    progress: impl FnMut(ModelState),
) -> Result<PathBuf, DownloadError> {
    download_from(MODELS_URL, model, models_dir, progress).await
}

/// [`download`], from `base_url` in place of [`MODELS_URL`]: in tests, a server on this
/// computer.
async fn download_from(
    base_url: &str,
    model: &LocalModel,
    models_dir: &Path,
    mut progress: impl FnMut(ModelState),
) -> Result<PathBuf, DownloadError> {
    if is_ready(model, models_dir) {
        return Ok(folder(model, models_dir));
    }
    fs::create_dir_all(models_dir).map_err(DownloadError::disk)?;
    remove_abandoned(model, models_dir, ABANDONED);
    let archive = models_dir.join(format!("{}.{}.part", model.archive, std::process::id()));
    let url = format!("{base_url}{}", model.archive);
    let fetched = fetch(&url, &archive, model, &mut progress).await;
    let result = match fetched {
        Ok((bytes, sha256)) => match check(model, bytes, &sha256) {
            Ok(()) => {
                progress(state(Stage::Unpacking));
                let (model, models_dir, path) = (*model, models_dir.to_path_buf(), archive.clone());
                tokio::task::spawn_blocking(move || unpack(&path, &model, &models_dir))
                    .await
                    .unwrap_or_else(|_| Err(DownloadError::Unpack("it stopped".to_string())))
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    let _ = fs::remove_file(&archive);
    result
}

/// Deletes the parts of `model`'s archive in `models_dir` that no download has written to for
/// `idle`: left behind by one that was killed (see the [module docs](self)).
fn remove_abandoned(model: &LocalModel, models_dir: &Path, idle: Duration) {
    let Ok(entries) = fs::read_dir(models_dir) else {
        return;
    };
    let prefix = format!("{}.", model.archive);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with(&prefix) && name.ends_with(".part")) {
            continue;
        }
        let idle_for = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok());
        if idle_for.is_some_and(|idle_for| idle_for >= idle) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Downloads `url` into the file `to`, refusing more than `model`'s size, and tells
/// `progress` how many bytes have come: when it starts, after each [`PROGRESS_STEP`], and at
/// the end. Returns how many bytes came and their SHA-256 in lowercase hex.
async fn fetch(
    url: &str,
    to: &Path,
    model: &LocalModel,
    progress: &mut impl FnMut(ModelState),
) -> Result<(u64, String), DownloadError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(DownloadError::network)?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(DownloadError::network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::Status(status.as_u16()));
    }
    let mut file = File::create(to).map_err(DownloadError::disk)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let of = model.archive_bytes;
    progress(downloading(bytes, of));
    let mut reported = bytes;
    loop {
        let chunk = tokio::time::timeout(STALL, response.chunk())
            .await
            .map_err(|_| DownloadError::Stalled)?
            .map_err(DownloadError::network)?;
        let Some(chunk) = chunk else { break };
        bytes += chunk.len() as u64;
        if bytes > model.archive_bytes {
            return Err(DownloadError::WrongSize {
                expected: model.archive_bytes,
                got: bytes,
            });
        }
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(DownloadError::disk)?;
        if bytes - reported >= PROGRESS_STEP || bytes == of {
            progress(downloading(bytes, of));
            reported = bytes;
        }
    }
    file.sync_all().map_err(DownloadError::disk)?;
    Ok((bytes, format!("{:x}", hasher.finalize())))
}

/// Whether a download of `bytes` bytes with SHA-256 `sha256` (lowercase hex) is `model`'s
/// archive.
pub fn check(model: &LocalModel, bytes: u64, sha256: &str) -> Result<(), DownloadError> {
    if bytes != model.archive_bytes {
        Err(DownloadError::WrongSize {
            expected: model.archive_bytes,
            got: bytes,
        })
    } else if sha256 != model.sha256 {
        Err(DownloadError::WrongChecksum)
    } else {
        Ok(())
    }
}

/// Unpacks `archive`, `model`'s checked archive, into `models_dir`: into a folder of its own
/// first, then moved into place in one step (see the [module docs](self)). Returns the
/// model's folder.
pub fn unpack(
    archive: &Path,
    model: &LocalModel,
    models_dir: &Path,
) -> Result<PathBuf, DownloadError> {
    let staging = models_dir.join(format!(
        ".unpacking-{}-{}",
        model.folder,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&staging);
    let result = unpack_into(archive, model, &staging).and_then(|unpacked| {
        let folder = folder(model, models_dir);
        match fs::rename(&unpacked, &folder) {
            Ok(()) => Ok(folder),
            // Another solrac-voice got there first.
            Err(_) if is_ready(model, models_dir) => Ok(folder),
            Err(error) => Err(DownloadError::disk(error)),
        }
    });
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Unpacks `archive` into `staging`, and returns the model's folder in it.
fn unpack_into(
    archive: &Path,
    model: &LocalModel,
    staging: &Path,
) -> Result<PathBuf, DownloadError> {
    let file = File::open(archive).map_err(DownloadError::disk)?;
    let bzip2 = bzip2::read::BzDecoder::new(BufReader::new(file));
    tar::Archive::new(bzip2)
        .unpack(staging)
        .map_err(|error| DownloadError::Unpack(error.to_string()))?;
    let unpacked = staging.join(model.folder);
    if !unpacked.join(model.model_file).is_file() {
        return Err(DownloadError::NotInArchive);
    }
    Ok(unpacked)
}

/// Why the model couldn't be downloaded. The messages are for the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// The CDN couldn't be reached, or the connection failed.
    Network(String),
    /// The CDN answered with this HTTP status.
    Status(u16),
    /// No data came for [`STALL`].
    Stalled,
    /// The download wasn't the archive's size.
    WrongSize {
        /// The archive's size.
        expected: u64,
        /// How much came (so far, if it was more).
        got: u64,
    },
    /// The download's SHA-256 wasn't the archive's.
    WrongChecksum,
    /// The archive couldn't be unpacked.
    Unpack(String),
    /// The archive didn't hold the model's file.
    NotInArchive,
    /// The models folder couldn't be written.
    Disk(String),
}

impl DownloadError {
    fn network(error: reqwest::Error) -> DownloadError {
        // Without the URL's query, though there is none: only what went wrong.
        DownloadError::Network(error.without_url().to_string())
    }

    fn disk(error: io::Error) -> DownloadError {
        DownloadError::Disk(error.to_string())
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::Network(error) => write!(f, "the CDN can't be reached ({error})"),
            DownloadError::Status(status) => write!(f, "the CDN answered {status}"),
            DownloadError::Stalled => {
                write!(f, "the download stalled for {} seconds", STALL.as_secs())
            }
            DownloadError::WrongSize { expected, got } => write!(
                f,
                "the download was {got} bytes, not the {expected} expected"
            ),
            DownloadError::WrongChecksum => write!(f, "the download's checksum is wrong"),
            DownloadError::Unpack(error) => write!(f, "it couldn't be unpacked ({error})"),
            DownloadError::NotInArchive => write!(f, "the archive doesn't hold the model"),
            DownloadError::Disk(error) => write!(f, "the models folder can't be written ({error})"),
        }
    }
}

impl std::error::Error for DownloadError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder in the system's temporary directory, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let path = std::env::temp_dir().join(format!(
                "solrac-voice-download-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A tar.bz2 archive of `files` (path, contents), and the model it is, in `dir`.
    fn archive(dir: &Path, files: &[(&str, &str)]) -> (PathBuf, LocalModel) {
        let path = dir.join("test-model.tar.bz2");
        let file = File::create(&path).unwrap();
        let bzip2 = bzip2::write::BzEncoder::new(file, bzip2::Compression::fast());
        let mut tar = tar::Builder::new(bzip2);
        for (name, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, contents.as_bytes())
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let bytes = fs::read(&path).unwrap();
        let sha256: &'static str = Box::leak(format!("{:x}", Sha256::digest(&bytes)).into());
        let model = LocalModel {
            id: "test",
            name: "A test model",
            purpose: ModelPurpose::Speak,
            license: "MIT",
            archive: "test-model.tar.bz2",
            archive_bytes: bytes.len() as u64,
            sha256,
            folder: "test-model",
            model_file: "model.onnx",
        };
        (path, model)
    }

    #[test]
    fn models_are_found_by_id() {
        assert_eq!(model("kokoro"), Some(KOKORO));
        assert_eq!(model("kokoro-int8"), Some(KOKORO_INT8));
        assert_eq!(model("parakeet"), Some(PARAKEET));
        for unknown in ["local/kokoro", "Kokoro", "whisper", ""] {
            assert_eq!(model(unknown), None, "{unknown}");
        }
        for model in MODELS {
            assert!(model.archive.starts_with(model.folder));
            assert_eq!(model.sha256.len(), 64);
            assert!(model.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn an_abandoned_part_is_deleted_and_a_live_one_kept() {
        let dir = TempDir::new("abandoned");
        let part = |pid: u32| dir.0.join(format!("{}.{pid}.part", KOKORO.archive));
        fs::write(part(1), "old").unwrap();
        fs::write(dir.0.join("other-model.tar.bz2.1.part"), "another model's").unwrap();
        // Just written: in use.
        remove_abandoned(&KOKORO, &dir.0, ABANDONED);
        assert!(part(1).exists());
        // Left alone long enough (here, no time at all).
        remove_abandoned(&KOKORO, &dir.0, Duration::ZERO);
        assert!(!part(1).exists());
        assert!(dir.0.join("other-model.tar.bz2.1.part").exists());
    }

    #[test]
    fn only_the_archive_built_into_this_program_passes_the_check() {
        let model = KOKORO;
        assert_eq!(check(&model, model.archive_bytes, model.sha256), Ok(()));
        assert_eq!(
            check(&model, model.archive_bytes - 1, model.sha256),
            Err(DownloadError::WrongSize {
                expected: model.archive_bytes,
                got: model.archive_bytes - 1
            })
        );
        let wrong = "0".repeat(64);
        assert_eq!(
            check(&model, model.archive_bytes, &wrong),
            Err(DownloadError::WrongChecksum)
        );
    }

    #[test]
    fn the_archive_is_unpacked_and_moved_into_place() {
        let dir = TempDir::new("unpack");
        let (path, model) = archive(
            &dir.0,
            &[
                ("test-model/model.onnx", "weights"),
                ("test-model/espeak-ng-data/en_dict", "rules"),
            ],
        );
        assert!(!is_ready(&model, &dir.0));
        let folder = unpack(&path, &model, &dir.0).unwrap();
        assert_eq!(folder, dir.0.join("test-model"));
        assert!(is_ready(&model, &dir.0));
        assert_eq!(
            fs::read_to_string(folder.join("espeak-ng-data/en_dict")).unwrap(),
            "rules"
        );
        // Nothing is left of the unpacking.
        let left: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.starts_with(".unpacking"))
            .collect();
        assert!(left.is_empty(), "{left:?}");

        // A second solrac-voice that unpacks too keeps the folder there.
        assert_eq!(unpack(&path, &model, &dir.0), Ok(folder));
    }

    #[test]
    fn an_archive_without_the_model_is_refused() {
        let dir = TempDir::new("no-model");
        let (path, model) = archive(&dir.0, &[("test-model/README.md", "no weights")]);
        assert_eq!(
            unpack(&path, &model, &dir.0),
            Err(DownloadError::NotInArchive)
        );
        assert!(!dir.0.join("test-model").exists());
    }

    #[test]
    fn a_file_that_isnt_an_archive_is_refused() {
        let dir = TempDir::new("not-archive");
        let (path, model) = archive(&dir.0, &[]);
        fs::write(&path, "not bzip2").unwrap();
        assert!(matches!(
            unpack(&path, &model, &dir.0),
            Err(DownloadError::Unpack(_))
        ));
        assert!(!dir.0.join("test-model").exists());
    }

    /// Serves `body` once over HTTP, on this computer, and returns the address to ask.
    fn serve_once(body: Vec<u8>) -> String {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // The request's headers end with an empty line; nothing in them matters here.
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                request.push(byte[0]);
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });
        format!("http://{address}/")
    }

    #[test]
    fn a_download_reports_its_bytes_then_unpacking() {
        let dir = TempDir::new("progress");
        // Letters from a simple generator compress poorly, so the archive is over two steps.
        let mut seed = 1u64;
        let noise: String = (0..3 * PROGRESS_STEP + PROGRESS_STEP / 2)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                char::from(b'a' + ((seed >> 33) % 26) as u8)
            })
            .collect();
        let (path, model) = archive(&dir.0, &[("test-model/model.onnx", &noise)]);
        let models_dir = dir.0.join("models");
        let url = serve_once(fs::read(&path).unwrap());
        let mut stages = Vec::new();
        let folder = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(download_from(&url, &model, &models_dir, |stage| {
                stages.push(stage)
            }))
            .unwrap();
        assert_eq!(folder, models_dir.join("test-model"));
        assert!(is_ready(&model, &models_dir));

        let of = model.archive_bytes;
        assert!(of > 2 * PROGRESS_STEP, "the archive is {of} bytes");
        assert_eq!(stages.last(), Some(&state(Stage::Unpacking)));
        let bytes: Vec<u64> = stages[..stages.len() - 1]
            .iter()
            .map(|state| match state {
                ModelState {
                    stage: Stage::Downloading,
                    bytes: Some(bytes),
                    of: Some(total),
                    ..
                } if *total == of => *bytes,
                other => panic!("expected the download's bytes, got {other:?}"),
            })
            .collect();
        // From nothing to the whole archive, a step or more apart but for the last.
        assert_eq!((bytes.first(), bytes.last()), (Some(&0), Some(&of)));
        for pair in bytes[..bytes.len() - 1].windows(2) {
            assert!(pair[1] - pair[0] >= PROGRESS_STEP, "{bytes:?}");
        }
        // At the start, at least once on the way, and at the end.
        assert!(bytes.len() >= 3, "{bytes:?}");
        // A model already there reports nothing.
        let mut again = Vec::new();
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(download_from(&url, &model, &models_dir, |stage| {
                again.push(stage)
            }))
            .unwrap();
        assert!(again.is_empty());
    }
}
