# LoomGUI v1.x / v2 待定机制

> 这里收留从主设计文档搬出的 **v1.x/v2 机制草稿**——实现期才该定的细节。主文档（`docs/design/00-main-design.md`）只写设计意图 + v1 精确契约；这些机制等真正实现、被代码验证后，再"毕业"回主文档。
> 这是**草稿不是契约**：具体字段/算法实现时按真实约束调，别照抄。

---

## 1. 虚拟化列表 `<l-list>`：slot 复用模型

**为何 defer**：v1 可滚动列表用 `div`+ScrollPane 手搓 item，不做虚拟化（v1-scope §2）。

**机制草稿**（将来实现起点）：
- 核心维护固定数量可视槽（槽数=可视窗口+缓冲）。数据滚动时核心把数据项映射到槽（item index → slot_id）：同 slot 这一帧 item5、下一帧 item6，**slot_id 稳定，NodeId 变**。
- RenderNode 公共头加 `slot_id: Option<u32>`（普通节点 None）。后端 diff **按复用键**复用渲染对象——`reuse_key = slot_id`（若非 None）否则 `node_id`。
- **两身份正交**：`NodeId`=逻辑身份（事件/命中/核心对象），`slot_id`=渲染复用身份（后端渲染对象池 key）。
- **核心强制不变量（防花屏）**：核心持 slot→node 映射，emit 时若某 slot 的 `(slot_id→node_id)` 本帧变化，该 slot **必发真实 payload 变体**（非 Unchanged），即便新 NodeId 自身属性未变——否则后端会在该 slot 上沿用旧 item 内容（花屏）。后端无需做 slot↔node diff，只按 reuse_key 查池。核心保证"slot 换内容时不会发 Unchanged"。
- 后端镜像池：`Dictionary<ReuseKey, RenderObject>`，每帧 stale-flag 同步，O(n)。
- ScrollPane 的视口/偏移/裁剪复用，虚拟化只额外加 slot 复用。

---

## 2. Shape mask（形状遮罩）+ 两遍 DFS 绘制序

**为何 defer**：v1 只做 rect mask（硬矩形裁剪），shape mask/paintingMode 是 v1.x。

**机制草稿**：
- RenderNode payload 加 `Mask { shape_ref, mode: MaskMode }`，`MaskMode { Write, Content, Erase }`。shape 用核心已有 mesh 几何。
- mask 的 Write/Content/Erase 是显式 RenderNode，核心保证三者配对发出。
- **遮罩是跨节点时序意图**：核心 DFS 算嵌套深度填入受影响节点的 `MaskContext`（rect/soft/shape + 嵌套深度 hint），后端不猜。嵌套深度 hint 帮后端选策略（如 Unity stencil 8-bit 限制 ~8 层，超限降级）。
- **两遍 DFS 的 sort_key 规则**（防批合重排越过遮罩边界）：
  - Pass 1：对每个 BatchingRoot 子树，按**重排后顺序**分配 sort_key——Write 最小，Content 居中，Erase 先占位待定。
  - Pass 2：`Erase.sort_key = max(该 mask 子树内所有 Content 的 sort_key) + 1`。
  - **批合重排区间约束**：重排只允许在 `[Write+1, Erase-1]` 内移动 Content，不得越过 Write 或 Erase。
- `mask_context` 是批合边界：不同遮罩上下文的 draw 即便 program 相同也不能合并。shape mask/paintingMode 的 Container 强制为 BatchingRoot（同 rect clip）。
- 后端自选机制：Unity stencil ref/compare、Godot canvas_group 离屏 RT、软件 alpha mask。

**soft clip（羽化）**、**paintingMode（离屏 RT，payload `PaintTarget { rt_id }`）** 同期 v1.x。

> 对照 fgui：fgui 是**单遍** DFS + stencil ref 翻倍法（深度=栈深度，无 hint 字段），mask write/erase 用不同 material 自然断批。LoomGUI 因把 mask 做成 first-class payload variant 才需要两遍 DFS——实现时评估是否改成 fgui 式单遍更简。

---

## 3. NativeHost（原生宿主）

**为何 defer**：v1 不接入引擎原生对象（v1-scope §2）。

