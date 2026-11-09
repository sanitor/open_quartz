# 推理节点设计

用户看到的是 **INFERENCE / 推理** 能力与具体任务；ONNX 是模型格式。生产执行为同 Slint wgpu 30 Device/Queue 的 Metal GPU resident 路径；ORT host Tensor 仅为显式 reference/诊断，不是 fallback。本文件保留原路径以兼容既有文档链接。

## 创建与配置

菜单按检测、超分辨率、背景移除、深度等任务分类，模型名称表达具体能力；自定义入口为 **Custom Model**。节点类型为 `Inference`，不再以执行库命名。

节点配置包含：

- 模型来源：catalog 或自定义文件；catalog/model ID 保持稳定。
- 任务：控制预处理、布局、阈值和输出解释，不从文件名推断。
- 输入/输出端口：保留图中的稳定 ID 和类型约束。
- 目标尺寸、score/IoU 等任务参数。
- 执行政策：UI 仅 Metal GPU / Auto Hardware，强制 RequireResident、AcceleratorOnly、Forbid CPU；见[推理架构](INFERENCE_ARCHITECTURE.md)。

当前自定义模型选择器仍接受 `.onnx`，这是实际支持的产物格式。模型元数据自省不证明其满足图像任务语义。图像适配器要求一个图像输入和一个任务选定的命名 FP32 输出，不要求模型只声明一个输出；未声明任务绑定的任意多输出模型明确拒绝，绝不静默取第一个 Tensor。独立 `oq-inference` 命名张量 API 另支持多输入/输出。

### 模型自省与任务契约

模型事实从两个来源分别报告：不依赖 ORT 的 `inspect_onnx(model)` / session `source_interface` 保留原始 ONNX 声明（IR/producer/model versions、domain、graph name、模型/图/值 doc-string 与 custom properties、graph opset imports）；运行时另报告自身 metadata，不把 runtime model-version MAX sentinel 当作源版本，producer_version 由源信息补齐。opset 直接从源文件读取，不再因 ORT API 21 限制而一律标记不可用。

源接口包含递归 Tensor/SparseTensor/Sequence/Map/Optional/Opaque/Unknown、精确 dtype 数字代码、UnknownRank 与 scalar 的区别、静态/符号/dynamic 维度及 denotation。Required/Optional/OverridableInitializer 输入明确分类；dense/sparse initializer 和 external-data 引用只描述、不复制/加载权重。运行时 `ValueKind`、完整 Tensor 元素分类、SHA-256/字节数/格式与原始 source facts 保持分离。自省成功不表示该类型受 FP32 执行接口支持。

`ImageInterfaceReport` 是通用 image convenience API 的结构报告，不代替任务绑定。typed `ImageTaskContract` 包含精确命名输入/选定输出、layout、adapter/decoder、preprocessing/resize 与声明/实际 shape constraints；声明在 frame access 前验证，实际输出在解码/交付前验证。任务拥有颜色与输出语义，不按长度猜 decoder。COCO decoder 要求 `[1,84,8400]`，通用检查不误拒 `[1,84,N]`。U²Net-P 全部 `1959`–`1965` 与显式 mask binding `1959` 保留；历史 ORT reference 的 409600 字节 RGBA parity 不是 resident 支持证据。JSON 分别报告 `model` 与 `taskContract`；`.scr` 推理当前不可用。

生产仅支持 SR3 的完整四层 FP32 Conv/ReLU 与 PixelShuffle ×3，GPU BT.601/resize 224×224 后执行固定 224→672 profile，再重建源宽高各 ×3 的 RGBA texture。任意源尺寸不表示 window tiling 或大图全分辨率质量。其他 catalog/custom 模型保留自省与契约，但在 frame access 前拒绝 resident 执行，不退回 ORT。

