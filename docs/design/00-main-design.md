# LoomGUI — 主设计文档

> 本文档是 LoomGUI 的**当前实现真相源**：设计意图 + v1 精确契约。v1.x/v2 的机制细节（实现期才定）在 `docs/roadmap/v1x-deferred.md`；决策理由在 `docs/decisions/`；审查在 `docs/review/`。
> 设计原则：契约描述**渲染意图**而非引擎实现机制；核心（Rust）引擎无关；后端各引擎自选实现。
> 参考实现：FairyGUI-unity（`F:\WorkSpace\projects\FairyGUI-unity`，渲染/对象模型/动画的原理参考）。

---

## 0. TL;DR

LoomGUI 是一款跨引擎游戏 UI 框架，目标"一次编辑、多引擎一致运行"，对标 FairyGUI。

**核心动机：AI 驱动的界面拼装。** 传统游戏 GUI 要美术设计 + 程序手工拼界面，耗人力。这步可由 AI 完成，但卡在"AI 无法精准感知结果"。**HTML 作 DSL 的根本理由**：AI 既能精确编辑（文本），又能从源码预测渲染结果（AI 对 HTML/CSS 有强先验）。**DSL 的首要设计准则：AI 读 HTML 能否正确预测渲染出的 UI**——这是所有 DSL 决策的第一判据（§1.1）。

技术取舍：
- **布局**：taffy 跑 flexbox（替换 fgui 的 Relations），支持流式、响应式、内在尺寸。
- **渲染**：自绘。Rust 核心每帧产出一棵**渲染树**（RenderNode，公共头 + enum payload，描述**渲染意图**）；后端镜像成原生渲染对象（Unity: GameObject+MeshRenderer；Godot: Node2D+canvas_item）。
- **文本**：Rust 用 ttf-parser + unicode-linebreak 测量与断行（核心产 TextLayout），后端据 TextLayout 光栅化生成 mesh。
- **FFI**：csbindgen（Rust→C ABI→C# P/Invoke），IL2CPP 友好。
- **流程纪律**：核心是纯 Rust 库，先在测试里验证（无 FFI），再 FFI 进引擎。

---

## 1. 概述与目标

### 1.1 要解决的问题
**核心动机：AI 驱动的界面拼装。** 传统游戏 GUI 需要美术设计、程序按设计图手工拼界面，耗费大量人力。拼/还原界面这步可由 AI 完成，但既有路径都卡在"AI 无法精准感知结果"：UI 编辑器 + MCP（AI 无法精确操作控件位置）、直接改 JSON/XML（AI 无法感知最终样子）。**HTML 作 DSL 的根本理由**：AI 既能精确编辑（文本），又能从源码预测/感知渲染结果（AI 对 HTML/CSS 有强先验）。**因此 DSL 的首要设计准则：AI 读 HTML 能否正确预测渲染出的 UI 长什么样**——背离浏览器语义的 divergence 都会损害此目标，须谨慎评估。

在此基础上，LoomGUI 也是 FairyGUI 的精神继承者：HTML+CSS 子集作 DSL、flexbox 替代锚点、Rust 核心覆盖多引擎，解决传统游戏 UI 布局弱、跨引擎不通用、各引擎渲染不一致的问题。

### 1.2 目标
- **G1 编辑一次，多引擎一致**：同一份 HTML/资源包，在 Unity（首发）及后续引擎上**布局/文本/几何一致**。
- **G2 流式布局**：flexbox 完整子集，支持响应式（分辨率/异形屏 safe-area）、动态内容、内在尺寸。
- **G3 运行时动态**：UI 在运行时可任意增删改节点、跑动画、响应数据变化。
- **G4 FairyGUI 级渲染质量 + 引擎生态集成**：自绘、批合、遮罩/裁剪、富文本、九宫格、序列帧；后期挂引擎特效、世界空间 UI。
- **G5 可扩展**：框架内置基础控件 + 项目自定义控件共存。

### 1.3 非目标
- 不做完整浏览器 CSS（无块级/行内流、无 float、无 grid——grid 后期）。
- 编辑器后期单独项目；本文只定 DSL 规范与运行时。
- 不做 Unity UGUI/UIToolkit 兼容层（纯自绘 + 原生渲染对象镜像）。

---

## 2. 设计原则

1. **核心即纯库**：所有引擎无关逻辑（解析、布局、几何生成、渲染状态、事件、动画）都在 Rust 核心，无引擎依赖、可单测。
2. **渲染树是契约，描述意图不描述机制**：核心↔后端接口是一帧的渲染树。描述"画什么/遮罩意图/绘制顺序"，**不规定**"用什么 GPU 机制"（stencil vs canvas_group、Material vs CanvasItem 由后端自选）。这是跨引擎的根基。
3. **照搬 fgui 的成熟机制，替换其布局**：渲染/批合/裁剪/对象模型/动画/资源管线借鉴 fgui；Relations 整套换成 taffy。
4. **围栏优先**：HTML/CSS 只支持明确子集，不支持即按策略处理（见 §4.1）。
5. **单 tick 入口（内部有序分步）**：整个核心只有一个每帧 `tick(dt)` 入口，内部按固定顺序分步驱动 GTween → ScrollPane 物理 → 样式 → 布局 → 事件 → 渲染（§15）。ScrollPane 物理是 GTween 之外的有序子步，非独立时钟。

---

## 3. 总体架构

### 3.1 分层

