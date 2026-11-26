# OpenQuartz repositories

The split starts at OpenQuartz commit `24ee19b`. The extracted repositories retain the history of their source directories.

| Repository | Ownership | Automation |
|---|---|---|
| [open_quartz](https://github.com/sanitor/open_quartz) | Core graph runtime, inference and OpenQuartz desktop | Independent CI. Tagged builds publish Windows and macOS Apple Silicon and Intel desktop archives. |
| [oq-media](https://github.com/sanitor/oq-media) | Native media library and tests | Independent CI. The manual D3D12 hardware job uses a Windows runner with the `openquartz-video-gpu` label. |
| [slint-code-editor](https://github.com/sanitor/slint-code-editor) | Rust code editor and Slint component sources | Independent CI for the library, examples and editing tests. |
| [oq-studio](https://github.com/sanitor/oq-studio) | Video editing application | Independent CI and tagged releases for Windows x64 and macOS Apple Silicon and Intel. Manual hardware E2E uses runners with the `openquartz-video-gpu` label. |

Each repository retains the original MIT license and copyright. Distributed third-party libraries retain their own notices. Applications lock oq-media and slint_code_editor to crates.io releases. Studio pins the OpenQuartz core to a Git revision. The code editor exposes its Slint source directory as Cargo build metadata.

Existing Studio project and media files can be opened from the separate application. Development and release commands are documented in the Studio repository.
