# OpenQuartz Studio — 视频编辑器

独立 Rust + Slint 应用，和原节点图应用 `apps/open_quartz_slint` 并列，提供素材库、硬件预览、多轨时间线及片段级 Nodes 渲染管线。**Nodes复用OpenQuartz节点编辑器交互与GPU图执行，工作区流程参考DaVinci Resolve Color页**；不声称完整Resolve功能对等，不包含其专有代码、品牌和资源。


## 启动

从仓库根目录：

```sh
npm ci
npm run dev:video-editor
# 打开已保存工程
npm run dev:video-editor -- --project /absolute/path/edit.oqvideo.json
```

命令会准备固定的 LGPL-only FFmpeg 8.1.1 `oq-vt-hw3-metal-encode-r2-audio1` SDK 和现有 FFmpeg helper，再启动原生应用。当前只验收 macOS / Metal / VideoToolbox；设备或素材不满足硬件路径时明确失败，不读取 CPU 图像做替代显示。

`dev:editor` 仍是代码编辑器组件示例；`dev:video-editor` 才是本应用。

播放性能对比建议使用优化构建；Debug中的Rust宿主代码未做完整优化（依赖已配置`opt-level=2`），不代表GPU shader以Debug精度运行：

```sh
cargo run --release --manifest-path apps/open_quartz_video_editor/Cargo.toml -- --project /absolute/path/edit.oqvideo.json
```

