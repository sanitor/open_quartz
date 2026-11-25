mod animation_key;
mod batch_media;
mod controller;
#[cfg(target_os = "macos")]
mod audio_mix;
#[cfg(target_os = "macos")]
mod audio_rate;
#[cfg(target_os = "macos")]
mod audio_waveform;
#[cfg(target_os = "macos")]
mod audio_output;
mod delivery;
mod document;
mod document_recovery;
mod media;
mod preview;
#[cfg(target_os = "macos")]
mod proxy;
mod title_raster;
mod clip_pipeline;
mod output_queue;
mod timeline_viewport;
mod color_scope;
mod color_lut;
#[path = "../../open_quartz_slint/src/graph_view_model.rs"]
mod graph_view_model;
#[path = "../../open_quartz_slint/src/graph_clipboard.rs"]
mod graph_clipboard;
#[path = "../../open_quartz_slint/src/inspector_forms.rs"]
mod inspector_forms;
#[path = "../../open_quartz_slint/src/shader_catalog.rs"]
mod shader_catalog;
#[path = "../../open_quartz_slint/src/uniform_literals.rs"]
mod uniform_literals;
#[path = "../../open_quartz_slint/src/corrector_editor.rs"]
mod corrector_editor;

use slint::ComponentHandle;
slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    media::configure_runtime()?;
    let mut args = std::env::args_os().skip(1);
    let mut project = None;
    while let Some(arg) = args.next() {
        if arg == "--project" {
            project = Some(std::path::PathBuf::from(
                args.next().ok_or("--project requires a file path")?,
            ));
        } else if arg == "--help" {
            println!(
                "OpenQuartz Studio\nUsage: open_quartz_video_editor [--project FILE.oqvideo.json]"
            );
            return Ok(());
        } else {
            return Err(format!("Unknown option: {}", arg.to_string_lossy()).into());
        }
    }
    let controls_on_left = match std::env::var("OPEN_QUARTZ_WINDOW_CONTROLS") {
        Ok(position) if position == "left" => true,
        Ok(position) if position == "right" => false,
        Ok(position) if position == "platform" => cfg!(target_os = "macos"),
        Ok(position) => return Err(format!(
            "OPEN_QUARTZ_WINDOW_CONTROLS must be left, right, or platform (got {position:?})"
        ).into()),
        Err(std::env::VarError::NotPresent) => cfg!(target_os = "macos"),
        Err(error) => return Err(error.into()),
    };
    let (bridge, configuration, description) = preview::PreviewBridge::new()?;
    let backend = bridge.backend.clone();
    slint::BackendSelector::new()
        .require_wgpu_30(configuration)
        .select()?;
    let window = EditorWindow::new()?;
    window.set_window_controls_on_left(controls_on_left);
    window.set_mac_style_controls(cfg!(target_os = "macos"));
    slint_code_editor::install_editor!(window, CodeEditorBridge);
    let controller = controller::Controller::new(&window, bridge, backend, description);
    if let Some(path) = project {
        controller.borrow_mut().open_path(path);
    }
    let weak = std::rc::Rc::downgrade(&controller);
    window.window().on_close_requested(move || {
        if let Some(controller) = weak.upgrade() {
            if !controller.borrow().may_close() {
                return slint::CloseRequestResponse::KeepWindowShown;
            }
        }
        slint::CloseRequestResponse::HideWindow
    });
    #[cfg(feature = "ui-automation")]
    if std::env::var_os("OPEN_QUARTZ_UI_CLOSE_STDIN").is_some() {
        let weak = window.as_weak();
        std::thread::spawn(move || {
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_ok() && line.trim() == "close" {
                let _ = weak.upgrade_in_event_loop(|window| {
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::CloseRequested);
                });
            }
        });
    }
    window.run()?;
    controller.borrow_mut().shutdown();
    drop(controller);
    drop(window);
    Ok(())
}