源接口查看：`cargo run -p oq-inference --example inspect_model -- MODEL.onnx`。resident 数值诊断命令与 Apple M4 Pro 的完整网络/reference、零 host image bytes、六 dispatch 和 lease/backpressure 证据见[推理架构](INFERENCE_ARCHITECTURE.md#resident-数值与所有权证据)。CPU reference 仅用于显式测试。

## Inspector 与状态

Inspector 区分项目配置、已保存的元数据和当前 live session：

| 内容 | 含义 |
|---|---|
| Model / Task | 逻辑模型与处理任务 |
| Source / Path | catalog 或自定义产物来源 |
| Backend / Fallback | 执行政策，不表示该模型已加载 |
| Live execution | 实际 MetalGpu / Metal、GPU placement、Device path 与 resident 政策；不支持时明确失败 |
| Input / Output | 已加载模型的接口信息 |
| Result / Error | 当前有效结果或明确失败原因 |

加载、下载、取消、卸载和失败是 live 资源状态，不写进 undo/redo 的逻辑模型事实。加载成功后仍需等待真实推理结果；项目中保存的 ready/backend 字样不能让节点假装已经运行。

政策改变参与模型资源身份，触发新准备请求。旧下载、session 构建或推理完成不能重新挂到新项目或同 NodeId 的替代模型。加载取消不等于底层调用立刻结束。

## 图执行

输入纹理就绪后，图适配器先提交上游 GPU work，再直接用同 Device/Queue 的 texture 执行 GPU 前处理、完整网络与 GPU 后处理。source pixels 不经 CPU，没有逐帧 host Tensor、读回或结果重上传。输出按任务/session/图/node 身份与输出契约校验后发布，下游标为 dirty；卸载/替换后旧结果不得复活。

`TextureHandle`、`GpuOutputHandle`、`RenderUpdate` 与 Slint 当前 handles 保留输出 lease，消费者持有期间不得复用 output pool 槽。最多三槽，饱和显式 backpressure；queue order/fences 保证使用顺序。PNG capture 与 reference 测试的 opt-in readback 不属于生产预览。

Render FPS 不等于推理完成率，重复显示同一结果不产生新的推理完成。计量与后端信息以真实执行结果为准。

## 项目与屏保兼容

新保存使用 `type: "inference"` 和 `inference*` 字段。读取接受旧 `type: "onnx"`、`onnx*` 与旧自定义路径字段；节点/端口/边 ID、模型 ID 和项目版本保持。

显式配置和旧 `preferDirectMl`/CPU 政策可作为项目事实解析，但不能放宽生产硬件约束；不支持的请求在 frame access 前报错，而非继续 CPU 兼容执行。

Windows `.scr` 推理导出和 stub 推理明确不可用（无合格 resident backend，CPU forbidden）。v3 包及旧 `onnx-model` 标记仍可读，不代表允许 readback → CPU → upload；非推理 Shader/硬件视频导出保留。

## 验收关注点

- canonical 与 legacy 项目往返，不丢模型配置、任务、端口或连线。
- 菜单和 Inspector 不再把 ONNX 作为用户能力分类。
- 严格驻留/位置政策在不支持时明确拒绝，不能先读回再假装满足。
- 多输入/输出、错误名称、维度和类型不静默截断或猜测。
- 模型/项目替换、取消、卸载、停止与迟到完成不发布旧纹理或事件。
- SR3 resident 完整网络与显式 ORT reference 数值比较；三槽池、错误 device、released lease 复用和饱和后非致命重试均已验证。本轮完整集成实测计数与未通过 coverage gates 见 [DESIGN](DESIGN.md#validation)，不复用旧 ORT/CoreML 阶段计数。

实现入口为 [`inference_runtime.rs`](../apps/open_quartz_slint/src/inference_runtime.rs)、[`inspector_inference.rs`](../apps/open_quartz_slint/src/inspector_inference.rs)、核心 [`inference/`](../crates/open_quartz/src/inference/) 和独立 [`oq-inference`](../crates/oq-inference/)。