**机制草稿**：
- 元素 `<l-native>` → NativeHost Node → RenderNode payload `NativeHost`（后端放用户引擎对象，不画自有 mesh）。
- **尺寸 push（不跨 FFI 回调查询）**：后端 push 尺寸给核心 `set_native_host_size(node_id, w, h)`，核心缓存值在 MeasureFunc 返回——避免每帧回调风暴 + 保持核心不碰引擎对象。
- **管线步**：§15 在 `set_input` 后、`tick` 前加 drain 步（`1.5 drain native host size pushes`），后端必须在 `tick(dt)` 调用前完成本帧所有 push（推荐：Update 开头采集 → push → set_input → tick），保证 solve 用本帧最新 intrinsic size。
- Unity：放用户 GameObject（粒子/3D/自定义），按布局 transform/clip 放置。NativeHost Dispose = detach（不销毁，用户拥有）。用户 GameObject 经后端 API `BindNativeHost(nodeId, go)` 注册。
- Godot：镜像成用户 Node2D 子树。

---

## 4. Controller / Gear / Transition（状态与编排）

**为何 defer**：v1 砍 Controller/Gear/Transition（v1-scope §2），v1 只用 GTween + Timers + ScrollPane 物理。

**机制草稿**：

### Controller（状态机，纯状态）
`Controller { name, selected_index, page_ids, page_names, actions }`。`set_selected_index`：记 previous、改 index、`parent.apply_controller(this)` 扇出到子节点 Gear + 派发 onChanged + **置子树 style dirty**（触发主文档 §5.3 重匹配）。Controller 不直接改 UI 属性，全靠 Gear + 样式重算。DSL 用 `[data-page]` 属性选择器（主文档 §4.5）。

### Gear（状态→属性映射）
- 每节点 `gears: [Option<Gear>; 10]`：Display/Xy/Size/Look/Color/Animation/Text/Icon/Display2/FontSize。
- 存储 `HashMap<page_id, Value>` + default。`Apply`：查当前页值 → 有 tween 的守卫 → 不可插值属性立即设 → kill 旧 tween → 往 TweenManager 提交插值 tween，`on_update` 写回。
- **双向同步 reentrancy 安全**（对齐 fgui `GearXY.cs:108-134`）：`gear_locked: Cell<bool>` 是**同步同栈帧守卫**——在每次 gear 写节点属性时（`on_update` 内、非 tween 的 `Apply` 内）**set → write → clear**，目的是阻止 `set_property → update_gear → UpdateState` 把刚写的值又读回 Gear 存储。**跨帧的 `on_update` 自己重新置位**（帧 N+1 的 TweenManager 回调里 set→write→clear），**不依赖**帧 N `Apply` 残留的 locked 状态（Apply 返回时已 clear）。per-Node bool 即够用——守卫只表"本栈帧有 gear 在写"，无需区分 10 个 gear 中哪个。

> ⚠️ 第四轮 review 曾把 gear_locked 误写为"Apply 时置位跨帧保住"——**那是错的**。正确机制如上：同步同栈帧。实现时以此为准。

### Transition（时间线 = 编排器，不自驱）
纯数据 `items: Vec<TransitionItem>` + 运行态 `total_tasks`（引用计数式完成检测）。`Play()` 把每个 item 翻译成 Tweener 提交 TweenManager：有 `tween_config` → `tween(start,end,dur).delay(time)`；瞬态帧 → `delayed_call(time)`。倒放 = 逆序 + start/end 互换 + delay 镜像。两点关键帧；多关键帧靠多 item 串行。嵌套 Transition 递归 + 完成回调递减父计数。带 `SetBreakpoint`(PlayFromTo 裁剪)。

---

## 5. 文本：v1.x 字段与跨引擎归一化

### TextLayout 预留字段
- `cluster`：v1 不带（无 shaping 时 cluster 与 glyph 1:1，无信息量）。v1.x 加 IME/光标/选区时再加——**届时 cluster 语义随 shaping 变**（rustybuzz 后 many-to-one/one-to-many），勿基于 1:1 设计光标。
- `font_id` per-glyph：v1 是 per-run（单 run 单字体，无 fallback）。v1.x emoji fallback 时升 per-glyph。
- `advance`：v1 不进 FFI 表（后端用 glyph 绝对坐标摆位，advance 是核心内部 pen 推进值）。