```
┌─────────────────────────────────────────────────────────────┐
│  HTML/CSS DSL  (人/编辑器/工具链 读写)                          │
├─────────────────────────────────────────────────────────────┤
│  打包器 loomgui_pkg (构建期工具)                                │
│   HTML+CSS+资源 → 二进制包(.pkg.bin) + 图集                     │
│   复用核心 parse/style 层；运行时不带解析器                       │
├─────────────────────────────────────────────────────────────┤
│  Rust 核心 (loomgui_core, 引擎无关)                             │
│   解析层(scraper+cssparser+极简匹配器) → 样式层(cascade)         │
│   → 布局层(taffy flexbox) → 场景图(Node 树)                     │
│   + 事件/命中 + 动画(GTween/ScrollPane 物理)                    │
│   → 渲染状态层(几何生成/批合重排/裁剪意图/绘制顺序)               │
│   → 每帧产出渲染树 (Vec<RenderNode>)  ← 核心↔后端契约            │
├─────────────────────────────────────────────────────────────┤
│  FFI 缝界 (csbindgen: Rust ↔ C ABI ↔ C# P/Invoke)            │
├─────────────────────────────────────────────────────────────┤
│  引擎后端 (Unity 首发; Godot 等)                                │
│   - 输入采集 → 注入核心                                         │
│   - 把渲染树镜像成原生渲染对象 (Unity GameObject+MeshRenderer /   │
│     Godot Node2D+canvas_item)，自选遮罩/排序机制                 │
│   - 消费 DrawState 缓存、提交渲染                                │
│   - 资源加载 → 注入核心纹理 id                                  │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 核心接口
```rust
let stage = Stage::new(config);
stage.load_package(pkg_bytes)?;                              // 注册包(二进制)
let root = stage.create_object("loom://pkg/MainUI")?;        // 实例化树
root.get_child("startBtn").set_text("开始");
stage.set_input(&input_events);                              // 注入本帧输入
stage.tick(dt);                                              // 推进布局/动画/事件
let render_nodes: &[RenderNode] = stage.render_nodes();      // 后端据此同步镜像
let pointer_on_ui: bool = stage.is_pointer_on_ui();          // 游戏输入消费查询
```

### 3.3 关键边界
- **Rust 核心**：解析、样式、布局、场景图、事件、动画、几何生成、渲染状态计算、批合重排、裁剪/顺序。产出 `Vec<RenderNode>` + 命中结果 + 事件。**不持任何引擎对象、不碰 GPU。**
- **引擎后端**：输入采集、渲染树→原生渲染对象镜像、mesh 上传、DrawState 缓存与提交、资源加载代理。
- **不跨越的**：核心不知道 GameObject/CanvasItem；后端不解析 DSL、不算布局、不生成几何。

---

## 4. HTML/CSS 围栏（DSL 规范子集）

> LoomGUI 只支持一个明确的子集。围栏是活文档，每加一个属性都要想清楚它在 taffy/渲染层是否值得。**v1 实际支持的冻结子集见 `docs/roadmap/v1-scope.md` §2**；本节是设计规范（每属性如何映射），范围含 v1.x 目标。

### 4.1 围栏总原则与忽略策略
- **LoomGUI `<div>` 永远是 flex 容器**（默认 `flex-direction: column`，即垂直堆叠）。不实现浏览器 block/inline flow——**只有 flex item 参与布局**。这是必要 divergence：我们只做 flexbox，不做 block flow。要水平排列显式写 `display: flex`（= row）。
  - **AI 可预测性口径**（§1.1）：AI 对 `<div>` 的浏览器先验里，"子节点垂直堆叠"这条**成立**，但"文本/行内元素在 div 内行内流"这条**不成立**（LoomGUI 无行内流）。围栏文档须告知 AI：**`<div>` 只装 flex item；文本+图/格式 span 的混排进 `<l-rich>`**。这是 AI 须纠正的唯一一条 div 行为偏差。
- **自定义元素 kebab-case**：`<l-list>`/`<l-loader>` 等用 `l-` 前缀避免与 HTML 冲突。
- **命名约定**：`data-*` 用于状态/数据属性（标准 HTML，AI 熟练，如 `data-page`）；`-l-*` 仅用于无 CSS 等价物的真扩展属性。
- **忽略策略分级**：
  - **装饰属性**（color/border-radius/filter/background-position…）：静默忽略 + 告警，布局语义不变。
  - **布局语义属性**（display 非 flex/none、position:absolute/sticky/fixed、float、grid、行内混排）：**编译期报错**（打包器/验证器拒收）。行内混排编译错误是"div 只装 flex item"规则的执法。

### 4.2 支持的元素
> v1 实现：`div`/`span`+裸文本/`img`/`button`（见 roadmap §2）。余 v1.x。

| 元素 | 映射节点 | 说明 |
|---|---|---|
| `<div>` / `<l-container>` | Container | 通用 flex 容器，可裁剪/遮罩，可挂 ScrollPane |
| `<span>` / 裸文本 | Text | 叶子文本节点 |
| `<img>` | Image | 贴图 quad（支持九宫格/平铺/填充） |
| `<button>` | Button | 交互按钮（状态：up/down/over/disabled） |
| `<l-rich>` | RichText | 富文本（内联标记） *(v1.x)* |
| `<input>` / `<l-textinput>` | TextInput | 可编辑文本 *(v1.x)* |
| `<l-graph>` | Graph | 矢量形状 *(v1.x)* |
| `<l-loader>` | Loader | 异步加载 *(v1.x)* |
| `<l-movie>` | MovieClip | 序列帧 *(v1.x)* |
| `<l-list>` | List | 虚拟化滚动列表（建在 ScrollPane 上） *(v1.x)* |
| `<l-slider>` / `<l-progress>` | Slider / ProgressBar *(v1.x)* |
| `<l-combobox>` / `<l-tree>` | ComboBox / Tree *(v1.x)* |
| `<l-native>` | NativeHost | 占布局位的原生宿主 *(v1.x)* |

**HTML→Node 映射规则**（解析层）：
- 相邻文本节点合并为一个 Text 叶子（保留 html5ever 空白折叠结果）。
- **行内混排不支持**：元素内"文本+元素+文本"混排（如 `<div>hello <img> world</div>`）、多 `<span>` 拼行内——验证器/打包器**编译期报错**，提示用单个 text 或 `<l-rich>`。不降级处理。
- `<b>/<i>/<u>` 在普通 div/span 内：编译期报错（内联格式化用 `<l-rich>`）。
- Text 叶子是 Container 的子节点之一，flex 正常参与布局。

### 4.3 富文本内联标记子集（`<l-rich>` 内部）*(v1.x)*
对齐 fgui HtmlParser：`<b> <i> <u> <s> <sub> <sup> <br> <font size= color=> <img src= width= height=> <a href=> <p align=>`。颜色支持 `#rrggbb` 单色与 `c1,c2,c3,c4` 四角渐变。RichText 内联排版（含 inline_objects）在文本测量回调内部完成，**不经过 taffy**。

### 4.4 支持的 CSS 属性（设计规范；v1 冻结子集见 roadmap §2）
**布局类（→ taffy）**：`display:flex/none`、`flex-direction`、`flex-wrap`、`gap`/`row-gap`/`column-gap`、`justify-content`、`align-items`、`align-self`、`flex`(grow/shrink/basis)、`width/height/min/max`(px/%/auto)、`padding`、`margin`、`border` 宽度、`position:relative`、`top/right/bottom/left`、`aspect-ratio`、`order`。

**视觉/渲染类**：`background-color`、`background-image`(url)、`background-size`(cover/contain/100%/tile)、`border`(color/width/solid)、`opacity`、`overflow`(visible/hidden)、`color`、`font-size`、`font-family`、`font-weight`、`font-style`、`text-align`、`line-height`、`letter-spacing`、`white-space`(nowrap/normal)。

**交互类**：`pointer-events`(auto/none)。

**九宫格**：`border-image-slice`（Image 九宫格切分，标准 CSS 拼法，AI 熟）。

> `border-radius`/`filter`/`clip-path`/`cursor`/`:focus`/`:nth-child`/属性选择器/`position:absolute`/`align-content` 等为 v1.x（见 roadmap §2 围栏冻结清单）。

### 4.5 状态与控制器 *(v1.x Controller/Gear)*
浏览器伪类 `:hover/:active/:focus/:disabled` 映射内置状态（v1 实现 `:hover/:active/:disabled`）。自定义状态（fgui Controller 多页）用 **`data-page` 属性 + 标准属性选择器**（AI 烂熟 `data-*`）：
```html
<div data-controller="tab" data-page="0"> ... </div>
<style>
  [data-controller="tab"][data-page="1"] .panel { opacity: 0.3; }
</style>
```
Controller 状态变化时，运行时把 `data-page` 写到挂载该 Controller 的元素上，子树用标准属性选择器匹配——cascade 天然生效（§5.3）。带过渡用 `-l-transition: 0.3s ease`（映射 GTween）。

### 4.6 明确不支持（围栏外）
`display:block/inline`、`float`、`position:sticky/fixed`、CSS 动画/transition（用框架 tween）、伪元素 `::before/::after`、`@media`（响应式用 safe-area+百分比+flex）。

---

## 5. 解析与样式层

### 5.1 解析栈
- **HTML 解析**：`scraper`（底层 html5ever，规范级）→ 只读 DOM 树；遍历构造 LoomGUI 元素树。打包器用，运行时不带（feature-gate）。
- **CSS 声明解析**：`cssparser`（Servo）解析 `{ prop: value; }` 声明块（cssparser 必装，scraper 不解析声明）。
- **选择器匹配**：**自写极简匹配器**（~100 行），覆盖围栏内的标签/类/id/后代/子代/`:hover`/`:active`/`:disabled`。不用 selectors crate——围栏选择器极窄，Servo 级通用引擎 + Element 适配器胶水是过度设计。

### 5.2 Cascade 子集（标准 CSS 子集，AI 可预测）
1. **Specificity（标准 CSS tuple a-b-c）**：`inline > id > class > tag`，按元组 `(id数, class数, tag数)` 字典序比较。属性选择器（`[data-page]`）与伪类（`:hover`）同归 class 级（b）。元组大者胜；相同按出现顺序（后者覆盖前者）。与浏览器/AI 先验一致。
2. **属性级合并**：多规则命中同一元素，逐 longhand 取最高优先级值。`flex` 简写按 MDN 展开（`flex:1`→grow=1,shrink=1,basis=0%），与单独 `flex-grow` 同优先级时按出现顺序。
3. **继承**（关键，不实现则 DSL 不可用）：白名单 inherited properties 默认从父继承——`color`、`font-size`、`font-family`、`font-weight`、`font-style`、`line-height`、`letter-spacing`、`white-space`、`text-align`、`visibility`。其余 non-inherited。
4. **不支持 `!important`**：围栏只有 4 个来源，`!important` 几乎没用武之地反增 cascade 复杂度，砍。全部 vanilla CSS，AI 训练数据海量。