**有界播放流水**。Play把工程snapshot交给preview worker；worker拥有唯一视频时钟，使用核心前瞻窗口、prefill、按PTS映射的序列deadline呈现和切点/gap预热。UI不再每tick提交帧，正常时钟推进不取消合法在途工作；seek/改图仍撤销旧发布资格。8K全尺寸图有显式2 GiB应用可观测共享预算和局部池上限，不能满足最小工作集时明确失败，不暗中降分辨率或改FP16算法。实现与验收见[有界流水播放](DESIGN.md#bounded-playback-proposal)。

缓冲按[异步阶段与图内资源生命周期](DESIGN.md#stage-queues-resource-lifetimes)组织。[多轨同帧汇合](DESIGN.md#same-frame-track-join)由同一帧需求协调所有活动视频源。带声音的序列播放由音频设备驱动画面时间。

窗口使用无边框模式，将窗口控件并入最上方业务顶栏，支持顶栏拖动和边缘缩放。与节点编辑器共用自绘按钮：macOS 默认左侧关闭/最小化/最大化，其他平台默认右侧最小化/最大化/关闭；视频硬件运行路径的平台限制不变。关闭按钮仍走未保存工程确认。

沿用同一位置配置，默认 `platform`，也可强制指定：

```sh
OPEN_QUARTZ_WINDOW_CONTROLS=right npm run dev:video-editor
# 可选值：left / right / platform
```

## Timeline Editing

- **IMPORT**：导入本地 H.264 / HEVC、8-bit SDR 视频，以及带明确时长的 AAC 或 PCM 音频。素材库显示支持的音频编码、采样率和声道数。视频文件必须可精确定位且有可信 stream origin/duration。
- **ADD CLIP** / 素材旁的 **+**：将选定素材放入目标轨道的可用空隙。普通移动保留其他片段的时间。同轨重叠时拒绝放置，跨轨重叠按轨道顺序合成。
- **Insert** 在播放头插入选中的视频素材，切开跨过播放头的片段并将本轨后续内容后移。**Overwrite** 用素材替换本轨对应区间，两侧剩余内容和其他切点保留。
- **Ripple delete** 删除选中的视频片段，并将本轨后续片段前移相同帧数。相邻视频片段可在前一个片段的 Inspector 中用 **Roll cut (frame)** 调整共同切点。两端外边界保持原位。上述命令联动链接音频，各自占一个撤销步骤。
- 时间线上拖动片段移动；拖两侧边缘修剪。**一次松手只有一个历史条目；Esc 取消拖动草稿**。
- 点击标尺定位，**PREVIOUS / NEXT** 逐序列帧步进，**SPLIT** 在播放头处分割，**DELETE** 为 lift 删除，保留空隙。空隙真实渲染不透明黑色。
- 右侧 **Timeline start / Timeline end** 用序列帧；**Source in / Source out** 用原始 stream ticks。结束点为半开边界；输入后按 Enter 提交。分割和修剪保留有理数源入点，不逐次截断到整数源帧。
- **UNDO / REDO**；**SEQUENCE** 设置固定画布和有理数帧率（如 30000/1001）。改名在顶部名称输入框中按 Enter。
- **SAVE / SAVE AS / OPEN**：独立 `.oqvideo.json` 格式，原子文件替换、相对素材路径、完整 SHA-256 指纹、未来 schema/未知字段拒绝。撤销栈不写入工程。
- 选中视频片段并把播放头移到片段内，即可在 Inspector 的 **Opacity (%)** 下用 **Add opacity keyframe** 添加透明度关键帧。曲线启用后，修改透明度会在当前帧新增或更新关键帧。**Remove opacity keyframe** 删除当前关键帧。相邻关键帧之间线性插值，曲线两端保持端点值。

## Nodes Workspace — Phase 7

Select **Nodes** with an active video clip to edit its independent static pipeline. The workspace reuses OpenQuartz's graph canvas, Inspector, WGSL editor and GPU executor. Project saving includes each clip pipeline inside its track. Opening schema 1 migrates each clip to a direct source→output pipeline.

- Source/Output nodes are protected; add Color/Filter/Distortion/Blend image effects from the catalog, add a custom Shader, connect compatible ports and remove serial effects. Copy/paste reuses the original graph semantics.
- Graph edits preflight on the shared GPU worker before one project-history commit. Shader syntax errors retain the editable draft and last valid frame; uniforms commit with Enter. Per-clip graphs stay isolated across clip switching and splitting.
- The workspace follows a panel layout: **Media / Nodes** tabs share the left library; the Nodes tab shows grouped, two-column square effect tiles; node settings use the same right property sidebar as clip timing. The clip-instance strip stays above the graph, separate from the full timeline.
- Current support: static single-image Shader, Constant, Math and ColorCorrector graphs. Inference, arbitrary resource nodes, arbitrary multi-source effects, time/feedback nodes, Gallery/versions, scopes and professional color management are not implemented.



### 界面与时间轴缩放

- 顶栏窗口控件、Q标志、软件名和工程名称输入框统一垂直居中。软件名保留大写18px；固定业务文案统一句首大写，常规文字12px、辅助信息11px；素材名和用户工程名不强制改大小写。
- 工具栏采用32px图标按钮，18px矢量图形；悬停显示操作名称tooltip，保留原有无障碍按钮名称。预览下三个传输按钮相对画面居中，时间码独立靠右。
- 时间轴支持 `0.0001–48 px/frame`，低于4px/frame继续按比例缩小，不再停在4px/frame。点击 **Fit entire sequence** 图标可将整个序列放入当前视口；缩放不改变项目帧范围，拖动/裁剪与标尺定位均使用真实小数缩放换算。

Nodes architecture and truthful implementation limits: [RENDER_PIPELINE_EDITOR_DESIGN.md](RENDER_PIPELINE_EDITOR_DESIGN.md).


文件对话框支持粘贴完整本地路径，也可点击 **Browse…** 使用系统文件选择器。macOS的OpenPanel是嵌套主线程事件循环；编辑器在进入选择器前先释放controller可变借用，以便Slint计时器/native回调重入，取消也返回原路径对话框。

快捷键：Space 播放/暂停；←/→ 步进；⌘/Ctrl+B 分割；Delete/Backspace lift；⌘/Ctrl+Z 撤销；⌘/Ctrl+Shift+Z 重做；⌘/Ctrl+S 保存；⌘/Ctrl+Shift+S 另存为；⌘/Ctrl+O 打开；⌘/Ctrl+I 导入；Option+Shift+↑/↓ 重排选中的轨道；Esc 取消。

## 实际渲染与边界

预览使用现有 `execution::FramePipeline`，不是复制原图应用的渲染循环。`PreviewBridge` 只负责有界最终意图、request/revision 身份和 UI/worker 交接；核心流水负责源选择、图执行、稳定帧和设备完成。Slint 与视频图共享同一个 wgpu 30 Device/Queue；普通预览与 GPU 缩略图不回传 CPU 像素，读回仅用于测试 oracle。

源选择使用目标处或之后第一张实际 presentation frame。播放到源片尾时，允许使用有明确帧时长且覆盖目标的末帧。监视器显示各活动视频源的实际 PTS。序列用有理数 CFR 帧位置，图层 contain-fit 后在同一画布内合成。缩略图至多保留六张不可变 GPU 图像，并计入共享可观测预算；VideoToolbox/driver 私有内存不冒报硬上限。

当前支持多视频轨有界播放/暂停、精确定位、切点预热和每片段独立效果。Edit 页的 **Add Track** 添加轨道，素材可直接拖入轨道，拖片段跨轨和拖轨头重排都进入工程撤销历史。监视器的 **Transform** 提供位置、缩放和旋转手柄，Inspector 同步显示变换并支持透明度。旧工程打开时迁移到 Schema 5，已有片段维持声音状态和静态透明度。[多视频轨合成设计](DESIGN.md#multi-track-composition)

**音频**：添加有声视频时，视频片段和可解码的音频默认链接。编辑视频时间、分割或删除会同步修改链接音频。音频素材用 **ADD CLIP** 放到播放头。音频轨提供静音、增益和锁定，音频片段 Inspector 提供增益、解除链接、独立时间与源入点编辑。播放用 macOS 音频设备时钟，底部显示输出电平。

**EXPORT** 将整个序列导出为 8-bit SDR H.264/MP4，沿用工程的画布尺寸和帧率。含音频片段的序列会写入 48 kHz 双声道 AAC。导出使用启动时的工程快照，窗口底部显示进度，工具栏可取消。完成时原子提交到新的本地 `.mp4` 路径。转场、文本、代理和音频波形属于后续交付。

## 验证

从仓库根目录执行：

```sh
npm run test:video-editor
npm run test:video-editor:ui:smoke
npm run test:video-editor:ui
npm run test:video-editor:coverage
```

`npm run test:video-editor:ui:smoke` executes the shared Edit and Nodes cases marked as core. The full UI and coverage commands execute every case in both suites.

`npm run test:video-editor:ui` runs timeline and Nodes native-window suites. The Nodes suite verifies decoded GPU effect pixels, port reconnect, shader/uniform edits, shared undo/redo, per-clip isolation and save/reopen. The Rust behavior suite includes worker clock/prefill, exact seek supersession, gap/cut final-frame retirement, hysteresis and full-extent budget arithmetic. This change did not run the coverage gate; historical coverage results are not a new pass claim.

Timeline-window acceptance includes 30-second media plus, 900-frame clips at default/maximum zoom, fit-to-sequence, fractional-scale trims, undo and persistence. Nodes-window acceptance uses actual red/blue H.264 fixtures and checks final monitor pixels; graph callback forwarding alone is not treated as success.

原生播放测量：`node scripts/measure-video-editor-playback.mjs <ui-automation-binary> <project.oqvideo.json> <output-dir> [seconds]`。实际8K素材/完整质量/单与双Corrector测量、预算与计数解释见[8K实测记录](DESIGN.md#eight-k-playback-evidence)；原始数据在`test-results/window-playback/`。不把viewport尺寸、时钟推进或worker提交当真实图像交付。

报告和复查文件位于`test-results/phase6/ui-*/`及`test-results/clip-pipeline/nodes-ui-*/`：HTML/JSON报告、截图、工程、生成素材和关闭日志。Windows/Linux尚未完成原生硬件窗口验收；原仓库SDK 100% gate保持原阈值和排除规则。