### 跨引擎归一化契约（Godot 接入时定）
v1 单后端（Unity）下，"同一 ttf 即一致"近似成立。Godot 接入后若跨引擎文本偏差不可接受，上**归一化契约**：
- advance/vertical metric **Rust 权威**：后端必须用 `glyphs_soa` 的 advance 与 `lines_soa` 的 baseline，**禁用**引擎 `CharacterInfo.advance`/自身 ascent。
- 引擎字体 API 降为"光栅化器"：只负责给定 glyph_id+size 返回 UV + 像素边界。
- 关 hinting（或 grid-fitting=none），保证光栅化字形边界落在核心测量的 advance 内。

### v1 文本简化的已知代价（非干净切，围栏文档须告知 AI）
v1 砍 rustybuzz/BiDi/fallback：
- **CJK + emoji**：emoji 来自 fallback 字体（不同 ttf），v1 无 fallback 链 → tofu/缺字，除非把 emoji 塞进 CJK ttf（不现实）。修需 v1.x 加"仅 emoji 的最小 fallback 链" + per-glyph font_id。
- **CJK + 组合符号**（拼音声调 nǐ、越南语声调）：无 shaping 时组合符放在 base 的 advance 位而非上方 → 错位。修需 GDEF/GPOS mark 附加 = shaping。
- **RTL/阿拉伯文**：不支持（目标市场外）。
- v1 仅支持：CJK + ASCII + CJK 标点。

### 测量缓存（实现期撞墙再加）
v1 naive 重算（500 节点够用）。若 taffy 反复调 measure 撞性能墙，加 `HashMap<(text_hash, font_id, font_size, constraint_width), (w,h)>`，text-content/style/width 变化驱动失效。

### line-height / kinsoku 算法
主文档只留原则（line-height 生效烤进 Line.height、换行以核心为准对齐 Chrome）。具体公式（CSS leading 上下分、CJK kinsoku 行首行尾标点约束）实现期对照 Chrome 调，不预设。

---

## 6. 包格式：v1.x 演进项

- **集中式迁移器链**（新 runtime 读旧包）：v1 用内联 `formatVersion` 兼容（对齐 fgui）。多版本累积后再上 per-version 迁移函数链。
- **`nextPos` 长度前缀 forward-compat**（旧 runtime 跳过未知字段）：v1 不带，v2 格式加字段时再加。
- **branches（多语言）/ highResolution（1x/2x/3x 分支）**：v1 单分辨率、无本地化分支。多语言/高清资源分支 v1.x。
- **scaleLevel**（MatchWidth/MatchHeight 模式、高清驱动）：v1 只 MatchWidthOrHeight。

---

## 7. 契约版本化（待第二个契约版本时定）

主文档不加版本字段/扩展列——无 v2 契约。将来真有第二个契约版本时再加：
- 公共头 `contract_version: u32` + `feature_flags: u64`。
- 可选扩展列以 arena 内**相对偏移（u32）**指向同 arena 内数据（绝不跨 FFI 传裸指针，C# 拷贝整块 arena 后在自身 buffer 内 seek）。feature_flags 门控哪些扩展列存在。
- SemVer：加可选字段=minor（旧后端忽略）；改必选/语义=major。
- 不变量：feature_flags 变化（节点获得/失去扩展列）视为 payload 变化——必发真实 payload 非 Unchanged。

---

## 8. 其它 v1.x

- **世界空间 UI**：NodeTransform 加 `Option<VertexMatrix>`（透视/斜切）。Unity 根 panel GameObject 可放世界空间 + 摄像机。
- **DrawState 扩展**：DrawFlags 加 `SoftClipped/Masked/AlphaMask/ColorFilter`；BlendMode 全 12 种（照搬 fgui src/dst 因子表，Multiply/Screen 触发 pma→ColorFilter）；ProgramId 加 BMFont/自定义。
- **SRP 混合渲染**（Unity）：自绘节点用自定义 SRP RendererFeature 批合（少 draw call），NativeHost/特效仍是 GameObject。
- **节点类型**：RichText/TextInput/Graph/Loader/MovieClip/List/Slider/ProgressBar/ComboBox/Tree/NativeHost。
- **CSS 扩展**：`position:absolute`/`align-content`/`border-radius`/`filter`/`clip-path`/`cursor`/`:focus`/`:nth-child`/属性选择器/`row-gap`等 longhand 已在 v1，余 v1.x。
