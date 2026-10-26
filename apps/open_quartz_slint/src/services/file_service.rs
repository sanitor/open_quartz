use std::path::Path;

pub fn read_project(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))
}

pub fn write_project(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("Cannot save {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{read_project, write_project};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    fn test_path() -> PathBuf {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "open-quartz-slint-file-service-{}-{sequence}.json",
            std::process::id()
        ))
    }

    #[test]
    fn writes_and_reads_project_contents() {
        let path = test_path();
        write_project(&path, "{\"version\":\"0.4.0\"}").unwrap();
        assert_eq!(read_project(&path).unwrap(), "{\"version\":\"0.4.0\"}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_missing_project_path() {
        let path = test_path();
        let _ = std::fs::remove_file(&path);
        let error = read_project(&path).unwrap_err();
        assert!(error.starts_with("Cannot read "));
    }
}
