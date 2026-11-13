# OpenQuartz Studio — 视频编辑器

独立 Rust + Slint 应用，和原节点图应用 `apps/open_quartz_slint` 并列。界面参考剪映桌面剪辑工作流：深色素材库、中央预览、右侧片段属性、底部时间线；使用 OpenQuartz 自有品牌，不包含剪映图标、模板或专有素材。

## 启动

从仓库根目录：

```sh
npm ci
npm run dev:video-editor
# 打开已保存工程
npm run dev:video-editor -- --project /absolute/path/edit.oqvideo.json
```

命令会准备固定的 LGPL-only FFmpeg 8.1.1 `oq-vt-hw3-metal-encode-r2` SDK 和现有 FFmpeg helper，再启动原生应用。当前只验收 macOS / Metal / VideoToolbox；设备或素材不满足硬件路径时明确失败，不读取 CPU 图像做替代显示。

`dev:editor` 仍是代码编辑器组件示例；`dev:video-editor` 才是本应用。

## Phase 6 可操作范围

- **IMPORT**：导入本地 H.264 / HEVC、8-bit SDR 视频。文件必须可精确定位且有可信 stream origin/duration。显示实际尺寸、平均帧率、在线状态和有界 GPU 缩略图；有音轨的素材明确提示仅使用视频。
- **ADD CLIP** / 素材旁的 **+**：将选定素材放入单视频轨的可用空隙。普通移动不 ripple；片段重叠被拒绝。
- 时间线上拖动片段移动；拖两侧边缘修剪。**一次松手只有一个历史条目；Esc 取消拖动草稿**。
- 点击标尺定位，**PREVIOUS / NEXT** 逐序列帧步进，**SPLIT** 在播放头处分割，**DELETE** 为 lift 删除，保留空隙。空隙真实渲染不透明黑色。
- 右侧 **Timeline start / Timeline end** 用序列帧；**Source in / Source out** 用原始 stream ticks。结束点为半开边界；输入后按 Enter 提交。分割和修剪保留有理数源入点，不逐次截断到整数源帧。
- **UNDO / REDO**；**SEQUENCE** 设置固定画布和有理数帧率（如 30000/1001）。改名在顶部名称输入框中按 Enter。
- **SAVE / SAVE AS / OPEN**：独立 `.oqvideo.json` 格式，原子文件替换、相对素材路径、完整 SHA-256 指纹、未来 schema/未知字段拒绝。撤销栈不写入工程。
- **RELINK**：恢复丢失素材，必须与保存的内容指纹匹配。同路径换文件会标为 offline，不静默使用新内容。

文件对话框支持粘贴完整本地路径，也可点击 **Browse…** 使用系统文件选择器。无测试专用导入接口；窗口自动化操作同一套实际控件。

快捷键：Space 播放/暂停；←/→ 步进；⌘/Ctrl+B 分割；Delete/Backspace lift；⌘/Ctrl+Z 撤销；⌘/Ctrl+Shift+Z 重做；⌘/Ctrl+S 保存；⌘/Ctrl+Shift+S 另存为；⌘/Ctrl+O 打开；⌘/Ctrl+I 导入；Esc 取消。

## 实际渲染与边界

预览使用现有 `execution::FramePipeline`，不是复制原图应用的渲染循环。`PreviewBridge` 只负责有界最终意图、request/revision 身份和 UI/worker 交接；核心流水负责源选择、图执行、稳定帧和设备完成。Slint 与视频图共享同一个 wgpu 30 Device/Queue；普通预览与 GPU 缩略图不回传 CPU 像素，读回仅用于测试 oracle。

源选择沿用已交付 Phase 3 的 `frame_at_ns` 契约：目标之后第一张实际 presentation frame；状态显示真实 source PTS，不以平均帧率猜 PTS。序列自身始终用有理数 CFR 帧位置。固定画布采用 contain-fit / 黑边。缩略图至多保留六张不可变 GPU 图像，并计入共享可观测预算；VideoToolbox/driver 私有内存不冒报硬上限。

当前提供基础播放/暂停与逐帧定位，不宣称已完成 Phase 7 的跨切点预热、吸附、完整变换/颜色效果、代理或性能保证。**本应用没有视频导出 UI、音频播放/剪辑、文本、转场或多轨**；核心 Phase 5 headless H.264/MP4 能力仍独立可用，编辑器导出和分发闭环属于 Phase 8。没有伪装可用的空导出按钮。

## 验证

从仓库根目录执行：

```sh
npm run test:video-editor
npm run test:video-editor:ui
npm run test:video-editor:coverage
```

最后一条会在同一次插桩下运行Rust行为测试和真实Slint窗口13项，再合并LLVM profile到`test-results/phase6/editor-coverage.json`；CI也执行该命令。未使用UI回显mock或只测forwarding的占位覆盖。

本轮窗口通过真实控件导入两段视频，设置序列，放置/移动/左右trim/split/lift/undo/redo，验证重叠拒绝、空隙像素和Escape取消，实际播放/暂停，再保存、正常退出、重开、错误relink拒绝及匹配指纹relink恢复。独立PNG oracle检查实际预览中心红/蓝/黑像素；保存后独立解析工程核对帧范围及stream PTS，不以控件文本代替结果。

报告和复查文件位于`test-results/phase6/ui-*/`：`report.html`、`result.json`、截图、工程、生成素材及关闭日志。平台限制与详细coverage数值见[Phase6设计记录](DESIGN.md#144-当前实施记录与后续范围)。原仓库SDK 100% gate保持原阈值和排除规则；不满足时不发布。
