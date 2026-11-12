use super::super::{RenderDevice, StableFrameBudget, StableFramePool};
use super::*;
use crate::outputs::FrameSnapshotIdentity;
use std::{sync::atomic::AtomicUsize, time::Duration};

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(buffer: *mut c_void, plane: usize) -> *const u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(buffer: *mut c_void, plane: usize) -> usize;
}

fn identity(alpha_mode: FrameAlphaMode) -> FrameSnapshotIdentity {
    FrameSnapshotIdentity {
        output_node_id: "source".into(),
        graph_revision: 1,
        evaluation_stamp: None,
        content_stamp: None,
        color_space: FrameColorSpace::Srgb,
        alpha_mode,
    }
}
fn drain(backend: &GpuBackend) {
    backend
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(10)),
        })
        .unwrap();
}
fn accepted(value: ConversionAdmission) -> ConvertedFrame {
    match value {
        ConversionAdmission::Accepted(frame) => frame,
        ConversionAdmission::WouldBlock(_) => panic!("unexpected full NV12 pool"),
    }
}

// Only tests map CV planes. Production has no CPU mapping or readback path.
fn read_planes(surface: &EncoderSurface) -> [Vec<u8>; 2] {
    let buffer = surface.pixel_buffer();
    assert_eq!(unsafe { CVPixelBufferLockBaseAddress(buffer, 1) }, 0);
    let planes = std::array::from_fn(|plane| {
        let height = if plane == 0 {
            surface.height()
        } else {
            surface.height() / 2
        } as usize;
        let width = surface.width() as usize; // UV has half as many two-byte texels.
        let stride = unsafe { CVPixelBufferGetBytesPerRowOfPlane(buffer, plane) };
        assert!(stride >= width);
        let base = unsafe { CVPixelBufferGetBaseAddressOfPlane(buffer, plane) };
        assert!(!base.is_null());
        let mut bytes = Vec::with_capacity(width * height);
        for row in 0..height {
            bytes.extend_from_slice(unsafe {
                std::slice::from_raw_parts(base.add(row * stride), width)
            });
        }
        bytes
    });
    assert_eq!(unsafe { CVPixelBufferUnlockBaseAddress(buffer, 1) }, 0);
    planes
}

