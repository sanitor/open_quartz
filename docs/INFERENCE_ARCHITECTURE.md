# 推理架构

用户能力为 **INFERENCE / 推理**；ONNX 是模型产物格式，不等于执行后端。生产实时绘制与推理的图像 payload 必须为硬件 buffer/texture，ORT host Tensor 仅供显式 reference/诊断，绝不是生产 fallback。


## Metal catalog 收益与结论（2026-11-09 实测）

Apple M4 Pro / Metal，Release 构建，1280×720 GPU 生成的确定性图案，每条路径预热 2–3 次、采样 30 次，CPU/GPU 按交替执行块串行测量。CPU 为完整 readback → preprocessing → ORT CPU → postprocessing → upload/fence；GPU 为驻留提交到实际完成的 wall latency。源生成、模型准备、诊断读回、视频解码与 UI 显示不计时。

| 模型 | 任务 | CPU P50 ms | GPU completion P50 ms | P50 加速比 | 主要收益 |
|---|---|---:|---:|---:|---|
| U²Net-P | 背景移除 | 84.92 | 9.28 | 9.15× | GPU 生成 alpha mask，省去逐帧 CPU 图像处理及结果上传 |
| MiDaS Small | 深度估计 | 39.03 | 4.12 | 9.48× | 深度推理、极值归约与可视化留在 GPU |
| MODNet | 人像抠图 | 87.04 | 9.79 | 8.89× | GPU 生成前景 alpha，省去 CPU 推理和结果重上传 |
| YOLO26n Semantic | 语义分割 | 16.72 | 2.98 | 5.61× | GPU 生成 class map；640×640 UInt8 class ID 与 CPU 逐值完全一致 |
| YOLOv8n | 目标检测 | 23.31 | 5.55 | 4.20× | 推理、检测后处理与框绘制留在 GPU；当前样本仅覆盖空检测结果 |
| RealESRGAN-x4 | 超分辨率 | 4228.36 | 340.51 | 12.42× | 原分辨率 720p 推理并输出 5120×2880，无逐帧 host 图像往返 |

**结论：** 六个模型在本机本输入下均降低了串行完成延迟。RealESRGAN 的相对和绝对耗时收益最大，但单帧仍需约 341 ms，不能据此宣称可实时处理 720p 视频。其余五个模型的 GPU P50 均低于 10 ms；这不保证尾延迟、流水吞吐或 UI 帧率。收益来自完整驻留路径，不能把加速比解释成单独模型内核的加速比。

**正确性与资源：** FP32 选定输出逐值满足 `abs(gpu-cpu) <= 1e-3 + 1e-3*abs(cpu)`，YOLO26n 的 UInt8 class ID 要求完全相等；最终 RGBA 最大差异不超过 1 byte。驻留路径报告 `hostImageBytes = 0`，显式 CPU reference 与验证读回不属于生产逐帧路径。六模型均验证三份输出 lease 的容量限制、释放后的 slot 复用和 session 销毁后持有输出的生命周期；RealESRGAN 另验证源尺寸切换与恢复。

**适用边界：** 确定性图案不是自然视频；YOLOv8n 此样本检测框为 0，未证明非空框解码正确性。以上数据不等于 GPU timestamp、UI 延迟、跨设备性能或完整生产验收。SR3 的固定尺寸网络与本节六个 catalog 模型分别记录，不能混用其分辨率和性能结论。

原始性能报告：`test-results/output-metrics/catalog-<model>-1280x720.json`，其中报告文件名使用 `midas` 和 `yolo26n` 对应 `midas-small` 和 `yolo26n-sem`。生命周期报告使用同名前缀加 `-lifecycle.json`。这些本地报告位于 git 忽略目录；可通过 `catalog_benchmark` 重现。
### Video → YOLO → Renderer 出图验证

YOLOv8n 的第一个输出是 `detections`（ROI），第二个才是 `overlay`（Sampler2d）。推理预览订阅必须按 sampler 类型选择输出，不能取第一个端口；否则图提交失败，模型加载计划未同步，界面后续误报缺少 Model ID。ROI 输出既可承载显式 host JSON，也可承载驻留检测 buffer 的 Resource handle；它不是可供 Preview 订阅的纹理。`NativePlayer::inference_detection_buffer` 提供 buffer 与 consumer lease，不能用伪造 JSON 或逐帧 CPU 读回代替。

实际操作：Video 选择视频文件 → image 接 YOLOv8n → **overlay** 接 Renderer → 在 YOLO 执行面板点击 **Load model** → Play。用原 1920×1080 VideoToolbox 视频验证了运动图像、非空检测叠框和暂停后画面保留；核心 smoke 接受了 5 次完成，检测 buffer 存在，video CPU-copy bytes 为 0。这是实际出图证据，不是非空检测框与 CPU reference 的逐框数值一致性证明。截图：`test-results/yolo-video-renderer-fixed.png`、`test-results/yolo-video-renderer-fixed-paused.png`。


