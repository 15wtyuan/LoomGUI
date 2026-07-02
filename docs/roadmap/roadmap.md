# LoomGUI 路线图

> v1 架构验证完成（v1a-v1e + showcase + v1.1-v1.3+ 动态树，家里机验收坑 58-95 修复），桌面 Mono 可演示。
> 本文是 **v1 全貌（已交付）+ v1 之后的三板块路线（v1.x 功能 / v other 编辑器 / v2 平台）+ v1.x/v2 机制草稿**。
> 设计契约见 `docs/design/main-design.md`；围栏属性权威见 `docs/design/fence.md`。

---

## 0. 当前状态（TL;DR）

- **v1 = 架构走通 + 桌面可演示**（demo-grade，非上线）。
- **距上线三缺口**：① 移动平台 ② 编辑器/AI 工作流闭环 ③ 关键控件（列表/富文本/输入/状态机）。
- **差异化已立**（别丢）：AI 可预测性（HTML-as-DSL，AI 能编辑+预测渲染）+ flexbox（超 fgui Relations）+ Rust 跨引擎共享核心 + 围栏验证器（打包期挡违规）。

---

## 1. v1 已交付

### 1.1 能力清单

**渲染**：贴图 quad + 纯文本 + 硬矩形裁剪（rect mask）；FairyBatching 重排 + 显式 mesh 合并（真 N→1 draw call）；Unity 后端 GameObject 镜像 + DrawState 缓存（MaterialManager）+ 提交。

**文本**：ttf-parser 测量 + unicode-linebreak 断行（砍 rustybuzz 复杂 shaping + unicode-bidi，亚洲/国内首发）；后端据 TextLayout（SOA 三表）生成 mesh；字体用包声明的同一 ttf（一致性根基）。

**事件**：命中（按等效绘制顺序逆序）+ click/hover/leave + 拖拽；多触摸（5 槽）+ CaptureTouch + 拖拽/滚动仲裁（阈值 + 退让）+ 键盘/焦点/Tab；`is_pointer_on_ui` 消费。

**布局**：taffy flexbox（围栏子集）；参考分辨率缩放（MatchWidthOrHeight）；safe-area（异形屏 uniform shrink-to-fit + letterbox）。

**滚动**：ScrollPane：惯性 + 回弹 + 滚动条 + 鼠标滚轮（自维护可变 target tween，不走 GTween）。

**资源**：打包器 `loomgui_pkg`（HTML+CSS+资源→.pkg.bin+图集）；二进制包加载（formatVersion + 迁移器）；图集（散图 shelf 打包）+ refcount。

**FFI**：csbindgen 通路 + SOA+多 arena 渲染树同步（blob v4，18 列）。

**状态/样式**：`:hover/:active/:disabled/:focus`（运行时伪类 + 样式 dirty 重匹配）；cascade 继承（打包期展开）+ 合并 + 出现顺序。

**动态树（v1.3+）**：代际 NodeId + slotmap + 9 个命令式 API（create/remove/move/set_text/set_src/set_style），详见 design §13.1。

### 1.2 v1 围栏冻结子集

> **权威清单 = `docs/design/fence.md`**（真相源 `loomgui_core/tests/fence_contract.rs`）。本节只标 v1 冻结口径，不重复属性表。

- **元素**：`div`(Container) / `span`+裸文本(Text) / `img`(Image) / `button`(Button)。围栏外标签报错（不降级）。**v1.x 设计层也不用 `<l-list>`/`<l-rich>`**——虚拟列表/富文本由代码层做，围栏不暴露，AI 不知道就不会写（design §4.1）。
- **CSS**：布局（flex 全家）+ 视觉（含 v1.x 已实现 `background-image`/`background-size`/`border-radius`/`filter`/`border-image-slice`）+ `transform`/`overflow(+x/y)`/`pointer-events`。值约束 + 围栏外静默忽略项见 fence.md §2。
- **选择器**：标签/类/id/后代/子代/分组 + `:hover/:active/:disabled/:focus`。
- **目标市场**：亚洲/国内首发（决定文本可砍 BiDi）。**平台**：v1 仅 Win/Mac 桌面 + Mono backend（IL2CPP/移动端 v2）。

### 1.3 v1 预览方案（临时，编辑器期换 WASM）

v1 还没有编辑器/WASM 渲染，用 **open-design Chromium 兜底**：围栏验证器（打包期挡违规）+ Chromium 预览 + head 内联 polyfill（`div{display:flex;flex-direction:column}` + `*{box-sizing:border-box}` + `body{margin:0}`，对齐 LoomGUI 契约：div 总是 flex column）。
- **预览可信清单**（fence.md §6）：flex/gap/color/px/background-image 可信；margin 折叠/文本换行/`position:absolute`/`display:grid`/`@media` 不可信。口径："信围栏规则，别信预览不可信项"。
- **v2 替换**：编辑器用 WASM 跑核心做零偏差预览。

