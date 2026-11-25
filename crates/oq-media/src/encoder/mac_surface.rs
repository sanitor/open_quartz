use super::EncoderProfile;
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
    fn CVBufferSetAttachment(
        buffer: *mut c_void,
        key: *const c_void,
        value: *const c_void,
        mode: u32,
    );
    static kCVImageBufferTransferFunctionKey: *const c_void;
    static kCVImageBufferColorPrimariesKey: *const c_void;
    static kCVImageBufferYCbCrMatrixKey: *const c_void;
    static kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ: *const c_void;
    static kCVImageBufferTransferFunction_ITU_R_2100_HLG: *const c_void;
    static kCVImageBufferColorPrimaries_ITU_R_2020: *const c_void;
    static kCVImageBufferYCbCrMatrix_ITU_R_2020: *const c_void;
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

/// Retained IOSurface-backed NV12 or P010 VideoToolbox encoder input.
/// Clones preserve all opaque accounting leases without mapping either plane.
pub struct EncoderSurface {
    pixel_buffer: *mut c_void,
    profile: EncoderProfile,
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
            profile: self.profile,
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
        Self::with_profile(width, height, EncoderProfile::Sdr)
    }
    pub fn with_profile(
        width: u32,
        height: u32,
        profile: EncoderProfile,
    ) -> Result<Self, MediaError> {
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
                profile.pixel_format(),
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
            let transfer = match profile {
                EncoderProfile::Sdr => None,
                EncoderProfile::Hdr10 => Some(kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ),
                EncoderProfile::Hlg => Some(kCVImageBufferTransferFunction_ITU_R_2100_HLG),
            };
            if let Some(transfer) = transfer {
                // kCVAttachmentMode_ShouldPropagate preserves these tags on the encoded sample.
                CVBufferSetAttachment(pixel_buffer, kCVImageBufferTransferFunctionKey, transfer, 1);
                CVBufferSetAttachment(
                    pixel_buffer,
                    kCVImageBufferColorPrimariesKey,
                    kCVImageBufferColorPrimaries_ITU_R_2020,
                    1,
                );
                CVBufferSetAttachment(
                    pixel_buffer,
                    kCVImageBufferYCbCrMatrixKey,
                    kCVImageBufferYCbCrMatrix_ITU_R_2020,
                    1,
                );
            }
            Ok(Self {
                pixel_buffer,
                profile,
                leases: [None, None],
            })
        }
    }
    pub fn profile(&self) -> EncoderProfile {
        self.profile
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
