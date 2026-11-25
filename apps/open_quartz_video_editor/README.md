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

Windows x64 可运行 `dev:video-editor`、`build:video-editor` 和 `test:video-editor`。`npm run test:video-editor:ui:stills` 验证静态图片导入、片段节点效果和保存后重新打开工程。视频导入与精确寻帧、音频、代理媒体和 MP4 导出仍依赖 macOS 媒体后端。

`npm run package:video-editor` 可将当前 Windows x64 版本打包为 `dist-desktop/OpenQuartz-Studio-*-windows-x64.zip`。解压整个 ZIP 后运行 `open_quartz_video_editor.exe`，运行库随包提供。

macOS 启动命令会准备固定的 LGPL-only FFmpeg 8.1.1 `oq-vt-hw3-metal-encode-r2-audio1-studio2` SDK 和开发测试使用的 FFmpeg helper。硬件视频路径在 Metal 和 VideoToolbox 上验收。设备或素材不满足硬件要求时会报告错误。

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

- **IMPORT**：导入本地 H.264 / HEVC、SDR 视频及标注 BT.2020 PQ 或 HLG 的 HEVC Main10 视频（SDR 显示预览），带明确时长的 AAC 或 PCM 音频，以及 SDR PNG / JPEG 图片。素材库显示支持的音频编码、采样率和声道数。视频文件必须可精确定位且有可信 stream origin/duration。
- **Import files** 支持在系统选择器里多选，也可逐行粘贴完整路径。成功导入的素材共用一个撤销步骤，重复项与失败项计入结果。素材离线时，**Relink folder** 在所选目录及子目录中按内容指纹查找原片，匹配结果一次应用。
- PNG 和 JPEG 可在素材库预览，放入时间线时默认持续三秒。可修剪、分割和变换，也可参与叠化。PNG 保留透明度，JPEG 按 EXIF 方向显示。预览和 MP4 导出使用同一套合成规则。
- **ADD CLIP** / 素材旁的 **+**：将选定素材放入目标轨道的可用空隙。普通移动保留其他片段的时间。同轨重叠时拒绝放置，跨轨重叠按轨道顺序合成。
- **Insert** 在播放头插入选中的视频或图片素材，切开跨过播放头的片段并将本轨后续内容后移。**Overwrite** 用素材替换本轨对应区间，两侧剩余内容和其他切点保留。
- 点击视频素材卡会打开源监视器。**Previous source frame**、**Next source frame** 和帧输入框按序列帧率查看源时间，监视器会显示实际取到的源 PTS。**Mark In / Mark Out** 选择源区间，Out 包含当前看到的帧。**Clear marks** 恢复完整素材。**Insert / Overwrite** 使用该区间，视频放到当前选中轨，有声素材可选择自动分轨、仅视频或指定已有音频轨。标记和目标选择保留在当前编辑会话中。
- **Ripple delete** 删除选中的视频片段，并将本轨后续片段前移相同帧数。相邻视频片段可在前一个片段的 Inspector 中用 **Roll cut (frame)** 调整共同切点。两端外边界保持原位。上述命令联动链接音频，各自占一个撤销步骤。
- 轨头的 **Sync** 决定该轨是否加入其他视频轨触发的波纹编辑，默认关闭并随项目保存。插入会在参与轨道上腾出同样长度的时间，波纹删除会清除同一范围并前移余下内容。跨越边界的片段自动分割。链接音频跟随原视频，启用 Sync 的音频轨会同步移动独立音频。活动轨及受影响的同步轨必须可编辑，整次操作可一次撤销。
- 选中同轨相邻片段中的前一个，在 Inspector 的 **Dissolve (frames)** 输入叠化帧数。叠化跨过切点，时间线上会标出实际范围。视频一侧需要留有源素材余量，图片可在过渡区持续显示。输入 **0** 可移除。视频叠化期间音频保持原切点。
- **Add title** 在播放头创建默认三秒的标题，并放在新的顶层 **Titles** 轨道。选中片段后可在 Inspector 中修改 **Title text** 和 **Title size (%)**。字号按画布高度计算，文字居中并自动换行。标题可用现有的时间线剪辑和 **Transform** 控件调整，并在预览和 MP4 导出中按轨道顺序透明叠加。
- **Add subtitle** 在播放头创建字幕，使用独立的 **Subtitles** 轨道。字幕首选时长为两秒，遇到下一段字幕会在其起点结束。在 Inspector 编辑多行文字后点击 **Apply subtitle text**，并用现有时间线控件调整字幕范围。**Import SRT** 按文件时间戳导入 UTF-8 纯文本 SRT，整次导入可撤销。字幕在画面底部显示白字和深色底板，并烧录进 MP4。**Export SRT** 可另存 UTF-8 字幕文件，选区中的字幕会裁切并从输出零点计时。
- **Speed (%)** 在 Inspector 中设置视频片段的恒定正向速度，范围为 25%–400%。片段起点保持原位，片段长度和链接音频随后调整。变速后的音高随速度变化。减速扩展遇到同轨冲突或锁定的链接音频时，整次编辑被拒绝。保存、预览和导出沿用同一源时间映射。
- 时间线上拖动片段移动；拖两侧边缘修剪。**一次松手只有一个历史条目；Esc 取消拖动草稿**。
- 在 Edit 页按住 Shift 点击片段可多选。水平拖动已选片段会同步移动整个选择，**DELETE** 一次删除这些片段。每个片段保留原轨道。标尺左侧 **SLIP** 拖动片段可调整源入点，**SLIDE** 可等量移动前后两个切点，保持中间片段时长。再次点击当前工具恢复移动。
- 时间线磁铁默认开启。移动单个视频片段、成组移动或拖动视频片段边缘时，接近播放头、命名标记或相关轨道边界会显示黄色吸附线。点击工具栏磁铁可以关闭或重新启用。成组移动时，组内任一片段的边缘都可作为吸附锚点。
- 用 **Add marker** 在播放头加标记，点击标尺上的标记可跳转。播放头处可改名或删除标记，标记随工程保存和撤销。
- 点击标尺定位，**PREVIOUS / NEXT** 逐序列帧步进，**SPLIT** 在播放头处分割，**DELETE** 为 lift 删除，保留空隙。空隙真实渲染不透明黑色。
- 右侧 **Timeline start / Timeline end** 用序列帧；**Source in / Source out** 用原始 stream ticks。结束点为半开边界；输入后按 Enter 提交。分割和修剪保留有理数源入点，不逐次截断到整数源帧。
- **UNDO / REDO**；**SEQUENCE** 设置固定画布和有理数帧率（如 30000/1001）。改名在顶部名称输入框中按 Enter。
- **SEQUENCE** 里可创建、命名、复制和切换序列。序列共享工程素材库，各自保留画布、帧率、片段、音轨和标记。编辑与导出作用于当前序列，序列操作随工程保存并可撤销。
- 在目标序列选择视频轨和播放头，在 **SEQUENCE** 选择来源并点 **Nest at playhead**。嵌套片段可做裁剪、变速、变换和加节点效果。子序列编辑会反映到父序列，实例保留原有位置和长度。子序列尺寸变化时，父序列监视器的变换抓手按实际子画布更新。子序列混音随片段一起播放和导出。重叠放置与循环引用会被拒绝。
- 视频轨标题下的混合菜单支持 Normal、Multiply、Screen 和 Overlay。相同模式用于监视器、播放和导出，锁定视频轨后禁止修改。
- 在片段 **Inspector → Mask** 可选矩形或椭圆，按源图像百分比设置中心、宽高和羽化。遮罩跟随片段的变换、裁剪与节点输出，并应用到监视器和导出。
- 监视器上方的 **Color scope** 可切换 RGB 直方图和亮度波形。示波器跟随可见画面，包括效果与合成，显示 SDR 程序画面的 8 位码值。
- 在 Nodes 的 **Grade versions** 可复制、命名、切换和删除片段的调色方案。版本保存独立的片段节点图及其动画参数。活动版本立刻用于监视器和导出，可撤销版本操作。
- 长时间线按窗口实例化片段控件。滚动会加载临近区间，拖拽结束时同步窗口。库、工程里的总片段数和编辑选择使用完整时间线。
- **Library → Input color** 可按素材选择 Sony S-Log3 / S-Gamut3.Cine、S-Log3 / S-Gamut3 或 S-Log2 / S-Gamut。选择 **Auto** 恢复自动解释。
- 在 Edit 的蒙版设置中选择 **Track mask forward**，从当前帧跟踪最多 240 帧的二维平移。用位置与宽高圈选有纹理的区域。轨迹记录源时间，随修剪、切分、预览和导出使用，可取消、清除和撤销。
- **Nodes → Blend** 提供底图与叠加图的两路图像输入；**Luma Mask** 用第二分支的亮度与 alpha 控制底图覆盖率。插入时第二路默认接 **Clip Source**，可拖线重接同片段的其他图像节点。**Amount** 控制混合强度。启用 ACEScg 时，两路在浮点工作域合成。
- **Color Corrector → Qualifier** 用色相、饱和度、明度和软边限制该节点的调色范围。用 Primaries 等面板调整入选像素，将 **Qualifier matte output** 设为 1 可输出黑白蒙版，供 Luma Mask 的第二路使用。
- **Nodes → Import CUBE LUT** 把线性 ACEScg 3D CUBE LUT 插入片段图。支持 2–33 点网格和输入域声明，进行三线性插值，超出输入域取边缘。LUT 随工程保存，可撤销和重开。冒烟使用 2 点网格，全量验证 33 点网格中的通道顺序和插值。
- **Color Corrector → Curves** 提供 Master 和独立 R、G、B 参数曲线。以 18% 灰为 0 EV，可编辑四个锚点的曝光偏移。主曲线与通道曲线在停止数域叠加，可独立旁路曲线阶段并重置。
- 监视器 **Color scope** 提供 RGB Parade 和 Vectorscope。RGB Parade 按画面横坐标分列显示各通道码值，Vectorscope 显示 Rec.709 Cb/Cr 分布。两者跟随监视器当前 SDR 图像。
- **Color Corrector → RGB Mixer** 在 ACEScg 中以 3×3 矩阵混合通道。每行控制一个输出通道，支持负系数、独立旁路、重置和撤销，保持 alpha 不变。
- 在 **Nodes** 监视器按 **Grab still** 固定当前画面的 SDR 参考图。参考图随工程保存，可在八张以内切换，通过 **Show still / Show live** 对照当前调色。删除参考图可撤销。
- 在 **Nodes** 选中具有主图像输入的节点后，可用 **Bypass node / Enable node** 切换。旁路保留参数与连线，写入工程历史，监视、播放与导出共享该状态。
- **Nodes → Compare original** 从同一帧左侧显示原素材，右侧显示当前调色结果。拖动分界可检查局部或临时全屏旁路片段节点。比较在关闭、换片段或播放时结束。
- **Nodes → View selected node** 将监视器切到所选图像节点，**View final output** 返回成片。切换只影响当前监视器，保留保存和导出的出线。
- **SEQUENCE → Managed color · ACEScg** 可将序列切换到浮点工作域，调色和合成完成后应用 SDR 监视变换。原工程默认维持原有 SDR 外观。
- MP4 色彩选项支持 **HDR10 PQ · BT.2020 / 1000 nits** 和 **HLG · BT.2020**，使用原始素材生成 HEVC Main10。适用范围与验收见 [HDR 工作流](HDR_WORKFLOW.md)。
- 在 MP4 导出窗口可用 **Add to queue** 冻结当前序列、范围和编码选项。用 **Render queue** 查看任务、串行启动、取消或重试。任务保留在当前应用会话，继续剪辑不会改动已排队的版本。
- **SAVE / SAVE AS / OPEN**：独立 `.oqvideo.json` 格式，原子文件替换、相对素材路径、完整 SHA-256 指纹、未来 schema/未知字段拒绝。撤销栈不写入工程。
- 编辑器后台保留未保存的工程快照，重新启动后可用 **Recover autosave** 打开副本，或用 **Discard autosave** 清除提示。每次手动覆盖保存会保留上一个版本，可用 **Previous version** 打开副本，再另存为。
- **Crop** 四个百分比字段裁切片段画面，保留原来的摆放位置。用 **Add transform keyframe** 在当前序列帧记录位置、缩放和旋转。建立关键帧后，在片段内改这些属性或拖预览手柄会更新当前位置的关键帧。在关键帧上可切换到下一帧的线性或缓入缓出插值，也可删除关键帧。
- 选中片段后点击 **Show keyframes**，可在时间线下切换变换、透明度和裁切轨道。单击选择关键帧，Shift 点击增减选择。拖动已选关键帧会成组移动，**Copy / Paste / Delete keyframes** 支持批量操作，粘贴以播放头为起点。移动、粘贴和删除各占一个撤销步骤。
- 选中一个有后继的关键帧，即可为这一段选择 **Hold、Linear、Smooth** 或 **Bezier**。选择 Bezier 后可拖动两个控制柄，调整时间与数值变化。每次松手提交一个可撤销编辑。
- 关键帧轨也可选择已连接片段效果中的浮点参数。**Add effect keyframe** 捕获当前值，在 **Effect keyframe value** 输入数值可写入播放头处的关键帧；移除后可撤销。改动会同时用于播放和导出。
- **Add crop keyframe** 可记录当前片段的四个裁切值。建立关键帧后，在片段内编辑裁切字段会更新对应帧，**Remove crop keyframe** 删除当前关键帧。裁切沿用片段时间，预览和导出都按相邻关键帧插值。
- 选中视频片段并把播放头移到片段内，即可在 Inspector 的 **Opacity (%)** 下用 **Add opacity keyframe** 添加透明度关键帧。曲线启用后，修改透明度会在当前帧新增或更新关键帧。**Remove opacity keyframe** 删除当前关键帧。相邻关键帧之间线性插值，曲线两端保持端点值。

