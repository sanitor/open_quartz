# 推理架构

用户能力为 **INFERENCE / 推理**；ONNX 是模型产物格式，不等于执行后端。生产实时绘制与推理的图像 payload 必须为硬件 buffer/texture，ORT host Tensor 仅供显式 reference/诊断，绝不是生产 fallback。

## 当前实现与边界

生产 macOS 路径使用与 Slint **wgpu 30 同一个 Metal Device/Queue**：

```text
GPU source texture
  → GPU BT.601 / resize 224×224
  → 完整四层 FP32 convolution + ReLU
  → PixelShuffle ×3 + RGB reconstruction
  → resident RGBA texture
  → Slint preview / downstream Shader
```

`oq-inference` 的独立 `gpu` feature 提供实际 GPU 模型编译器与 `GpuImageSession`，不依赖 OpenQuartz 图或 Slint。编译器验证原始 ONNX topology、weights 与 reshape layout，运行完整网络，不以近似滤镜替代。`GpuImageSession` scratch 为 STORAGE-only；模型文件解析、weights 初始上传与 scalar controls 属于准备边界。逐帧 source pixels 不进入 CPU，不创建 host image Tensor，不做 CPU 前后处理、读回或结果重上传。

卷积使用按已验证层几何专门编译的 FP32 内核：一个 64-thread workgroup 覆盖 32 个输出通道 × 32 个邻接像素，每线程累加 4×4 register tile；归约以 K=32 的输入/权重共享分块执行，workgroup storage 为 8KB。输出仍是 NCHW；9-channel 尾部、padding、bias 和 ReLU 保持原模型语义。专门编译和权重上传只发生在准备阶段，不增加逐帧 CPU 图像处理。

当前 resident 模型仅 **super-resolution-3x（SR3）**，网络 profile 固定 `[1,1,224,224] → [1,1,672,672]`。任意源尺寸仍先缩到 224×224，最终 RGBA 尺寸为源宽高各 ×3。这不是 window tiling，也不是原始大图全分辨率推理；不据此声称大图完整质量或更快吞吐。catalog 其他模型保留自省与任务契约，不表示已支持 resident 执行。其他模型或平台在 frame access 前报错。

## 执行政策与方案

生产 backend 为 `MetalGpu`、provider 为 `Metal`，plan 报告 GPU placement 与 Device transfer path。生产强制 `RequireResident`、`AcceleratorOnly`、CPU fallback `Forbid`；UI 仅提供 **Metal GPU / Auto Hardware**。Auto Hardware 不表示可以退到 ORT/CPU。旧项目配置仍可解析，但不能放宽实时硬件约束。

独立 ORT CPU、Windows DirectML、macOS CoreML EP 与命名 FP32 Tensor API 保留为显式 reference/诊断接口及 examples。CoreML EP 的 provider 注册或混合 CoreML/CPU dispatch 不能证明 resident、GPU/ANE-only，也不能成为生产兼容路径。Direct Core ML、WinML、CUDA 与其他平台的 resident 支持没有由本路径实现。

Windows `.scr` 的推理导出与 stub 执行明确不可用：没有合格 resident backend，禁止回退到同步 readback → CPU → upload。非推理 Shader/硬件视频导出仍保留。

## 模型事实、自省与任务契约

`inspect_onnx(model)` 使用 prost 0.14 解析源声明，无需 ORT 或设备；它不是完整 ONNX 语义验证器。`source_interface` 保存原始 `ArtifactInterface`：IR version、producer name/version、domain、model version、graph name、模型/图/值 doc-string 与 custom properties、graph opset imports。源版本与 runtime metadata 分开，不把 ORT model-version MAX sentinel 当作源版本。opset 来自源 import；producer_version 由源信息补齐。

源类型递归保留 Tensor、SparseTensor、Sequence、Map、Optional、Opaque、Unknown 和精确 dtype 数字代码（包括未知代码）；UnknownRank 与 rank-zero scalar 分开；维度保留静态值、symbol、dynamic 与 denotation。输入分类 Required、Optional、OverridableInitializer。dense/sparse initializer 的名称、类型、尺寸、data-location 与 external-data 路径/offset/length/未知键作为引用报告；**自省**不复制 payload，也不加载外部权重文件。实际 GPU 编译准备需要解析并验证支持模型的权重，不能与纯自省混同。

