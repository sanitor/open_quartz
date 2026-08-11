use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use open_quartz::gpu::{GpuBackend, GpuExecutor};
use open_quartz::runtime::{DataPathMode, Runtime, RuntimeCapabilities, RuntimeFrameInput};
use open_quartz::types::{Graph, InputMode, NodeType};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetDC, ReleaseDC, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetSystemMetrics, PeekMessageW, PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, HMENU, MSG, PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN,
    SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_NCCREATE,
    WM_QUIT, WNDCLASSW, WS_CHILD, WS_POPUP, WS_VISIBLE,
};
use windows_core::PCWSTR;

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn run(
    project_json: &str,
    renderer_node_id: &str,
    parent: Option<HWND>,
    resource_overrides: &HashMap<String, String>,
) -> Result<(), String> {
    EXIT_REQUESTED.store(false, Ordering::Release);
    let graph = parse_graph(project_json)?;
    validate_graph(&graph)?;
    let mut renderer = NativeRenderer::new(graph, renderer_node_id, resource_overrides)?;
    let window = create_window(parent)?;
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
    };

    let mut message = MSG::default();
    let mut next_frame = Instant::now();
    loop {
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                return Ok(());
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if EXIT_REQUESTED.load(Ordering::Acquire) {
            unsafe {
                let _ = DestroyWindow(window);
            }
            continue;
        }
        let now = Instant::now();
        if now >= next_frame {
            let (mut rgba, width, height) = renderer.render()?;
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            present_bgra(window, &rgba, width, height)?;
            next_frame = now + Duration::from_millis(16);
        } else {
            std::thread::sleep((next_frame - now).min(Duration::from_millis(2)));
        }
    }
}

struct NativeRenderer {
    runtime: Runtime,
    executor: GpuExecutor,
    renderer_node_id: String,
    started_at: Instant,
}

impl NativeRenderer {
    fn new(
        graph: Graph,
        renderer_node_id: &str,
        resource_overrides: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|error| format!("Cannot create screen saver GPU adapter: {error}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("OpenQuartz Screen Saver"),
            ..Default::default()
        }))
        .map_err(|error| format!("Cannot create screen saver GPU device: {error}"))?;
        let backend = Arc::new(GpuBackend::from_device(device, queue));
        let mut executor = GpuExecutor::new(backend);
        let mut runtime = Runtime::new_native(RuntimeCapabilities {
            data_paths: vec![DataPathMode::CpuCopy],
        });
        runtime.set_graph(&graph).map_err(|error| error.to_json())?;
        let plan = runtime
            .execution_plan()
            .ok_or_else(|| "Screen saver graph did not produce an execution plan".to_owned())?;
        executor
            .sync_plan(plan)
            .map_err(|error| error.to_string())?;
        upload_images(&graph, resource_overrides, &mut executor)?;
        runtime.play(0).map_err(|error| error.to_json())?;
        Ok(Self {
            runtime,
            executor,
            renderer_node_id: renderer_node_id.to_owned(),
            started_at: Instant::now(),
        })
    }

    fn render(&mut self) -> Result<(Vec<u8>, u32, u32), String> {
        let now_ns = self
            .started_at
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let (width, height) = self
            .runtime
            .execution_plan()
            .map(|plan| (plan.default_width.max(1), plan.default_height.max(1)))
            .unwrap_or((960, 540));
        self.runtime
            .advance(&RuntimeFrameInput {
                now_ns,
                date: date_uniform(SystemTime::now()),
                mouse: [0.0; 4],
                resolution: [width as f32, height as f32, 1.0],
            })
            .map_err(|error| error.to_json())?;
        let commands = self.runtime.drain_commands();
        self.runtime
            .execute_gpu(&mut self.executor, &commands)
            .map_err(|error| error.to_json())?;
        let output = self
            .executor
            .output_handle(&self.renderer_node_id)
            .ok_or_else(|| "Screen saver renderer has no GPU output".to_owned())?;
        let rgba = pollster::block_on(self.executor.read_output_rgba(&self.renderer_node_id))
            .map_err(|error| error.to_string())?;
        Ok((rgba, output.width, output.height))
    }
}

fn parse_graph(project_json: &str) -> Result<Graph, String> {
    let value: serde_json::Value = serde_json::from_str(project_json)
        .map_err(|error| format!("Invalid project JSON: {error}"))?;
    let graph = value.get("graph").unwrap_or(&value);
    serde_json::from_value(graph.clone()).map_err(|error| format!("Invalid project graph: {error}"))
}

fn validate_graph(graph: &Graph) -> Result<(), String> {
    for node in &graph.nodes {
        if node.node_type == NodeType::Onnx {
            return Err(format!(
                "Screen saver node '{}' requires the native ONNX adapter, which is not packaged in this export",
                node.data.label
            ));
        }
        if node.node_type == NodeType::Input && node.data.input_mode == Some(InputMode::Video) {
            return Err(format!(
                "Screen saver node '{}' requires the native video adapter, which is not packaged in this export",
                node.data.label
            ));
        }
    }
    Ok(())
}

