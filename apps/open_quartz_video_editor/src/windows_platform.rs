use i_slint_backend_winit::Backend;
use i_slint_core::{InternalToken, SlintContextWeak};
use slint::platform::{Clipboard, EventLoopProxy, Platform, PlatformError, WindowAdapter};
use std::cell::{Ref, RefCell};
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::Duration;

/// Slint keeps the selected platform in thread-local storage. The wrapper
/// lets the editor release the actual backend, including its shared D3D12
/// configuration and event-loop state, before Windows enters TLS teardown.
pub(super) struct WindowsPlatform(PlatformProxy);

#[derive(Clone)]
struct PlatformProxy(Rc<RefCell<Option<Backend>>>);

impl WindowsPlatform {
    pub(super) fn install(
        configuration: slint::wgpu_30::WGPUConfiguration,
        hdr_monitor: crate::hdr_monitor::Monitor,
    ) -> Result<Self, PlatformError> {
        let backend = Backend::builder()
            .with_renderer_name("femtovg-wgpu")
            .request_graphics_api(i_slint_core::graphics::RequestedGraphicsAPI::WGPU30(
                configuration,
            ))
            .with_custom_application_handler(Box::new(hdr_monitor))
            .build()?;
        let platform = PlatformProxy(Rc::new(RefCell::new(Some(backend))));
        slint::platform::set_platform(Box::new(platform.clone()))
            .map_err(PlatformError::SetPlatformError)?;
        Ok(Self(platform))
    }
}

impl Drop for WindowsPlatform {
    fn drop(&mut self) {
        let backend = self.0 .0.borrow_mut().take();
        drop(backend);
    }
}

impl PlatformProxy {
    fn backend(&self) -> Result<Ref<'_, Backend>, PlatformError> {
        Ref::filter_map(self.0.borrow(), Option::as_ref)
            .map_err(|_| PlatformError::NoEventLoopProvider)
    }
}

impl Platform for PlatformProxy {
    fn bind_context(&self, context: SlintContextWeak, token: InternalToken) {
        if let Ok(backend) = self.backend() {
            backend.bind_context(context, token);
        }
    }

    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        self.backend()?.create_window_adapter()
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        self.backend()?.run_event_loop()
    }

    fn process_events(
        &self,
        timeout: Option<Duration>,
        token: InternalToken,
    ) -> Result<ControlFlow<()>, PlatformError> {
        self.backend()?.process_events(timeout, token)
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        self.backend().ok()?.new_event_loop_proxy()
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if let Ok(backend) = self.backend() {
            backend.set_clipboard_text(text, clipboard);
        }
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        self.backend().ok()?.clipboard_text(clipboard)
    }

    fn cursor_flash_cycle(&self) -> Duration {
        self.backend()
            .map_or(Duration::ZERO, |backend| backend.cursor_flash_cycle())
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.backend()?.open_url(url)
    }
}