运行时 `ValueMetadata` 独立报告名字、可选性、initializer 标记与 Tensor/Sequence/Map/Optional/Unknown `ValueKind`。Tensor 类型分类保留 Float16、BFloat16、Float32、Float64、有符号/无符号整数、Bool、String、Complex64/128 与 Unsupported。产物 SHA-256、字节数、格式与 session generation 另行记录。可自省类型不代表可运行该类型。

`ImageInterfaceReport` 只报告 image convenience API 的结构兼容性，不推断任务或色彩语义。typed `ImageTaskContract` 记录精确命名输入/选定输出、NCHW layout、adapter/decoder、preprocessing、resize、color 与声明/具体 shape 约束。模型声明在 frame access 前验证，具体结果在解码/交付前验证。任务要求一个图像输入和一个明确选定输出，不要求模型只声明一个输出；未知多输出模型没有 binding 时拒绝，不取声明顺序第一项。

catalog U²Net-P 的全部七个输出 `1959`–`1965` 保留在 source facts，背景移除契约明确绑定 `1959`；COCO decoder 要求 `[1,84,8400]`，通用结构检查不误拒 `[1,84,N]`。这些契约与历史 reference 数值验证仍有意义，**不代表这些模型已可用于生产 resident 图**。诊断 JSON 分开报告 `model` 与 `taskContract`。

## 准备、调度与资源所有权

应用模型 worker 根据 typed `LoadInference` 意图解析来源、下载/校验缓存并准备 GPU 模型；政策参与资源 key。文件/weights 解析与 initial upload 在准备边界执行，不在每帧热路径读取模型文件。ticket/cancel、项目 revision、node/session generation 保证过期准备结果不能挂回当前节点。项目只保存逻辑模型、任务与政策，不保存 live queue、指针、下载进度或设备地址。

`NativePlayer` 在 render worker 上协调 `PlaybackSession`、`RenderDevice` 与内部 `InferenceTasks`。上游 GPU work 先按同一 queue 提交，再使用输入 texture 执行网络，输出直接进入下游 Shader/Slint。queue order 与 completion fences 约束 producer/consumer 生命周期。request/task id、FrameStamp 与 graph/node/session 身份仍用于拒绝过期交付，完整输出契约和容量校验先于发布；内容时间不能冒充显示时间。

输出最多三槽。`TextureHandle` / `GpuOutputHandle` / `RenderUpdate` 与 Slint 当前 handles 保留输出 lease owner；只要任一消费者持有，output pool 不得复用该槽。三槽全部占用时合并为有界 queued/ready 重试，不提交第四个任务、不增加失败计数、不覆盖既有纹理或事件；释放 lease 后使用最新 source 重试。Pause/Stop/卸载/关闭取消新派发或发布身份，不提前释放尚在 GPU/消费者使用的资源。`completed` 表示 publication-ready，不是物理 GPU fence 完成；submission wall 不等于 GPU 执行时间。

## 媒体与允许的 CPU 边界

实时视频图要求硬件解码与硬件帧导入，没有 FFmpeg RGBA 软件 fallback；不支持的相机、Linux 或无 D3D12 的 Windows 路径明确拒绝。文件图片/framebuffer 的一次性 decode-ingress CPU 解码与 initial upload 是显式准备边界；不是允许逐帧 source pixels 经 CPU。PNG capture、数值 reference 与测试读回必须单独 opt in，不是生产预览步骤。

## Resident 数值与所有权证据

本轮 `resident_compare` 在 **Apple M4 Pro / Metal** 使用 GPU 生成的 8×8 source texture（无 `COPY_DST`），执行完整网络并生成 24×24 输出。与显式 ORT CPU reference 的 byte error **max 0 / mean 0**；host image bytes **0**，六次 dispatch，scratch **26,292,224 B**，weights **238,628 B**。错误 device 在 submit 前拒绝；三槽 pool 饱和产生 backpressure，释放 lease 后可复用。

首个 completion wall **69.494292 ms** 包含诊断 copy 与首次 pipeline compilation，不是 GPU timestamp、稳态吞吐或 UI 端到端性能结论。CPU reference 与输出读回只用于这个显式诊断，不能转成生产步骤或 fallback。

持久可复现命令（需准备 ORT runtime 与 SR3 模型；命令不是新增验收记录）：

```sh
ORT_DYLIB_PATH=... cargo run --release -p oq-inference --features gpu,ort --example resident_compare -- MODEL.onnx REPORT_DIR
```

