use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::outputs::{DeliveryPolicy, OutputKey, OutputSubscription, OutputTransport};
use crate::player::{PlaybackSession};
use crate::project::{DataType, Graph, SdkError, SdkErrorCode};

use super::playback::lock_runtime;
use super::subscription::Subscription;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputPolicy {
    delivery: DeliveryPolicy,
}

impl OutputPolicy {
    pub fn latest() -> Self {
        Self {
            delivery: DeliveryPolicy::Latest,
        }
    }

    pub fn every() -> Self {
        Self {
            delivery: DeliveryPolicy::Every,
        }
    }

    pub fn on_change() -> Self {
        Self {
            delivery: DeliveryPolicy::OnChange,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Output {
    key: OutputKey,
    data_type: DataType,
    runtime: Weak<Mutex<PlaybackSession>>,
    subscription_counter: Arc<AtomicU64>,
}

impl Output {
    pub fn key(&self) -> &OutputKey {
        &self.key
    }

    pub fn subscribe(&self, policy: OutputPolicy) -> Result<Subscription, SdkError> {
        let runtime = self.runtime.upgrade().ok_or_else(|| {
            SdkError::new(SdkErrorCode::Disposed, "Output player has been closed")
        })?;
        let subscription_id = format!(
            "sdk-output-{}",
            self.subscription_counter.fetch_add(1, Ordering::Relaxed)
        );
        let transport = if matches!(self.data_type, DataType::Sampler2d | DataType::SamplerCube) {
            OutputTransport::Preview
        } else {
            OutputTransport::Value
        };
        lock_runtime(&runtime)?.subscribe_output(OutputSubscription {
            subscription_id: subscription_id.clone(),
            output: self.key.clone(),
            delivery: policy.delivery,
            transport,
            max_width: None,
            max_height: None,
        })?;
        Ok(Subscription::new(Arc::downgrade(&runtime), subscription_id))
    }
}

pub(super) fn build_outputs(
    graph: &Graph,
    runtime: &Arc<Mutex<PlaybackSession>>,
    subscription_counter: &Arc<AtomicU64>,
) -> Vec<Output> {
    graph
        .nodes()
        .iter()
        .flat_map(|node| {
            node.data.outputs.iter().map(|port| Output {
                key: OutputKey::new(&node.id, &port.id),
                data_type: port.data_type,
                runtime: Arc::downgrade(runtime),
                subscription_counter: subscription_counter.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "output_coverage_tests.rs"]
mod coverage_tests;
