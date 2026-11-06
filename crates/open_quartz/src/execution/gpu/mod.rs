pub mod backend;
#[cfg(windows)]
pub mod dxgi;
pub mod executor;
#[cfg(not(target_arch = "wasm32"))]
pub mod presenter;
pub mod preview;
pub mod readback;
pub mod target;
#[cfg(all(target_os = "macos", feature = "native-video"))]
pub mod videotoolbox;

pub use crate::execution::shader::{BLIT_FRAG, FULLSCREEN_VERT_WITH_UV};
pub use backend::{GpuBackend, TextureHandle};
#[cfg(windows)]
pub use dxgi::{D3d12VideoFormat, D3d12VideoFrame, DxgiSharedTextureExporter};
pub use executor::{GpuExecutionError, GpuExecutor, GpuOutput, GpuOutputHandle};
#[cfg(not(target_arch = "wasm32"))]
pub use presenter::{
    GpuPresentationFrame, GpuPresenter, LatestFrameMailbox, PresentationBackendKind,
    PresentationCapabilities, PresentationQueueStats, PresentationSubmitStats, PresenterRegistry,
    SharedTextureExporter, SharedTextureFrame, SharedTexturePlatform, SharedTexturePresenter,
};
pub use preview::{GpuPreviewImage, GpuPreviewReader};
pub use readback::{align_bytes_per_row, copy_padded_rgba};
pub use target::{RenderTarget, TextureFormat};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "gpu_coverage_tests.rs"]
mod coverage_tests;
