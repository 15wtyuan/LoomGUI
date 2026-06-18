# 架构决策记录（ADR）

> 记录 LoomGUI 关键设计决策的**为什么**（背景/决策/后果）。主文档（`docs/design/00-main-design.md`）是最终设计的真相源；本文件记"为什么这么定"和"从什么改过来的"，供长期追溯。
> 每条决策一个编号，按时间顺序。审查轮次详见 `docs/review/`。

---

## ADR-001 核心用 Rust，flexbox（taffy）布局

- **背景**：要跨引擎（Unity 首发、Godot 等）。布局能力要强（流式/响应式）。FairyGUI 的 Relations 是锚点式、不能流式。
- **决策**：Rust 写引擎无关核心；用 taffy 跑 flexbox 替换 fgui 的 Relations。Rust 一份代码覆盖多引擎。
- **后果**：核心可单测、可编译 WASM（编辑器）+ C ABI（引擎）。代价：FFI（csbindgen）+ Rust 学习曲线。

## ADR-002 渲染：渲染树 + 后端原生镜像（非扁平 draw-call 流）

- **背景**：初版想用"Rust 产扁平 draw-call 流、Unity `Graphics.DrawMesh` 直画、不用 GameObject"——批合更彻底、后端更薄。
- **决策**：**改为 fgui 路线**——Rust 产渲染树，后端镜像成原生渲染对象（Unity GameObject+MeshRenderer / Godot Node2D+canvas_item）。
- **理由**：draw-call 流**彻底切断引擎生态**（无法挂 Unity 粒子/3D 特效、难做世界空间 UI、接不上光照/后处理）。游戏 UI 常要特效，是硬需求。
- **后果**：失去"自己合并 mesh"的极限批合（改回 fgui 的 DrawState 复用 + FairyBatching 重排 + 借引擎批合）。换来引擎全生态集成。纯 2D 重 UI 后期可用 SRP 混合补性能。

## ADR-003 契约描述渲染意图，不描述引擎机制（去 Unity-flavored）

- **背景**：第三轮审查发现 §8 契约泄漏 Unity 假设（`y=height-y` 翻转是 Unity 特有；stencil+Material ref 是 Unity 图形管线；Godot 2D 遮罩用 canvas_group 不用 stencil）。契约若 Unity-flavored，Godot 后端会撞墙。
- **决策**：契约只描述**渲染意图**（画什么/遮罩意图/绘制顺序），不规定 GPU 机制。`StencilWrite{ref}` → `Mask{shape, mode}`（ref 出契约）；MaterialManager → DrawState 缓存；sortingOrder → 绘制优先级；坐标系核心左上、**核心无 `height-y`**、翻转是后端根 Stage 一次性变换（Godot 的 flip=单位矩阵）。
- **后果**：v1 Unity 后端照常用 stencil/Material（只是契约层换中立描述，零实现返工）；Godot 后端可做（用 canvas_group/clip）。

## ADR-004 文本 mesh 在后端生成，非文本几何在核心

- **背景**：初版说"几何全在 Rust 核心"（含文本，号称"比 fgui 进一步"）。但动态字形 UV 只有引擎字体 API（Unity `RequestCharactersInTexture`）才有，Rust 看不到 → 死结。
- **决策**：非文本几何（quad/形状/九宫格）在核心生成（一致性、数据量小）；**文本 mesh 在后端生成**，核心只产 TextLayout（位置/advance/cluster/断行）。
- **理由**：文本光栅化本质引擎相关；真正要跨引擎一致的是布局/断行/box 尺寸（已在 Rust）；字形像素差异各引擎 GPU 决定，无法跨引擎一致。
- **后果**：§6.1"比 fgui 进一步"措辞修正。代价：TextLayout 要跨 FFI（→ SOA 三表，ADR-006）。

## ADR-005 去掉 NativeHandle（Rust 不持引擎句柄）