## 七模型完整 E2E 回归

正式入口：`npm run test:desktop:inference`。本机 Apple M4 Pro / Metal 的完整运行已通过：7/7 原模型经真实生产 worker 执行；37/37 同帧独立参考用例通过（七模型各含原图、镜像图、常量图、两个不同视频帧组，RealESRGAN 另含源尺寸切换与恢复）；原生 UI 预检与 14 个模型×图像/视频场景共 15/15 通过。生产进程设置不存在的 ORT 路径；CPU oracle 在独立进程执行；UI 模型/Renderer 菜单、连接、Load、播放控制、卸载重载、保存重开均真实操作，不直接注入推理结果。输入资源预置通过普通项目加载，不声称覆盖系统文件选择器。

数值验收按明确批准的稳定契约执行，而非把 ORT FP32 当成逐位真值。MODNet 的 16 个 InstanceNormalization 在诊断参考图中使用稳定 FP64 归约/affine 后转回 FP32；本次常量输入最大原始输出误差 `8.9138746e-5`，零容差违反，生产模型未替换。YOLO26 全部 logits 满足原 `1e-3 + 1e-3*abs(cpu)` 容差，双方 UInt8 class ID 都精确等于自身同次执行 logits 的首项 ArgMax；原图 1、镜像图 1、更新视频帧 2 个像素因近乎平票不同，均满足预先声明的误差区间重叠判据。报告原样保留 `parity.raw.passed:false` / `exactIdPassed:false` 和每个坐标、类别、logits、间隔、逻辑与观测误差；稳定语义判定独立为通过，不声称 class ID 完全相等。

