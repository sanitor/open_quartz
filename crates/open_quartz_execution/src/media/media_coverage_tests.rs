use super::*;

struct Decoder {
    paused: bool,
    sought: i64,
    next: Result<Option<ExternalVideoFrame>, String>,
}

impl NativeVideoDecoder for Decoder {
    fn capabilities(&self) -> NativeDecoderCapabilities {
        NativeDecoderCapabilities {
            decoder: "test".to_owned(),
            surface_kinds: vec![],
            pixel_formats: vec![],
            zero_cpu_copy: false,
        }
    }
    fn next_frame(&mut self) -> Result<Option<ExternalVideoFrame>, String> {
        self.next.clone()
    }
    fn pause(&mut self) -> Result<(), String> {
        self.paused = true;
        Ok(())
    }
    fn resume(&mut self) -> Result<(), String> {
        self.paused = false;
        Ok(())
    }
    fn seek(&mut self, timestamp_ns: i64) -> Result<(), String> {
        self.sought = timestamp_ns;
        Ok(())
    }
}

struct Importer;
impl ExternalVideoFrameImporter for Importer {
    fn supported_surface_kinds(&self) -> &[NativeVideoSurfaceKind] {
        &[]
    }
    fn import(&mut self, _: ExternalVideoFrame) -> Result<ImportedVideoFrame, String> {
        Err("import".to_owned())
    }
}

#[test]
fn gpu_source_forwards_pause_resume_and_seek() {
    let mut source = NativeVideoGpuSource::new(
        Decoder {
            paused: false,
            sought: 0,
            next: Ok(None),
        },
        Importer,
    );
    source.pause().unwrap();
    source.resume().unwrap();
    source.seek(42).unwrap();
    assert!(source.next_frame().unwrap().is_none());

    let frame = ExternalVideoFrame {
        surface_kind: NativeVideoSurfaceKind::IoSurface,
        pixel_format: VideoPixelFormat::Nv12,
        width: 1,
        height: 1,
        planes: vec![],
        pts_ns: 0,
        duration_ns: 1,
        sync: ExternalFrameSync::None,
    };
    let mut import_error = NativeVideoGpuSource::new(
        Decoder {
            paused: false,
            sought: 0,
            next: Ok(Some(frame)),
        },
        Importer,
    );
    assert!(import_error.next_frame().is_err());
    let mut decode_error = NativeVideoGpuSource::new(
        Decoder {
            paused: false,
            sought: 0,
            next: Err("decode".to_owned()),
        },
        Importer,
    );
    assert!(decode_error.next_frame().is_err());
}
