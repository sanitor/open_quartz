pub mod backend;
#[cfg(windows)]
pub mod dxgi;
pub mod executor;
pub mod presenter;
pub mod preview;
pub mod readback;
pub mod target;

pub use backend::{GpuBackend, TextureHandle, BLIT_FRAG, FULLSCREEN_VERT_WITH_UV};
#[cfg(windows)]
pub use dxgi::{D3d12VideoFormat, D3d12VideoFrame, DxgiSharedTextureExporter};
pub use executor::{GpuExecutionError, GpuExecutor, GpuOutput, GpuOutputHandle};
pub use presenter::{
    GpuPresentationFrame, GpuPresenter, LatestFrameMailbox, PresentationBackendKind,
    PresentationCapabilities, PresentationQueueStats, PresentationSubmitStats, PresenterRegistry,
    SharedTextureExporter, SharedTextureFrame, SharedTexturePlatform, SharedTexturePresenter,
};
pub use preview::{GpuPreviewImage, GpuPreviewReader};
pub use readback::{align_bytes_per_row, copy_padded_rgba};
pub use target::{RenderTarget, TextureFormat};
