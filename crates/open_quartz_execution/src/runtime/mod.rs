mod api;
mod clock;
mod contract;
mod output;
mod presentation;
#[cfg(not(target_arch = "wasm32"))]
mod execution;

#[cfg(not(target_arch = "wasm32"))]
pub use execution::{
    ExecutionRuntime, NativeFrameRendered, NativeOnnxConfig, NativeOnnxLoadOptions,
    NativeOutputEvent, NativeRuntimeInfo,
};

pub use api::Runtime;
pub use api::{HostBackend, ResourceDescriptor, RuntimeFrameInput};
pub use clock::{ClockState, CompositionClock};
pub use output::OutputRegistry;
pub use presentation::{
    PresentationPlanner, PresentationSubscription, Viewport as PresentationViewport,
};

pub use contract::{
    public_surface_manifest, AsyncCompletionEnvelope, ContentStamp, DataPathMode, DeliveryPolicy,
    FrameStamp, OutputDelivery, OutputDeliveryBatch, OutputKey, OutputPayload, OutputState,
    OutputSubscription, OutputTransport, PresentationFit, PresentationItem, PresentationSet,
    PublicSurfaceManifest, RuntimeCapabilities, SubscriptionInvalidation, Viewport,
};
