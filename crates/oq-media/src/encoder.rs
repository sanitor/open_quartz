use std::any::Any;
use std::ffi::c_void;
use std::sync::Arc;

use crate::{ErrorKind, MediaError};

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        pixel_format: u32,
        attributes: *const c_void,
        pixel_buffer: *mut *mut c_void,
    ) -> i32;
    fn CVPixelBufferRetain(pixel_buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferRelease(pixel_buffer: *mut c_void);
    fn CVPixelBufferGetWidth(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetDataSize(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetIOSurface(pixel_buffer: *mut c_void) -> *mut c_void;
    static kCVPixelBufferIOSurfacePropertiesKey: *const c_void;
    static kCVPixelBufferMetalCompatibilityKey: *const c_void;
}
#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceGetAllocSize(surface: *mut c_void) -> usize;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFRelease(value: *const c_void);
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    static kCFBooleanTrue: *const c_void;
}

/// Retained IOSurface-backed NV12 VideoToolbox encoder input.
/// Clones preserve all opaque accounting leases without mapping either plane.
pub struct EncoderSurface {
    pixel_buffer: *mut c_void,
    leases: [Option<Arc<dyn Any + Send + Sync>>; 2],
}
unsafe impl Send for EncoderSurface {}
unsafe impl Sync for EncoderSurface {}
impl Clone for EncoderSurface {
    fn clone(&self) -> Self {
        unsafe {
            CVPixelBufferRetain(self.pixel_buffer);
        }
        Self {
            pixel_buffer: self.pixel_buffer,
            leases: self.leases.clone(),
        }
    }
}
impl Drop for EncoderSurface {
    fn drop(&mut self) {
        unsafe {
            CVPixelBufferRelease(self.pixel_buffer);
        }
    }
}
impl EncoderSurface {
    pub fn new(width: u32, height: u32) -> Result<Self, MediaError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(MediaError::time(
                ErrorKind::UnsupportedContent,
                "VideoToolbox NV12 dimensions must be non-zero and even",
            ));
        }
        unsafe {
            let empty_keys: [*const c_void; 0] = [];
            let empty_values: [*const c_void; 0] = [];
            let empty_iosurface = CFDictionaryCreate(
                std::ptr::null(),
                empty_keys.as_ptr(),
                empty_values.as_ptr(),
                0,
                (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
                (&raw const kCFTypeDictionaryValueCallBacks).cast(),
            );
            if empty_iosurface.is_null() {
                return Err("Cannot create IOSurface attributes".into());
            }
            let keys = [
                kCVPixelBufferIOSurfacePropertiesKey,
                kCVPixelBufferMetalCompatibilityKey,
            ];
            let values = [empty_iosurface, kCFBooleanTrue];
            let attrs = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                2,
                (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
                (&raw const kCFTypeDictionaryValueCallBacks).cast(),
            );
            CFRelease(empty_iosurface);
            if attrs.is_null() {
                return Err("Cannot create VideoToolbox surface attributes".into());
            }
            // kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange (420v).
            let mut pixel_buffer = std::ptr::null_mut();
            let status = CVPixelBufferCreate(
                std::ptr::null(),
                width as usize,
                height as usize,
                0x34323076,
                attrs,
                &mut pixel_buffer,
            );
            CFRelease(attrs);
            if status != 0 || pixel_buffer.is_null() {
                return Err(format!(
                    "Cannot allocate IOSurface-backed NV12 encoder input ({status})"
                )
                .into());
            }
            Ok(Self {
                pixel_buffer,
                leases: [None, None],
            })
        }
    }
    pub fn width(&self) -> u32 {
        unsafe { CVPixelBufferGetWidth(self.pixel_buffer) as u32 }
    }
    pub fn height(&self) -> u32 {
        unsafe { CVPixelBufferGetHeight(self.pixel_buffer) as u32 }
    }
    pub fn pixel_buffer(&self) -> *mut c_void {
        self.pixel_buffer
    }
    pub fn allocation_size(&self) -> u64 {
        unsafe {
            let surface = CVPixelBufferGetIOSurface(self.pixel_buffer);
            if surface.is_null() {
                CVPixelBufferGetDataSize(self.pixel_buffer) as u64
            } else {
                IOSurfaceGetAllocSize(surface) as u64
            }
        }
    }
    pub fn with_lease(mut self, lease: Arc<dyn Any + Send + Sync>) -> Self {
        if let Some(slot) = self.leases.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(lease);
        } else {
            // A surface has one physical pool owner and one independent consumer lease.
            // Further owners are composed only on the exceptional third attachment.
            let prior = self.leases[0].take().unwrap();
            self.leases[0] = Some(Arc::new((prior, self.leases[1].take().unwrap())));
            self.leases[1] = Some(lease);
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub time_base: crate::RationalTimeBase,
    pub bitrate: u64,
    pub queue_capacity: usize,
    /// Zero disables reordering; 1 or 2 enables native two-frame reorder depth.
    /// VideoToolbox does not expose an exact maximum B-frame-run control.
    pub max_b_frames: u32,
    pub overwrite: bool,
}
#[derive(Clone)]
pub struct EncodeFrame {
    pub surface: EncoderSurface,
    pub pts: i64,
    pub duration: i64,
}
pub enum EncodeAdmission {
    Accepted,
    WouldBlock(EncodeFrame),
}
#[derive(Clone, Debug)]
pub struct EncoderReport {
    pub input_frames: u64,
    pub packets: u64,
    pub hardware: bool,
    pub encoder: String,
    pub time_base: crate::RationalTimeBase,
}
#[derive(Clone, Debug)]
pub enum EncoderStatus {
    Running,
    Draining,
    ReadyToCommit(EncoderReport),
    Completed(EncoderReport),
    Cancelled,
    Failed(MediaError),
}

#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
mod mp4;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use mp4::Mp4Encoder;