**打包期展开继承**：打包器在编译期就把继承展开成每节点的 `base_style` → 运行时零继承开销，二进制包里每节点带基础样式。这是打包器的核心价值。

### 5.3 运行时状态匹配与样式 dirty 机制
`:hover/:active/:disabled` 是运行时状态驱动的伪类，`[data-page]` 是运行时状态驱动的属性选择器（§4.5），匹配时查节点当前状态。机制（朴素）：
- **极简匹配器查状态**：匹配时直接查节点状态（hover=输入命中、active/disabled=节点属性、`data-page`=Controller 当前页）。
- **样式 dirty 档**：DirtyFlags 含 `style` 档（§6.3）。状态变化 → 标受影响子树 style dirty → 重跑 cascade（极简匹配器 + 合并 + 继承展开）→ 置 layout dirty。
- **朴素重算，不缓存**：v1 节点少、选择器窄，全量重算几微秒，不做状态指纹缓存。
- v1 实现 `:hover/:active/:disabled`（按钮交互必需）；`[data-page]`+Controller/Gear 是 v1.x。

### 5.4 CSS 值 → taffy 映射层
cssparser 给 Token，要落到 taffy 强类型 style + 渲染属性。映射规则（实打实工作量，易错）：
- `flex` 简写四种语法展开；`gap/padding/margin/border` 1~4 值展开四向。
- `background-size: cover/contain/100%/tile` → Image 的 `fill_mode`（渲染层，非 taffy）。
- `width: auto` vs `100%` vs `200px` → taffy `LengthPercentageAuto`（auto=内容驱动 MeasureFunc）。
- **margin 不折叠**（flex 语义，对齐 CSS flexbox 规范）：flex item 相邻 margin 求和、不折叠成 max——与浏览器 block flow 不同。围栏文档告知 AI：**子项间距用 `gap`，别用 margin**（margin 控间距在 LoomGUI 与 Chrome 预览表现不同，gap 一致）。
- 围栏外/非法值统一取 taffy `Style::DEFAULT` + 告警。

### 5.5 打包器产物边界（静态 + 动态）
> **必要性**：fgui 用命令式状态设置（无 cascade），但 LoomGUI 因选了 CSS 伪类 DSL（AI 先验）**必须有运行时重匹配**——此动态规则表是 DSL 决策的必要代价，非 fgui 借鉴。

打包器不能把运行时状态伪类编译成 flat style。产物分两部分：
1. **静态编译产物**：节点树结构、每节点继承展开后的 `base_style`、静态资源引用、图集。
2. **动态规则表**：带伪类/状态属性的选择器规则集（`:hover`/`[data-page]`/`:nth-child`），以"规则→属性映射"存，运行时 cascade 叠到 base_style 上。

运行时样式 = `base_style + 命中动态规则的合并`。

---

## 6. 对象模型（场景图）

### 6.1 核心单 Node + 后端原生镜像
fgui 把 GObject（逻辑）与 DisplayObject（渲染，包引擎对象）分两层。LoomGUI：
- **核心只有一个持久 `Node` 类型**：持逻辑状态（布局/样式/变换/事件）+ 几何生成能力。核心不知引擎对象，不需核心侧 DisplayObject。
- **后端有原生镜像对象**（Unity GameObject+MeshRenderer / Godot Node2D+canvas_item）——引擎对象存在的地方，也是 v1.x 特效集成接入点（NativeHost）。
- 核心每帧**产出瞬态 RenderNode 状态**，后端据此同步镜像。

**几何生成分工**：非文本几何（图片 quad/形状/九宫格/填充）在 Rust 核心生成（确定性、跨引擎一致、数据量小）；**文本 mesh 例外——在后端生成**，核心只产 TextLayout，因为动态字形 UV 只有引擎字体 API 才有（见 §9）。

### 6.2 Node 类型层级
> v1 实现：Container/Button/Image/Text。余 v1.x。

```
Node (基类: 变换/尺寸/可见/touchable/事件/sortingOrder)
├── Container        (唯一持有 children，可裁剪/遮罩，批合边界候选；可挂 ScrollPane)
│   ├── Button       (状态: up/down/over/disabled)
│   ├── List         (虚拟化滚动列表，建在 ScrollPane 上) *(v1.x)*
│   ├── ComboBox / Slider / ProgressBar / Tree *(v1.x)*
├── Image            (贴图 quad: 普通/九宫格/平铺/填充)
├── Text             (纯文本)
├── RichText / TextInput / Loader / MovieClip / Graph *(v1.x)*
└── NativeHost       (原生宿主，参与布局/裁剪) *(v1.x)*
```
约束：只有 Container 能拥有 children；叶子不带 children 数组。

### 6.3 Node 核心数据结构
```rust
struct Node {
    id: NodeId,
    parent: Option<NodeId>,
    transform: Transform2D,     // x/y/rotation/scale_x/scale_y/pivot（渲染/命中层，不进 taffy）
    style_size: SizeStyle,      // 用户声明值 (width/height/min/max/flex_basis)（进 taffy）
    measured_size: (f32, f32),  // taffy solve 后写入（只读）
    layout_rect: Rect,          // 父坐标系最终矩形（只读，不含 transform）
    alpha: f32, visible: bool, touchable: bool, grayed: bool,
    color_tint: Color,
    style: ResolvedStyle,       // §4 CSS 子集解析产物
    dirty: DirtyFlags,          // style/mesh/text/layout/batching/outline/transform
    // listeners 在业务侧（C# LoomEventHandler），非核心 Node 字段（v1c.2 §10.2 方向 A：路由降级业务侧）
    children: Option<Vec<NodeId>>,           // 仅 Container
    sorting_order: i32,                      // 绘制优先级（与 children 顺序共同决定等效绘制序，§10.1）
    clip_rect: Option<Rect>,                 // 矩形裁剪
    // v1.x: gears/gear_locked/controller（见 roadmap）
}
```

**关键分层**（命中/动画/布局一致性的根基）：
- `transform`（x/y/scale/rotation）是**渲染/命中层**偏移，**不进 taffy**，改 transform 只置 `transform_dirty`（命中+渲染刷新，不 solve）。
- `style_size`/flex 才进 taffy，改了置 `layout_dirty` 触发 solve。
- 命中几何 = `layout_rect` 经**累计 transform（含父链）**变换后的 AABB。

### 6.4 尺寸模型 → flexbox 映射
| LoomGUI/CSS | taffy |
|---|---|
| `width/height`(px/%) | `size` |
| `min/max` | `min_size`/`max_size` |
| `flex-basis` / `flex-grow/shrink` | 同名 |
| `flex-direction/wrap/gap` / `justify/align-*` | 同名 |
| `padding/border-width/margin` | `padding`/`border`/`margin` |
| `position:relative`+insets | `Relative`+`inset`（视觉偏移，不影响兄弟布局） |
| 内容自适应（文本/图片） | `MeasureFunc` 回调（§7.2） |

### 6.5 生命周期
```
构造（从包反序列化或运行时 new）
  → 注册到父 Container（更新 taffy 树）
  → 改属性 → 置对应 dirty（不立即重算）
  → 每帧 tick：style dirty → 重 cascade；layout dirty → taffy solve；
              transform dirty → 刷新命中几何；mesh/text dirty → 重生成几何/TextLayout
  → 产出 RenderNode → 后端同步镜像
  → Dispose：从父移除、释放纹理引用(refcount)、清事件/tween、后端销毁镜像对象
```
**与 fgui 关键区别**：fgui 改属性立即推 DisplayObject（无 layout pass）。LoomGUI 改属性只置 dirty，每帧统一 solve。**所有布局都是帧末一致**。

---

## 7. 布局层（taffy 集成）

### 7.1 taffy 树与场景图同步
场景图 Container 树 ↔ taffy 节点树一一对应。增删 Container 同步增删 taffy 节点；改 style 同步改 taffy style 并标记子树 layout dirty。

