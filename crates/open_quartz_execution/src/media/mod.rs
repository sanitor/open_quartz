use crate::gpu::GpuOutputHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeVideoSurfaceKind {
    Dxgi,
    IoSurface,
    DmaBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPixelFormat {
    Nv12,
    P010,
    Rgba8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalFrameSync {
    None,
    Fence { handle: u64, value: u64 },
    SyncFile { fd: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalVideoPlane {
    pub handle: u64,
    pub offset: u64,
    pub stride: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalVideoFrame {
    pub surface_kind: NativeVideoSurfaceKind,
    pub pixel_format: VideoPixelFormat,
    pub width: u32,
    pub height: u32,
    pub planes: Vec<ExternalVideoPlane>,
    pub pts_ns: i64,
    pub duration_ns: u64,
    pub sync: ExternalFrameSync,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeDecoderCapabilities {
    pub decoder: String,
    pub surface_kinds: Vec<NativeVideoSurfaceKind>,
    pub pixel_formats: Vec<VideoPixelFormat>,
    pub zero_cpu_copy: bool,
}

pub trait NativeVideoDecoder: Send {
    fn capabilities(&self) -> NativeDecoderCapabilities;
    fn next_frame(&mut self) -> Result<Option<ExternalVideoFrame>, String>;
    fn pause(&mut self) -> Result<(), String>;
    fn resume(&mut self) -> Result<(), String>;
    fn seek(&mut self, timestamp_ns: i64) -> Result<(), String>;
}

#[derive(Clone)]
pub struct ImportedVideoFrame {
    pub texture: GpuOutputHandle,
    pub pts_ns: i64,
    pub duration_ns: u64,
}

pub trait ExternalVideoFrameImporter: Send {
    fn supported_surface_kinds(&self) -> &[NativeVideoSurfaceKind];
    fn import(&mut self, frame: ExternalVideoFrame) -> Result<ImportedVideoFrame, String>;
}

pub struct NativeVideoGpuSource<D, I> {
    decoder: D,
    importer: I,
}

impl<D, I> NativeVideoGpuSource<D, I> {
    pub fn new(decoder: D, importer: I) -> Self {
        Self { decoder, importer }
    }
}

impl<D: NativeVideoDecoder, I: ExternalVideoFrameImporter> NativeVideoGpuSource<D, I> {
    pub fn next_frame(&mut self) -> Result<Option<ImportedVideoFrame>, String> {
        self.decoder
            .next_frame()?
            .map(|frame| self.importer.import(frame))
            .transpose()
    }

    pub fn pause(&mut self) -> Result<(), String> {
        self.decoder.pause()
    }

    pub fn resume(&mut self) -> Result<(), String> {
        self.decoder.resume()
    }

    pub fn seek(&mut self, timestamp_ns: i64) -> Result<(), String> {
        self.decoder.seek(timestamp_ns)
    }
}
