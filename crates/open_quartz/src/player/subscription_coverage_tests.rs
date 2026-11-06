use super::*;
use std::sync::Arc;
use std::thread;

use crate::execution::runtime::RuntimeCapabilities;
use crate::player::{Environment, OpenQuartz, OutputPolicy};
use crate::project::SdkErrorCode;

use super::super::test_support::{player_graph, recover_lock};

#[test]
fn subscription_close_handles_disposed_and_dropped_owners() {
    let sdk = OpenQuartz::new(Environment::headless());
    let graph = player_graph();
    let disposed_owner = sdk.player(&graph).build().unwrap();
    let mut disposed_subscription = disposed_owner.outputs()[0]
        .subscribe(OutputPolicy::latest())
        .unwrap();
    let disposed_runtime = disposed_subscription.runtime.upgrade().unwrap();
    recover_lock(&disposed_runtime).dispose().unwrap();
    assert!(disposed_subscription.close().is_err());
    drop(disposed_owner);

    let mut orphaned_subscription = {
        let owner = sdk.player(&graph).build().unwrap();
        owner.outputs()[0]
            .subscribe(OutputPolicy::latest())
            .unwrap()
    };
    orphaned_subscription.close().unwrap();
    assert!(orphaned_subscription.is_closed());
}

#[test]
fn poisoned_subscription_preserves_runtime_lock_errors() {
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
    let mut poisoned_subscription = Subscription {
        runtime: Arc::downgrade(&poisoned),
        subscription_id: "poisoned".to_owned(),
        closed: false,
    };
    assert_eq!(
        poisoned_subscription.close().unwrap_err().code,
        SdkErrorCode::InvalidState
    );
}