### 7.2 内在尺寸：MeasureFunc
taffy 对"尺寸取决于内容"的节点回调 `MeasureFunc(known_dimensions) -> measured_size`：
- **文本**：调文本测量子模块（§9），给定约束宽返回 `(text_width, text_height)`。必须廉价、无副作用（auto-size/shrink 反复调用）。
- **图片**：原始像素尺寸或声明尺寸。
- **RichText 内联对象** *(v1.x)*：在测量回调内部 query 每个 img/input 的 (w,h) 参与断行（不经 taffy）。**内联对象纯 intrinsic 尺寸**——(w,h) 来自声明 px 或纹理像素，不得用 `%`/`flex`（否则测量回调死循环），带 `%`/`flex` 是编译期错误。异步纹理加载不触发重布局。

### 7.3 响应式与异形屏
- **resize**：屏幕尺寸变 → 根 taffy 节点 size 变 → 整树 solve。
- **safe-area**：后端把 insets 注入核心；CSS 用百分比 + `-l-safe-area` 环境变量表达避让。
- **动态内容/数据变化**：改文本/增删子节点 → 置 dirty → 下帧 solve。

### 7.4 参考分辨率 / DPI 缩放
商业游戏标准：设计稿 1080×1920 在 1440×2560 整体等比放大。百分比+flex 解决相对布局，解决不了"整张 UI 在大屏上多大"。
- Stage 持 `design_resolution` + `match_mode`。后端注入屏幕尺寸 + safe-area，核心算 scale + 根 size。
- **整体缩放**：根 Stage 一个 scale，整树缩放（不逐节点）。
- v1 用 MatchWidthOrHeight（最常用）；MatchWidth/MatchHeight、高清资源分支(scaleLevel) v1.x。
- **叠加顺序**：先参考分辨率整体 scale → 再百分比/flex 布局 → 最后 safe-area 避让。

### 7.5 布局时机
运行时算。每帧只在 dirty 时 solve；taffy 支持请求子树布局。布局结果供渲染与命中消费。

---

## 8. 渲染层（自绘，渲染树契约）

> **核心原则（契约意图化）**：渲染树契约描述**渲染意图**（画什么/遮罩意图/绘制顺序），**不规定**引擎实现机制。后端各自选择：Unity 用 stencil/Material/GameObject，Godot 用 canvas_group/CanvasItem/z_index。引擎字眼只出现在 §14 Unity 后端章节，不进契约。

### 8.1 坐标系（核心唯一真相源）
- 核心统一**左上原点、y 向下**。**核心代码不出现任何 `height-y` 翻转**。
- 翻转是**后端根 Stage 一次性 y-flip 变换**：Unity 根 GameObject 挂 (1,-1,1) scale（LoomGUI 自选；比 fgui 逐节点 `y=-y` 取负更干净——只翻一次；副作用：winding 反转 → Unity shader 须 `Cull Off`）；Godot flip 矩阵=单位矩阵（2D 本就左上 y 下）。
- 后端镜像时所有坐标都在核心坐标系下，由根 Stage 统一翻转，不在 mesh/输入/命中分别翻转。

### 8.2 几何生成：VertexBuffer + MeshFactory（在核心）
- `VertexBuffer { verts, uvs, uvs2, colors, indices }` + 输入 `content_rect/uv_rect/vertex_color/texture_size`，对象池化。
- `trait MeshFactory { fn on_populate_mesh(&self, vb: &mut VertexBuffer); }`，各形状实现：矩形/九宫格/平铺/进度填充/多边形(Ear-clipping)/椭圆/圆角矩形/折线/组合。
- 基础方法：`add_vert/add_quad/add_triangles/append/insert/repeat_colors/generate_outline/generate_shadow`。
- **rotated 纹理 UV 修正**（图集旋转打包用）：`new_y = y_min + uv.x - x_min; new_x = x_min + y_max - uv.y`。
- **非文本 mesh 由核心生成、跨 FFI 传后端**，后端上传。**文本节点例外**：核心只产 TextLayout，文本 mesh 后端据 TextLayout 光栅化+拼 quad（§9）。

### 8.3 纹理：TextureView（去引擎化）
```rust
struct TextureView {
    root_tex: TexId,           // 纹理 id（引擎上传后返回）
    alpha_tex: Option<TexId>,
    region: RectPx, offset: Vec2, original_size: Vec2, rotated: bool,
    uv_rect: Rect, ref_count: i32,
}
```
- 图集：一张大纹理(root) + N 个轻量 TextureView（只存 UV）。子 view 首引用连带 root；归零 `on_release` 通知后端可卸载。
- 核心只持 `TexId`（整数）；GPU 生命周期全在后端。

### 8.4 DrawState 语义（去 Unity 化，不实例化材质）
核心不算材质对象，只算 draw 所需状态：
- `DrawFlags`(u32)：`Clipped|Grayed`（+ v1.x: SoftClipped/Masked/AlphaMask/ColorFilter）+ 用户 keyword 高位。
- `BlendMode`：照搬 fgui src/dst 因子表概念，v1 实现基础几种（Normal 等）；blend 作为 draw state，不编进 shader variant。
- `ProgramId`：Image/Text（+ v1.x: BMFont/自定义）。
- 后端按 `(program+flags+blend+texture+mask_context)` 维护 **DrawState 缓存**（Unity 侧等价 MaterialManager）。

### 8.5 批合：FairyBatching（保序重排）
两元素能并入同批 ⟺ **DrawState 相同**（AABB 不相交则可重排聚拢；同 DrawState 相交仍可合，合并后 index buffer 保相对绘制序——v1b.4 坑 23）。
- 算法照搬 fgui `DoFairyBatching`：稳定插入排序 + AABB 重叠检测，只在无视觉歧义时把同 DrawState 元素前移。
- 核心算每节点 `sort_key`（重排后绘制顺序），后端据此设该引擎的排序字段（Unity sortingOrder / Godot z_index）。
- **批合边界结构强制**（照搬 fgui）：DFS 遇 `clip_rect` 的 Container **强制其为 BatchingRoot**；批合收集**不下钻**进 root 子树（root 子树独立成批）。批物理上跨不过裁剪边界。（v1.x: shape mask/paintingMode 也是 root 边界。）
- 批合局部（每 BatchingRoot 独立）。**v1b.4 起 core 合并 mesh**：`reorder_for_batching`（fgui DoFairyBatching core 化，同 DrawState 不相交元素聚拢）+ `merge_meshes`（连续同 DrawState Mesh→单 merged payload）→ 真 N→1 draw call。merged transform=0/alpha=1 让 blob re-base+alpha 烤对 merged 无效（blob/MirrorPool 零改，spec §9）。锚 node_id（batch 内 min）解动画 GO 抖动。fgui 本身不合并（靠 Unity Dynamic Batching 隐式），LoomGUI core 显式合并补之。

### 8.6 裁剪/遮罩（意图表达，机制后端自选）
v1 实现 **rect mask（硬矩形裁剪）**：意图=矩形区域裁剪。核心给 clip_box；后端自选实现（Unity: shader uniform `|clipPos|>1` discard；Godot: canvas_item_set_clip；软件: scanline）。`mask_context`（rect clip 上下文）是批合边界（§8.5）。**嵌套 `overflow:hidden`**：clip 区域取**祖先 clip 链的交集**（核心 DFS 累积交集，每 clip 上下文绑定一个交集后的 rect；后端每 context 一个 clip uniform，照搬 fgui 折叠语义——非逐层独立裁剪）。

soft clip（羽化）/shape mask（形状遮罩，含 Write/Content/Erase 时序）/paintingMode（离屏 RT）是 **v1.x**（机制见 roadmap）。

### 8.7 RenderNode 契约（公共头 + enum payload，意图化）
```rust
struct RenderNode {
    // —— 公共头 ——
    node_id: NodeId,
    parent_id: Option<NodeId>,
    visible: bool,
    alpha: f32, grayed: bool,
    color_tint: Color,
    transform: NodeTransform,      // 本地变换 + pivot 偏移
    blend: BlendMode,
    mask_context: MaskContext,     // rect clip 上下文（v1）
    sort_key: u32,                 // FairyBatching 重排后绘制顺序
    // —— 按类型分叉 ——
    payload: NodePayload,
}

enum NodePayload {
    Unchanged,                                                 // 本帧不传，后端沿用上帧
    Mesh    { mesh_ref, texture, alpha_tex, program, flags },  // 非文本自绘（九宫格在 mesh 里）
    Text    { layout_ref, font, program, flags },              // 文本：后端据 TextLayout 生成 mesh
    // v1.x: Mask { shape_ref, mode } / PaintTarget { rt_id } / NativeHost —— 见 roadmap
}
```

