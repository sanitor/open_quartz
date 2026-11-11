# OpenQuartz Video Editor

视频编辑应用设计目录，暂定内部名称 **OpenQuartz Video Editor**。

**当前交付仅为设计，不是可运行的视频编辑器。** 本目录尚无 Cargo package、Slint 窗口或剪辑/导出实现，不添加启动命令，也不改变现有 OpenQuartz 图编辑应用。

- [完整设计、实施阶段与验收标准](DESIGN.md)
- [Phase 0～8 落地与逐阶段提交计划](DESIGN.md#141-phase-提交序列与依赖)
- [硬性要求：按现实 revise 文档，每个实现 Phase 同时交付单元测试与真实 E2E](DESIGN.md#142-每个-phase-的硬性完成与提交协议)
- [固定 Shotcut/Olive 参考、行为对齐与授权边界](DESIGN.md#17-开源参考基线与行为约束)
- [软件栈、模块接口与线程所有权](DESIGN.md#18-明确的软件栈与运行接口)；模块归属见 [§7](DESIGN.md#7-模块分层与复用边界)
- [原 OpenQuartz 渲染/执行分离决策](../../docs/DESIGN.md#execution-rendering-split)：NativePlayer 与编辑器共用核心 FramePipeline，内部 FrameExecutionSession 执行图，不重建 GPU 引擎
- [预取、水位、切点预热与有界跨阶段流水](DESIGN.md#9-预览缓存与-gpu-生命周期)：区分 scrub 意图、连续播放窗口和导出背压，属于首阶段设计要求而非现成能力
- [代码量、人力、日历规模与扩展估算](DESIGN.md#19-代码量与工作规模估算)
- [仓库媒体会话与编解码设计](../../docs/MEDIA_CODEC_ARCHITECTURE.md)
- [现有独立媒体库与实际支持范围](../../crates/oq-media/README.md)
- [现有 Slint 图编辑应用](../open_quartz_slint/README.md)

实施原则：独立时间线工程，复用已验证的媒体/GPU/推理能力；不复制现有应用控制器，不把节点图保存格式当成剪辑工程，不把实时预览帧率当成导出时间基准。

本轮设计采用 Shotcut 约束选定编辑行为、Olive 参考渲染/缓存职责；实现使用 Rust/Slint、wgpu/Metal、libav/VideoToolbox，不引入 Qt/MLT，不复制 GPL 源码。首阶段范围仍是 macOS Apple Silicon 单视频轨、无音轨剪辑与导出；规模估算包含共享媒体库/核心增量、测试与分发工作，不代表 Shotcut 全功能移植或已验证交付日期。
