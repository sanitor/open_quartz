# oq-media

`oq-media`（Rust 导入名 `oq_media`）是在本仓孵化的独立媒体库。`oq` 记录 OpenQuartz 出处，不代表依赖；OpenQuartz 是消费者。公共媒体类型不依赖节点图、Slint 或 wgpu。

目前实现 macOS H.264/HEVC 文件的进程内 VideoToolbox 严格硬解，提供 `Session`、保留 AVFrame/CVPixelBuffer 的 `FrameLease`、PTS、source/timeline epoch、reset、取消及 drain。跨平台会话、编码、任意 seek 和独立播放器组件仍未实现，不能把当前切片当作完整跨平台媒体框架。

## 构建与使用

macOS 的 `videotoolbox` feature 使用固定 ABI 的 FFmpeg 8.1.1 动态 libav。先从仓库根目录准备 SDK，再运行示例或测试：

```sh
node scripts/prepare-media-sdk.mjs
cargo run -p oq-media --release --features videotoolbox --example decode -- VIDEO.mp4
cargo test -p oq-media --features videotoolbox --locked
```

SDK 构建脚本、补丁和 LGPL 对应源码分发当前由宿主仓库提供；独立部署仍需配置 libav 的链接和运行库发现。媒体核心没有 OpenQuartz Rust 依赖，但 SDK 准备尚未提取成独立分发工具。

运行时要求 `oq-vt-hw2-metal` 构建标识。其补丁要求 H.264/HEVC 使用硬件解码，并让 macOS 解码输出请求 Metal 兼容，而非 OpenGL 兼容。旧 `oq-vt-hw1` 运行库不能通过此版本的检查；重新运行准备脚本会依据新构建标识重建 SDK。源码归档、精确补丁、构建参数及 LGPL 许可证随 SDK 一起提供。

帧保留原生表面，不映射到 CPU。当前 OpenQuartz 的可选 GPU 消费者通过 CoreVideo/Metal 导入平面，并转换为 SDR sRGB RGBA；该适配尚位于宿主执行层，不是本 crate 的 API。当前支持 NV12/P010/BGRA8、BT.709/BT.470BG primaries 及 BT.709/SMPTE170M/sRGB transfer；BT.470BG 在逆 transfer 后以 D65 线性 RGB 原色矩阵转换到 BT.709，再编码 sRGB，不冒充 BT.709 源。HDR、旋转、crop、非方形像素和隔行呈现等未实现语义明确报错。

解码帧的 `ColorInfo.primaries` 与 YUV `matrix` 独立。H.264 SPS 与 MP4 容器色彩标记冲突时，使用 libav 解码帧报告的实际色彩元数据，不用容器的 BT.709 标签覆盖 BT.470BG/BT.601。真实 1920×1080 H.264 Baseline MP4 曾因此在 Choose File 时被拒绝；现已验证原生文件选择后的首帧、独立 Video 节点播放、暂停和恢复，并由用户确认正常。像素回归用已知 YUV 色块验证 PAL→709 色域转换、容器冲突和交替原色时的 pipeline 缓存隔离。

## 8K 性能与 OpenGL 兼容慢路径

记录日期：2026-11-04。设备：Apple M4 Pro、12 CPU 核、24 GiB 内存、macOS 26.5.1、Metal。以下是特定设备与合成素材的结果，不是跨平台保证或真实电影素材全集结论。

### 根因与单变量实验

FFmpeg 8.1.1 的 `libavcodec/videotoolbox.c` 在 macOS 构建解码输出属性时，无条件设置 `kCVPixelBufferIOSurfaceOpenGLTextureCompatibilityKey=true`。我们的消费者使用 Metal。对同一份 8-bit HEVC 码流，仅改变这一兼容属性，就能使 NV12 输出吞吐从约 56 FPS 恢复到约 155 FPS。

直接 VideoToolbox 实验强制硬解，并查询 `UsingHardwareAcceleratedVideoDecoder=true`；逐帧提交、逐帧等待，完整输出 300 帧、不丢帧。前 32 帧预热，三轮取中位数。输出位深和像素格式保持不变。

| 原生输出属性 | HEVC 8-bit → NV12/420v | HEVC 10-bit → P010/x420 |
|---|---:|---:|
| 不要求图形 API 兼容 | 156.02 FPS | 154.79 FPS |
| OpenGL 兼容 | 56.41 FPS | 152.59 FPS |
| Metal 兼容 | 155.27 FPS | 155.50 FPS |

这排除了“8-bit 固有地更慢”、GPU 色彩转换、软件解码 fallback，以及同步等待策略不同等解释。Apple 内部的额外布局转换、复制或同步没有分别测量；只能确认兼容属性对吞吐的因果影响，不能声称已经证明某一种内部机制。

### 同一 oq-media 可执行文件的 SDK A/B

使用同一个 Release `decode` 示例和相同输入，仅切换 libav：原 SDK 与只将 macOS OpenGL 属性替换为 Metal 的诊断 SDK。相同 x265 ultrafast 参数生成的 8/10-bit 文件均为 7680×4320、30 FPS、300 帧、BT.709 limited-range SDR、约 78 Mbps。时间包含启动和完整 300 帧输出，stdout 由父进程捕获；没有 GPU 转换或 UI。

| 素材 | 原 SDK 中位数（范围） | Metal 诊断 SDK 中位数（范围） |
|---|---:|---:|
| x265 8-bit | 53.69（53.57–54.63）FPS | 141.04（139.04–142.80）FPS |
| x265 10-bit | 138.74（135.04–139.41）FPS | 138.11（136.65–138.34）FPS |
| 原硬件编码器生成的 8-bit 素材 | 53.34（52.68–53.61）FPS | 136.83（78.10–138.10）FPS |