源接口查看：`cargo run -p oq-inference --example inspect_model -- MODEL.onnx`，无需 ORT。旧 `coreml_compare` 只属于 reference 诊断，不作为 resident 验收入口。Render FPS、完成率、host completion wall 与 GPU 时间必须区分；没有测得的 GPU 时间不伪造为零或以 CPU 计时代替。

## SR3 稳态性能验证（2026-11-09）

在 Apple M4 Pro / Metal、Release 构建上，以同一个原始 SR3 模型和 GPU 生成图案进行比较。每条路径每种尺寸预热 10 次，再分三个交替执行块采样 60 次。CPU 路径为 GPU 读回 → 现有 CPU 前处理/ORT CPU/后处理 → GPU 上传并等待完成；resident 路径等待实际 inference submission 完成。模型加载、首次编译、源生成、验证读回、视频解码和 UI 显示不计时。

| 输入 → 最终输出 | CPU P50 ms | 优化前 GPU P50 ms | 分块 GPU P50 ms | 相对 CPU 加速 | 相对原 GPU 加速 |
|---|---:|---:|---:|---:|---:|
| 224×224 → 672×672 | 13.92 | 44.39 | 4.34 | 3.20× | 10.22× |
| 640×360 → 1920×1080 | 15.66 | 44.53 | 4.42 | 3.54× | 10.07× |
| 1280×720 → 3840×2160 | 22.57 | 44.83 | 4.74 | 4.76× | 9.46× |

优化前数据为本地基线 `e269dea` 的原内核；CPU 与优化后 GPU 在同一次最终运行中测量。全部输出逐字节核对，max byte error 均为 1，resident host image bytes 均为 0。额外稠密有符号权重回归以独立 FP64 参考覆盖全部隐藏通道、非中心 taps、padding、ReLU、尾部 9 通道和每个最终像素。

原始样本、P95、均值、各 CPU 阶段与测量边界：[优化前](../test-results/output-metrics/resident-speed-baseline-2026-11-09.json)、[优化后](../test-results/output-metrics/resident-speed-optimized-2026-11-09.json)。这些是单帧串行 submission-completion wall latency，不是 GPU timestamp、流水吞吐或 UI FPS。外部 Metal timestamp markers 在资格验证中返回陈旧/非单调计数，已移除不可靠测量路径；`gpuExecutionTimeMs` 明确为 null，不用 CPU wall 冒充纯 GPU 执行时间。网络始终 224→672，大图输出不能解释为原生分辨率推理。

复现同一 case（ORT 仅供显式 CPU 对照）：

```sh
ORT_DYLIB_PATH=... cargo run --release -p open_quartz --features native-runtime,native-onnx --example resident_benchmark -- MODEL.onnx REPORT.json
```

优化后回归：workspace 270、完整 native/backend 373（1 ignored）、桌面 79、真实缓存模型及 CPU reference 2 通过；release 稠密数值与资源生命周期 2 通过。实际 Slint 窗口使用不存在的 ORT 路径仍显示优化后 24×24 共享纹理，Play/Pause 保持有效且四项逐帧 host bytes 均 0；证据见 [DESIGN](DESIGN.md#validation)。以下 cutover 基线的 UI/Shader、coverage 和打包结果没有在本次优化中重跑。

## 项目兼容与验收范围

新保存使用 `inference` / `inference*`；旧 `onnx` / `onnx*` 与路径别名保持可读，节点/端口/边 ID、catalog ID 与项目版本保留。格式兼容不等于继续允许运行 CPU 政策。ORT 库名、`ORT_DYLIB_PATH` 与旧屏保资源标记仍表示真实格式，而不是生产执行承诺。

本轮 workspace 270、完整 native/backend 372、桌面 79 通过；真实模型 opt-in 2 通过，另以不存在的 ORT 路径证明生产 graph-worker 无需 ORT。原生窗口实际显示 GPU source 8×8 → SR3 → Renderer 24×24，Play/Pause 保留共享纹理，逐帧 host readback/upload/Tensor bytes 均 0。交互 UI 18/18、Shader 34/34、依赖边界与 macOS portable 打包通过。两项整体 100% coverage gates 仍失败，精确百分比与截图/报告见 [DESIGN 验证基线](DESIGN.md#validation)。Windows/Linux/相机/移动端 resident 能力没有由 Apple Metal 证据覆盖。

相关文档：[推理节点](ONNX_NODE_DESIGN.md)、[模块与所有权](DESIGN.md#module-ownership)、[媒体架构](MEDIA_CODEC_ARCHITECTURE.md)。