## Nodes Workspace — Phase 7

Select **Nodes** with an active video or still image clip to edit its independent static pipeline. The workspace reuses OpenQuartz's graph canvas, Inspector, WGSL editor and GPU executor. Project saving includes each clip pipeline inside its track. Opening schema 1 migrates each clip to a direct source→output pipeline.

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

当前支持多视频轨有界播放/暂停、精确定位、切点预热和每片段独立效果。Edit 页的 **Add Track** 添加轨道，素材可直接拖入轨道，拖片段跨轨和拖轨头重排都进入工程撤销历史。监视器的 **Transform** 提供位置、缩放和旋转手柄，Inspector 同步显示变换并支持透明度。旧工程打开时迁移到 Schema 27，保留原有切点。[多视频轨合成设计](DESIGN.md#multi-track-composition)

**音频**：添加有声视频时，视频片段和可解码的音频默认链接。编辑视频时间、分割或删除会同步修改链接音频。音频素材用 **ADD CLIP** 放到播放头。音频轨提供静音、增益和锁定，音频片段 Inspector 提供增益、解除链接、独立时间与源入点编辑。播放用 macOS 音频设备时钟，底部显示输出电平。 时间线会在后台解码波形。独立音频可在原轨拖动、拖两端修剪，并用 **SPLIT** 在播放头分割。选中独立音频后可拖顶部淡入淡出手柄，或在 Inspector 按采样填写长度。播放和导出使用同一线性增益曲线，裁短片段时淡化长度会随之收窄。

**独听与混音**。轨头 **Solo** 让选中的独听轨参与监听与导出，仍可使用轨道静音。轨头显示每轨电平，底部输出读数的 **clipping** 表示合成信号超过满幅。相邻音频可在左侧片段的 **Audio crossfade (samples)** 输入交叉淡化长度，输入 0 移除。首版交叉淡化适用于原速片段，两段原片需要留出切点延伸所需的声音。

**代理预览** 用素材库的 **Prepare proxy** 在后台生成最多 640 × 360 的视频代理。代理通过逐帧验证后可用 **Use proxies** 切换，**Use originals** 恢复原片预览。**Prepare all proxies** 依次准备全部在线视频，显示完成进度。取消会停止当前任务及队列中的后续任务，已验证的代理保留并可复用。导出始终用原片，工程继续保存原素材的路径和指纹。

**EXPORT** 导出整个序列或指定区间，格式为 8-bit SDR H.264/MP4，沿用工程画布尺寸和帧率。默认起点为 0，结束点为序列总帧数。修改 **Start frame** 和 **End frame (exclusive)** 可按帧选区，结束帧不写入成片。音频同步裁切为 48 kHz 双声道 AAC，成片时间从零开始。分辨率预设等比适配指定尺寸，帧率预设按序列时间选择源帧，码率预设控制 H.264 目标码率。导出使用启动时的工程快照，窗口底部显示进度，工具栏可取消。完成时原子提交到新的本地 `.mp4` 路径。

## macOS 发行包

`npm run package:video-editor` 构建当前 Mac 架构的独立 **OpenQuartz Studio.app** 并输出到 `dist-desktop/`。应用包包含同一次构建产出的 LGPL FFmpeg 媒体库和代理工具，以及对应源码、补丁和许可文件。打包脚本重新解包 ZIP，移除开发媒体路径后检查签名及动态库依赖，运行 Studio 的 `--help`，并用随包 FFmpeg 转封装测试素材。发布工作流在 macOS 构建机运行同一命令。包使用 ad-hoc 签名，外部发布需要 Developer ID 签名与 Apple 公证。

## 验证

从仓库根目录执行：

```sh
npm run test:video-editor
npm run test:video-editor:ui:smoke
npm run test:video-editor:ui
npm run test:video-editor:coverage
```

`npm run test:video-editor:ui:smoke` 运行 Edit 和 Nodes 中标记为核心的原生用例。`npm run test:video-editor:ui` 和覆盖率命令执行两套用例的全量。仓库级入口为 `npm run test:e2e:smoke` 和 `npm run test:e2e:full`。

Rust 测试覆盖嵌套节点时钟、时间线窗口边界、GPU 帧预热和精确定位。原生 UI 还检查拖动后的持久化、效果前后的真实 GPU 像素和独立解码的交付文件。覆盖率命令将 Rust 测试与原生 UI 的插桩结果合并至 `test-results/phase6/editor-coverage.json`。

Timeline-window acceptance includes 30-second media plus, 900-frame clips at default/maximum zoom, fit-to-sequence, fractional-scale trims, undo and persistence. Nodes-window acceptance uses actual red/blue H.264 fixtures and checks final monitor pixels; graph callback forwarding alone is not treated as success.

原生播放测量：`node scripts/measure-video-editor-playback.mjs <ui-automation-binary> <project.oqvideo.json> <output-dir> [seconds]`。实际8K素材/完整质量/单与双Corrector测量、预算与计数解释见[8K实测记录](DESIGN.md#eight-k-playback-evidence)；原始数据在`test-results/window-playback/`。不把viewport尺寸、时钟推进或worker提交当真实图像交付。

报告和复查文件位于`test-results/phase6/ui-*/`及`test-results/clip-pipeline/nodes-ui-*/`：HTML/JSON报告、截图、工程、生成素材和关闭日志。Windows/Linux尚未完成原生硬件窗口验收；原仓库SDK 100% gate保持原阈值和排除规则。