x265 8-bit 恢复 **2.63×**，10-bit 基本不变，位深倒挂消失。最后一行保留了诊断 SDK 首轮 78.10 FPS 的异常低值，未删除异常轮次。原生属性 A/B 和 x265 控制组更稳定，根因判断不依赖这次异常。

### 正式 SDK 修复后复测

通过 `node scripts/prepare-media-sdk.mjs` 重建正式 `oq-vt-hw2-metal` SDK，再重编 Release `decode` 示例。使用相同完整文件计时口径、三轮交错顺序，不使用诊断 SDK：

| 素材 | 正式修复版中位数 | 三轮范围 |
|---|---:|---:|
| x265 8-bit | 135.14 FPS | 134.39–136.08 FPS |
| x265 10-bit | 135.29 FPS | 133.57–135.49 FPS |
| 原硬件编码器生成的 8-bit 素材 | 132.69 FPS | 114.27–134.15 FPS |

与前述原 SDK 的 x265 8-bit 中位数相比约 **2.52×**；不同轮次的设备状态会影响绝对值，不能把诊断版与正式版的小差异归因于构建标识。本次正式复测仍证明 8/10-bit 倒挂已经消除。

媒体核心 3 项回归通过；GPU 色彩回归覆盖 H.264 NV12 full/limited-range、HEVC 8-bit NV12 和 HEVC 10-bit P010，提交后释放 decoder/frame 再显式 readback 验证像素。正式 SDK 的 8K 8-bit 素材在 Slint/Metal 示例中实际验证首帧、Play、Pause 后计数稳定、Resume 和 Stop/rewind；播放帧计数推进且始终 `CPU upload 0 bytes`，窗口普通退出码为 0。这不是屏幕 FPS 基准。旧 `oq-vt-hw1` SDK 已实际验证被拒绝，二次准备 SDK 命中缓存。


### 早期完整媒体链路对比（修复前）

此前以历史 macOS Tauri 的软件媒体路径作基线：FFmpeg 4.4 软件解码 → CPU RGBA → stdout pipe → wgpu 上传 → Video/Renderer → 960×540 预览 readback 与数据封装；新路径使用 VideoToolbox 原生表面、GPU 色彩转换与共享纹理。

Release freerun，解码和 GPU 重叠、一个待处理帧、有界队列、不丢帧且不重复计数；每轮完整消费 300 帧，剔除前 30 帧预热，三轮中位数，等待 GPU 完成。预览双方均采样全尺寸 8K 输出到 960×540；旧路径还含 readback/payload，新路径保留 GPU 纹理。这里不含 WebView IPC/JS 绘制、Slint/FemtoVG、OS compositor、音频或屏幕刷新。

| 当时的素材 | 历史软件媒体路径 | 修复前 VideoToolbox/Metal 路径 | 倍率 |
|---|---:|---:|---:|
| HEVC 8-bit | 21.67 FPS | 49.92 FPS | 2.30× |
| HEVC 10-bit | 6.32 FPS | 136.53 FPS | 21.60× |

**这张表是旧实现记录，8-bit 约 50 FPS 不是硬件极限。** 不能将本次解码 A/B 的 141 FPS 直接代入完整链路表，也不能将历史媒体路径重建说成两套完整应用的屏幕 FPS。当前 Tauri 也消费更新后的 NativeHost；保留的历史整包 benchmark 因 `Clock is not running` 失败，没有有效整包 freerun 成绩。

拆测旧 FFmpeg 4.4：只解码到 YUV/null 时，8/10-bit 分别约 153.45/141.31 FPS；转 RGBA 后写 `/dev/null`，尚未经过 stdout pipe 或 GPU 上传时，分别降至 51.26/7.56 FPS。旧 10-bit 路径的主要损失在 RGBA 转换/输出阶段，不能称作“硬件解码器加速 20 倍”。旧路径每帧还需上传 132,710,400 字节（126.56 MiB）CPU RGBA，新路径没有这次 CPU RGBA 上传，但仍有 GPU 转换，不能称完整链路完全无拷贝。

### 复现素材与测量边界

相同 x265 参数、仅改变位深的素材生成命令（替换 `PIXEL_FORMAT` 为 `yuv420p` 或 `yuv420p10le`）：

```sh
ffmpeg -f lavfi -i testsrc2=size=7680x4320:rate=30 -frames:v 300 \
  -c:v libx265 -preset ultrafast -crf 25 -pix_fmt PIXEL_FORMAT \
  -x265-params pools=4:frame-threads=2:log-level=error \
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 \
  -color_range tv -tag:v hvc1 output.mp4
```

不同 FFmpeg/x265 版本可能生成不同码流。调查所用 bundled FFmpeg 4.4 文件 SHA-256：

- x265 8-bit：`448f8911c8fd0170c414a515339d98aa9e7c9ff0b0f3ea74f75d4c1d06c549b3`
- x265 10-bit：`6934b5eccc762544731704247da0ae6f0e4916cb86ab8e4193658626ad36eefa`

从父进程启动 Release `decode` 示例并捕获 stdout，按 300/墙钟秒数计算完整文件 FPS；核实 300 帧、实际尺寸、NV12/P010、首末 PTS 和 `hardware: Some(true)`。不要逐帧打印到终端后把终端速度算作解码速度。公平比较时保持设备负载、素材、构建模式和计时边界一致；解码吞吐、帧交付吞吐与屏幕 FPS 必须分别报告。

调查原始 JSON 保存在本机忽略目录 `test-results/media-bitdepth/`；该目录不随 crate 发布。本节保留关键测量方法、结果和边界，阅读不依赖本机产物。