- **背景**：曾设计 Rust 持"不透明句柄（NativeHandle=IntPtr）"引用 NativeHost 的用户 GameObject。
- **决策**：**删除**。所有引擎对象（含 NativeHost 用户对象）后端拥有，后端维护 `NodeId→对象` 映射。Rust 只持 NodeId（整数），跨 FFI 只传 id + 数据 buffer。
- **理由**：Rust 持句柄没必要——后端完全可中介。少一个不必要的抽象。
- **后果**：契约更简。NativeHost 用户对象经后端 API `BindNativeHost(nodeId, go)` 注册。

## ADR-006 TextLayout 跨 FFI 用 SOA 三表

- **背景**：文本 mesh 归后端（ADR-004）后，核心要把 TextLayout（三层嵌套：lines→runs→glyphs）跨 FFI 传后端。通用 arena 装不下嵌套变长，C# 解析空白。
- **决策**：TextLayout 不进通用 arena，单独 SOA 三表：`glyphs_soa[]`（扁平字形）、`runs_soa[]`（glyph 起止+font+format）、`lines_soa[]`（run 起止+y/baseline/width）。Text payload 带六个 u32 指向三表切片。cluster 现在就带（v1.x IME/光标要用）。
- **后果**：C# 读三张定长表 O(1) 跳转，零嵌套解析。同手法用于所有变长 payload（mesh 已扁平）。

## ADR-007 FFI：SOA 公共头 + 按类型多 arena + C# 拷贝

- **背景**：RenderNode 公共头 + enum payload 跨 FFI。要避免 IL2CPP marshalling 税 + Unity Boehm GC 卡帧。
- **决策**：公共头 SOA 定长并行数组（含 `payload_arena_idx/offset/len` 三元组）+ 按类型多个 per-frame arena（mesh/text/mask 各一）。C# `tick` 内拷完到预分配 buffer（拷贝非 pin）。Rust 下帧 reset arena 零分配。"沿用上帧"用 `payload=Unchanged` 变体。C# 用 `Span<byte>` 读，禁 `Marshal.PtrToStructure`。
- **后果**：零 GC、零 marshalling 税。代价：每帧一次 memcpy（几十 KB，可忽略）。

## ADR-008 打包器提前到 v1，图集优先级高

- **背景**：v1 要"二进制包加载"，但打包器（HTML→.pkg.bin）原排 v2。**没人产出包→运行时无 UI 可加载→v1 第一帧都画不出**（鸡生蛋）。且散图每张一个 draw call，UI 复杂就崩性能。
- **决策**：打包器 `loomgui_pkg` 提前到 v1（CLI，复用核心 parse/style 层，feature-gate 带 scraper 全家桶）。图集打包内置、优先级高（v1 早期，非 v1.x）。
- **后果**：v1 能开工。运行时仍零解析器（O2 成立）——打包在构建期/开发机。

## ADR-009 运行时只走二进制包（不解析 HTML）

- **背景**：曾考虑运行时也支持直接 HTML（调试/热重载）。
- **决策**：运行时**只支持二进制包**。热重载 = 重新编译 DSL→二进制再热重载二进制。HTML 解析（scraper 全家桶）只在打包器/编辑器，feature-gate 不进运行时。
- **后果**：游戏运行时二进制精简（无 parser）。scraper 重量不影响运行时。

## ADR-010 cascade 必须有继承；打包期展开

- **背景**：初版 cascade 子集只写"inline>id>class>tag"，漏了继承。color/font 这些浏览器默认继承，不实现 → 每个 span 重复写字体 → DSL 不可用。
- **决策**：cascade 四要素完整（优先级/出现顺序/属性合并/继承白名单）。**打包期把继承展开**成每节点 base_style（构建期树静态可算）→ 运行时零继承开销。
- **后果**：DSL 可写简洁。打包器价值兑现。

## ADR-011 运行时伪类 + 样式 dirty 机制

- **背景**：`:hover/:active` 是 v1 保留项，但它们和 `:l-page(n)` 同类（运行时状态驱动伪类），selectors 要运行时求值。原 DirtyFlags 无 style 档。
- **决策**：DirtyFlags 加 `style` 档。Controller/输入状态变 → 标子树 style dirty → 重 cascade（selectors 重匹配 + 合并）→ 置 layout dirty。Node→selectors::Element 适配器查状态。匹配结果按 (selector, 状态指纹) 缓存。
- **后果**：v1 按钮 hover/active 可用。:l-page+Controller/Gear 后续。

