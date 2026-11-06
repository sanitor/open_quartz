use super::*;
use std::thread;

use crate::execution::runtime::RuntimeCapabilities;
use crate::player::{Environment, OpenQuartz};

use super::super::test_support::{player_graph, recover_lock};

#[test]
fn output_keys_and_delivery_policies_cover_value_and_texture_outputs() {
    let sdk = OpenQuartz::new(Environment::headless());
    let graph = player_graph();
    let player = sdk.player(&graph).build().unwrap();
    assert_eq!(player.outputs().len(), 2);
    assert_eq!(player.outputs()[0].key().node_id, "source");
    let mut latest = player.outputs()[0]
        .subscribe(OutputPolicy::latest())
        .unwrap();
    let mut every = player.outputs()[0]
        .subscribe(OutputPolicy::every())
        .unwrap();
    let texture_output = player.outputs()[1].clone();
    let changed = texture_output.subscribe(OutputPolicy::on_change()).unwrap();
    drop(changed);
    latest.close().unwrap();
    every.close().unwrap();
    every.close().unwrap();
}

#[test]
fn outputs_reject_dropped_players_and_missing_runtime_outputs() {
    let sdk = OpenQuartz::new(Environment::headless());
    let graph = player_graph();
    let dropped_output = {
        let owner = sdk.player(&graph).build().unwrap();
        owner.outputs()[0].clone()
    };
    assert_eq!(
        dropped_output
            .subscribe(OutputPolicy::latest())
            .err()
            .expect("dropped output")
            .code,
        SdkErrorCode::Disposed
    );

    let missing_runtime = Arc::new(Mutex::new(Runtime::new(RuntimeCapabilities {
        data_paths: vec![],
    })));
    recover_lock(&missing_runtime)
        .set_graph(&Graph::default())
        .unwrap();
    let missing_output = Output {
        key: OutputKey::new("missing", "missing"),
        data_type: DataType::Float,
        runtime: Arc::downgrade(&missing_runtime),
        subscription_counter: Arc::new(AtomicU64::new(1)),
    };
    assert!(missing_output.subscribe(OutputPolicy::latest()).is_err());
}

#[test]
fn poisoned_output_preserves_runtime_lock_errors() {
    let poisoned = Arc::new(Mutex::new(Runtime::new(RuntimeCapabilities {
        data_paths: vec![],
    })));
    let worker_runtime = poisoned.clone();
    let _ = thread::spawn(move || {
        let _guard = recover_lock(&worker_runtime);
        panic!("poison runtime mutex");
    })
    .join();
    drop(recover_lock(&poisoned));
    let poisoned_output = Output {
        key: OutputKey::new("source", "value"),
        data_type: DataType::Float,
        runtime: Arc::downgrade(&poisoned),
        subscription_counter: Arc::new(AtomicU64::new(1)),
    };
    assert_eq!(
        poisoned_output
            .subscribe(OutputPolicy::latest())
            .err()
            .unwrap()
            .code,
        SdkErrorCode::InvalidState
    );
}
