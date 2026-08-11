#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod native_host;

#[cfg(windows)]
mod windows_host {
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::env;
    use std::fs::{self, File};
    use std::io::{Read, Seek, SeekFrom};
    use std::mem::size_of;
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, MessageBoxW, IDCANCEL, IDNO, IDYES, MB_ICONERROR, MB_ICONINFORMATION,
        MB_OK, MB_YESNOCANCEL,
    };
    use windows_core::{PCWSTR, PWSTR};

    const MAGIC: &[u8; 16] = b"OQSCRPKG00000003";
    const FOOTER_LEN: u64 = 24;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ScreenSaverManifest {
        version: u32,
        export_id: String,
        name: String,
        project_json: String,
        renderer_node_id: String,
        exposed_inputs: Vec<ExposedInput>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ExposedInput {
        node_id: String,
        label: String,
        kind: String,
    }

    enum LaunchMode {
        Configure(Option<HWND>),
        Run(Option<HWND>),
    }

    pub fn main() {
        if let Err(error) = run() {
            message_box(
                None,
                &error,
                "OpenQuartz Screen Saver",
                MB_OK | MB_ICONERROR,
            );
        }
    }

    fn run() -> Result<(), String> {
        let package_path = env::current_exe().map_err(|error| error.to_string())?;
        let manifest = read_manifest(&package_path)?;
        if manifest.version != 3 {
            return Err(format!(
                "Unsupported screen saver version {}",
                manifest.version
            ));
        }
        match parse_mode(&env::args().skip(1).collect::<Vec<_>>()) {
            LaunchMode::Configure(owner) => configure(&manifest, owner),
            LaunchMode::Run(parent) => launch_renderer(&manifest, parent),
        }
    }

    fn read_manifest(package_path: &Path) -> Result<ScreenSaverManifest, String> {
        let mut file = File::open(package_path).map_err(|error| error.to_string())?;
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        if length < FOOTER_LEN {
            return Err("Invalid OpenQuartz screen saver package".to_owned());
        }
        file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))
            .map_err(|error| error.to_string())?;
        let mut length_bytes = [0_u8; 8];
        file.read_exact(&mut length_bytes)
            .map_err(|error| error.to_string())?;
        let manifest_length = u64::from_le_bytes(length_bytes);
        let mut magic = [0_u8; 16];
        file.read_exact(&mut magic)
            .map_err(|error| error.to_string())?;
        if &magic != MAGIC || manifest_length > length - FOOTER_LEN {
            return Err("Invalid OpenQuartz screen saver footer".to_owned());
        }
        file.seek(SeekFrom::Start(length - FOOTER_LEN - manifest_length))
            .map_err(|error| error.to_string())?;
        let mut bytes = vec![0_u8; manifest_length as usize];
        file.read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    fn parse_mode(args: &[String]) -> LaunchMode {
        let mut index = 0;
        while index < args.len() {
            let argument = args[index].to_ascii_lowercase();
            if argument == "/s" || argument == "-s" {
                return LaunchMode::Run(None);
            }
            if argument == "/p" || argument == "-p" {
                let parent = args.get(index + 1).and_then(|value| parse_hwnd(value));
                return LaunchMode::Run(parent);
            }
            if let Some(value) = argument
                .strip_prefix("/p:")
                .or_else(|| argument.strip_prefix("-p:"))
            {
                return LaunchMode::Run(parse_hwnd(value));
            }
            if argument == "/c" || argument == "-c" {
                let owner = args.get(index + 1).and_then(|value| parse_hwnd(value));
                return LaunchMode::Configure(owner);
            }
            if let Some(value) = argument
                .strip_prefix("/c:")
                .or_else(|| argument.strip_prefix("-c:"))
            {
                return LaunchMode::Configure(parse_hwnd(value));
            }
            index += 1;
        }
        LaunchMode::Configure(None)
    }

    fn parse_hwnd(value: &str) -> Option<HWND> {
        value
            .parse::<isize>()
            .ok()
            .map(|value| HWND(value as *mut _))
    }

    fn launch_renderer(manifest: &ScreenSaverManifest, parent: Option<HWND>) -> Result<(), String> {
        let settings = read_settings(&settings_path(&manifest.export_id)?)?;
        crate::native_host::run(
            &manifest.project_json,
            &manifest.renderer_node_id,
            parent,
            &settings,
        )
    }

    fn configure(manifest: &ScreenSaverManifest, owner: Option<HWND>) -> Result<(), String> {
        let foreground = unsafe { GetForegroundWindow() };
        let owner = owner.or_else(|| (!foreground.0.is_null()).then_some(foreground));
        if manifest.exposed_inputs.is_empty() {
            message_box(
                owner,
                "This screen saver has no configurable inputs.",
                &manifest.name,
                MB_OK | MB_ICONINFORMATION,
            );
            return Ok(());
        }
        let settings_path = settings_path(&manifest.export_id)?;
        let mut settings = read_settings(&settings_path)?;
        for input in &manifest.exposed_inputs {
            let current = settings
                .get(&input.node_id)
                .map(|path| path.as_str())
                .unwrap_or("Exported default");
            let prompt = format!(
                "{}\n\nCurrent: {}\n\nYes: choose a {}\nNo: use the exported default\nCancel: leave settings unchanged",
                input.label, current, input.kind
            );
            let result = message_box(
                owner,
                &prompt,
                &manifest.name,
                MB_YESNOCANCEL | MB_ICONINFORMATION,
            );
            if result == IDCANCEL {
                return Ok(());
            }
            if result == IDNO {
                settings.remove(&input.node_id);
                continue;
            }
            if result == IDYES {
                if let Some(path) = choose_file(owner, input)? {
                    settings.insert(input.node_id.clone(), path);
                }
            }
        }
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string())?;
        fs::write(settings_path, bytes).map_err(|error| error.to_string())?;
        message_box(
            owner,
            "Screen saver settings were saved.",
            &manifest.name,
            MB_OK | MB_ICONINFORMATION,
        );
        Ok(())
    }

    fn choose_file(owner: Option<HWND>, input: &ExposedInput) -> Result<Option<String>, String> {
        let mut file_buffer = vec![0_u16; 32_768];
        let filter = if input.kind == "video" {
            wide("Video files\0*.mp4;*.webm;*.mov;*.avi;*.mkv;*.ogg\0All files\0*.*\0\0")
        } else {
            wide("Image files\0*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.gif;*.tif;*.tiff\0All files\0*.*\0\0")
        };
        let title = wide(&format!("Choose {}", input.label));
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner.unwrap_or_default(),
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(file_buffer.as_mut_ptr()),
            nMaxFile: file_buffer.len() as u32,
            lpstrTitle: PCWSTR(title.as_ptr()),
            Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY,
            ..Default::default()
        };
        if !unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
            return Ok(None);
        }
        let length = file_buffer
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(file_buffer.len());
        String::from_utf16(&file_buffer[..length])
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn settings_path(export_id: &str) -> Result<PathBuf, String> {
        if export_id.is_empty()
            || !export_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("Invalid screen saver export ID".to_owned());
        }
        let base = env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        Ok(base
            .join("OpenQuartz")
            .join("ScreenSavers")
            .join(export_id)
            .join("settings.json"))
    }

    fn read_settings(path: &Path) -> Result<HashMap<String, String>, String> {
        if !path.is_file() {
            return Ok(HashMap::new());
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    fn message_box(
        owner: Option<HWND>,
        text: &str,
        caption: &str,
        style: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE,
    ) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
        let text = wide(text);
        let caption = wide(caption);
        unsafe {
            MessageBoxW(
                owner,
                PCWSTR(text.as_ptr()),
                PCWSTR(caption.as_ptr()),
                style,
            )
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_standard_modes() {
            assert!(matches!(parse_mode(&[]), LaunchMode::Configure(None)));
            assert!(matches!(parse_mode(&["/s".into()]), LaunchMode::Run(None)));
            assert!(matches!(
                parse_mode(&["/p".into(), "42".into()]),
                LaunchMode::Run(Some(_))
            ));
            assert!(matches!(
                parse_mode(&["/c:73".into()]),
                LaunchMode::Configure(Some(_))
            ));
        }

        #[test]
        fn rejects_unsafe_export_ids() {
            assert!(settings_path("safe-id_1").is_ok());
            assert!(settings_path("../unsafe").is_err());
        }
    }
}

#[cfg(windows)]
fn main() {
    windows_host::main();
}

#[cfg(not(windows))]
fn main() {}
