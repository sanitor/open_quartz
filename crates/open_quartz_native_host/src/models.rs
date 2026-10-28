use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use open_quartz_execution::onnx::{
    NativeOnnxOptions, NativeOnnxProvider, OnnxSession, OnnxSessionInfo,
};
use sha2::{Digest, Sha256};

use crate::{NativeHost, NativeOnnxConfig};

/// Disk/HTTP policy shared by native shells. Call from a blocking resource worker.
pub struct ModelStore {
    directory: PathBuf,
}

impl ModelStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn path(&self, model_id: &str) -> Result<PathBuf, String> {
        if model_id.is_empty()
            || !model_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("Model ID must contain only ASCII letters, digits, '-' or '_'".into());
        }
        Ok(self.directory.join(format!("{model_id}.onnx")))
    }

    pub fn is_downloaded(&self, model_id: &str) -> Result<bool, String> {
        let path = self.path(model_id)?;
        let lock = self.lock_model(model_id)?;
        lock.lock_shared()
            .map_err(|e| format!("Cannot acquire shared model cache lock: {e}"))?;
        Ok(path.is_file() && fs::metadata(path).map_err(|e| e.to_string())?.len() > 0)
    }

    pub fn read_model(&self, model_id: &str) -> Result<Vec<u8>, String> {
        let path = self.path(model_id)?;
        let lock = self.lock_model(model_id)?;
        lock.lock_shared()
            .map_err(|e| format!("Cannot acquire shared model cache lock: {e}"))?;
        fs::read(&path).map_err(|e| format!("Cannot read model {}: {e}", path.display()))
    }

    fn lock_model(&self, model_id: &str) -> Result<File, String> {
        self.path(model_id)?;
        fs::create_dir_all(&self.directory)
            .map_err(|e| format!("Cannot create model cache: {e}"))?;
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.directory.join(format!(".{model_id}.lock")))
            .map_err(|e| format!("Cannot open model cache lock: {e}"))
    }

    /// Open a cached model under the same lock held by downloads and Tauri readers.
    pub fn open_cached(
        &self,
        model_id: &str,
        config: NativeOnnxConfig,
        prefer_direct_ml: bool,
    ) -> Result<PreparedOnnxModel, String> {
        let lock = self.lock_model(model_id)?;
        lock.lock_shared()
            .map_err(|e| format!("Cannot acquire shared model cache lock: {e}"))?;
        PreparedOnnxModel::open(&self.path(model_id)?, config, prefer_direct_ml)
    }

    /// an empty digest means integrity is not verified, not that a hash matched.
    pub fn download(
        &self,
        model_id: &str,
        url: &str,
        expected_size: u64,
        sha256: &str,
        mut progress: impl FnMut(u64, u64) -> Result<(), String>,
    ) -> Result<PathBuf, String> {
        let path = self.path(model_id)?;
        let model_lock = self.lock_model(model_id)?;
        model_lock
            .lock()
            .map_err(|e| format!("Cannot acquire model cache lock: {e}"))?;
        progress(0, expected_size)?;
        if path.is_file() && fs::metadata(&path).map_err(|e| e.to_string())?.len() > 0 {
            if verify_model_file(&path, expected_size, sha256).is_ok() {
                return Ok(path);
            }
        }
        fs::create_dir_all(&self.directory)
            .map_err(|e| format!("Cannot create model cache: {e}"))?;
        static NEXT_DOWNLOAD: AtomicU64 = AtomicU64::new(0);
        let temporary = self.directory.join(format!(
            ".{model_id}.{}.{}.part",
            std::process::id(),
            NEXT_DOWNLOAD.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .map_err(|e| format!("Cannot initialize model download: {e}"))?;
            let mut response = client
                .get(url)
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .map_err(|e| format!("Model download failed: {e}"))?;
            let length = response.content_length();
            if expected_size > 0 && length.is_some_and(|length| length != expected_size) {
                return Err(format!("Model download size mismatch: expected {expected_size} bytes, server declared {}", length.unwrap()));
            }
            let total = if expected_size > 0 {
                expected_size
            } else {
                length.unwrap_or(0)
            };
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| format!("Cannot create model download: {e}"))?;
            let mut buffer = [0_u8; 65_536];
            let mut received = 0;
            loop {
                let count = response
                    .read(&mut buffer)
                    .map_err(|e| format!("Model download interrupted: {e}"))?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])
                    .map_err(|e| format!("Cannot write model: {e}"))?;
                received += count as u64;
                progress(received, total)?;
            }
            if received == 0
                || length.is_some_and(|length| length != received)
                || (expected_size > 0 && expected_size != received)
            {
                return Err(format!("Model download is empty or incomplete: expected {total} bytes, received {received}"));
            }
            file.sync_all()
                .map_err(|e| format!("Cannot flush model: {e}"))?;
            drop(file);
            verify_model_file(&temporary, expected_size, sha256)?;
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|e| format!("Cannot replace invalid cached model: {e}"))?;
            }
            fs::rename(&temporary, &path)
                .map_err(|e| format!("Cannot publish model download: {e}"))?;
            Ok(path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn verify_model_file(path: &Path, expected_size: u64, digest: &str) -> Result<(), String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("Cannot inspect cached model: {e}"))?
        .len();
    if size == 0 || (expected_size > 0 && size != expected_size) {
        return Err(format!(
            "Model size mismatch: expected {expected_size} bytes, found {size}"
        ));
    }
    verify_digest(path, digest)
}

fn verify_digest(path: &Path, expected: &str) -> Result<(), String> {
    if expected.is_empty() {
        return Ok(());
    }
    let mut file = File::open(path).map_err(|e| format!("Cannot read model for checksum: {e}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("Cannot hash model: {e}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected) {
        return Err(format!("Model SHA-256 mismatch: {}", path.display()));
    }
    Ok(())
}

/// Own the session off the GPU/UI threads until its request identity is accepted.
pub struct PreparedOnnxModel {
    session: OnnxSession,
    config: NativeOnnxConfig,
}

impl PreparedOnnxModel {
    pub fn open(
        path: &Path,
        config: NativeOnnxConfig,
        prefer_direct_ml: bool,
    ) -> Result<Self, String> {
        let model = fs::read(path)
            .map_err(|e| format!("Cannot read ONNX model {}: {e}", path.display()))?;
        let provider = if prefer_direct_ml && cfg!(windows) {
            NativeOnnxProvider::DirectMl
        } else {
            NativeOnnxProvider::Cpu
        };
        let session = OnnxSession::from_memory_with_options(
            &model,
            NativeOnnxOptions {
                provider,
                allow_cpu_fallback: true,
            },
        )?;
        Ok(Self { session, config })
    }

    pub fn attach(self, host: &mut NativeHost, node_id: String) -> Result<OnnxSessionInfo, String> {
        host.load_onnx(node_id, self.session, self.config)
    }
}