### 1.4 Unity 胶水任务（G1-G14，已交付）

| # | 任务 |
|---|---|
| G1 | 打包器 `loomgui_pkg` |
| G2 | Stage MonoBehaviour 驱动（唯一 tick 入口） |
| G3 | 根 Stage 挂 Unity（Camera/GameObject + 根 y-flip） |
| G4 | 输入采集→扁平事件→FFI 注入（新旧输入系统 + IME character） |
| G5 | IME/字符输入（v1 最小 PC 键盘字符级） |
| G6 | 字体资源进 Unity + 注册给核心（同一 ttf） |
| G7 | 纹理加载（磁盘→Unity→GPU→TexId） |
| G8 | 坐标翻转（根 Stage 一次性） |
| G9 | GameObject 镜像池 diff（NodeId→GO，slot 复用、Mask 独立、Unchanged） |
| G10 | DrawState 缓存（MaterialManager）+ Image/Text shader |
| G11 | csbindgen 生成 + native lib 构建脚本 |
| G12 | 参考分辨率 Unity 侧落地 |
| G13 | Domain reload / Play mode 重置保护 |
| G14 | 滚动条 Unity 侧渲染 |

### 1.5 v1 验收标准

能演示：① 按钮+文本+图片面板 ② 可滚动容器（惯性+回弹+滚动条）③ 按钮 hover/active 视觉反馈 ④ 自适应分辨率（1080×1920 等比缩放） ⑤ UI 挡住时游戏不响应点击 ⑥ HTML 经打包器产出二进制包加载。

**性能基线**：500 节点静态 UI 每帧无卡顿；冷帧/换页帧（500 节点全 dirty）FFI 拷贝 + arena 解析 ≤ 2ms（v1e dirty hash → Unchanged，静态帧≈0 upload）。

### 1.6 v1 明确不做（推 v1.x/v2）

富文本、软裁剪/形状遮罩(paintingMode)、Transition+Controller+Gear 编排、列表虚拟化、滚动分页/吸附/下拉刷新、IME 完整链路+软键盘、字体 fallback 链、完整 NativeHost、rustybuzz 复杂 shaping+BiDi、IL2CPP+移动端、grid、CSS transition。

> 注：border-image-slice 九宫格（v1.3 已做）。v1d（滚动/键盘/transform/动画/safe-area）子轮已全交付，明细见 git history + docs/pitfalls.md。

---

## 2. v1.x — 上线功能必备 + AI 可预测性

**排期定稿（2026-06-30）**：v1.x 单一编号，一功能一号，按完成序递增；补丁不占号。首要判据 = AI 可预测性 → 先填"静默忽略"视觉 gap + 绘制质量（低风险快赢），再上线控件。本机唯一编码机，串行推进。

| v1.x | 项 | why | 状态 |
|---|---|---|---|
| v1.1 | **background-image**（+坑79 共存视觉补丁）| AI 必写 `background-image:url`；围栏内却零解析，静默忽略=契约违背 | ✅ |
| v1.2 | **border-radius（圆角 mesh）** | AI 必写 CSS；围栏外静默丢弃违背可预测性 | ✅ |
| v1.3 | **ColorFilter + 九宫格 slice + profiling** | 色调统一 + disabled 灰化升级；UI 皮肤缩放不变形；draw call/GC/内存实机达标 | ✅ |
| v1.3+ | **动态树重构（地基）** | v1 static-tree 撞墙（v1.4 列表/v1.5+ 全需运行时改树）。代际 NodeId + slotmap + 动态 API，非功能号 | ✅ 待家里机验 |
| v1.4 | **虚拟化列表 + soft clip** | 背包/排行榜/邮件必备，v1 手搓 div+scroll 无 slot 复用。建在动态树之上 | 待开 |
| v1.5 | **Controller / Gear / Transition** | 标签页/弹窗/过场/状态切换必备 | 待开 |
| v1.6 | **富文本（inline layout）** | 聊天/物品描述必备。多样式/图文混排，复用 v1 文本测量。内部 NodeKind，不暴露标签 | 待开 |
| v1.7 | **TextInput / IME（光标/选区/composing）** | 登录/搜索必备。IME 最重，可能需 rustybuzz | 待开 |

---

## 3. v other — 编辑器工作流（独立并行，不阻塞主线）

**壳 = open-design 桌面 app**（不自建；Apache-2.0；nexu-io；agent-driven 生成器；插件架构，不改源码）。不选 design.md（Google）：类别错配（token 字典非编辑器）。不自建壳：复用 open-design 省项目管理/对话/导出/部署基建。