**关键约定**：
- **九宫格**：核心九宫格 MeshFactory 生成 16 顶点 mesh，作为普通 Mesh payload。
- **Unchanged**：不 dirty 的节点用此变体，不进 arena；后端见 Unchanged → 不动该 **node_id** 的渲染对象。Unchanged 是独立变体（非 dirty_bits 位），enum 只留真实 payload 类型。
- **Text 节点的 text dirty**（防静默陈旧文本）：DirtyFlags 含独立 `text` 位。Text 节点发 `Text` 变体当且仅当 `text_dirty || mesh_dirty`——**box 尺寸不变不算 Unchanged**（"10"→"09" 同宽仍必发 `Text` 重光栅化）。`set_text`/font 变化置 `text_dirty`（级联 layout_dirty+mesh_dirty）。
- **NodeTransform**：本地变换 + pivot 偏移。（v1.x: 加 `VertexMatrix` 支持透视/世界空间 UI。）

后端每帧：diff `render_nodes` 与镜像池（按 node_id 增删复用）→ 同步对应 payload（Mesh 上传 mesh、Text 据 layout 生成 mesh）→ 设 transform/排序/遮罩/blend。

### 8.8 绘制顺序
单一全局递增计数器 `rendering_order`，每帧重置，DFS 中"分配即自增"。批合区内不分配，等 BatchingRoot 按重排后顺序统一分配。最终顺序 = 树序 × 批合重排 × 裁剪边界。（v1.x shape mask 的 Write/Content/Erase 时序 + 两遍 DFS sort_key 规则见 roadmap。）

> **契约演化**：当真有第二个契约版本时再加版本字段/扩展列机制——现在不加（无 v2 契约）。FFI 演进 = 编辑 Rust struct + 重新 csbindgen，无在线扩展协议。

---

## 9. 文本（ttf-parser + unicode-linebreak，测量在核心）

### 9.1 测量与渲染分离（一致性根基）
- **Rust 核心拥有测量 + 断行**（确定性，跨引擎一致）：ttf-parser 取真实度量（`hhea`/`os2` ascent/descent/line-gap，**不照搬 fgui `fontSize*1.25` 估算**）+ unicode-linebreak（换行机会，CJK 逐字）。
- **文本 mesh 在后端生成**：核心产 TextLayout，后端用引擎字体 API 光栅化产 UV、按 TextLayout 位置拼 quad mesh。
- **advance/断行/box 尺寸一律以 Rust 为准**（跨引擎一致），仅字形 UV/光栅化在引擎侧。
- **字体资产契约**：包内声明**逻辑字体名 + 度量源 ttf**，核心用此 ttf 算度量；**各后端用同一 ttf 光栅化**（Unity 加载进字体系统、Godot 加载进 DynamicFont）。font_id 是逻辑 id。
- **换行/white-space 原则**：`white-space:normal/nowrap` 生效，换行以核心（unicode-linebreak，CJK 逐字）为准。对齐 Chrome 行为是目标（含 CJK 行首/行尾标点约束），具体 kinsoku 配置实现期对照 Chrome 调，本文不钉死算法。
- 复杂 shaping（rustybuzz 连字/合字）+ BiDi（unicode-bidi, RTL）+ 字体 fallback 链：v1 砍（亚洲/国内首发）。

> **v1 文本简化假设与已知代价**（CJK+emoji→tofu、组合符号→错位、RTL 不支持）、**v1.x 跨引擎归一化契约升级**（Godot 接入时定 advance/metric 权威、关 hinting）见 `docs/roadmap/v1x-deferred.md`。

### 9.2 TextLayout 产物（SOA 三表，跨 FFI）
```rust
struct TextLayout { text_width: f32, text_height: f32, lines: Vec<Line> }
struct Line { y, height, baseline, width, runs: Vec<GlyphRun>, inline_objects: Vec<(x,y,w,h,obj_id)> }
struct GlyphRun { font_id, font_size, format, glyphs: Vec<Glyph> }
struct Glyph { glyph_id, codepoint, x, y, bearing_x, bearing_y }   // v1：绝对坐标 + bearing；codepoint 供引擎字体 API（Unity GetCharacterInfo 按 char 取），glyph_id 供 ttf 直连后端
```
- **glyph 存绝对坐标**（核心已累加 advance、已应用 text-align 偏移），后端拼 quad 零累加：`quad_min = (glyph.x + bearing_x, glyph.y + bearing_y)`，再按光栅化字形像素边界扩展。`advance` 是核心内部 pen 推进值，**不进 FFI 表**。
- `bearing_x/bearing_y` = pen 位到字形 quad 左上的偏移（字形 left/top bearing，来自 ttf-parser glyph bbox）。
- `font_id` per-run（v1 单 run 单字体；v1.x emoji fallback 时升 per-glyph）。
- **垂直度量**：`Line.height`/`baseline` 由核心按 CSS 语义算（`line-height` 生效并烤进 `Line.height`，对齐 Chrome）；后端只按 `line.y`/`line.baseline` 摆字形，**不得自己再套 line-height**。

**跨 FFI 时 SOA 三表化**（§14.3）：`glyphs_soa[]`（每项=glyph_id/x/y/bearing_x/bearing_y）、`runs_soa[]`（每 run=glyph 起止+font_id+font_size+format）、`lines_soa[]`（每行=run 起止+y/height/baseline/width）。Text payload 带六个 u32 指向三表切片。富文本内联对象加第 4 张 `objects_soa[]`（v1.x）。

### 9.3 测量的可重入性
auto-size/shrink 反复测。`measure(known_dimensions)` 必须廉价、无副作用、可被 taffy 反复调用。测量与渲染用**同一套字体度量**（同一 ttf）。

### 9.4 字体资产
- **位图字体**进包（字形 atlas + 字形表/UV）。
- **动态字体**不进包，运行时全局注册或从引擎字体资源加载（必须用包声明的同一 ttf）。核心定义 `Font` trait。

---

## 10. 事件与输入

### 10.1 命中测试（核心拥有，引擎无关）
核心消费布局结果做命中。输入 stage 坐标点 →
1. `world_to_local`：用**累计 transform（含父链）**的逆矩阵把点投到本地（不用裸 layout_rect）。
2. `visible && touchable` 门控。
3. 裁剪：有 `clip_rect` 必须包含；有 `hit_area`（trait，Rect/Shape/PixelMask）则委托。
4. **子节点按"等效绘制顺序"逆序遍历**（顶层优先），第一个命中即返回。
5. 容器自身 fallback：`opaque && content_rect.contains(point)`。

**等效绘制顺序**（避免视觉/命中错位）= children 顺序经 `sorting_order` 重排后的结果，非 children 原序。`sorting_order` 非零的子节点排在前面（顶层）。
- 结果按帧号缓存，有效期到下帧 tick 开始（事件回调中改 DOM 不立即刷新命中，避免反馈环）。
- 命中几何 = `layout_rect` 经累计 transform 变换后的 AABB（动画中的元素命中正确）。

### 10.2 事件路由（DOM 三阶段）
> **实现位置（v1c.2 方向 A）**：路由在**业务侧（C# `LoomEventHandler`）**，非核心。核心只保留命中（§10.1）+ 命中 diff（hover/active 状态 + RollOver/Out 产出）+ 伪类 rematch。路由/listener 是业务 UI API（Godot 后端用 GDScript 重写）。语义照 fgui `EventDispatcher`（`temp/FairyGUI-unity/.../Scripts/Event/`）。
- `dispatch(target, type)`：目标直派（单节点 capture+bubble 回调，不沿链；RollOver/Out、focus/dragMove/sizeChanged）。对齐 fgui `DispatchEvent`。
- `bubble(target, type)`：**capture(链反向，全跑) + bubble(链正向，可 stop)**；`StopPropagation` 在 bubble 阶段中断（capture 不检查 stop，照 fgui）。Down/Up/Move/Click 用此。对齐 fgui `BubbleEvent`。
- `broadcast(root, type)`：子树深搜（added/removedFromStage）。**defer v1.x**（无 added/removed 事件，无消费者）。
- listener 表在业务侧（C# `Dictionary<nodeId, Dictionary<EventType, EventBridge>>`），EventBridge = capture + bubble 两组多播回调；remove 用**委托引用**（非 ListenerId）；EventContext 对象池（target/currentTarget/phase + StopPropagation/PreventDefault）。
- **核心↔业务侧边界**：核心产 target 事件（`EventRecord{node_id=target, type, x, y}`）+ RollOver/Out 多目标 diff；业务侧沿 `node_parent` 链路由。`EventRecord` 零改（业务侧按 type 分流 bubble/直派）。

