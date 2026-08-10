use serde::Serialize;
use std::sync::Mutex;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureStreamCapability {
    pub available: bool,
    pub adapter_luid: Option<u64>,
    pub stream_ready: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureStreamTestFrame {
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub presented: bool,
}

#[derive(Default)]
pub struct TextureStreamCapabilityState {
    value: Mutex<TextureStreamCapability>,
}

impl TextureStreamCapabilityState {
    pub fn set(&self, value: TextureStreamCapability) {
        if let Ok(mut current) = self.value.lock() {
            *current = value;
        }
    }

    pub fn get(&self) -> TextureStreamCapability {
        self.value
            .lock()
            .map(|value| value.clone())
            .unwrap_or_else(|_| TextureStreamCapability {
                available: false,
                adapter_luid: None,
                stream_ready: false,
                reason: Some("TextureStream capability lock is poisoned".to_owned()),
            })
    }
}

#[cfg(windows)]
mod platform {
    use super::{TextureStreamCapability, TextureStreamTestFrame};
    use open_quartz::gpu::SharedTextureFrame;
    use std::{
        cell::RefCell,
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, AtomicU32, Ordering},
            Arc,
        },
        time::Instant,
    };
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment;
    use windows::Win32::Foundation::{HANDLE, HMODULE, LUID};
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
        ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
        ID3D11VideoProcessorEnumerator, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV,
        D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
        D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
        D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
        D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
    };
    use windows::Win32::Graphics::Direct3D11on12::{
        D3D11On12CreateDevice, ID3D11On12Device, D3D11_RESOURCE_FLAGS,
    };
    use windows::Win32::Graphics::Direct3D12::{
        D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12Resource,
        D3D12_COMMAND_QUEUE_DESC, D3D12_RESOURCE_STATE_COMMON,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, IDXGIAdapter, IDXGIFactory4, IDXGIKeyedMutex, IDXGIResource,
    };
    use windows_core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT, PCWSTR};

    const STREAM_ID: &str = "open-quartz-renderer";
    const ALLOWED_ORIGINS: &[&str] = &[
        "http://tauri.localhost",
        "https://tauri.localhost",
        "http://localhost:5173",
    ];

    windows_core::imp::define_interface!(
        ICoreWebView2ExperimentalEnvironment12,
        ICoreWebView2ExperimentalEnvironment12_Vtbl,
        0x96c27a45_f142_4873_80ad_9d0cd899b2b9
    );
    windows_core::imp::interface_hierarchy!(ICoreWebView2ExperimentalEnvironment12, IUnknown);

    #[repr(C)]
    pub struct ICoreWebView2ExperimentalEnvironment12_Vtbl {
        pub base__: IUnknown_Vtbl,
        pub create_texture_stream: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            PCWSTR,
            *mut core::ffi::c_void,
            *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        pub get_render_adapter_luid:
            unsafe extern "system" fn(*mut core::ffi::c_void, *mut u64) -> windows_core::HRESULT,
        pub add_render_adapter_luid_changed: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
            *mut i64,
        )
            -> windows_core::HRESULT,
        pub remove_render_adapter_luid_changed:
            unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
    }

    windows_core::imp::define_interface!(
        ICoreWebView2ExperimentalTextureStream,
        ICoreWebView2ExperimentalTextureStream_Vtbl,
        0xafca8431_633f_4528_abfe_7fc3bedd8962
    );
    windows_core::imp::interface_hierarchy!(ICoreWebView2ExperimentalTextureStream, IUnknown);

    #[repr(C)]
    pub struct ICoreWebView2ExperimentalTextureStream_Vtbl {
        pub base__: IUnknown_Vtbl,
        pub get_id: usize,
        pub add_allowed_origin:
            unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR, i32) -> windows_core::HRESULT,
        pub remove_allowed_origin: usize,
        pub add_start_requested: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
            *mut i64,
        ) -> windows_core::HRESULT,
        pub remove_start_requested:
            unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
        pub add_stopped: usize,
        pub remove_stopped: usize,
        pub create_texture: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            u32,
            u32,
            *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        pub get_available_texture: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        pub close_texture: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        pub present_texture: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        pub stop: usize,
        pub add_error_received: usize,
        pub remove_error_received: usize,
        pub set_d3d_device: usize,
        pub add_web_texture_received: usize,
        pub remove_web_texture_received: usize,
        pub add_web_texture_stream_stopped: usize,
        pub remove_web_texture_stream_stopped: usize,
    }

    windows_core::imp::define_interface!(
        ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler,
        ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler_Vtbl,
        0x62d09330_00a9_41bf_a9ae_55aaef8b3c44
    );
    windows_core::imp::interface_hierarchy!(
        ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler,
        IUnknown
    );

    #[repr(C)]
    pub struct ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler_Vtbl {
        pub base__: IUnknown_Vtbl,
        pub invoke: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
            *mut core::ffi::c_void,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct StartRequestedHandlerObject {
        vtable: *const ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler_Vtbl,
        references: AtomicU32,
        start_requested: Arc<AtomicBool>,
    }

    unsafe extern "system" fn start_handler_query_interface(
        this: *mut core::ffi::c_void,
        iid: *const GUID,
        interface: *mut *mut core::ffi::c_void,
    ) -> HRESULT {
        if iid.is_null() || interface.is_null() {
            return HRESULT(0x80004003u32 as i32);
        }
        *interface = core::ptr::null_mut();
        if *iid == IUnknown::IID
            || *iid == ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler::IID
        {
            *interface = this;
            start_handler_add_ref(this);
            return HRESULT(0);
        }
        HRESULT(0x80004002u32 as i32)
    }

    unsafe extern "system" fn start_handler_add_ref(this: *mut core::ffi::c_void) -> u32 {
        let object = &*(this as *mut StartRequestedHandlerObject);
        object.references.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn start_handler_release(this: *mut core::ffi::c_void) -> u32 {
        let object = &*(this as *mut StartRequestedHandlerObject);
        let remaining = object.references.fetch_sub(1, Ordering::Release) - 1;
        if remaining == 0 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this as *mut StartRequestedHandlerObject));
        }
        remaining
    }

    unsafe extern "system" fn start_handler_invoke(
        this: *mut core::ffi::c_void,
        _sender: *mut core::ffi::c_void,
        _args: *mut core::ffi::c_void,
    ) -> HRESULT {
        let object = &*(this as *mut StartRequestedHandlerObject);
        object.start_requested.store(true, Ordering::Release);
        println!("[oq:native] webview-texture-stream start-requested");
        HRESULT(0)
    }

    static START_REQUESTED_HANDLER_VTABLE:
        ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler_Vtbl =
        ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler_Vtbl {
            base__: IUnknown_Vtbl {
                QueryInterface: start_handler_query_interface,
                AddRef: start_handler_add_ref,
                Release: start_handler_release,
            },
            invoke: start_handler_invoke,
        };

    fn create_start_requested_handler(
        start_requested: Arc<AtomicBool>,
    ) -> ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler {
        let object = Box::new(StartRequestedHandlerObject {
            vtable: &START_REQUESTED_HANDLER_VTABLE,
            references: AtomicU32::new(1),
            start_requested,
        });
        unsafe { Interface::from_raw(Box::into_raw(object).cast()) }
    }

    windows_core::imp::define_interface!(
        ICoreWebView2ExperimentalTexture,
        ICoreWebView2ExperimentalTexture_Vtbl,
        0x0836f09c_34bd_47bf_914a_99fb56ae2d07
    );
    windows_core::imp::interface_hierarchy!(ICoreWebView2ExperimentalTexture, IUnknown);

    #[repr(C)]
    pub struct ICoreWebView2ExperimentalTexture_Vtbl {
        pub base__: IUnknown_Vtbl,
        pub get_handle:
            unsafe extern "system" fn(*mut core::ffi::c_void, *mut HANDLE) -> windows_core::HRESULT,
        pub get_resource: unsafe extern "system" fn(
            *mut core::ffi::c_void,
            *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT,
        pub get_timestamp:
            unsafe extern "system" fn(*mut core::ffi::c_void, *mut u64) -> windows_core::HRESULT,
        pub set_timestamp:
            unsafe extern "system" fn(*mut core::ffi::c_void, u64) -> windows_core::HRESULT,
    }

    struct VideoProcessorPipeline {
        width: u32,
        height: u32,
        enumerator: ID3D11VideoProcessorEnumerator,
        processor: ID3D11VideoProcessor,
    }

    #[derive(Clone)]
    struct SharedBridgeSlot {
        width: u32,
        height: u32,
        sync_handle: Option<u64>,
        wrapped: ID3D11Texture2D,
        bridge_texture: ID3D11Texture2D,
        source: ID3D11Texture2D,
        fence: Option<ID3D12Fence>,
    }

    pub struct TextureStream {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        bridge_device: ID3D11Device,
        bridge_context: ID3D11DeviceContext,
        d3d12_device: ID3D12Device,
        d3d12_queue: ID3D12CommandQueue,
        interface: ICoreWebView2ExperimentalTextureStream,
        start_requested_handler: ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler,
        start_requested: Arc<AtomicBool>,
        has_started: bool,
        texture_size: Option<(u32, u32)>,
        start_requested_token: i64,
        video_pipeline: Option<VideoProcessorPipeline>,
        shared_bridges: HashMap<u64, SharedBridgeSlot>,
    }

    fn texture_size_changed(current: Option<(u32, u32)>, width: u32, height: u32) -> bool {
        current != Some((width, height))
    }

    pub(crate) fn presentation_timestamp_ns(started_at: Instant) -> u64 {
        started_at.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    impl ICoreWebView2ExperimentalEnvironment12 {
        fn render_adapter_luid(&self) -> windows_core::Result<u64> {
            unsafe {
                let mut luid = 0;
                (Interface::vtable(self).get_render_adapter_luid)(
                    Interface::as_raw(self),
                    &mut luid,
                )
                .ok()?;
                Ok(luid)
            }
        }

        fn create_texture_stream(
            &self,
            device: &ID3D11Device,
        ) -> windows_core::Result<ICoreWebView2ExperimentalTextureStream> {
            let stream_id = STREAM_ID.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            unsafe {
                let mut raw = core::ptr::null_mut();
                (Interface::vtable(self).create_texture_stream)(
                    Interface::as_raw(self),
                    PCWSTR(stream_id.as_ptr()),
                    Interface::as_raw(device),
                    &mut raw,
                )
                .ok()?;
                Ok(Interface::from_raw(raw))
            }
        }
    }

    impl ICoreWebView2ExperimentalTextureStream {
        fn add_allowed_origin(&self, origin: &str) -> windows_core::Result<()> {
            let origin = origin.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            unsafe {
                (Interface::vtable(self).add_allowed_origin)(
                    Interface::as_raw(self),
                    PCWSTR(origin.as_ptr()),
                    0,
                )
                .ok()
            }
        }

        fn add_start_requested(
            &self,
            handler: &ICoreWebView2ExperimentalTextureStreamStartRequestedEventHandler,
        ) -> windows_core::Result<i64> {
            unsafe {
                let mut token = 0;
                (Interface::vtable(self).add_start_requested)(
                    Interface::as_raw(self),
                    Interface::as_raw(handler),
                    &mut token,
                )
                .ok()?;
                Ok(token)
            }
        }

        fn remove_start_requested(&self, token: i64) -> windows_core::Result<()> {
            unsafe {
                (Interface::vtable(self).remove_start_requested)(Interface::as_raw(self), token)
                    .ok()
            }
        }

        fn available_texture(&self) -> windows_core::Result<ICoreWebView2ExperimentalTexture> {
            unsafe {
                let mut raw = core::ptr::null_mut();
                (Interface::vtable(self).get_available_texture)(Interface::as_raw(self), &mut raw)
                    .ok()?;
                Ok(Interface::from_raw(raw))
            }
        }

        fn create_texture(
            &self,
            width: u32,
            height: u32,
        ) -> windows_core::Result<ICoreWebView2ExperimentalTexture> {
            unsafe {
                let mut raw = core::ptr::null_mut();
                (Interface::vtable(self).create_texture)(
                    Interface::as_raw(self),
                    width,
                    height,
                    &mut raw,
                )
                .ok()?;
                Ok(Interface::from_raw(raw))
            }
        }

        fn present_texture(
            &self,
            texture: &ICoreWebView2ExperimentalTexture,
        ) -> windows_core::Result<()> {
            unsafe {
                (Interface::vtable(self).present_texture)(
                    Interface::as_raw(self),
                    Interface::as_raw(texture),
                )
                .ok()
            }
        }
    }

    impl ICoreWebView2ExperimentalTexture {
        fn resource(&self) -> windows_core::Result<IUnknown> {
            unsafe {
                let mut raw = core::ptr::null_mut();
                (Interface::vtable(self).get_resource)(Interface::as_raw(self), &mut raw).ok()?;
                Ok(Interface::from_raw(raw))
            }
        }

        fn set_timestamp(&self, timestamp: u64) -> windows_core::Result<()> {
            unsafe {
                (Interface::vtable(self).set_timestamp)(Interface::as_raw(self), timestamp).ok()
            }
        }
    }

    fn com_step<T>(result: windows_core::Result<T>, step: &str) -> windows_core::Result<T> {
        result.map_err(|error| windows_core::Error::new(error.code(), format!("{step}: {error}")))
    }

    struct KeyedMutexGuard {
        mutex: IDXGIKeyedMutex,
        release_key: u64,
    }

    impl KeyedMutexGuard {
        fn acquire(
            resource: &ID3D11Texture2D,
            acquire_key: u64,
            release_key: u64,
        ) -> windows_core::Result<Self> {
            let mutex = resource.cast::<IDXGIKeyedMutex>()?;
            unsafe {
                mutex.AcquireSync(acquire_key, 1_000)?;
            }
            Ok(Self { mutex, release_key })
        }
    }

    impl Drop for KeyedMutexGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = self.mutex.ReleaseSync(self.release_key);
            }
        }
    }

    struct WrappedResourceGuard {
        device: ID3D11On12Device,
        resource: ID3D11Resource,
        context: ID3D11DeviceContext,
    }

    impl WrappedResourceGuard {
        fn acquire(
            device: ID3D11On12Device,
            resource: ID3D11Resource,
            context: ID3D11DeviceContext,
        ) -> Self {
            unsafe {
                device.AcquireWrappedResources(&[Some(resource.clone())]);
            }
            Self {
                device,
                resource,
                context,
            }
        }
    }

    impl Drop for WrappedResourceGuard {
        fn drop(&mut self) {
            unsafe {
                self.device
                    .ReleaseWrappedResources(&[Some(self.resource.clone())]);
                self.context.Flush();
            }
        }
    }

    impl Drop for TextureStream {
        fn drop(&mut self) {
            let _ = self
                .interface
                .remove_start_requested(self.start_requested_token);
            let _ = &self.start_requested_handler;
        }
    }

    impl TextureStream {
        fn video_pipeline(
            &mut self,
            width: u32,
            height: u32,
        ) -> windows_core::Result<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor)> {
            let recreate = self
                .video_pipeline
                .as_ref()
                .is_none_or(|pipeline| pipeline.width != width || pipeline.height != height);
            if recreate {
                let video_device = com_step(
                    self.device.cast::<ID3D11VideoDevice>(),
                    "QI ID3D11VideoDevice",
                )?;
                let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputFrameRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    InputWidth: width,
                    InputHeight: height,
                    OutputFrameRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    OutputWidth: width,
                    OutputHeight: height,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                };
                let enumerator = com_step(
                    unsafe { video_device.CreateVideoProcessorEnumerator(&content) },
                    "CreateVideoProcessorEnumerator",
                )?;
                let processor = com_step(
                    unsafe { video_device.CreateVideoProcessor(&enumerator, 0) },
                    "CreateVideoProcessor",
                )?;
                self.video_pipeline = Some(VideoProcessorPipeline {
                    width,
                    height,
                    enumerator,
                    processor,
                });
            }
            let pipeline = self.video_pipeline.as_ref().expect("video pipeline exists");
            Ok((pipeline.enumerator.clone(), pipeline.processor.clone()))
        }

        fn present_rgba_texture(
            &mut self,
            source: &ID3D11Texture2D,
            width: u32,
            height: u32,
            timestamp: u64,
        ) -> windows_core::Result<TextureStreamTestFrame> {
            let start_requested = self.start_requested.swap(false, Ordering::AcqRel);
            if !self.has_started && !start_requested {
                return Ok(TextureStreamTestFrame {
                    width,
                    height,
                    format: "pending".to_owned(),
                    presented: false,
                });
            }
            let size_changed = texture_size_changed(self.texture_size, width, height);
            let texture = if start_requested || size_changed {
                self.has_started = true;
                let texture = com_step(
                    self.interface.create_texture(width, height),
                    "CreateTexture",
                )?;
                self.texture_size = Some((width, height));
                texture
            } else {
                match self.interface.available_texture() {
                    Ok(texture) => texture,
                    Err(error) if error.code() == HRESULT(0x80070103u32 as i32) => {
                        let texture = com_step(
                            self.interface.create_texture(width, height),
                            "CreateTexture",
                        )?;
                        self.texture_size = Some((width, height));
                        texture
                    }
                    Err(error) => {
                        return Err(windows_core::Error::new(
                            error.code(),
                            format!("GetAvailableTexture: {error}"),
                        ));
                    }
                }
            };
            let resource = com_step(texture.resource(), "Texture.Resource")?;
            let resource = com_step(resource.cast::<ID3D11Texture2D>(), "QI ID3D11Texture2D")?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                resource.GetDesc(&mut desc);
            }
            let keyed_mutex = com_step(KeyedMutexGuard::acquire(&resource, 0, 0), "AcquireSync")?;
            unsafe {
                if desc.Format == DXGI_FORMAT_NV12 {
                    let (enumerator, processor) = self.video_pipeline(width, height)?;
                    let video_device = com_step(
                        self.device.cast::<ID3D11VideoDevice>(),
                        "QI ID3D11VideoDevice",
                    )?;
                    let video_context = com_step(
                        self.context.cast::<ID3D11VideoContext>(),
                        "QI ID3D11VideoContext",
                    )?;
                    let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                        FourCC: 0,
                        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_VPIV::default(),
                        },
                    };
                    let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                        ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_VPOV::default(),
                        },
                    };
                    let mut input_view = None;
                    let mut output_view = None;
                    com_step(
                        video_device.CreateVideoProcessorInputView(
                            source,
                            &enumerator,
                            &input_desc,
                            Some(&mut input_view),
                        ),
                        "CreateVideoProcessorInputView",
                    )?;
                    com_step(
                        video_device.CreateVideoProcessorOutputView(
                            &resource,
                            &enumerator,
                            &output_desc,
                            Some(&mut output_view),
                        ),
                        "CreateVideoProcessorOutputView",
                    )?;
                    let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                        Enable: true.into(),
                        pInputSurface: core::mem::ManuallyDrop::new(input_view),
                        ..Default::default()
                    };
                    com_step(
                        video_context.VideoProcessorBlt(
                            &processor,
                            &output_view.ok_or_else(|| {
                                windows_core::Error::new(
                                    windows_core::HRESULT(0x80004005u32 as i32),
                                    "Cannot create video processor output view",
                                )
                            })?,
                            0,
                            &[stream],
                        ),
                        "VideoProcessorBlt",
                    )?;
                } else if desc.Format == DXGI_FORMAT_R8G8B8A8_UNORM {
                    self.context.CopyResource(&resource, source);
                } else {
                    return Err(windows_core::Error::new(
                        windows_core::HRESULT(0x80070032u32 as i32),
                        format!("Unsupported WebView2 texture format {:?}", desc.Format),
                    ));
                }
                self.context.Flush();
            }
            drop(keyed_mutex);
            com_step(texture.set_timestamp(timestamp), "Texture.Timestamp")?;
            com_step(self.interface.present_texture(&texture), "PresentTexture")?;
            Ok(TextureStreamTestFrame {
                width: desc.Width,
                height: desc.Height,
                format: format!("{:?}", desc.Format),
                presented: true,
            })
        }

        fn create_shared_bridge(
            &self,
            frame: &SharedTextureFrame,
        ) -> windows_core::Result<SharedBridgeSlot> {
            let mut source12: Option<ID3D12Resource> = None;
            com_step(
                unsafe {
                    self.d3d12_device.OpenSharedHandle(
                        HANDLE(frame.resource_handle as usize as *mut _),
                        &mut source12,
                    )
                },
                "ID3D12Device.OpenSharedHandle(resource)",
            )?;
            let source12 = source12.ok_or_else(|| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80004005u32 as i32),
                    "OpenSharedHandle returned no resource",
                )
            })?;
            let fence = if let Some(sync_handle) = frame.sync_handle {
                let mut fence = None;
                com_step(
                    unsafe {
                        self.d3d12_device
                            .OpenSharedHandle(HANDLE(sync_handle as usize as *mut _), &mut fence)
                    },
                    "ID3D12Device.OpenSharedHandle(fence)",
                )?;
                Some(fence.ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "OpenSharedHandle returned no fence",
                    )
                })?)
            } else {
                None
            };
            let on12 = com_step(
                self.bridge_device.cast::<ID3D11On12Device>(),
                "QI ID3D11On12Device",
            )?;
            let mut wrapped = None;
            com_step(
                unsafe {
                    on12.CreateWrappedResource(
                        &source12,
                        &D3D11_RESOURCE_FLAGS::default(),
                        D3D12_RESOURCE_STATE_COMMON,
                        D3D12_RESOURCE_STATE_COMMON,
                        &mut wrapped,
                    )
                },
                "CreateWrappedResource",
            )?;
            let wrapped = wrapped.ok_or_else(|| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80004005u32 as i32),
                    "CreateWrappedResource returned no texture",
                )
            })?;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: frame.width,
                Height: frame.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: 0,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32,
            };
            let mut bridge_texture = None;
            com_step(
                unsafe {
                    self.bridge_device
                        .CreateTexture2D(&desc, None, Some(&mut bridge_texture))
                },
                "CreateTexture2D shared bridge",
            )?;
            let bridge_texture = bridge_texture.ok_or_else(|| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80004005u32 as i32),
                    "Cannot create shared bridge texture",
                )
            })?;
            let shared_handle = com_step(
                unsafe { bridge_texture.cast::<IDXGIResource>()?.GetSharedHandle() },
                "IDXGIResource.GetSharedHandle",
            )?;
            let mut source = None;
            com_step(
                unsafe { self.device.OpenSharedResource(shared_handle, &mut source) },
                "ID3D11Device.OpenSharedResource",
            )?;
            Ok(SharedBridgeSlot {
                width: frame.width,
                height: frame.height,
                sync_handle: frame.sync_handle,
                wrapped,
                bridge_texture,
                source: source.ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "OpenSharedResource returned no texture",
                    )
                })?,
                fence,
            })
        }

        fn present_shared_frame(
            &mut self,
            frame: &SharedTextureFrame,
        ) -> windows_core::Result<TextureStreamTestFrame> {
            let rebuild = self
                .shared_bridges
                .get(&frame.resource_handle)
                .is_none_or(|slot| {
                    slot.width != frame.width
                        || slot.height != frame.height
                        || slot.sync_handle != frame.sync_handle
                });
            if rebuild {
                let slot = self.create_shared_bridge(frame)?;
                self.shared_bridges.insert(frame.resource_handle, slot);
            }
            let slot = self
                .shared_bridges
                .get(&frame.resource_handle)
                .expect("shared bridge slot exists")
                .clone();
            if let Some(fence) = &slot.fence {
                com_step(
                    unsafe { self.d3d12_queue.Wait(fence, frame.sync_value) },
                    "ID3D12CommandQueue.Wait",
                )?;
            }
            let bridge_mutex = com_step(
                KeyedMutexGuard::acquire(&slot.bridge_texture, 0, 1),
                "AcquireSync bridge producer",
            )?;
            let on12 = com_step(
                self.bridge_device.cast::<ID3D11On12Device>(),
                "QI ID3D11On12Device",
            )?;
            {
                let wrapped_guard = WrappedResourceGuard::acquire(
                    on12,
                    slot.wrapped.cast::<ID3D11Resource>()?,
                    self.bridge_context.clone(),
                );
                unsafe {
                    self.bridge_context
                        .CopyResource(&slot.bridge_texture, &slot.wrapped);
                }
                drop(wrapped_guard);
            }
            unsafe {
                self.bridge_context.Flush();
            }
            drop(bridge_mutex);
            let source_mutex = com_step(
                KeyedMutexGuard::acquire(&slot.source, 1, 0),
                "AcquireSync bridge consumer",
            )?;
            let result = self.present_rgba_texture(
                &slot.source,
                frame.width,
                frame.height,
                frame.timeline_ns / 1_000,
            );
            drop(source_mutex);
            result
        }
    }

    fn split_luid(value: u64) -> LUID {
        LUID {
            LowPart: value as u32,
            HighPart: (value >> 32) as u32 as i32,
        }
    }

    fn create_d3d11_device(
        luid: u64,
    ) -> windows_core::Result<(
        ID3D11Device,
        ID3D11DeviceContext,
        ID3D11Device,
        ID3D11DeviceContext,
        ID3D12Device,
        ID3D12CommandQueue,
    )> {
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory2(Default::default())?;
            let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(split_luid(luid))?;
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let mut d3d12_device = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut d3d12_device)?;
            let d3d12_device: ID3D12Device = d3d12_device.ok_or_else(|| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80004005u32 as i32),
                    "D3D12CreateDevice returned no device",
                )
            })?;
            let d3d12_queue: ID3D12CommandQueue =
                d3d12_device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default())?;
            let queues = [Some(d3d12_queue.cast::<IUnknown>()?)];
            let mut bridge_device = None;
            let mut bridge_context = None;
            D3D11On12CreateDevice(
                &d3d12_device,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 as u32,
                None,
                Some(&queues),
                0,
                Some(&mut bridge_device),
                Some(&mut bridge_context),
                None,
            )?;
            Ok((
                device.ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "D3D11CreateDevice returned no device",
                    )
                })?,
                context.ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "D3D11CreateDevice returned no context",
                    )
                })?,
                bridge_device.ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "D3D11On12CreateDevice returned no device",
                    )
                })?,
                bridge_context.ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "D3D11On12CreateDevice returned no context",
                    )
                })?,
                d3d12_device,
                d3d12_queue,
            ))
        }
    }

    thread_local! {
        static TEXTURE_STREAM: RefCell<Option<TextureStream>> = const { RefCell::new(None) };
    }

    pub fn present_shared_frame(
        frame: &SharedTextureFrame,
    ) -> Result<TextureStreamTestFrame, String> {
        TEXTURE_STREAM.with(|current| {
            current
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| "WebView2 TextureStream is not initialized".to_owned())?
                .present_shared_frame(frame)
                .map_err(|error| format!("Cannot present shared WebView2 texture: {error}"))
        })
    }

    pub fn initialize(environment: &ICoreWebView2Environment) -> TextureStreamCapability {
        let experimental = match environment.cast::<ICoreWebView2ExperimentalEnvironment12>() {
            Ok(value) => value,
            Err(error) => {
                return TextureStreamCapability {
                    available: false,
                    adapter_luid: None,
                    stream_ready: false,
                    reason: Some(format!(
                        "WebView2 TextureStream interface is unavailable: {error}"
                    )),
                };
            }
        };
        let luid = match experimental.render_adapter_luid() {
            Ok(value) if value != 0 => value,
            Ok(_) => {
                return TextureStreamCapability {
                    available: true,
                    adapter_luid: None,
                    stream_ready: false,
                    reason: Some("WebView2 renderer adapter is not ready".to_owned()),
                };
            }
            Err(error) => {
                return TextureStreamCapability {
                    available: true,
                    adapter_luid: None,
                    stream_ready: false,
                    reason: Some(format!("Cannot query WebView2 renderer adapter: {error}")),
                };
            }
        };
        let result = (|| -> windows_core::Result<TextureStream> {
            let (device, context, bridge_device, bridge_context, d3d12_device, d3d12_queue) =
                create_d3d11_device(luid)?;
            let stream = experimental.create_texture_stream(&device)?;
            for origin in ALLOWED_ORIGINS {
                stream.add_allowed_origin(origin)?;
            }
            let start_requested = Arc::new(AtomicBool::new(false));
            let start_requested_handler = create_start_requested_handler(start_requested.clone());
            let start_requested_token = stream.add_start_requested(&start_requested_handler)?;
            Ok(TextureStream {
                device,
                context,
                bridge_device,
                bridge_context,
                d3d12_device,
                d3d12_queue,
                interface: stream,
                start_requested_handler,
                start_requested_token,
                start_requested,
                has_started: false,
                texture_size: None,
                video_pipeline: None,
                shared_bridges: HashMap::new(),
            })
        })();

        match result {
            Ok(stream) => {
                TEXTURE_STREAM.with(|current| {
                    *current.borrow_mut() = Some(stream);
                });
                TextureStreamCapability {
                    available: true,
                    adapter_luid: Some(luid),
                    stream_ready: true,
                    reason: None,
                }
            }
            Err(error) => TextureStreamCapability {
                available: true,
                adapter_luid: Some(luid),
                stream_ready: false,
                reason: Some(format!("Cannot create WebView2 TextureStream: {error}")),
            },
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{presentation_timestamp_ns, texture_size_changed};
        use std::time::{Duration, Instant};

        #[test]
        fn recreates_webview_texture_only_when_presentation_size_changes() {
            assert!(texture_size_changed(None, 3840, 1920));
            assert!(!texture_size_changed(Some((3840, 1920)), 3840, 1920));
            assert!(texture_size_changed(Some((3840, 1920)), 1920, 1080));
        }

        #[test]
        fn presentation_timestamp_stays_monotonic_across_graph_clock_resets() {
            let runtime_started_at = Instant::now();
            std::thread::sleep(Duration::from_millis(1));
            let before_graph_reset = presentation_timestamp_ns(runtime_started_at);
            std::thread::sleep(Duration::from_millis(1));
            let after_graph_reset = presentation_timestamp_ns(runtime_started_at);
            assert!(before_graph_reset > 0);
            assert!(after_graph_reset > before_graph_reset);
        }
    }
}

#[cfg(windows)]
pub(crate) use platform::{initialize, present_shared_frame, presentation_timestamp_ns};
