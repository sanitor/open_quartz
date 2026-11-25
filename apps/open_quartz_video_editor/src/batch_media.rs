use crate::document::{fingerprint, MediaAsset};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub struct RelinkMatch {
    pub id: String,
    pub fingerprint: String,
    pub path: PathBuf,
}

pub fn find_originals(folder: &Path, assets: &[MediaAsset]) -> Result<Vec<RelinkMatch>, String> {
    let folder = folder.canonicalize().map_err(|error| error.to_string())?;
    if !folder.is_dir() {
        return Err("Select a directory containing original media".into());
    }
    let mut missing: HashMap<_, _> = assets
        .iter()
        .filter(|asset| !asset.online)
        .map(|asset| (asset.fingerprint.clone(), asset))
        .collect();
    let sizes: HashSet<_> = missing.values().map(|asset| asset.size_bytes).collect();
    let mut matches = Vec::new();
    let mut folders = vec![folder];
    while let Some(current) = folders.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("Cannot read {}: {error}", current.display()))?;
        let mut entries = entries
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            if missing.is_empty() {
                return Ok(matches);
            }
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            let path = entry.path();
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file() {
                let size = entry.metadata().map_err(|error| error.to_string())?.len();
                if !sizes.contains(&size) {
                    continue;
                }
                let checksum = fingerprint(&path)?;
                if let Some(asset) = missing.remove(checksum.as_str()) {
                    matches.push(RelinkMatch {
                        id: asset.id.clone(),
                        fingerprint: checksum,
                        path,
                    });
                }
            }
        }
    }
    Ok(matches)
}