**机制**（调研确认）：`od project import <baseDir>` 导入目录为工作区 → daemon 把 project cwd = baseDir → open-design spawn harness（Claude Code 等）在该 cwd → harness 自动读 cwd 的 `CLAUDE.md` + `.claude/skills/`。

**LoomGUI editor 层**（shell-agnostic，模板源 `editor/`，init 脚本注入设计师工作区）：
1. **init 脚本**（`editor/init.mjs`，Node 单文件）：交互输工作区/输出/harness → 拷围栏规则 + skill 进目标工作区。CLAUDE.md 增量合并（标签包裹，不覆盖用户已有）。
2. **围栏规则**（`editor/rules/<harness>/CLAUDE.md.tmpl`，三 harness：claude/opencode/codex）：围栏权威清单见 fence.md。AI 守围栏生成 HTML+CSS。
3. **skill**（`editor/skill/loomgui-editor/`，封装 loomgui_pkg 不暴露）：教 AI 围栏生成 + 跑 `pack.mjs` 验证+打包（违规非零退出 AI 自纠，合规产出 pkg.bin）。**严 polyfill 固化**：SKILL 强制 head 内联 polyfill（防设计师漏抄预览塌）。
4. **打包桥**：`loomgui_pkg` CLI（验证+打包合一）。

**预览妥协**：open-design Chromium iframe ≠ taffy（字体度量/flex/margin 折叠/position:absolute 分歧）。skill 教"信围栏规则别信预览不可信项"（fence.md §6）；真实靠 Unity 验（家里机）。v2 WASM 跑核心做零偏差预览。

**围栏验证**（单一真相源）：`loomgui_core/tests/fence_contract.rs` 可执行围栏契约。`cargo test -p loomgui_core fence_contract` 是防漂移门。

---

## 4. v2 — 平台 + 生态 + 特效

| # | 项 | why |
|---|---|---|
| 1 | **移动 + IL2CPP + WebGL** | 平台（原 v1.x 移出，移植工作重单独 v2）。上线游戏必备 |
| 2 | **多引擎（Godot）** | 验证跨引擎一致性。Rust 核心共享的价值兑现 |
| 3 | **多语言 / 异步加载 / 热更新** | 上线运营必备 |
| 4 | **WASM 零偏差预览** | 替换 v other 近似。AI 闭环所见即所得 |
| 5 | **shape mask / alpha mask / paintingMode** | 异形遮罩/离屏 RT/特效隔离 |
| 6 | **BlurFilter / DropShadow / Glow** | 模糊/阴影/发光（PNG 皮肤能补，故推 v2） |
| 7 | **BlendMode 扩展（Add/Screen/...）** | v1 仅 Normal。特效混合 |
| 8 | **椭圆 / 多边形 / RadialFill mesh** | 几何扩展（v1 仅矩形 quad） |

---

## 5. v1.x/v2 机制草稿

> 收留从主设计搬出的 **v1.x/v2 机制草稿**——实现期才该定的细节。主文档只写设计意图 + v1 契约；这些机制等实现验证后"毕业"回主文档。**草稿不是契约**：字段/算法实现时按真实约束调。

### 5.1 虚拟化列表：slot 复用模型（v1.4）

核心维护固定数量可视槽（item index → slot_id）：同 slot 这一帧 item5、下一帧 item6，**slot_id 稳定，NodeId 变**。后端 diff 按复用键复用渲染对象——`reuse_key = slot_id`（若非 None）否则 `node_id`。两身份正交：NodeId=逻辑身份（事件/命中），slot_id=渲染复用身份。**核心不变量（防花屏）**：slot 换内容时必发真实 payload 非 Unchanged。

### 5.2 Shape mask + 两遍 DFS（v2）

RenderNode payload 加 `Mask{shape_ref, mode: MaskMode}`，MaskMode{Write,Content,Erase}。遮罩是跨节点时序意图：核心 DFS 算嵌套深度填 `MaskContext`。两遍 DFS sort_key 规则（防批合越界）：Pass1 按 Write 最小/Content 居中分配；Pass2 `Erase.sort_key = max(子树 Content)+1`。批合重排约束在 `[Write+1, Erase-1]` 内。后端自选：Unity stencil / Godot canvas_group / 软件 alpha mask。soft clip（羽化）、paintingMode（离屏 RT）同期 v2。

### 5.3 NativeHost（v2）

v1d.3 已做 **NativeHost-lite**（div 占位 + 后端 `BindNativeHost` 跟随 world transform + 显隐 + 排序，core 零改）。完整版加：尺寸 push（后端 push 给核心 `set_native_host_size`，核心缓存值在 MeasureFunc 返回——避免每帧回调风暴）、hit/clip/所有权/Godot 镜像。管线加 drain 步（set_input 后、tick 前，后端须完成本帧 size push）。