## ADR-012 行为正确性：事件不重 solve / transform 不进 taffy / 命中按等效绘制序 / 触摸仲裁

- **背景**：第二轮审查发现：事件后"再 solve 一轮"能死循环；transform vs layout 二义性致命中错位；命中按 children 序 vs 绘制按重排版致视觉/命中错位；拖拽/滚动无仲裁。
- **决策**：
  - 事件回调的布局改动**延迟到下帧 solve**（不当前帧重 solve，避免反馈环）。
  - transform 不进 taffy（仅 transform_dirty，刷新命中几何）；命中 = layout_rect 经累计 transform 的 AABB。
  - 命中按**等效绘制顺序**（children 经 sorting_order 重排）逆序。
  - 拖拽/滚动仲裁：阈值（滚动 20px/拖拽 10px）+ 先达阈值者赢 + 双向退让。
  - click：Stage 绝对坐标算距离 + Move 中超阈值取消 + down_targets 断裂沿祖先回退。
- **后果**：行为正确性闭环（照搬 fgui 成熟范式 + 适配帧末 solve 架构）。

## ADR-013 ScrollPane 惯性不走 GTween（单时钟例外）

- **背景**：§11 原则"全部走 GTween 单时钟"。但 ScrollPane 惯性走 GTween 的话，content 异步变化时 tween 的固定 end 会跳变。
- **决策**：ScrollPane 惯性**不走 GTween**，自维护可变 target 的 tween（`_tween_start/change/time`），content size 变化时按状态补偿。明确声明为单时钟原则例外。
- **后果**：滚动中 content 动态加载不跳变。

## ADR-014 Gear reentrancy 用深度计数

- **背景**：`gear_locked: Cell<bool>` 单值，GTween on_update 回调里改属性→update_gear 可能污染 Gear 存储。
- **决策**：`gear_locked: Cell<u8>`（深度计数）。Gear.Apply 触发的写 locked>0，setter 跳过回写。
- **后果**：reentrancy 安全。

## ADR-015 Godot 后端 = Node2D + canvas_item 自绘

- **背景**：§17 曾写"Godot 镜像成 Control/Node2D 或 RenderingServer 待定"。但这反推 v1 契约，是关键架构未决。
- **决策**：**现在拍板 Godot = Node2D + RenderingServer canvas_item 自绘**（与 Unity GameObject+MeshRenderer 严格对仗）。**否决 Control 路线**（会用 Godot 自己布局，与核心 taffy 双系统冲突）。
- **理由**：零 v1 成本，锁死契约方向。坐标系 Godot 本就左上 y 下（根 flip=单位矩阵）。遮罩用 canvas_group（非 stencil）。
- **后果**：v1 契约按"自绘镜像"定，Godot 后端 v2 可做。

## ADR-016 渲染契约版本化

- **背景**：RenderNode 公共头加字段 = 全后端重编（SOA 定长）。无版本化机制。
- **决策**：公共头带 `contract_version: u32` + `feature_flags: u64`。演进字段留可选扩展列（offset 指针）。SemVer：加可选=minor，改必选=major。
- **后果**：契约可长期演进。

## ADR-017 文本 v1 砍 BiDi/复杂 shaping；平台收窄 Win/Mac+Mono

- **背景**：v1 全套文本（rustybuzz+bidi）+ 全平台 FFI 工作量大。
- **决策**：亚洲/国内首发 → v1 砍 unicode-bidi + rustybuzz 复杂 shaping（保留 ttf-parser+linebreak）。v1 平台仅 Win/Mac 桌面 + Mono backend（IL2CPP/移动端 v1.x）。
- **后果**：省 ~2.5 人月。代价：v1 不支持阿语/RTL/复杂连字。含中东/全球市场时必须补。
