#![cfg(all(target_os = "macos", feature = "gpu", feature = "native-video"))]

use open_quartz::rendering::{GpuBackend, TextureFormat};
use oq_media::{ColorMatrix, PixelFormat, Session};
use std::{path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};

struct Fixture(PathBuf);
impl Drop for Fixture { fn drop(&mut self) { let _=std::fs::remove_file(&self.0); } }

#[test]
fn eight_and_ten_bit_hardware_surfaces_preserve_color_and_range() {
    let ffmpeg=std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/open_quartz_slint/runtime/ffmpeg"));
    pollster::block_on(async {
        let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter=instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.unwrap();
        let(device,queue)=adapter.request_device(&wgpu::DeviceDescriptor{required_features:adapter.features()&wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,..Default::default()}).await.unwrap();
        let backend=GpuBackend::from_device(device,queue);
        // These cases distinguish bit-depth normalization, limited/full-range,
        // and the 601/709 chroma coefficients; neutral white/gray alone cannot.
        for (index,(color,codec,pixel,matrix,range,expected,format)) in [
            ("red","libx264","yuv420p","bt709","tv",[255u8,0,0],PixelFormat::Nv12),
            ("blue","libx264","yuvj420p","smpte170m","pc",[0,0,255],PixelFormat::Nv12),
            ("red","libx265","yuv420p","bt709","tv",[255,0,0],PixelFormat::Nv12),
            ("red","libx265","yuv420p10le","bt709","tv",[255,0,0],PixelFormat::P010),
        ].into_iter().enumerate() {
            let suffix=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let file=Fixture(std::env::temp_dir().join(format!("oq-vt-color-{suffix}-{index}.mp4")));
            let filter=format!("scale=in_color_matrix=bt601:out_color_matrix={}:out_range={}",if matrix=="bt709"{"bt709"}else{"bt601"},if range=="pc"{"full"}else{"limited"});
            let mut command=Command::new(&ffmpeg);
            command.env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FALLBACK_LIBRARY_PATH")
                .args(["-hide_banner","-loglevel","error","-f","lavfi","-i",&format!("color=c={color}:size=128x72:rate=10:duration=0.2"),"-vf",&filter,"-c:v",codec,"-pix_fmt",pixel,"-colorspace",matrix,"-color_range",range,"-color_trc","bt709","-y"]);
            if codec=="libx265" {command.args(["-x265-params","pools=1:frame-threads=1:log-level=error","-tag:v","hvc1"]);}
            assert!(command.arg(&file.0).status().unwrap().success());
            let mut session=Session::open(&file.0).unwrap();
            let lease=session.next_frame().unwrap().unwrap();
            assert_eq!(session.actual_pipeline().hardware,Some(true));
            assert_eq!(lease.frame.format(),format);
            assert_eq!(lease.frame.color().full_range,range=="pc");
            assert_eq!(lease.frame.color().matrix,if matrix=="bt709"{ColorMatrix::Bt709}else{ColorMatrix::Bt601});
            let output=backend.create_texture(lease.frame.width(),lease.frame.height(),TextureFormat::Rgba8Unorm);
            backend.upload_videotoolbox(&lease.frame,&output).unwrap();
            drop(lease);drop(session);
            let rgba=backend.read_texture_rgba(&output.texture,output.width,output.height).await.unwrap();
            for pixel in rgba.chunks_exact(4) {
                assert!((0..3).all(|c|pixel[c].abs_diff(expected[c])<=12),"{color}/{codec}/{range}: {pixel:?}");
                assert_eq!(pixel[3],255);
            }
        }
    });
}

#[test]
fn decoded_pal_primaries_override_container_tags_and_convert_to_srgb() {
    use oq_media::ColorPrimaries;
    let ffmpeg=std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/open_quartz_slint/runtime/ffmpeg"));
    let suffix=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let base=Fixture(std::env::temp_dir().join(format!("oq-pal-{suffix}.mp4")));
    let tagged=Fixture(std::env::temp_dir().join(format!("oq-pal-tags-{suffix}.mp4")));
    let bt709=Fixture(std::env::temp_dir().join(format!("oq-709-{suffix}.mp4")));
    let mut child=Command::new(&ffmpeg).env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .args(["-hide_banner","-loglevel","error","-f","rawvideo","-pixel_format","yuv420p","-video_size","64x64","-framerate","1","-i","-","-frames:v","1","-c:v","libx264","-qp","0","-color_primaries","bt470bg","-color_trc","smpte170m","-colorspace","smpte170m","-color_range","tv","-y"])
        .arg(&base.0).stdin(std::process::Stdio::piped()).spawn().unwrap();
    let yuv=[vec![137u8;64*64],vec![84u8;32*32],vec![72u8;32*32]].concat();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(),&yuv).unwrap();
    assert!(child.wait().unwrap().success());
    assert!(Command::new(&ffmpeg).env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .args(["-hide_banner","-loglevel","error","-i"]).arg(&base.0)
        .args(["-c:v","copy","-color_primaries","bt709","-color_trc","bt709","-colorspace","bt709","-movflags","+write_colr","-y"])
        .arg(&tagged.0).status().unwrap().success());
    assert!(Command::new(&ffmpeg).env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .args(["-hide_banner","-loglevel","error","-i"]).arg(&base.0)
        .args(["-c:v","copy","-bsf:v","h264_metadata=colour_primaries=1","-y"])
        .arg(&bt709.0).status().unwrap().success());
    pollster::block_on(async {
        let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter=instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.unwrap();
        let(device,queue)=adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
        let backend=GpuBackend::from_device(device,queue);
        // Reuse one backend and alternate primaries: pipeline caching must not
        // reuse the PAL gamut transform for a later BT.709 source.
        for (path,primaries,expected) in [(&base.0,ColorPrimaries::Bt470Bg,[49u8,209,72]),(&bt709.0,ColorPrimaries::Bt709,[67,209,68]),(&tagged.0,ColorPrimaries::Bt470Bg,[49,209,72])] {
            let mut session=Session::open(path).unwrap();
            let frame=session.next_frame().unwrap().unwrap().frame;
            assert_eq!(frame.color().primaries,primaries);
            assert_eq!(frame.color().matrix,ColorMatrix::Bt601);
            let output=backend.create_texture(64,64,TextureFormat::Rgba8Unorm);
            backend.upload_videotoolbox(&frame,&output).unwrap();
            let rgba=backend.read_texture_rgba(&output.texture,64,64).await.unwrap();
            // Independent D65 PAL->709 RGB/XYZ derivation for YUV(137,84,72),
            // limited range + BT.601 + SMPTE170M inverse OETF + sRGB OETF.
            // Skipping the gamut matrix yields red=67 instead of 49.
            for pixel in rgba.chunks_exact(4) {
                assert!((0..3).all(|c|pixel[c].abs_diff(expected[c])<=3),"{primaries:?} pixel: {pixel:?}");
                assert_eq!(pixel[3],255);
            }
        }
    });
}