fn upload_images(
    graph: &Graph,
    resource_overrides: &HashMap<String, String>,
    executor: &mut GpuExecutor,
) -> Result<(), String> {
    for node in &graph.nodes {
        if node.node_type != NodeType::Input || node.data.input_mode != Some(InputMode::Image) {
            continue;
        }
        let image = if let Some(path) = resource_overrides.get(&node.id) {
            image::open(path).map_err(|error| format!("Cannot load {}: {error}", path))?
        } else if let Some(data_url) = &node.data.image_data_url {
            let encoded = data_url
                .split_once(',')
                .map(|(_, payload)| payload)
                .ok_or_else(|| {
                    format!("Image node '{}' has an invalid data URL", node.data.label)
                })?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| {
                    format!("Cannot decode image node '{}': {error}", node.data.label)
                })?;
            image::load_from_memory(&bytes).map_err(|error| {
                format!("Cannot decode image node '{}': {error}", node.data.label)
            })?
        } else {
            continue;
        };
        let rgba = image.to_rgba8();
        executor
            .upload_rgba(&node.id, rgba.as_raw(), rgba.width(), rgba.height())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn create_window(parent: Option<HWND>) -> Result<HWND, String> {
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|error| format!("Cannot resolve screen saver module: {error}"))?;
    let class_name = wide("OpenQuartzScreenSaverHost");
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err("Cannot register screen saver window class".to_owned());
    }
    let (style, x, y, width, height, parent_window) = if let Some(parent) = parent {
        let mut rect = RECT::default();
        unsafe { GetClientRect(parent, &mut rect) }
            .map_err(|error| format!("Cannot read preview window size: {error}"))?;
        (
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
            Some(parent),
        )
    } else {
        (
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            unsafe { GetSystemMetrics(SM_CXSCREEN) },
            unsafe { GetSystemMetrics(SM_CYSCREEN) },
            None,
        )
    };
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(wide("OpenQuartz Screen Saver").as_ptr()),
            style,
            if parent.is_some() { 0 } else { x },
            if parent.is_some() { 0 } else { y },
            width,
            height,
            parent_window,
            None::<HMENU>,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|error| format!("Cannot create screen saver window: {error}"))
}

fn present_bgra(window: HWND, bgra: &[u8], width: u32, height: u32) -> Result<(), String> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(window, &mut rect) }
        .map_err(|error| format!("Cannot read screen saver window size: {error}"))?;
    let dc = unsafe { GetDC(Some(window)) };
    if dc.0.is_null() {
        return Err("Cannot acquire screen saver drawing context".to_owned());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: width.saturating_mul(height).saturating_mul(4),
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        StretchDIBits(
            dc,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            0,
            0,
            width as i32,
            height as i32,
            Some(bgra.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        let _ = ReleaseDC(Some(window), dc);
    }
    Ok(())
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let _ = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            LRESULT(1)
        }
        WM_KEYDOWN | WM_LBUTTONDOWN | WM_CLOSE => {
            EXIT_REQUESTED.store(true, Ordering::Release);
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn date_uniform(now: SystemTime) -> [f32; 4] {
    let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = (seconds % 86_400) as f32;
    let (year, month, day) = civil_from_days(days);
    [year as f32, month as f32, day as f32, seconds_of_day]
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_a_frame_through_the_shared_rust_kernel() {
        let graph: Graph = serde_json::from_value(json!({
            "nodes": [
                {
                    "id": "shader",
                    "type": "shader",
                    "position": { "x": 0.0, "y": 0.0 },
                    "data": {
                        "type": "shader",
                        "label": "Red",
                        "shaderCode": "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }",
                        "inputs": [],
                        "outputs": [{ "id": "output", "label": "output", "dataType": "sampler2D", "direction": "output" }],
                        "uniforms": {},
                        "autoSize": false,
                        "width": 2,
                        "height": 2
                    }
                },
                {
                    "id": "renderer",
                    "type": "renderer",
                    "position": { "x": 100.0, "y": 0.0 },
                    "data": {
                        "type": "renderer",
                        "label": "Renderer",
                        "shaderCode": "",
                        "inputs": [{ "id": "input", "label": "inputImage", "dataType": "sampler2D", "direction": "input" }],
                        "outputs": [],
                        "uniforms": {}
                    }
                }
            ],
            "edges": [{
                "id": "shader-renderer",
                "source": "shader",
                "sourceHandle": "output",
                "target": "renderer",
                "targetHandle": "input"
            }]
        }))
        .unwrap();
        let mut renderer = NativeRenderer::new(graph, "renderer", &HashMap::new()).unwrap();
        let (rgba, width, height) = renderer.render().unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn converts_unix_days_for_shader_date_uniforms() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }
}