### 10.3 指针路由 / 触摸捕获 / 点击判定
> **v1c.3 实现状态（@ 2beb6e7）**：多触摸槽 + CaptureTouch/touch monitor 已实现（core 5 槽 `TouchSlot` + EventRecord/PointerEvent 加 `touch_id` + active/hovered 全局 union recompute + Move 对齐 fgui 无 monitor 不产）。**偏离本节设计**：鼠标+触摸**同帧共存**（非互斥，鼠标 touch_id=-1 绕开 fingerId 冲突）；capture 仅 cap/bub 各加一 monitor（消费即清，照 fgui，非「多 monitor 共存」的字面 N 个）；click 判定**沿用 v1c.2 简化**（down_node==hit && <10px），本节「Move 超阈值取消/双击/downTargets 兜底」**defer v1c.4**。详见 spec `docs/superpowers/specs/2026-06-23-v1c.3-multi-touch-design.md`。
- 多触摸槽（支持 N）：`target / down_targets 链 / touch_monitors / click 状态`。
- **capture_touch 多 monitor 共存**：一个触摸可有多个 monitor，move/end 派发给所有 monitor（照搬 fgui）。手指移出仍持续收事件。
- **Click 判定**（照搬 fgui）：距离按 Stage 绝对坐标算（阈值鼠标 ~10px/触摸 ~50px）；**Move 中超阈值即取消 click**（拖拽 100px 再拖回不触发）；双击 350ms 窗口；down_targets 链断裂时沿当前 target 祖先链找兜底节点派发。
- RollOver/Out：每帧命中后 diff 一次（非每 move），布局 solve 引起的位置变化**不触发** hover diff（v1c.3 实现照此：Stationary 不刷新，元素动后 hover 不跟随，defer v1c.4）。

### 10.4 拖拽 / 焦点 / 手势仲裁
- **节点级 draggable** + **ScrollPane 滚动** 都要 capture 同一触摸——**仲裁**（照搬 fgui）：各自定义手势阈值（滚动 ~20px、拖拽 ~10px），未达都 return、click 照常；达阈值那一刻**先达者赢**，另一方查全局 `dragging_node`/`scrolling_pane` 主动退让；垂直滚动列表里的水平拖拽，比较位移量决定归属。
- 拖拽：超阈值触发 `onDragStart`（可 prevent_default），`drag_bounds` 局部 clamp，全局 `dragging_node` 单例。
- 焦点：`Stage.focused: Option<NodeId>`，`focusable/tab_stop`，Tab 导航深搜。

### 10.5 引擎输入桥
核心定义 `InputProvider` trait（指针/键/触摸/IME character），后端实现并每帧注入。坐标核心**左上原点**；翻转在后端根 Stage 一次性做。**IME 组合字符从引擎文本输入事件拿，不是按键码**。

### 10.6 UI 输入消费（is_pointer_on_ui）
游戏第一天就撞的墙。**极简**（对齐 fgui）：核心命中后存当前指针命中的 NodeId，暴露事实查询：
```rust
stage.is_pointer_on_ui() -> bool   // = 命中目标非空且非根
```
不做消费策略/consume 标志/每指针数组。游戏自己在输入管线查此 bool。`pointer-events:none` 控制节点参不参与命中，不是消费与否。

---

## 11. 动画与状态（单时钟）

> **原则**：整个核心只有一个动画时钟 `TweenManager::update(dt)`。Controller/Gear/Transition 都不自建 update，全是"事件→往 TweenManager 提交/kill tweener→tweener 回调写节点属性"。v1 实现 GTween + Timers + ScrollPane 物理；Controller/Gear/Transition 是 v1.x（机制见 roadmap）。

### 11.1 GTween（补间引擎，唯一时钟）
- `TweenManager { active, pool }`，池化。
- `Tweener`：统一 `TweenValue{x,y,z,w,d}` + `value_size(1..6)`（float/Vec2/3/4/Color/double；6=shake）。
- 链式 builder：`tween(start,end,dur).delay().ease().repeat(,yoyo).on_complete()`。
- 缓动：Linear/Sine/.../Elastic/Back/Bounce 的 In/Out/InOut + Custom，`EaseManager` 纯函数（Penner 方程）。
- 特殊：`DelayedCall`、`Shake`、`SetPath`(贝塞尔)、`smoothStart`(吸收首帧大 dt)。
- **prop_type 分层**（关键）：tween 写属性区分 "transform 属性"（x/y/scale/rotation，置 `transform_dirty`，不 solve）vs "layout 属性"（width/height/flex，置 `layout_dirty` 触发 solve）。位置/缩放动画走 transform 不触发 solve。

### 11.2 Transition（时间线 = 编排器，不自驱）*(v1.x)*
纯数据 `items: Vec<TransitionItem>`。`Play()` 把每个 item 翻译成 Tweener 提交 TweenManager。两点关键帧；多关键帧靠多 item 串行。嵌套 Transition 递归 + 完成回调递减父计数。

### 11.3 Controller（状态机，纯状态）*(v1.x)*
`Controller { selected_index, page_ids, ... }`。`set_selected_index` 改 index + 扇出 + 派发 onChanged + 置子树 style dirty（触发 §5.3 重匹配）。Controller 不直接改 UI 属性，全靠 Gear + 样式重算。

### 11.4 Gear（状态→属性映射）*(v1.x)*
每节点多个 Gear（Display/Xy/Size/Look/Color/FontSize...），存储 `HashMap<page_id, Value>`。`Apply` 查当前页值 → kill 旧 tween → 往 TweenManager 提交插值 tween，`on_update` 写回。reentrancy 守卫（同步同栈帧 set→write→clear，防 set_property→update_gear→UpdateState 回写污染）对齐 fgui `GearXY`。详细机制（含 `gear_locked`、Controller 扇出）见 roadmap。

### 11.5 Timers
独立通用周期/延时回调（unscaled_dt），与动画解耦。`CallLater`（下一帧）、`AddUpdate`（每帧）。

---

## 12. 资源 / 包系统

### 12.1 双格式
- **编辑期/源**：HTML（结构）+ CSS（样式）+ 资源清单。
- **发布产物**：编译成**单一二进制 blob**（`.pkg.bin`）。体积压到 XML/HTML 的 1/3~1/5、加载无需解析器、少分配。
- 运行时**只认二进制**（含热重载：重编译 DSL→二进制再热重载）；HTML 解析只在打包器/编辑器。
- **二进制包由打包器 `loomgui_pkg` 产出**（构建期工具，复用核心 parse/style 层）。

### 12.2 二进制包格式（借鉴 fgui _fui）
- Header：**formatVersion** + 魔数 + compressed flag。
- 头部 indexTable + `Seek(blockIndex)` 块跳转：组件描述分块，运行时只读需要的块。
- 全局 stringTable + `ReadS(ushort)` 下标：字符串去重。
- 跨资源引用统一 URL（`loom://pkgName#resId`），存 id 不存内容。
- **版本协商**：Header `formatVersion` + runtime 声明 `min/max_supported_version`。新 runtime 读旧包按 `formatVersion` 内联兼容（对齐 fgui `buffer.version >= N` 模式）；集中式迁移器链待多版本累积后再上。同 Stage 不允许混载不同 major version 包。

### 12.3 图集
散图 → 图集 → root TextureView + 子 TextureView（只存 UV）。`rotated`/`trim+originalSize+offset` 打包期记录、运行时还原。
> **图集是刚需**（同图集的图才能批合，散图每张一个 draw call）。打包器内置图集打包（散图→大图 + AtlasSprite 表），算法用简单矩形打包（shelf/guillotine），够用即可。

