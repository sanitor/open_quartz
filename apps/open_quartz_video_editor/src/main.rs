mod controller;
mod document;
mod media;
mod preview;

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
    let (bridge, configuration, description) = preview::PreviewBridge::new()?;
    let backend = bridge.backend.clone();
    slint::BackendSelector::new()
        .require_wgpu_30(configuration)
        .select()?;
    let window = EditorWindow::new()?;
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
