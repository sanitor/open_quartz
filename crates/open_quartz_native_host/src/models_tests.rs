use super::models::ModelStore;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;

fn serve(
    payload: Vec<u8>,
    status: u16,
    declared_size: usize,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.read(&mut [0; 4096]);
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {declared_size}\r\nConnection: close\r\n\r\n").unwrap();
        let _ = stream.write_all(&payload);
    });
    (url, worker)
}
#[test]
fn download_publishes_verified_bytes_and_uses_cache_offline() {
    let directory = tempfile::tempdir().unwrap();
    let store = ModelStore::new(directory.path().to_owned());
    let model = include_bytes!("../../open_quartz/tests/data/image_identity.onnx");
    let digest = format!("{:x}", Sha256::digest(model));
    let (url, worker) = serve(model.to_vec(), 200, model.len());
    let mut progress = Vec::new();
    let path = store
        .download("identity", &url, 0, &digest, |received, total| {
            progress.push((received, total));
            Ok(())
        })
        .unwrap();
    worker.join().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), model);
    assert_eq!(
        progress.last(),
        Some(&(model.len() as u64, model.len() as u64))
    );
    assert_eq!(
        store
            .download(
                "identity",
                "http://127.0.0.1:1/offline",
                0,
                &digest,
                |_, _| Ok(())
            )
            .unwrap(),
        path
    );
    assert!(store
        .download(
            "identity",
            "http://127.0.0.1:1/offline",
            0,
            &"0".repeat(64),
            |_, _| Ok(())
        )
        .unwrap_err()
        .contains("Model download failed"));
    assert_eq!(std::fs::read(&path).unwrap(), model);
    std::fs::write(&path, b"corrupt").unwrap();
    let (replacement_url, replacement_worker) = serve(model.to_vec(), 200, model.len());
    assert_eq!(
        store
            .download(
                "identity",
                &replacement_url,
                model.len() as u64,
                &digest,
                |_, _| Ok(())
            )
            .unwrap(),
        path
    );
    replacement_worker.join().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), model);
}

#[test]
fn failed_or_cancelled_downloads_never_become_cached_models() {
    for (status, length, expected, cancel, digest) in [
        (404, 3, 0, false, ""),
        (200, 30, 0, false, ""),
        (200, 3, 4, false, ""),
        (200, 3, 0, true, ""),
        (200, 3, 0, false, "bad-digest"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_owned());
        let (url, worker) = serve(b"abc".to_vec(), status, length);
        let result = store.download("failure", &url, expected, digest, |received, _| {
            if cancel && received > 0 {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        });
        worker.join().unwrap();
        assert!(
            result.is_err(),
            "status={status} length={length} expected={expected} cancel={cancel}"
        );
        assert!(!store.is_downloaded("failure").unwrap());
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "only persistent advisory lock remains"
        );
    }
}

#[test]
fn concurrent_process_style_cache_repair_is_serialized_per_model() {
    let directory = tempfile::tempdir().unwrap();
    let store = ModelStore::new(directory.path().to_owned());
    let model = include_bytes!("../../open_quartz/tests/data/image_identity.onnx").to_vec();
    let digest = format!("{:x}", Sha256::digest(&model));
    std::fs::write(store.path("concurrent").unwrap(), b"corrupt").unwrap();
    let (url, worker) = serve(model.clone(), 200, model.len());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let directory = directory.path().to_owned();
            let url = url.clone();
            let digest = digest.clone();
            std::thread::spawn(move || {
                ModelStore::new(directory)
                    .download("concurrent", &url, 0, &digest, |_, _| Ok(()))
                    .unwrap()
            })
        })
        .collect();
    let paths: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    worker.join().unwrap();
    assert!(paths.iter().all(|path| path == &paths[0]));
    assert_eq!(std::fs::read(&paths[0]).unwrap(), model);
}

#[test]
fn model_ids_cannot_escape_the_cache_directory() {
    let directory = tempfile::tempdir().unwrap();
    let store = ModelStore::new(directory.path().to_owned());
    for id in ["", "..", "../outside", "/absolute", "a/b", "a\\b", "a:b"] {
        assert!(store.path(id).is_err(), "{id}");
    }
}