### 5.4 Controller / Gear / Transition（v1.5）

**Controller**（状态机，纯状态）：`set_selected_index` 改 index + 扇出子节点 Gear + 派发 onChanged + 置子树 style dirty。DSL 用 `[data-page]` 属性选择器（design §4.5）。
**Gear**（状态→属性映射）：每节点 `gears`，存储 `HashMap<page_id, Value>`。Apply 查当前页值 → kill 旧 tween → 提交插值 tween。reentrancy 守卫：`gear_locked` 同步同栈帧（set→write→clear），防 `set_property→update_gear→UpdateState` 回写污染。
**Transition**（时间线=编排器，不自驱）：纯数据 `items: Vec<TransitionItem>`。Play 翻译成 Tweener 提交 TweenManager。倒放=逆序+start/end 互换+delay 镜像。

### 5.5 文本：v1.x 字段与跨引擎归一化

- **cluster**：v1 不带（无 shaping 时与 glyph 1:1）。v1.x 加 IME/光标/选区时再加，**届时 cluster 语义随 shaping 变**（rustybuzz 后 many-to-one），勿基于 1:1 设计光标。
- **font_id** per-glyph：v1 per-run（单字体）。v1.x emoji fallback 升 per-glyph。
- **跨引擎归一化契约**（Godot 接入时定）：advance/vertical metric Rust 权威（后端禁用引擎 `CharacterInfo.advance`）；引擎字体 API 降为光栅化器；关 hinting。
- **v1 文本简化代价**：emoji→tofu（无 fallback）、组合符号→错位（无 shaping）、RTL 不支持。

### 5.6 包格式：v1.x 演进项

集中式迁移器链（多版本累积后）；`nextPos` 长度前缀 forward-compat（v2 加字段）；branches（多语言）/highResolution（1x/2x/3x）；scaleLevel（MatchWidth/MatchHeight）。v1 当前 formatVersion 8（详见 docs/pitfalls.md §1 包格式）。

### 5.7 契约版本化（待第二个契约版本时定）

主文档不加版本字段——无 v2 契约。将来真有第二版本时：公共头 `contract_version:u32` + `feature_flags:u64` + 可选扩展列（arena 内相对偏移，绝不跨 FFI 传裸指针）。SemVer：加可选=minor，改必选=major。不变量：feature_flags 变化视为 payload 变化（必发真实 payload 非 Unchanged）。

### 5.8 其它 v1.x/v2

- **世界空间 UI**：NodeTransform 加 `Option<VertexMatrix>`（透视/斜切）。
- **DrawState 扩展**：DrawFlags 加 SoftClipped/Masked/AlphaMask/ColorFilter；BlendMode 全 12 种；ProgramId 加 BMFont/自定义。
- **SRP 混合渲染**（Unity）：自绘节点用自定义 SRP RendererFeature 批合。
- **节点类型**：RichText/TextInput/Graph/Loader/MovieClip/Slider/ProgressBar/ComboBox/Tree/NativeHost（内部 NodeKind，**不暴露为 HTML 标签**）。
- **CSS 扩展**：border-radius/filter/border-image-slice/:focus/overflow-x/y/row-gap 等已在 v1 实现，余 v1.x。

---

## 6. 关键决策（why）+ 对标基线

### 6.1 关键决策

- **移动+IL2CPP 推 v2**（非 v1.x）：v1.x 聚焦功能必备，平台移植工作量重。
- **编辑器用 open-design 不自建**：复用其插件/对话/导出机制，省自建壳基建。
- **shape mask/filter 拆分**：border-radius/background-image/soft clip/ColorFilter 进 v1.x（AI 必写不可推 + 配合功能）；特效（blur/glow/异形 mask/blend）推 v2（PNG 皮肤/九宫格能补）。
- **v other 并行**：编辑器工作流独立于 runtime，不阻塞 v1.x/v2。

### 6.2 对标基线 + 成熟度

- **对标 FairyGUI**：10 年沉淀，跨引擎（Unity/Cocos/UE/Laya），可视化编辑器，30 示例，MIT。LoomGUI 精神继承 + 布局替换（flexbox 代 Relations）。
- **v1 成熟度**：架构完整（FFI/打包/Unity 后端/事件/滚动/动效/状态 全）+ 桌面可演示 + 性能 500 节点静态无卡顿。距上线 = v1.x（功能）+ v other（编辑器）+ v2（平台）。
- **LoomGUI 差异化**（对标 fgui 的竞争力）：AI 可预测性（HTML-DSL，fgui .fui 二进制 AI 不能编辑）+ flexbox（流式/响应式/内在尺寸，超 fgui 锚点）+ Rust 跨引擎共享核心（fgui 各引擎独立 SDK）+ 围栏验证器（AI 第一道反馈）。
