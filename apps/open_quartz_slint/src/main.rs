mod app_controller;
mod graph_view_model;
mod render_bridge;
mod services {
    pub mod file_service;
}

use std::path::PathBuf;

use slint::ComponentHandle;

use app_controller::AppController;
use render_bridge::RenderBridge;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project_path = project_argument()?;
    let (renderer, runtime_status) = match RenderBridge::initialize() {
        Ok((renderer, configuration)) => {
            slint::BackendSelector::new()
                .require_wgpu_30(configuration)
                .select()?;
            (
                Some(renderer),
                "NativeHost initialized with the Slint GPU device and queue.".to_owned(),
            )
        }
        Err(error) => {
            // The shell and project I/O remain usable, but no substitute image is rendered.
            slint::BackendSelector::new().select()?;
            (None, format!("Renderer not initialized: {error}"))
        }
    };
    let window = AppWindow::new()?;
    let controller = AppController::new(&window, renderer, runtime_status, project_path)?;
    window.run()?;
    controller.borrow_mut().shutdown();
    drop(controller);
    drop(window);
    Ok(())
}

fn project_argument() -> Result<Option<PathBuf>, String> {
    let mut args = std::env::args_os().skip(1);
    let mut project = None;
    while let Some(argument) = args.next() {
        if argument == "--project" {
            if project.is_some() {
                return Err("--project may only be specified once".to_owned());
            }
            project = Some(PathBuf::from(
                args.next().ok_or("--project requires a path")?,
            ));
        } else if argument == "--help" || argument == "-h" {
            println!("Usage: open_quartz_slint [--project <path>]");
            println!("Without a project path, opens an SDK-created gradient sample.");
            std::process::exit(0);
        } else {
            return Err(format!("Unknown argument: {}", argument.to_string_lossy()));
        }
    }
    Ok(project)
}