// Independent f64 numeric oracle. Alpha composition is linear-light, and each
// sample receives the 709 OETF before chroma is averaged over the 2x2 footprint.
fn reference(
    pixels: &[u8],
    width: usize,
    height: usize,
    mode: InputInterpretation,
) -> [Vec<u8>; 2] {
    fn rgb(pixel: &[u8], mode: InputInterpretation) -> [f64; 3] {
        let alpha = if mode == InputInterpretation::SdrSrgbRgba8OpaqueBlack {
            1.0
        } else {
            f64::from(pixel[3]) / 255.0
        };
        std::array::from_fn(|channel| {
            let mut v = f64::from(pixel[channel]) / 255.0;
            if mode == InputInterpretation::SdrSrgbRgba8PremultipliedBlack {
                v = if alpha > 0.0 {
                    (v / alpha).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
            let linear = if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            } * alpha;
            if linear < 0.018 {
                4.5 * linear
            } else {
                1.099 * linear.powf(0.45) - 0.099
            }
        })
    }
    fn components(rgb: [f64; 3]) -> [f64; 3] {
        let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        [y, (rgb[2] - y) / 1.8556, (rgb[0] - y) / 1.5748]
    }
    let colors: Vec<_> = pixels.chunks_exact(4).map(|p| rgb(p, mode)).collect();
    let y = colors
        .iter()
        .map(|c| {
            (16.0 + 219.0 * components(*c)[0])
                .round()
                .clamp(16.0, 235.0) as u8
        })
        .collect();
    let mut uv = Vec::new();
    for row in (0..height).step_by(2) {
        for col in (0..width).step_by(2) {
            let mean = std::array::from_fn(|channel| {
                [
                    row * width + col,
                    row * width + col + 1,
                    (row + 1) * width + col,
                    (row + 1) * width + col + 1,
                ]
                .iter()
                .map(|index| colors[*index][channel])
                .sum::<f64>()
                    * 0.25
            });
            let c = components(mean);
            uv.extend(
                c[1..]
                    .iter()
                    .map(|v| (128.0 + 224.0 * v).round().clamp(16.0, 240.0) as u8),
            );
        }
    }
    [y, uv]
}
fn assert_oracle(actual: &[Vec<u8>; 2], expected: &[Vec<u8>; 2]) {
    for plane in 0..2 {
        assert_eq!(actual[plane].len(), expected[plane].len());
        for (index, (&a, &e)) in actual[plane].iter().zip(&expected[plane]).enumerate() {
            assert!(
                a.abs_diff(e) <= 1,
                "plane {plane}, byte {index}: GPU {a}, oracle {e}"
            );
        }
    }
}

#[test]
fn native_nv12_primary_codes_stride_transfer_and_alpha_match_independent_oracle() {
    pollster::block_on(async {
        let mut device = RenderDevice::new()
            .await
            .expect("hardware Metal required")
            .0;
        let backend = device.backend();
        let stable = StableFramePool::new(
            backend.clone(),
            StableFrameBudget {
                max_frames: 2,
                max_bytes: 1024 * 1024,
            },
        )
        .unwrap();
        let mut converter = Nv12Converter::new(backend.clone(), 1, 1024 * 1024).unwrap();
        assert!(converter.prepare_surfaces(66, 4, 1).unwrap());
        let cases = [
            ([0, 0, 0, 255], [16, 128, 128]),
            ([255, 255, 255, 255], [235, 128, 128]),
            ([255, 0, 0, 255], [63, 102, 240]),
            ([0, 255, 0, 255], [173, 42, 26]),
            ([0, 0, 255, 255], [32, 240, 118]),
            ([128, 128, 128, 255], [115, 128, 128]),
        ];
        converter.set_input_interpretation(InputInterpretation::SdrSrgbRgba8Black);
        for (color, known) in cases {
            let pixels = color.repeat(66 * 4);
            device.upload_rgba("source", &pixels, 66, 4).unwrap();
            let input = stable
                .try_capture(&device, "source", identity(FrameAlphaMode::Straight))
                .unwrap();
            let converted = accepted(converter.try_convert(input).unwrap());
            drain(&backend);
            assert!(converted.ready.is_ready());
            assert_eq!(converted.ready.completed_at(), None);
            let actual = read_planes(&converted.surface);
            assert_oracle(
                &actual,
                &reference(&pixels, 66, 4, InputInterpretation::SdrSrgbRgba8Black),
            );
            assert_eq!([actual[0][0], actual[1][0], actual[1][1]], known);
            drop(converted);
        }
        for mode in [
            InputInterpretation::SdrSrgbRgba8Black,
            InputInterpretation::SdrSrgbRgba8PremultipliedBlack,
            InputInterpretation::SdrSrgbRgba8OpaqueBlack,
        ] {
            converter.set_input_interpretation(mode);
            let pixels: Vec<u8> = (0..66 * 4)
                .flat_map(|i| match i % 4 {
                    0 => [255, 30, 12, 128],
                    1 => [22, 210, 88, 0],
                    2 => [45, 66, 220, 201],
                    _ => [7, 11, 15, 255],
                })
                .collect();
            device.upload_rgba("source", &pixels, 66, 4).unwrap();
            let input = stable
                .try_capture(&device, "source", identity(mode.alpha()))
                .unwrap();
            let converted = accepted(converter.try_convert(input).unwrap());
            drain(&backend);
            assert_oracle(
                &read_planes(&converted.surface),
                &reference(&pixels, 66, 4, mode),
            );
            drop(converted);
        }
    });
}

#[test]
fn small_pool_returns_same_input_and_waits_for_last_encoder_surface_consumer() {
    pollster::block_on(async {
        let mut device = RenderDevice::new()
            .await
            .expect("hardware Metal required")
            .0;
        let backend = device.backend();
        let stable = StableFramePool::new(
            backend.clone(),
            StableFrameBudget {
                max_frames: 2,
                max_bytes: 1024,
            },
        )
        .unwrap();
        let mut converter = Nv12Converter::new(backend.clone(), 1, 1024 * 1024).unwrap();
        converter.set_input_interpretation(InputInterpretation::SdrSrgbRgba8OpaqueBlack);
        converter.set_trace_enabled(true);
        let wakes = Arc::new(AtomicUsize::new(0));
        let notified = wakes.clone();
        converter.set_waker(Arc::new(move || {
            notified.fetch_add(1, Ordering::Relaxed);
        }));
        assert!(converter.prepare_surfaces(2, 2, 1).unwrap());
        device
            .upload_rgba("source", &[255, 0, 0, 255].repeat(4), 2, 2)
            .unwrap();
        let red = stable
            .try_capture(&device, "source", identity(FrameAlphaMode::Opaque))
            .unwrap();
        let first = accepted(converter.try_convert(red).unwrap());
        let encoder_consumer = first.surface.clone();
        device
            .upload_rgba("source", &[0, 0, 255, 255].repeat(4), 2, 2)
            .unwrap();
        let blue = stable
            .try_capture(&device, "source", identity(FrameAlphaMode::Opaque))
            .unwrap();
        let original_texture = blue.output().texture.clone();
        let blue = match converter.try_convert(blue).unwrap() {
            ConversionAdmission::WouldBlock(input) => input,
            _ => panic!("busy surface reused"),
        };
        assert!(Arc::ptr_eq(&original_texture, &blue.output().texture));
        drain(&backend);
        assert!(first.ready.is_ready());
        assert!(first.ready.completed_at().is_some());
        assert_eq!(read_planes(&first.surface)[0], vec![63; 4]);
        drop(first);
        let blue = match converter.try_convert(blue).unwrap() {
            ConversionAdmission::WouldBlock(input) => input,
            _ => panic!("encoder consumer did not hold slot"),
        };
        drop(encoder_consumer);
        assert!(wakes.load(Ordering::Relaxed) >= 2);
        let second = accepted(converter.try_convert(blue).unwrap());
        // Converter/session drop is not permission to recycle in-flight input,
        // CVMetal wrappers or output allocation. The submitted callback owns them.
        drop(converter);
        drop(stable);
        drain(&backend);
        assert!(second.ready.is_ready());
        assert_eq!(read_planes(&second.surface)[0], vec![32; 4]);
    });
}

#[test]
fn physical_credit_deduplicates_surface_identity_and_survives_pool_drop() {
    pollster::block_on(async {
        let device = RenderDevice::new()
            .await
            .expect("hardware Metal required")
            .0;
        let backend = device.backend();
        let budget = backend.frame_budget();
        let mut converter = Nv12Converter::new(backend.clone(), 1, 1024 * 1024).unwrap();
        assert!(converter.prepare_surfaces(64, 64, 1).unwrap());
        let surface = converter.slots[0].surface.clone();
        let bytes = surface.allocation_size();
        assert_eq!(budget.usage(), (1, bytes));
        let duplicate = budget
            .try_acquire((0, surface.pixel_buffer() as usize), 1, bytes)
            .unwrap()
            .unwrap();
        assert_eq!(budget.usage(), (1, bytes));
        drop(converter);
        assert_eq!(budget.usage(), (1, bytes));
        drop(duplicate);
        assert_eq!(budget.usage(), (1, bytes));
        drop(surface);
        assert_eq!(budget.usage(), (0, 0));
    });
}

#[test]
fn invalid_dimensions_capacity_and_color_contract_fail_before_submission() {
    pollster::block_on(async {
        let mut device = RenderDevice::new()
            .await
            .expect("hardware Metal required")
            .0;
        let backend = device.backend();
        assert!(Nv12Converter::new(backend.clone(), 0, 1024).is_err());
        let mut converter = Nv12Converter::new(backend.clone(), 1, 1024 * 1024).unwrap();
        for (width, height, count) in [(0, 2, 1), (3, 2, 1), (2, 3, 1), (2, 2, 0), (2, 2, 2)] {
            assert!(converter.prepare_surfaces(width, height, count).is_err());
        }
        assert!(converter.prepare_surfaces(2, 2, 1).unwrap());
        let stable = StableFramePool::new(
            backend.clone(),
            StableFrameBudget {
                max_frames: 3,
                max_bytes: 1024,
            },
        )
        .unwrap();
        device
            .upload_rgba("source", &[128, 128, 128, 255].repeat(4), 2, 2)
            .unwrap();
        let missing = stable
            .try_capture(&device, "source", identity(FrameAlphaMode::Opaque))
            .unwrap();
        assert!(converter
            .try_convert(missing)
            .err()
            .unwrap()
            .contains("explicitly declared"));
        converter.set_input_interpretation(InputInterpretation::SdrSrgbRgba8Black);
        let mismatched = stable
            .try_capture(&device, "source", identity(FrameAlphaMode::Opaque))
            .unwrap();
        assert!(converter
            .try_convert(mismatched)
            .err()
            .unwrap()
            .contains("alpha metadata"));
        assert!(!converter.slots[0].busy.load(Ordering::Acquire));
        let mut bad_color = identity(FrameAlphaMode::Straight);
        bad_color.color_space = FrameColorSpace::LinearBt709;
        let linear = stable.try_capture(&device, "source", bad_color).unwrap();
        assert!(converter
            .try_convert(linear)
            .err()
            .unwrap()
            .contains("color metadata"));
        let mut tiny = Nv12Converter::new(backend, 1, 1).unwrap();
        assert!(tiny
            .prepare_surfaces(2, 2, 1)
            .err()
            .unwrap()
            .contains("byte budget"));
    });
}

#[test]
fn preparation_observes_shared_physical_pressure_before_decode() {
    pollster::block_on(async {
        let device = RenderDevice::new()
            .await
            .expect("hardware Metal required")
            .0;
        let backend = device.backend();
        let budget = backend.frame_budget();
        let (surfaces, bytes) = budget.limits();
        let held = budget
            .try_acquire((2, usize::MAX), surfaces, bytes)
            .unwrap()
            .unwrap();
        let mut converter = Nv12Converter::new(backend, 1, 1024 * 1024).unwrap();
        assert!(!converter.prepare_surfaces(64, 64, 1).unwrap());
        assert!(converter.slots.is_empty());
        drop(held);
        assert!(converter.prepare_surfaces(64, 64, 1).unwrap());
        assert_eq!(budget.usage().0, 1);
    });
}