报告：`test-results/inference-e2e/production.json`；完整原生窗口报告与截图：`test-results/inference-e2e/ui-2026-11-10T13-47-52-135Z/report.html`。这些是固定 320×192 公有领域样本的本机正确性与显示验收，不是 720p 性能、任意模型精度、模型对现实内容的识别质量或远端 CI 已通过的承诺。模型/fixture 哈希、诊断 feature 与复现步骤见[开发文档](DEVELOPMENT.md#七模型推理-e2e-回归)。
常规回归另通过 core 211、独立 inference backend 37（1 ignored）、桌面 79（2 opt-in ignored）、JavaScript 报告/参考 13 项。独立 `test:desktop:inference:performance` 已完成七模型 Release 串行采样（每路径预热 3、采样 30），结果位于 `test-results/inference-e2e/performance/summary.json`；SR3 含其原 benchmark 的三种源尺寸，其他六模型源为 1280×720。性能任务不设置共享 runner 的严格 FPS 门槛，也不代替上述自然样本 E2E。

## 当前实现与边界

生产 macOS 路径使用与 Slint **wgpu 30 同一个 Metal Device/Queue**。下图为 SR3 专用 profile；其余六个目录模型通过 `CatalogGpuSession` 执行原始 native GPU graph 与对应 GPU 前后处理：

```text
GPU source texture
  → GPU BT.601 / resize 224×224
  → 完整四层 FP32 convolution + ReLU
  → PixelShuffle ×3 + RGB reconstruction
  → resident RGBA texture
  → Slint preview / downstream Shader
```

`oq-inference` 的独立 `gpu` feature 提供实际 GPU 模型编译器、SR3 的 `GpuImageSession` 和六模型的 `CatalogGpuSession`，不依赖 OpenQuartz 图或 Slint。编译器验证原始 ONNX topology、weights 与 reshape layout，运行完整网络，不以近似滤镜替代。正常 `GpuImageSession` scratch 为 STORAGE-only；显式 `gpu-diagnostics` 测试构建才允许复制原始输入和有效输出区。模型文件解析、weights 初始上传与 scalar controls 属于准备边界。逐帧 source pixels 不进入 CPU，不创建 host image Tensor，不做 CPU 前后处理、读回或结果重上传。

卷积使用按已验证层几何专门编译的 FP32 内核：一个 64-thread workgroup 覆盖 32 个输出通道 × 32 个邻接像素，每线程累加 4×4 register tile；归约以 K=32 的输入/权重共享分块执行，workgroup storage 为 8KB。输出仍是 NCHW；9-channel 尾部、padding、bias 和 ReLU 保持原模型语义。专门编译和权重上传只发生在准备阶段，不增加逐帧 CPU 图像处理。

Metal 后端可执行七个目录 profile：SR3、RealESRGAN-x4、U²Net-P、MODNet、MiDaS-small、YOLOv8n、YOLO26n-sem。SR3 网络固定 `[1,1,224,224] → [1,1,672,672]`，任意源尺寸仍缩到 224×224 并重建源宽高各 ×3，不是 tiling 或原始大图全分辨率推理；RealESRGAN 按源分辨率执行并输出 ×4。其他平台与未知模型拒绝，不退回 ORT。执行支持与完整回归是否通过分开报告：入口为 `npm run test:desktop:inference`，详见[开发文档](DEVELOPMENT.md#七模型推理-e2e-回归)。
此前 `catalog_benchmark` 的 Apple M4 Pro 720p 记录覆盖六模型原始选定输出与最终 RGBA，但绕过桌面模型加载和图输出契约；它不是完整 E2E。历史梯度样本最大字节差异 1，YOLOv8n 检测框为 0；不能由该结果外推自然图像、非空检测、其他硬件或完整生产验收。
六个 catalog 模型各自另执行了不计时的 Metal 生命周期检查：同时持有三份 consumer output lease 时拒绝第四次提交；释放一个 lease 后复用对应 slot；销毁 session 后等待中仍可读到持有中的输出纹理。RealESRGAN 另验证源分辨率从 1280×720 切到 640×360、再恢复 1280×720 后，输出尺寸分别为 2560×1440 与 5120×2880。每个模型额外执行 3 次 CPU 与 3 次 GPU completion 样本；含 lifecycle 字段的专用记录为 `test-results/output-metrics/catalog-<model>-1280x720-lifecycle.json`，不作为性能统计。SR3 的独立真实 Metal pool/resource/device 验收及多尺寸性能记录见下方历史条目。

上述诊断将 GPU completion-wall 与完整 ORT CPU readback/preprocess/infer/postprocess/upload 分开测量，不是 UI 延迟或 GPU timestamp。`realesrgan-x4` 的 720p 源输入输出 `[1,3,2880,5120]`，CPU P50 4228.36 ms、GPU completion P50 340.51 ms（12.42×）；历史梯度样本的 44,236,800 个 FP32 值满足容差，58,982,400 个 RGBA 字节最大差异 1。其他原始报告为 `test-results/output-metrics/catalog-<model>-1280x720.json`，与完整 E2E 的 `test-results/inference-e2e/production.json` 分开保存。

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

`NativePlayer` 在原 render worker 上拥有实时 CompositionClock 和共享 `execution::FramePipeline`。pipeline 持有 `RenderDevice`、`VideoSources`、`InferenceTasks`、稳定帧池及 `FrameExecutionSession`；session 本身只持有 GraphInstance/OutputHub 与求值 stamp，不持有推理 session 或设备资源。PlaybackSession 为同一无时钟图/输出 owner 加 clock，不另维护执行状态。模型准备仍在应用 worker，真实网络仍由独立 oq-inference 执行，不新增 crate/设备。

Phase 2 demand 携带 request ID、graph revision、显式 FrameEvaluationInput/FrameStamp 与可选版本化 ExternalFrameInput/StableFrame。一个已接收请求或未取走终态即占 admission；所属 worker 调用 `progress`，`poll_completion` 只取结果。上游 GPU work 先按同一 queue 提交，遇到推理命令便在**同一 demand 内**等待该输入 stamp 的有效完成，再执行下游 Shader/Renderer，而不是下一次 clock tick 使用旧推理纹理。request/task id、FrameStamp 与 graph/node/session 身份用于拒绝过期交付；完整契约和容量校验先于纹理注册/发布。内部推理通知只消费自己的订阅交付，不排空外部 OutputHub 订阅。queue order/fences 约束 producer/consumer 生命周期，内容时间不能冒充显示时间。

推理输出最多三槽。`TextureHandle` / `GpuOutputHandle` / `RenderUpdate` 与 Slint handles 保留 output lease；任一消费者持有时不得复用。饱和时有界等待/重试，不提交第四个任务，不覆盖旧纹理/事件，也不把容量等待计为推理失败；当前 pipeline 保留同一 demand，不用后续帧替换它的依赖。全部依赖满足后，目标图输出再复制为稳定帧；Submitted(StableFrame) 表示图与 copy 已提交，物理 GPU 完成由 `frame.ready` 单独表示，消费者释放决定池复用。推理 metrics 的 `completed` 仍是 publication-ready，不是物理 fence 完成；submission wall 不是 GPU 时间。Pause/Stop/卸载/关闭取消派发/发布身份，不提前释放 GPU/消费者仍使用的资源。精确媒体时间选择、预取、多帧窗口和跨阶段 credit 均属后续 Phase，当前媒体导入仍阻塞。

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
