//! Identity of an open source file and its original pathname. No media decoding lives here.
use std::fs::{File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    change_seconds: i64,
    #[cfg(unix)]
    change_nanoseconds: i64,
}
impl Identity {
    fn read(metadata: Metadata) -> Result<Self, String> {
        if !metadata.is_file() {
            return Err("export source must be a regular file".into());
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().map_err(|e| e.to_string())?,
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            change_seconds: metadata.ctime(),
            #[cfg(unix)]
            change_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

/// Pins the original read handle and rejects source replacement or in-place edits.
/// Metadata is checked at source preparation and every export step/commit. This
/// detects changes, not a claim of immutable or adversarially tamper-proof storage.
#[derive(Clone)]
pub(crate) struct SourceFileIdentity {
    path: PathBuf,
    file: Arc<File>,
    identity: Identity,
}
impl SourceFileIdentity {
    pub(crate) fn capture(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|e| format!("Cannot open source {}: {e}", path.display()))?;
        let identity = Identity::read(file.metadata().map_err(|e| e.to_string())?)?;
        let source = Self {
            path: path.to_owned(),
            file: Arc::new(file),
            identity,
        };
        source.validate()?;
        Ok(source)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let handle = Identity::read(self.file.metadata().map_err(|e| e.to_string())?)?;
        let named =
            Identity::read(std::fs::metadata(&self.path).map_err(|e| {
                format!("Source {} changed or disappeared: {e}", self.path.display())
            })?)?;
        if handle != self.identity || named != self.identity {
            return Err(format!(
                "Source {} changed after preparation; export cannot mix source revisions",
                self.path.display()
            ));
        }

        Ok(())
    }
    pub(crate) fn is_target(&self, path: &Path) -> Result<bool, String> {
        let source = std::fs::canonicalize(&self.path).map_err(|e| e.to_string())?;
        if path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let target = std::fs::metadata(path).map_err(|e| e.to_string())?;
                if target.dev() == self.identity.device && target.ino() == self.identity.inode {
                    return Ok(true);
                }
            }
            return std::fs::canonicalize(path)
                .map(|target| target == source)
                .map_err(|e| e.to_string());
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_and_in_place_edits_are_rejected_with_original_handle_retained() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source");
        std::fs::write(&path, b"first").unwrap();
        let original = SourceFileIdentity::capture(&path).unwrap();
        original.validate().unwrap();
        assert!(original.is_target(&path).unwrap());
        let alias = dir.path().join("hardlink");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(original.is_target(&alias).unwrap());
        assert!(!original.is_target(&dir.path().join("new-target")).unwrap());
        let replacement = dir.path().join("replacement");
        std::fs::write(&replacement, b"other").unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert!(original.validate().is_err());
        let current = SourceFileIdentity::capture(&path).unwrap();
        std::fs::write(&path, b"edited-content").unwrap();
        assert!(current.validate().is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(current.validate().is_err());
    }
}