### 12.4 引用计数与生命周期
- `TextureView` 自带 `ref_count`，子视图首引用连带 root。
- 渲染组件换纹理自动 AddRef/ReleaseRef。
- 归零 `on_release` 冒泡到资源项 → 通知后端资源管理器卸载。
- `UnloadPolicy`（Destroy/Unload/Custom/None）；`Reload`（卸 native、留壳）低内存必备。

### 12.5 加载与实例化管线（三层分离）
1. `load_package`：只解析描述、建资源项索引（快、可常驻）。
2. `get_item_asset`：按需加载，按类型分发，同步/异步；加载器抽象成 trait，后端注入。
3. `create_object`：工厂 NewObject + 递归 `construct_from_resource`。
- **异步实例化**（大 UI）：先拍平成 `DisplayListItem[]`，再分帧逐项 NewObject + 对象池回填。

### 12.6 扩展机制
照搬 fgui `SetPackageItemExtension`：包内某组件可由用户 Rust struct / 引擎类接管实例化。

---

### 12.7 滚动容器（ScrollPane）
游戏 UI 里可滚动容器远多于虚拟化长列表，移动端要惯性/回弹/分页/吸附。

**模型**：Container 有"可滚动"模式（挂 ScrollPane，非新节点类型）。ScrollPane 持 `content`（子树）/`viewport`（可视矩形）/`scroll_type`(H/V/Both)/`scroll_pos`（偏移）。
- taffy 算 content 总尺寸；视口 = Container measured_size；`scroll_pos` 是 content 根的 transform 偏移（不重布局，只平移）；视口裁剪 = Container clip_rect。
- **惯性回弹物理**：**不走 GTween**（content 异步变化时 GTween 的固定 end 会跳变）。ScrollPane 自维护可变 target 的 tween，content size 变化时按状态补偿 start、不突变。tick 时机 = §15 (2b')，紧随 GTween、在 style/layout 前，保证本帧 scroll 偏移进 transform 与命中。**禁止 GTween 直接 tween `scroll_pos`**（API 层挡，避免双写）。
- v1 能力：滚动类型、惯性+回弹、滚动条、鼠标滚轮。分页/吸附/下拉刷新、虚拟化 `<l-list>` v1.x。

---

## 13. 动态 UI / 数据模型

### 13.1 命令式节点 API
```rust
let c = Container::new();
c.add_child(img); c.remove_child(img); c.set_child_index(...);
node.set_text(...); node.set_position(...); node.set_style(...);
node.add_event_listener(Click, cb);
```
所有操作只置 dirty，帧末统一 solve + 重生成几何。

### 13.2 数据驱动的列表虚拟化（`<l-list>`）*(v1.x)*
建在 ScrollPane 上：核心维护固定数量可视槽（item index → slot），后端按 slot 复用渲染对象（不销毁重建，零 GC）。两身份正交：NodeId=逻辑身份（事件/命中），slot=渲染复用身份。**slot 复用的核心不变量**（slot 换内容时必发真实 payload 非 Unchanged，防花屏）与 reuse_key 机制见 roadmap。

### 13.3 数据绑定
命令式 API + 数据驱动列表为主。声明式绑定（`data-bind:text="user.name"`）后期加。挂在好的场景图上，后加不痛。

### 13.4 响应式重布局
所有动态变化（resize/safe-area/数据变/增删节点）→ 置 dirty → 下帧 taffy solve。

### 13.5 性能对策
- 别每帧重建整棵 DSL；传结构化增量。
- 只 relayout 变化子树。
- 缓存：命中按帧、DrawState 按 key、mesh 按 dirty、渲染对象镜像按 node_id 复用池。（v1.x 虚拟列表按 slot_id 复用。）

---

## 14. FFI 与 Unity 后端

### 14.1 方案：csbindgen
csbindgen 是为 Unity/IL2CPP 设计的主流绑定生成器（Cysharp MagicPhysX/NativeCompressions 全平台验证）。
- Rust 端 `#[no_mangle] extern "C"` + `csbindgen` 生成 C# `[DllImport]`。
- `csharp_use_function_pointer(false)` 切 Mono 模式（IL2CPP 友好）；`csharp_dll_name_if` 处理 iOS `__Internal`。
- `[GroupedNativeMethods]` context 指针模式适合"持有 Stage 句柄"。

### 14.2 IL2CPP 必须注意的坑
- **回调必须 `static` + `[MonoPInvokeCallback]`**（instance delegate 直接崩）。影响 Rust→C# 回调（事件）。
- **iOS**：静态库 + `[DllImport("__Internal")]`。
- **string 永远走 UTF-8 `byte*`**。
- **内存所有权严格隔离**：跨边界传 POD/指针/扁平 buffer。
- 高频调用控制 marshalling：用扁平数组（pin 或拷贝）。

### 14.3 跨边界数据与内存模型
**一块 SOA 公共头 + 多个按类型分区的 per-frame arena，C# tick 内拷完**：
```
每帧 FFI 传：
1. RenderNode 公共头 SOA（定长字段并行存储）：
   node_ids[], parent_ids[], visible[], alpha[], grayed[],
   color_tints[], transforms[], blends[], mask_contexts[], sort_keys[],
   payload_kinds[], payload_arena_idx[], payload_offsets[], payload_lens[]
   —— 定长。(arena_idx, offset, len) 三元组定位 payload 在哪个 arena 的哪段。
   Unchanged 节点 payload_kinds=Unchanged，三元组为空。
2. 多个按类型分区的 per-frame arena（变长 payload，每种一个 arena）：
   mesh_arena   : 扁平 verts[f32]/uvs[f32]/colors[u32]/indices[u16] + count
   text_arena   : v1a 扁平 glyphs[{codepoint,pen_x,pen_y}] + 节点级 font_size/color（单字体简化；CJK 逐字断行 v1b.5 已落地 via unicode-linebreak，但 runs/lines 三表随多字体 fallback 仍 v1.x defer，§9.2）
   —— 每种 arena 一种结构，C# 按 payload_kind 选解析器。
```

**坐标空间**：SOA `transforms[]`（local_x/local_y）与 clip rect 均为**绝对 design 坐标**（核心 layout 累加 parent origin）。后端不做逐节点 parent 累加——根 Stage transform 一次性映射 design→world（§8.1）。

**内存所有权**：公共头 SOA + 各 arena 都是 Rust 侧 per-frame。**公共头 + 所有 arena 在 tick 返回前由 C# 原子拷贝到托管 buffer**（拷贝而非 pin）；tick 返回后 Rust 即可 reset，C# 后续只读自身拷贝。Rust 下帧开头 reset arena（复用零分配）。**"沿用上帧"**：不 dirty 节点 payload=Unchanged，不进 arena，后端不动该 node_id 的渲染对象。

**读取约定**：任何变长 payload 拍平成扁平 SOA，**禁止嵌套结构跨 FFI**。每变体 byte 布局定死（写进契约附录）。**C# 用 `Span<byte>` + `BinaryPrimitives` 读，禁用 `Marshal.PtrToStructure`**（IL2CPP struct 对齐坑）。**绝不跨 FFI 传裸指针**。

**C# buffer 大小策略**（v1，详见 roadmap）：每帧 payload 大小变（静态帧≈只 header、冷帧/换页帧满载），用 `ArrayPool<byte>.Shared.Rent(本帧实际大小)` 池化租用，用完 `Return`——零 GC、无 worst case 常驻。预算：单帧 FFI 拷贝 + arena 解析 ≤ 2ms @ 500 节点全 dirty。

**其它跨边界数据**：Stage 句柄（C# 持 opaque `IntPtr`）；输入事件（扁平数组）；回调（static + MonoPInvokeCallback）；纹理（核心只认 TexId，C# 上传后注册 id↔Texture2D）。

> FFI 传的是**完整渲染树**（SOA+arena，含全部状态），不是"只传 NodeId"。Rust 不持/不解引用任何 Unity 对象，跨 FFI 只传整数 id + 数据 buffer。

### 14.4 Unity 后端职责
1. MonoBehaviour 驱动：每帧 `set_input` + `tick(dt)` → 取 `render_nodes` → 同步镜像。
2. **GameObject 镜像池**：`node_id → GameObject`，diff 渲染树增删复用；每节点 `MeshFilter+MeshRenderer`。
3. **同步**：上传 mesh 到 MeshFilter（非文本）；文本据 TextLayout 光栅化+拼 quad；按 `(program+flags+blend+texture+mask_context)` 从 DrawState 缓存（MaterialManager）取/建 Material；设 transform、sortingOrder、blend/stencil、clip uniform。rect 遮罩用 shader uniform `_ClipBox` discard（§8.6；shape mask 才用 stencil）。
4. 输入采集：Unity 新/旧输入系统 → 扁平事件（含 IME character）。
5. 资源加载：Addressables/YooAsset → 纹理上传 → 注册 TexId。字体用包声明的同一 ttf。
6. 坐标：根 Stage GameObject 挂 (1,-1,1) scale 一次性 y-flip。blob 的 local_x/local_y 与 clip_rect 是绝对 design 坐标（§14.3），**v1a/Phase 2 渲染对象全部挂根 GO、localPosition=绝对**（flatten，避免巢状 SetParent 双计父位置）；GO 层级巢状 + 父相对坐标留待 transform 继承需求（v1c+ 事件 / 旋转缩放容器）。

> Unity 后端的 `MeshFilter+MeshRenderer+MaterialManager+sortingOrder+stencil` 是 §8 契约的**一种实现**，几何数据来自核心，后端不生成非文本几何。
> v1.x: NativeHost（放用户 GameObject，tick 前 push 尺寸）、世界空间 UI、slot 复用池——见 roadmap。

### 14.5 构建管线
- Rust 交叉编译产出多平台原生库（`.dll`/`.so`/`.dylib`/iOS `.a`/Android `.so`）。
- 放 Unity `Plugins/`，配 Platform/CPU。
- csbindgen 生成 C# 绑定源码纳入 Unity 工程（单独 asmdef）。
- Unity Domain Reload 保护：`[RuntimeInitializeOnLoadMethod(SubsystemRegistration)]` 重置 native 状态。

### 14.6 渲染对象镜像的生命周期与性能
**所有权**：Rust 核心拥有场景图 + 渲染状态（真相源）；后端拥有渲染对象镜像（派生缓存）。Rust 绝不创建/销毁引擎对象。
- **每帧脏增量同步**：后端维护 `Dictionary<NodeId, RenderObject>`。每帧：(1) 池中对象置 stale；(2) 遍历 render_nodes，按 node_id 查池——命中清 stale 并按 payload 更新/Unchanged 跳过、未命中新建；(3) 仍 stale 的销毁。**O(n) 每帧**，禁 O(n²)。静态 UI 每帧同步≈0。
- 真正每帧开销是引擎自身遍历渲染对象做剔除/批合/提交——靠 DrawState 复用 + FairyBatching 缓解。纯 2D 重 UI 不够 → 升级 SRP 混合。
- **回收**：节点 Dispose → 下帧不在渲染树 → 后端按 node_id 销毁镜像（或核心发"已移除列表"立即清理）。（v1.x 虚拟列表按 slot 复用，不销毁重建。）
- **无 double-free/use-after-free**：Rust 只持整数 id，从不解引用引擎对象。

---

## 15. 更新循环（每帧管线）

```
引擎 update:
  1. set_input()                       ← 后端采集指针/键/触摸/IME，扁平数组注入
  2. stage.tick(dt):
     a. Timers.update(dt)
     b. TweenManager.update(dt)        ← GTween 推进；tweener 回调写节点属性（置 transform_dirty/layout_dirty）
     b'. ScrollPane 物理 update        ← 自维护可变 target tween（§12.7，不走 GTween），写 content 根 transform
     c. style dirty → 重 cascade → 置 layout dirty
     d. layout dirty → taffy solve（子树）→ 写 measured_size/layout_rect（含文本 MeasureFunc）
     e. transform_dirty → 刷新命中几何
     f. 命中测试（按帧缓存）→ 命中 diff（hover/active 状态 + RollOver/Out 产出）→ 伪类 rematch
     g. 渲染 pass: DFS 整树
        - mesh dirty → 重生成几何；text dirty → 重测+重产 TextLayout
        - 累积 alpha/grayed/裁剪上下文（save/restore）
        - FairyBatching 重排 → 分配 sort_key → 收集 RenderNode
     h. 输出 Vec<RenderNode>（按 sort_key 排序）
  3. 后端消费 render_nodes → 同步镜像；borrow_events → 事件路由（capture+bubble，§10.2 业务侧）→ 业务回调 → 提交渲染
```

**关键**：
- **事件回调里改的布局属性延迟到下帧 solve**——不在当前帧重 solve（避免"布局→事件→布局"反馈环）。事件触发的布局变化只置 dirty。
- **命中语义**：本帧输入在 (2f) 命中测试用 (2d) 本帧刚 solve 的布局——即**输入命中当前帧布局**。代价：事件回调内改的布局延下帧 solve，故同帧内事件回调移动的节点不影响本帧后续命中（缓存到下帧 tick 开始有效）。有意如此，避免反馈环。
- transform 改动不触发 solve（仅 transform_dirty，e 阶段刷新命中几何）。动画改 transform 每帧廉价；改 layout style 才 solve。

---

## 16. 代码结构（Rust workspace）

```
loomgui/
├── loomgui_core/           # 引擎无关核心（纯库，可单测）
│   ├── parse/              # HTML(scraper) + CSS(cssparser) + 极简选择器匹配器 —— feature 'parse'，运行时不带
│   ├── style/              # cascade（继承/合并/顺序）+ CSS值→taffy映射 + 运行时状态查询
│   ├── layout/             # taffy 集成 + MeasureFunc
│   ├── scene/              # Node 树、Container、各叶子类型
│   ├── render/             # VertexBuffer, MeshFactory*, FairyBatching, clip 意图, RenderNode
│   ├── text/               # ttf-parser + linebreak → TextLayout (SOA 三表)
│   ├── event/              # 命中、bubble/capture、capture_touch、拖拽/滚动仲裁、focus
│   ├── anim/               # TweenManager, Timers, ScrollPane 物理（+ v1.x: Transition/Controller/Gear）
│   ├── asset/              # 包格式(formatVersion)、TextureView、refcount、load/instantiate
│   └── stage.rs            # Stage: tick(input,dt) → render_nodes
├── loomgui_pkg/            # 打包器 CLI：HTML+CSS+资源→.pkg.bin+图集，复用 core 的 parse feature
├── loomgui_ffi_c/          # C ABI 导出（extern "C" + 手写薄包装）
├── loomgui_unity/          # csbindgen 生成 C# 绑定 + Unity 后端（GameObject 镜像/DrawState 缓存）
├── loomgui_editor/         # 后期：编辑器（Web/Tauri，WASM 调同一核心）
└── tests/                  # 核心单测 + 快照（DSL→render_nodes JSON）+ 跨引擎一致性（normalized draw list diff）
```
核心可编译为 WASM（给编辑器）和 C ABI（给引擎），同一份代码。

**测试策略**：
- 核心纯 Rust 单测 + 快照测试（DSL→render_nodes JSON）覆盖 90%。
- 跨引擎一致性：每后端产 "normalized draw list"（`[texture, blend, mask, verts_hash, sort_key]` 元组列表）做跨引擎 diff，验证绘制意图一致；真出图抽样像素 diff（阈值化，容忍字形抗锯齿差异）。

---

## 17. 跨引擎扩展（Unity 之外）

- **Godot 后端**：镜像成 **Node2D + RenderingServer canvas_item 自绘**（与 Unity GameObject+MeshRenderer 严格对仗）。**否决 Control 路线**（会用 Godot 自己布局，与核心 taffy 双系统冲突）。坐标系：Godot 2D 本就左上 y 下，根 flip 矩阵=单位矩阵。遮罩用 canvas_group/clip（Godot 的实现选择，非 stencil）。
- **SRP 混合渲染**（Unity 增强，v1.x）：自绘节点用自定义 SRP RendererFeature 批合绘制（少 draw call），特效仍是 GameObject——性能 + 引擎集成兼得。渲染树契约不变，只换后端执行策略。
- 新后端只需实现：消费 `Vec<RenderNode>` + 输入注入 + 资源加载。契约（§8）引擎中立，新后端不动核心。
