mod app_controller;
mod graph_clipboard;
mod graph_view_model;
mod inspector_forms;
mod inspector_onnx;
mod onnx_runtime;
mod render_bridge;
mod shader_catalog;
mod code_editor;
mod services {
    pub mod desktop_export;
    pub mod file_service;
}

use std::path::PathBuf;

use slint::ComponentHandle;

use app_controller::AppController;
use render_bridge::RenderBridge;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    services::desktop_export::configure_runtime()?;
    let options = match CliOptions::parse(std::env::args_os().skip(1))? {
        ParseOutcome::Options(options) => options,
        ParseOutcome::Help => {
            print_usage();
            return Ok(());
        }
    };
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
    let controller = AppController::new(
        &window,
        renderer,
        runtime_status,
        options.project,
        options.graph_stress,
    )?;
    window.run()?;
    controller.borrow_mut().shutdown();
    drop(controller);
    drop(window);
    Ok(())
}

const DEFAULT_GRAPH_STRESS_COUNT: usize = 100;
const MAX_GRAPH_STRESS_COUNT: usize = 300;

#[derive(Debug, PartialEq, Eq)]
struct CliOptions {
    project: Option<PathBuf>,
    graph_stress: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
enum ParseOutcome {
    Options(CliOptions),
    Help,
}

impl CliOptions {
    fn parse(
        arguments: impl IntoIterator<Item = impl Into<std::ffi::OsString>>,
    ) -> Result<ParseOutcome, String> {
        let mut arguments = arguments.into_iter().map(Into::into).peekable();
        let mut project = None;
        let mut graph_stress = None;

        while let Some(argument) = arguments.next() {
            if argument == "--project" {
                if project.is_some() {
                    return Err("--project may only be specified once".to_owned());
                }
                let path = arguments.next().ok_or("--project requires a path")?;
                if is_recognized_option(&path) {
                    return Err("--project requires a path".to_owned());
                }
                project = Some(PathBuf::from(path));
            } else if argument == "--graph-stress" {
                if graph_stress.is_some() {
                    return Err("--graph-stress may only be specified once".to_owned());
                }
                let count = match arguments.peek() {
                    Some(next) if !is_recognized_option(next) => {
                        let value = arguments.next().expect("peeked argument must exist");
                        value
                            .to_str()
                            .ok_or("--graph-stress count must be valid UTF-8")?
                            .parse::<usize>()
                            .map_err(|_| "--graph-stress count must be a positive integer")?
                    }
                    _ => DEFAULT_GRAPH_STRESS_COUNT,
                };
                if count == 0 {
                    return Err("--graph-stress count must be greater than zero".to_owned());
                }
                if count > MAX_GRAPH_STRESS_COUNT {
                    return Err(format!(
                        "--graph-stress count must not exceed {MAX_GRAPH_STRESS_COUNT}"
                    ));
                }
                graph_stress = Some(count);
            } else if argument == "--help" || argument == "-h" {
                return Ok(ParseOutcome::Help);
            } else {
                return Err(format!("Unknown argument: {}", argument.to_string_lossy()));
            }
        }

        if project.is_some() && graph_stress.is_some() {
            return Err("--project and --graph-stress cannot be used together".to_owned());
        }

        Ok(ParseOutcome::Options(Self {
            project,
            graph_stress,
        }))
    }
}

fn is_recognized_option(argument: &std::ffi::OsStr) -> bool {
    argument == "--project"
        || argument == "--graph-stress"
        || argument == "--help"
        || argument == "-h"
}

fn print_usage() {
    println!("Usage: open_quartz_slint [--project <path> | --graph-stress [count]]");
    println!("Without an option, opens an SDK-created gradient sample.");
    println!(
        "--graph-stress creates a stress-test graph; count defaults to {DEFAULT_GRAPH_STRESS_COUNT} and is capped at {MAX_GRAPH_STRESS_COUNT}."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<ParseOutcome, String> {
        CliOptions::parse(arguments.iter().copied())
    }

    #[test]
    fn parses_no_options() {
        assert_eq!(
            parse(&[]),
            Ok(ParseOutcome::Options(CliOptions {
                project: None,
                graph_stress: None,
            }))
        );
    }

    #[test]
    fn parses_project_path() {
        assert_eq!(
            parse(&["--project", "example.oq"]),
            Ok(ParseOutcome::Options(CliOptions {
                project: Some(PathBuf::from("example.oq")),
                graph_stress: None,
            }))
        );
    }

    #[test]
    fn parses_project_path_starting_with_dash() {
        assert_eq!(
            parse(&["--project", "-draft.json"]),
            Ok(ParseOutcome::Options(CliOptions {
                project: Some(PathBuf::from("-draft.json")),
                graph_stress: None,
            }))
        );
    }

    #[test]
    fn graph_stress_uses_default_count() {
        assert_eq!(
            parse(&["--graph-stress"]),
            Ok(ParseOutcome::Options(CliOptions {
                project: None,
                graph_stress: Some(100),
            }))
        );
    }

    #[test]
    fn graph_stress_parses_explicit_count() {
        assert_eq!(
            parse(&["--graph-stress", "250"]),
            Ok(ParseOutcome::Options(CliOptions {
                project: None,
                graph_stress: Some(250),
            }))
        );
    }

    #[test]
    fn graph_stress_does_not_consume_following_flag() {
        assert_eq!(
            parse(&["--graph-stress", "--project", "example.oq"]),
            Err("--project and --graph-stress cannot be used together".to_owned())
        );
    }

    #[test]
    fn rejects_zero_graph_stress_count() {
        assert_eq!(
            parse(&["--graph-stress", "0"]),
            Err("--graph-stress count must be greater than zero".to_owned())
        );
    }

    #[test]
    fn rejects_graph_stress_count_above_cap() {
        assert_eq!(
            parse(&["--graph-stress", "301"]),
            Err("--graph-stress count must not exceed 300".to_owned())
        );
    }

    #[test]
    fn rejects_project_and_graph_stress_combination() {
        assert_eq!(
            parse(&["--project", "example.oq", "--graph-stress", "10"]),
            Err("--project and --graph-stress cannot be used together".to_owned())
        );
    }

    #[test]
    fn help_is_a_parse_outcome() {
        assert_eq!(parse(&["--help"]), Ok(ParseOutcome::Help));
        assert_eq!(parse(&["-h"]), Ok(ParseOutcome::Help));
    }

    #[test]
    fn rejects_missing_project_path() {
        assert_eq!(
            parse(&["--project", "--graph-stress"]),
            Err("--project requires a path".to_owned())
        );
    }

    #[test]
    fn rejects_invalid_graph_stress_count() {
        assert_eq!(
            parse(&["--graph-stress", "many"]),
            Err("--graph-stress count must be a positive integer".to_owned())
        );
    }

    #[test]
    fn rejects_duplicate_options_and_unknown_arguments() {
        assert_eq!(
            parse(&["--project", "one", "--project", "two"]),
            Err("--project may only be specified once".to_owned())
        );
        assert_eq!(
            parse(&["--graph-stress", "--graph-stress"]),
            Err("--graph-stress may only be specified once".to_owned())
        );
        assert_eq!(
            parse(&["--unknown"]),
            Err("Unknown argument: --unknown".to_owned())
        );
    }
}
