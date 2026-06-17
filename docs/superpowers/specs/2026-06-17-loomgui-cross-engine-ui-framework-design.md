# LoomGUI 跨引擎游戏 UI 框架 — 技术设计文档

> 状态：草案 v2（待评审）
> 日期：2026-06-17
> 参考：FairyGUI-unity（`F:\WorkSpace\projects\FairyGUI-unity`，作为渲染/对象模型/动画的原理参考）
> 本文是**设计文档**，不含实现代码。后续实现计划由本文拆分而来。
> v1→v2 主要变更：渲染契约从"扁平 draw-call 流"改为"渲染树 + 后端镜像原生 GameObject"（恢复 fgui DisplayObject 路线，以支持 Unity 特效/世界空间 UI）；改名 LoomGUI。

---

## 0. TL;DR

LoomGUI 是一款跨引擎游戏 UI 框架，目标是"一次编辑、多引擎一致运行"，对标 FairyGUI，但用 **HTML 作 DSL**、**flexbox（taffy）做布局**、**Rust 写引擎无关核心**。首发引擎为 **Unity**（通过 csbindgen 做 FFI）。

核心取舍：
- **布局**：用 taffy 跑 flexbox（替换 fgui 的 Relations），支持真正的流式布局、响应式、内在尺寸。
- **渲染（fgui 路线）**：自绘。Rust 核心持有布局树，每帧产出一棵**渲染树**（每节点的几何/材质状态/变换/裁剪/绘制顺序）；**Unity 后端把渲染树镜像成原生 GameObject**（每节点 `MeshFilter+MeshRenderer`，等价 fgui 的 NGraphics/DisplayObject）。这样 Unity 粒子特效/世界空间 UI/光照后处理都能接入；同时核心拥有全部引擎无关逻辑（布局/文本/几何/批合重排/裁剪/顺序），保证跨引擎一致。
- **文本**：Rust 用 rustybuzz（shaping）+ ttf-parser（度量）+ unicode-linebreak/bidi 自己做测量与断行；引擎按 Rust 算出的 run 渲染（关闭自动换行）。
- **FFI**：csbindgen（Rust→C ABI→C# P/Invoke），IL2CPP 友好，回调用 `static` + `[MonoPInvokeCallback]`。
- **流程纪律**：核心是纯 Rust 库，先在编辑器/测试里验证（无 FFI），再把**同一个**核心 FFI 进 Unity。

---

## 1. 概述与目标

### 1.1 要解决的问题
传统游戏 UI（Unity UGUI / Godot Control）布局能力弱（锚点/绝对定位为主），跨引擎不通用，且各引擎渲染表现不一致。FairyGUI 证明了"独立编辑器 + 跨引擎运行时 + 自绘"路线可行，但它的布局仍是锚点式、DSL 是私有二进制格式。

LoomGUI 想做的是 FairyGUI 的精神继承者，但：
1. 用 **HTML + CSS 子集** 作 DSL（标准、可读、工具链友好）；
2. 用 **flexbox** 替代锚点定位（更现代、更适合响应式）；
3. 核心用 **Rust** 写，一份代码覆盖多引擎。

### 1.2 目标
- **G1 编辑一次，多引擎一致**：同一份 HTML/资源包，在 Unity（首发）及后续引擎（Godot 等）上**布局/文本/几何一致**（最终像素受各引擎 GPU/shader 影响，但结构一致）。
- **G2 流式布局**：flexbox 完整子集，支持响应式（分辨率/异形屏 safe-area）、动态内容、内在尺寸。
- **G3 运行时动态**：UI 在运行时可任意增删改节点、跑动画、响应数据变化。
- **G4 FairyGUI 级渲染质量 + Unity 生态集成**：自绘、批合、遮罩/裁剪、富文本、九宫格、序列帧；且能挂 Unity 粒子特效、支持世界空间 UI。
- **G5 可扩展**：框架内置基础控件 + 项目自定义控件共存。

### 1.3 非目标（v1 明确不做）
- 不做完整浏览器 CSS（无块级/行内流、无 float、无 grid——grid 留待后期）。
- 不做编辑器（后期单独项目；本文只定 DSL 规范与运行时）。
- 不做 Unity UGUI / UIToolkit 兼容层（v1 纯自绘 + 原生 GameObject 镜像，不走 UGUI Canvas）。

---

## 2. 设计原则

1. **核心即纯库**：所有引擎无关逻辑（解析、布局、几何生成、渲染状态、事件、动画）都在 Rust 核心里，无引擎依赖、无全局 IO、可单测。
2. **渲染树是契约**：核心↔后端的接口是"一帧的渲染树"。核心产出每节点的渲染状态；后端把它镜像成该引擎的原生场景对象并提交。核心不碰 GPU/原生对象，后端不碰布局/几何生成。
3. **照搬 fgui 的成熟机制，替换其布局**：渲染/批合/裁剪/对象模型/动画/资源管线借鉴 fgui；Relations 整套换成 taffy。
4. **围栏优先**：HTML/CSS 只支持明确子集，不支持的不报错即忽略并告警。子集越窄，解析/布局/渲染表面积越小越稳。
5. **不可逆决策先定，可逆决策后补**：场景图形状、渲染契约、FFI 模型先定死；动画/数据绑定/高级特效可层叠追加。
6. **单时钟**：整个核心只有一个每帧 `tick(dt)` 入口驱动布局/动画/事件/渲染流水线。

---

## 3. 总体架构

### 3.1 分层蛋糕

```
┌─────────────────────────────────────────────────────────────┐
│  HTML/CSS DSL  (人/编辑器/工具链 读写)                          │
├─────────────────────────────────────────────────────────────┤
│  Rust 核心 (loomgui_core crate, 引擎无关)                      │
│   ┌──────────┐  ┌──────────┐  ┌──────────────────────────┐  │
│   │ 解析层    │→│ 样式层    │→│ 布局层 (taffy flexbox)     │  │
│   │scraper   │  │cssparser │  │ 节点尺寸→taffy style      │  │
│   │+selectors│  │+cascade  │  │ measure 回调(文本测量)     │  │
│   └──────────┘  └──────────┘  └────────────┬─────────────┘  │
│                                              ↓                │
│   ┌──────────────────────────────────────────────────────┐  │
│   │ 场景图 (Node 树: 容器/图片/文本/形状/按钮/列表...)      │  │
│   │  + 事件/命中  + 动画(GTween)  + 状态(Controller/Gear) │  │
│   └────────────────────────────┬─────────────────────────┘  │
│                                 ↓ 每帧渲染 pass              │
│   ┌──────────────────────────────────────────────────────┐  │
│   │ 渲染状态层 (自绘, 引擎无关)                            │  │
│   │ 几何生成(VertexBuffer/mesh factory) + 批合重排        │  │
│   │ (FairyBatching, 算 sort_key) + 裁剪栈(stencil/rect)   │  │
│   └────────────────────────────┬─────────────────────────┘  │
│                                 ↓ 每帧产出                   │
│                  渲染树 (Vec<RenderNode>)  ← 核心↔后端契约    │
├─────────────────────────────────────────────────────────────┤
│  FFI 缝界 (csbindgen: Rust ↔ C ABI ↔ C# P/Invoke)            │
├─────────────────────────────────────────────────────────────┤
│  引擎后端 (Unity 首发; 后续 Godot 等)                          │
│   - 输入采集 → 注入核心                                        │
│   - 把渲染树镜像成原生 GameObject (MeshFilter+MeshRenderer)   │
│     同步 mesh/材质(MaterialManager)/变换/sortingOrder/裁剪     │
│   - 提交 Unity 渲染 (借 Unity 动态批合/SRP Batcher)            │
│   - 资源加载 (Addressables/YooAsset) → 注入核心纹理 id        │
│   - NativeHost 节点: 容纳用户 Unity 粒子/3D/自定义渲染器        │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 核心接口（核心对外暴露的 API 形状）

```rust
let stage = Stage::new(config);
stage.load_package(pkg_bytes)?;                              // 注册包
let root = stage.create_object("loom://pkg/MainUI")?;        // 实例化树

// 运行时动态操作（命令式 API，见 §13）
root.get_child("startBtn").set_text("开始");

stage.tick(&input_events, dt);                               // 引擎喂输入+dt
let render_nodes: &[RenderNode] = stage.render_nodes();      // 后端据此同步 GameObject 镜像
```

`RenderNode` 是核心↔后端契约（见 §8.6）。后端维护 `node_id → GameObject` 的镜像池，每帧按 `render_nodes` 增删改同步。

### 3.3 关键边界
- **Rust 核心**：解析、样式、布局、场景图、事件、动画、几何生成、渲染状态计算、批合重排、裁剪/顺序。产出 `Vec<RenderNode>` + 命中结果 + 事件。**不持有任何引擎对象、不碰 GPU。**
- **引擎后端**：输入采集（鼠标/键盘/触摸/IME）、**渲染树→原生 GameObject 镜像**、mesh 上传、材质实例化与缓存、blend/stencil state 应用、提交渲染、资源加载代理。
- **不跨越的**：核心不知道 GameObject；后端不解析 DSL、不算布局、不生成几何。

### 3.4 为什么是"渲染树 + 原生镜像"而非"扁平 draw-call 流"
扁平 draw-call 流（`Graphics.DrawMesh` 直画、无 GameObject）批合更彻底、后端更薄，但**彻底切断 Unity 生态**：无法把 Unity 粒子特效作为 UI 子节点、难做世界空间 UI、接不上 Unity 光照/后处理/物理拾取/插件。游戏 UI 常要特效（按钮闪光、奖励爆裂、血条火焰），这是硬需求。故采用 fgui 路线：每个渲染节点镜像成一个原生 GameObject，特效/世界空间 UI/插件都能以 GameObject 子节点形式接入。

---

## 4. HTML/CSS 围栏（DSL 规范子集）

> 这是 LoomGUI 与浏览器最大的不同：**我们只支持一个明确的子集**，不支持的不报错即忽略并告警。围栏内的特性才进解析/样式/布局/渲染的表面积。

### 4.1 围栏的总原则
- **容器默认是 flex**：与浏览器默认 block 不同。LoomGUI 中每个容器节点 `display: flex`（`flex-direction: row`）。要纵向堆叠写 `flex-direction: column`。这是**有意背离浏览器默认**，因为我们的布局模型就是 flexbox。
- **文本是节点，不是流**：纯文本内容被包成一个文本节点（leaf），不做浏览器那种行内文字与块级混排。
- **自定义元素用 kebab-case**：`<l-list>`、`<l-loader>`、`<l-movie>` 等框架特有元素用前缀，避免与 HTML 冲突。

### 4.2 支持的元素

| 元素 | 映射节点 | 说明 |
|---|---|---|
| `<div>` / `<l-container>` | Container | 通用 flex 容器，可裁剪/遮罩 |
| `<span>` / 裸文本 | Text | 叶子文本节点 |
| `<l-rich>` | RichText | 富文本（支持内联标记） |
| `<img>` | Image | 贴图 quad（支持九宫格/平铺/填充） |
| `<button>` | Button | 交互按钮（内置 Controller 状态：up/down/over/disabled） |
| `<input>` / `<l-textinput>` | TextInput | 可编辑文本 |
| `<l-graph>` | Graph | 矢量形状（矩形/圆/椭圆/多边形/线/圆角矩形） |
| `<l-loader>` | Loader | 异步外部图/序列帧加载器 |
| `<l-movie>` | MovieClip | 序列帧动画 |
| `<l-list>` | List | 虚拟化滚动列表（数据驱动，见 §13） |
| `<l-slider>` / `<l-progress>` | Slider / ProgressBar | 滑块/进度条 |
| `<l-combobox>` | ComboBox | 下拉 |
| `<l-tree>` | Tree | 树形视图 |
| `<l-native>` | NativeHost | 占布局位的原生宿主：backing 为用户 GameObject，可塞 Unity 粒子/3D/自定义渲染器 |

### 4.3 富文本内联标记子集（`<l-rich>` 内部）
对齐 fgui 的 HtmlParser 已知子集：`<b> <i> <u> <s> <sub> <sup> <br> <font size= color=> <img src= width= height=> <a href=> <p align=>`。颜色支持 `#rrggbb` 单色与 `c1,c2,c3,c4` 四角色渐变。

### 4.4 支持的 CSS 属性

**布局类（→ taffy）**
- `display`: `flex` | `none`（无 `block`/`inline`/`grid`；`grid` 后期）
- `flex-direction`, `flex-wrap`, `gap`, `row-gap`, `column-gap`
- `justify-content`, `align-items`, `align-self`, `align-content`
- `flex-grow`, `flex-shrink`, `flex-basis`, `flex`（简写）
- `width`, `height`, `min-*`, `max-*`：值 `px` / `%` / `auto`
- `padding`（四向）, `margin`（四向）, `border` 宽度（参与布局盒）
- `position`: `relative` | `absolute`；`top/right/bottom/left`
- `aspect-ratio`, `order`

**视觉/渲染类**
- `background-color`, `background-image`(`url()`), `background-size`(`cover/contain/100%/tile`), `background-position`
- `border`(`color`/`width`/`style:solid`), `border-radius`
- `opacity`, `overflow`(`visible`/`hidden` → 矩形裁剪), `clip-path`(后期)
- `color`, `font-size`, `font-family`, `font-weight`, `font-style`
- `text-align`, `line-height`, `letter-spacing`, `white-space`(`nowrap`/`normal`)
- `filter`: `grayscale` / `brightness` / `blur`（v1 只 grayscale，对应 fgui grayed）

**交互类**
- `pointer-events`: `auto` | `none`；`cursor`

**九宫格**：`border-image-slice` / 自定义 `-l-slice`（决定 Image 的九宫格切分）。

### 4.5 状态与控制器（fgui Controller/Gear 的 HTML 表达）
浏览器 CSS 的 `:hover/:active/:focus/:disabled` 直接映射到内置状态。自定义状态（fgui 的多页 Controller）用 **data 属性 + 自定义伪类**：
```html
<div data-controller="tab" data-page="0"> ... </div>
<style>
  [data-controller="tab"]:l-page(1) .panel { opacity: 0.3; }
</style>
```
`:l-page(n)` 是 LoomGUI 自定义伪类，运行时由 Controller 切页驱动 Gear 改属性（详见 §11）。

### 4.6 明确不支持的（围栏外）
`display:block/inline/grid`、`float`、`position:sticky/fixed`、CSS 动画/transition（用框架自己的 tween/transition）、伪元素 `::before/::after`、复杂选择器（只支持标签/类/id/后代/子代/`:nth-child`/属性选择/自定义伪类）、`@media`（响应式用框架 safe-area + 百分比 + flex 表达）。

> 围栏是**活文档**：每加一个属性都要想清楚它在 taffy/渲染层是否值得。v1 起步可以更窄。

---

## 5. 解析层

- **HTML 解析**：`scraper`（底层 `html5ever`，规范级）→ 得到一棵只读 DOM 树；遍历它构造 LoomGUI 元素树。
- **CSS 声明解析**：`cssparser`（Servo 同款）解析 `{ prop: value; }` 声明块。`scraper` 的 `selectors` 只做选择器匹配，**不**解析声明，所以 cssparser 必装。
- **样式层（cascade 子集）**：来源优先级 `inline > id > class > tag`，可选 `!important`。不做完整 cascade 算法。
- **选择器匹配**：`selectors` 支持 `.class`/`#id`/`div > span`/`:nth-child`/属性选择/自定义伪类 `:l-page()`（注册到 selectors 的匹配扩展）。
- **产物**：一棵带解析样式的元素树（`Element`：tag、属性、合并 style）。这棵"源表示"会编译成运行时场景图（§6、§12）。

> 运行时**完全不走 HTML 解析**——编辑器把 DSL 编译成二进制包（§12），运行时从二进制建场景图（热重载也是重编译二进制再加载）。解析层（scraper/cssparser/selectors）只在编辑器/工具链，feature-gate 不进运行时（§16.1）。

---

## 6. 对象模型（场景图）

### 6.1 设计决策：核心单 Node + 后端原生镜像
fgui 把 `GObject`（逻辑）与 `DisplayObject`（渲染，且包一个 Unity GameObject）分两层。在我们这里：
- **核心（Rust）只有一个持久 `Node` 类型**：它同时持有逻辑状态（布局/样式/变换/事件/gear/controller）和几何生成能力。核心不知道 GameObject，不需要核心侧的 DisplayObject。
- **后端（Unity）有原生镜像对象**（GameObject+MeshRenderer），等价 fgui 的 DisplayObject/NGraphics——这是 GameObject 存在的地方，也是 Unity 特效集成的接入点。
- 核心每帧**产出瞬态 `RenderNode` 状态**（几何/材质/变换/裁剪/顺序），后端据此同步其 GameObject 镜像。

所以 fgui 的"逻辑对象/渲染对象"那道缝被保留，只是搬到**核心↔后端边界**：核心 Node（持久、引擎无关）↔ 后端 NativeRenderObject（持久、引擎专属，镜像 GameObject）。这比 fgui 把两层都塞在 C# 运行时里更干净。**几何生成的分工**：非文本几何（图片 quad / 形状 / 九宫格 / 进度填充）在 Rust 核心生成（确定性、跨引擎一致、数据量小）；**文本 mesh 是例外——在后端生成**，核心只产出 `TextLayout`（位置/advance/cluster/断行）作输入（见 §9），因为动态字形 UV 只有引擎字体 API 才有。后端的 NGraphics 等价物负责上传 mesh + 材质 + 变换 + 顺序。

### 6.2 Node 类型层级（基于 fgui GObject 提炼）
```
Node (基类: 变换/尺寸/可见/touchable/事件/gear/controller)
├── Container        (唯一持有 children，可裁剪/遮罩，是批合边界候选；可挂 ScrollPane 模式)
│   ├── Button       (内置 Controller: up/down/over/disabled)
│   ├── List         (虚拟化滚动列表)
│   ├── ComboBox
│   ├── Slider / ProgressBar
│   └── Tree
├── Image            (贴图 quad: 普通/九宫格/平铺/填充)
├── Graph            (形状: 矩形/圆/椭圆/多边形/线)
├── Text             (纯文本)
├── RichText         (富文本 + 内联对象)
├── TextInput        (可编辑)
├── Loader           (异步加载图/序列帧)
├── MovieClip        (序列帧)
└── NativeHost       (原生宿主: backing=用户 GameObject, 参与 UI 布局/裁剪)
```
**约束（照搬 fgui）**：只有 `Container` 能拥有 children；叶子节点不带 children 数组。

### 6.3 Node 核心数据结构（设计级，非最终）
```rust
struct Node {
    id: NodeId,
    parent: Option<NodeId>,
    transform: Transform2D,     // x/y/rotation/scale_x/scale_y/pivot/pivot_as_anchor
    style_size: SizeStyle,      // 用户声明值 (width/height/min/max/flex_basis)
    measured_size: (f32, f32),  // taffy solve 后写入（只读）
    layout_rect: Rect,          // 父坐标系最终矩形（只读）
    alpha: f32, visible: bool, touchable: bool, grayed: bool,
    style: ResolvedStyle,       // §4 CSS 子集
    dirty: DirtyFlags,          // mesh/layout/batching/outline
    listeners: HashMap<EventType, EventBridge>,
    gears: [Option<Gear>; 10],
    gear_locked: Cell<bool>,
    children: Option<Vec<NodeId>>,   // None = 叶子
    sorting_order: i32,
    clip_rect: Option<Rect>,
    mask: Option<NodeId>,
}
```

### 6.4 尺寸模型 → flexbox 映射（关键）
| LoomGUI / CSS | taffy |
|---|---|
| `width/height` (`px`/`%`) | `size` |
| `min-width/max-width` 等 | `min_size` / `max_size` |
| `flex-basis` | `flex_basis` |
| `flex-grow` / `flex-shrink` | `flex_grow` / `flex_shrink` |
| `flex-direction/wrap/gap` | 同名 |
| `justify/align-*` | 同名 |
| `padding` / `border-width` / `margin` | `padding` / `border` / `margin` |
| `position: absolute` + insets | taffy `position: Absolute` + `inset` |
| 内容自适应（文本/图片内在尺寸） | taffy `MeasureFunc` 回调（§7、§9） |
| NativeHost（原生内容尺寸） | `MeasureFunc` 回后端查询用户 GameObject 的 bounds |

### 6.5 生命周期
```
构造（从包/DSL 反序列化或运行时 new）
  → 注册到父 Container（更新 taffy 树）
  → 用户改属性 → 置 dirty（不立即重算）
  → 每帧 tick：layout dirty → taffy solve → 写 measured_size/layout_rect
                mesh dirty   → 重新生成几何 → 产出 RenderNode
  → 后端同步：新增/移除/更新 GameObject 镜像
  → Dispose：从父移除、释放纹理引用(refcount)、清事件/gear/tween、后端销毁 GameObject
```
**与 fgui 的关键区别**：fgui 改属性立即推送 DisplayObject（无 layout pass）。我们改属性只置 dirty，每帧一次 taffy solve 统一下推。须向用户明确"所有布局都是帧末一致"。

---

## 7. 布局层（taffy 集成）

### 7.1 taffy 树与场景图的同步
场景图的 Container 树 ↔ 一棵 taffy 节点树一一对应。增删 Container 同步增删 taffy 节点；改 style 同步改 taffy style 并标记该子树 dirty。

### 7.2 内在尺寸：MeasureFunc
taffy 对"尺寸取决于内容"的节点（文本、自适应图片、NativeHost）回调 `MeasureFunc(known_dimensions) -> measured_size`：
- **文本**：调文本测量子模块（§9），给定约束宽返回 `(text_width, text_height)`。必须廉价、无副作用。
- **图片**：原始像素尺寸或声明尺寸。
- **RichText 内联对象**：先 query 每个 img/input 的 (w,h) 再参与断行。
- **NativeHost**：回调后端查询用户 GameObject 的包围盒（跨 FFI）。

### 7.3 响应式与异形屏（动态布局触发器）
- **resize**：屏幕尺寸变化 → 根 taffy 节点 size 变 → 整树 solve。
- **safe-area（异形屏）**：引擎把 insets 注入核心；CSS 用百分比 + `-l-safe-area` 环境变量表达避让。
- **动态内容/数据变化**：改文本/增删子节点 → 置 dirty → 下帧 solve。
- **横竖屏/分辨率适配**：百分比 + flex + safe-area，不依赖 CSS `@media`。

### 7.4 参考分辨率 / DPI 缩放
商业游戏标准做法：设计稿 1080×1920，在 1440×2560 屏幕整体等比放大、UI 不变形。百分比+flex 只解决相对布局，解决不了"整张 UI 在大屏上该多大"——这是移动端 UI 框架入门门槛（对照 fgui `UIContentScaler`）。

- **参考分辨率 + 等比缩放**：Stage 持 `design_resolution` + `match_mode`。后端把屏幕尺寸 + safe-area 注入核心，核心算出 scale + 根 size。
- **整体缩放**：根 Stage 一个 scale，整树缩放（不逐节点）。
- **match 模式**：v1 只做 `MatchWidthOrHeight`（最常用，覆盖 90%）；`MatchWidth`/`MatchHeight` 后期。对照 fgui 的 `ScaleWithScreenSize`。
- **高清资源分支**：scaleLevel（1x/2x/3x）驱动 §12.2 的 `highResolution` 数组选不同倍率资源。
- **safe-area 是缩放之后叠加的避让**，不是替代。

### 7.4 布局时机
运行时算（已定）。每帧只在 dirty 时 solve；taffy 支持请求子树布局。布局结果供渲染与命中测试消费。

---

## 8. 渲染层（自绘，渲染树契约）

> 借鉴 fgui 成熟机制。与 fgui 的差异：(a) 几何生成在 Rust 核心（不在后端）；(b) 后端把渲染树镜像成 GameObject 而非自己生成几何。批合沿用 fgui 方案（材质复用 + FairyBatching 重排 + 借 Unity 动态批合），不自己合并 mesh——换来 Unity 特效/世界空间集成（§3.4）。纯 2D 重 UI 后期可对"无原生子节点"子树做 mesh 合并优化。

### 8.1 几何生成：VertexBuffer + MeshFactory（照搬，在核心）
- `VertexBuffer { verts, uvs, uvs2, colors, indices }` + 输入 `content_rect/uv_rect/vertex_color/texture_size`，对象池化。
- `trait MeshFactory { fn on_populate_mesh(&self, vb: &mut VertexBuffer); }`，各类形状实现：矩形/九宫格/平铺/进度填充/多边形(Ear-clipping)/椭圆/圆角矩形/折线/组合。
- 基础方法：`add_vert/add_quad/add_triangles/append/insert/repeat_colors/generate_outline/generate_shadow/fix_uv_for_arbitrary_quad`。
- **rotated 纹理 UV 修正公式**：`new_y = y_min + uv.x - x_min; new_x = x_min + y_max - uv.y`。
- y 轴：核心内部统一**左上原点、y 向下**；后端适配层负责到 Unity 屏幕坐标的翻转。
- **非文本 mesh 由核心生成、跨 FFI 传给后端**，后端上传到该节点的 MeshFilter。**文本节点例外**：核心只产 `TextLayout`（§9），文本 mesh 由后端据 TextLayout 光栅化 + 拼 quad 生成（动态字形 UV 只有引擎侧才有）。

### 8.2 纹理：TextureView（对应 fgui NTexture，去 Unity 化）
```rust
struct TextureView {
    root_tex: TexId,           // GPU 纹理 id（引擎上传后返回）
    alpha_tex: Option<TexId>,
    region: RectPx, offset: Vec2, original_size: Vec2, rotated: bool,
    uv_rect: Rect, ref_count: i32,
}
```
- 图集：一张大纹理(root) + N 个轻量 TextureView（只存 UV）。子 view 首引用连带 root；归零 `on_release` 通知引擎可卸载。
- 核心只持 `TexId`（整数）；GPU 生命周期全在后端。

### 8.3 材质语义：MaterialFlags + BlendMode + ShaderId（照搬，去实例化）
核心不算 Material 对象，只算 draw 所需状态：
- `MaterialFlags`(u32)：`Clipped|SoftClipped|StencilTest|AlphaMask|Grayed|ColorFilter|Combined` + 用户 keyword 高位。
- `BlendMode`(12 种，照搬 fgui src/dst 因子表；`Multiply/Screen` 触发 pma→ColorFilter)。blend 作为 draw state，不编进 shader variant。
- `ShaderId`：`Image / Text / BMFont / 自定义`。
- **StencilTest 是 state 不是 keyword**（避免 variant 爆炸）。
- 后端按 `(shader + flags + blend + texture + stencil_ref + stencil_compare)` 维护 `MaterialManager`（等价 fgui），实例化/缓存 Material。**key 必须含 stencil ref/compare**：不同遮罩深度的内容节点即便 shader 相同，stencil state 不同也不能复用同一 Material 实例（否则跨遮罩层材质错配）。

### 8.4 批合：FairyBatching（fgui 路线）
两个元素能并入同一批 ⟺ **材质实例相同** 且 **AABB 不相交**（保序重排，避免遮挡错乱）。
- 算法照搬 fgui `DoFairyBatching`：稳定插入排序 + AABB 重叠检测，只在无视觉歧义时把同材质元素前移。
- 核心计算每个节点的 `sort_key`（重排后的绘制顺序），后端据此设 `MeshRenderer.sortingOrder`，借 Unity 动态批合/SRP Batcher 合并同材质相邻 draw。
- 批合是**局部的**（每个 BatchingRoot 独立）；裁剪/遮罩/paintingMode 天然是 root 边界。
- **不合并 mesh**（每个节点各自 MeshRenderer）；纯 2D 重 UI 后期可对无原生子节点的子树合并优化。

### 8.5 裁剪/遮罩四机制（照搬 UpdateContext 栈式管理）
随树 DFS 进出，手动 save/restore：
- **rect mask（硬裁剪）**：求交 + 算 clipBox，shader `|clipPos|>1` 丢弃。
- **soft clip（羽化）**：额外传 softness。
- **stencil mask（模板遮罩）**：ref 位掩码左移（最多 ~8 层，8-bit stencil 限制，须文档标注）。**ref 由核心在 DFS 时算好**，填入每个受影响节点的 `ClipState.stencil_read_ref`（内容读）和 mask 节点的 `stencil_write_ref`（写）。mask 写入（AlphaMask 材质）、内容读（StencilTest Equal）、eraser 清位（Zero）都是**显式 RenderNode 变体**（§8.6 的 StencilWrite / StencilErase）——后端给 eraser 建独立 GameObject，sortingOrder 排在 mask 子树之后。遮罩形状可是任意图（非只矩形）。
- **paintingMode（离屏）**：压栈清零裁剪栈，渲染到 RenderTexture，帧末 capture。
- **clip 上下文是批合边界**：不同裁剪深度的 draw 即便 shader 相同也不能合并。
- 后端按核心给的 ClipState 设材质 uniform / stencil state（等价 fgui UpdateContext.ApplyClippingProperties）。

### 8.6 RenderNode 契约（核心↔后端，公共头 + enum payload）

契约按"公共头 + 按类型分叉的 payload"组织。**v1 只实现 Mesh + Text 两个变体 + 矩形 clip**；stencil/RT/NativeHost 变体先占位（v1.x/v2 填），加变体不破坏旧后端——这是为已上路线图的特性预留接口，非过度设计。

```rust
struct RenderNode {
    // —— 公共头 ——
    node_id: NodeId,
    parent_id: Option<NodeId>,
    slot_id: Option<u32>,          // 虚拟化复用键：后端按 slot 复用 GameObject，而非 NodeId（§13.2）
    visible: bool,
    alpha: f32, grayed: bool,
    color_tint: Color,             // 顶点色（区别于 alpha；MaterialPropertyBlock 或 vertex color）
    transform: NodeTransform,      // 含 pivot 偏移 + 可选透视 VertexMatrix（世界空间 UI 用，v2）
    blend: BlendMode,
    clip: ClipState,               // 含 stencil_write_ref / stencil_read_ref（核心 DFS 算好填入，见 §8.5）
    sort_key: u32,                 // FairyBatching 重排后的绘制顺序
    // —— 按类型分叉 ——
    payload: NodePayload,
}

enum NodePayload {
    Mesh    { mesh_ref, texture, alpha_tex, shader, flags },   // 非文本自绘（九宫格在 mesh 里，§8.1）
    Text    { layout_ref, font, shader, flags },               // 文本：后端据 TextLayout 生成 mesh（§9）
    StencilWrite { ref_value },                                // mask 写入（显式节点，AlphaMask 材质）
    StencilErase  { ref_value },                               // eraser 清位（显式节点，DFS 末尾，独立 GO）
    PaintTarget   { rt_id },                                   // 离屏 RT 资源（paintingMode，跨节点依赖）
    NativeHost,                                                // v1.x：后端放置用户 GameObject，不画自有 mesh
}
```

**关键约定**：
- **stencil 是跨节点的时序协议**，不是单节点状态。ref 由核心在 DFS 时算好（位掩码左移，§8.5）填入每个受影响节点的 `ClipState`；后端不猜、不重算。mask 写入 / eraser 清位是**显式 RenderNode**（StencilWrite / StencilErase 变体），后端给 eraser 建独立 GameObject，sortingOrder 排在 mask 子树之后（等价 fgui 的隐藏 StencilEraser GameObject）。不再有"藏在 NGraphics 内部的隐藏对象"。
- **九宫格**：在核心的九宫格 MeshFactory 生成 16 顶点 mesh（§8.1），作为普通 Mesh payload，不进材质——契约不变。
- **MaterialManager 的 key 必须含 stencil ref/compare**（§8.3）：不同遮罩深度的内容节点即便 shader 相同也不能复用同一 Material 实例。
- **slot_id**：虚拟化列表的 item 复用键。后端 diff 按 slot_id 复用 GameObject（旧 item 滚出、新 item 滚入同一 slot → 复用同一 GO），而非按 NodeId 销毁重建（§13.2）。v1 字段就位、语义 v1.x 启用。
- **NodeTransform**：本地变换 + pivot 偏移（内容矩形相对变换原点）+ `Option<VertexMatrix>`（透视/斜切，世界空间 UI 用，v2 填）。

后端每帧：diff `render_nodes` 与镜像池（按 node_id 增删、按 slot_id 复用）→ 同步对应 payload（Mesh 上传 mesh、Text 据 layout 生成 mesh、StencilWrite/Erase 建/更新独立 GO）→ 设 transform/sortingOrder/clip/blend/stencil。

### 8.7 绘制顺序
单一全局递增计数器 `rendering_order`，每帧重置，DFS 中"分配即自增"。批合区内不分配，等 BatchingRoot 按重排后顺序统一分配。mask 的 eraser 排在子树末尾。最终顺序 = 树序 × 批合重排 × 裁剪边界。

---

## 9. 文本（rustybuzz + ttf-parser）

### 9.1 测量与渲染的分离（一致性根基）
- **Rust 核心拥有测量 + 断行**（确定性，跨引擎一致）：rustybuzz 做 shaping（连字/合字/阿拉伯，**整段 shape，不能逐字符取 advance 相加**）+ ttf-parser 取真实度量（`hhea`/`os2` ascent/descent/line-gap，**不照搬 fgui 的 `fontSize*1.25` 估算**）+ unicode-linebreak（换行机会，CJK 逐字）+ unicode-bidi（RTL）。
- **后端只渲染 run**：接收 Rust 的 `TextLayout`，**关闭自动换行/自动测量**，按 run 绝对坐标逐字画 quad。
- 换行点、行宽、box 尺寸跨引擎一致；仅字形光栅化有细微差异。

### 9.2 TextLayout 产物
```rust
struct TextLayout { text_width: f32, text_height: f32, lines: Vec<Line> }
struct Line { y, height, baseline, width, runs: Vec<GlyphRun>, inline_objects: Vec<(x,y,w,h,obj_id)> }
struct GlyphRun { font_id, font_size, format, glyphs: Vec<(glyph_id, x_off, y_off, advance, cluster)> }
```
- 每字形带 `(x_off, y_off, advance, cluster)`：渲染按绝对位置画，cluster 映射回源文本（选区/光标/链接/打字机/省略号必需）。
- `letter_spacing` 后处理加到 `x_advance`；上下标用小字号 run + baseline 偏移。
- 富文本/emoji/内联图片是**布局的一部分**：断行前先解析标记、query 每个内联对象 (w,h)。

### 9.3 测量的可重入性
auto-size / shrink 反复测（二分搜索字号）。`measure(known_dimensions)` 必须廉价、无副作用、可被 taffy 反复调用。测量与渲染必须用**同一套字体度量**。

### 9.4 字体资产
- **位图字体**进包（字形 atlas + 字形表/UV）。
- **动态字体**不进包，运行时全局注册或从引擎字体资源加载。核心定义 `Font` trait。**文本 mesh 在后端生成**：核心产出 `TextLayout`（位置/advance/cluster/断行），后端用 Unity `RequestCharactersInTexture` 光栅化产 UV、按 TextLayout 位置拼 quad mesh。advance/断行/box 尺寸一律以 Rust 为准（跨引擎一致），仅字形 UV/光栅化在引擎侧。

---

## 10. 事件与输入

### 10.1 自绘树的命中测试（核心拥有，引擎无关）
核心消费布局结果做命中：输入 stage 坐标点 →
1. `world_to_local`（累计变换逆矩阵把点投到本地）。
2. `visible && touchable` 门控。
3. 裁剪：有 `clip_rect` 必须包含；有 `hit_area`（trait，Rect/Shape/PixelMask）则委托。
4. **子节点逆序遍历**（顶层优先），第一个命中即返回。
5. 容器自身 fallback：`opaque && content_rect.contains(point)`。
- 结果按帧号缓存。命中完全消费布局 AABB。
- 像素精确命中：预生成 1bit/pixel 掩码（零拷贝指向大 buffer），照搬 fgui `PixelHitTestData`。
- **可选**：后端可对 NativeHost/世界空间 UI 提供 Unity 物理碰撞器拾取（等价 fgui ColliderHitTest），但核心的几何命中是主路径。

### 10.2 事件路由（DOM 三阶段，照搬 fgui）
- `dispatch(target, type)`：目标直派（focus/dragMove/sizeChanged）。
- `bubble(target, type)`：**捕获(链反向) + 冒泡(链正向)**；`stop_propagation` 中断。
- `broadcast(root, type)`：子树深搜（added/removedFromStage）。
- 每节点 `HashMap<EventType, EventBridge>`，`EventBridge` 持 capture + bubble 两组回调。Rust 回调用注册返回的 `ListenerId` remove。
- `EventContext` 对象池复用。

### 10.3 指针路由 / 触摸捕获 / 点击判定
- 多触摸槽（支持 N）：`target / down_targets 链 / touch_monitors / click 状态`。
- `capture_touch()`：节点加入 `touch_monitors`，移动/抬起并进派发链——手指移出仍持续收事件（拖拽/滚动依赖）。
- Click 判定：抬起距按下阈值（鼠标 ~10px / 触摸 ~50px）、双击 350ms、down_targets 链匹配。
- RollOver/Out：hover 链 diff（不冒泡）。

### 10.4 拖拽 / 焦点
- 节点级 `draggable`：超灵敏度阈值触发 `onDragStart`（可 prevent_default），`drag_bounds` 局部坐标 clamp，全局 `dragging_node` 单例。另可做 DragDrop（替身图标 + onDrop）。
- 焦点：`Stage.focused: Option<NodeId>`，`focusable/tab_stop/tab_stop_children` flag，Tab 导航深搜下一个 `_accept_tab` 节点。

### 10.5 引擎输入桥
核心定义 `InputProvider` trait（指针/键/触摸/IME character），后端实现并每帧注入。坐标约定：核心**左上原点**，后端做 `y = height - y` 翻转。**IME 组合字符必须从引擎文本输入事件拿，不是按键码**。

### 10.6 UI 输入消费（is_pointer_on_ui）
游戏第一天就撞的墙：UI 挡住时游戏不能响应点击。**对齐 fgui，极简**：核心命中测试后存"当前指针命中的 NodeId"（§10.1 已有），暴露一个事实查询：
```rust
stage.is_pointer_on_ui() -> bool   // = 命中目标非空且非根
```
- **不做消费策略/consume 标志/每指针数组**。fgui 证明一个 bool 够用——游戏自己在输入管线里 `if (stage.is_pointer_on_ui()) return;` 决定要不要响应。
- `pointer-events: none`（§4.4）控制的是"该节点参不参与命中"，不是"消费不消费"，别混。
- 多点触摸暴露主指针的命中结果（对齐 fgui `touchTarget`）。

---

## 11. 动画与状态（单时钟）

> 原则：**整个核心只有一个动画时钟 `TweenManager::update(dt)`**。Controller / Gear / Transition 都不自建 update，全部是"事件 → 往 TweenManager 提交/kill tweener → tweener 回调写节点属性"。

### 11.1 GTween（补间引擎，唯一时钟）
- `TweenManager { active, pool }`，池化。
- `Tweener`：统一 `TweenValue{x,y,z,w,d}` + `value_size(1..6)`（float/Vec2/3/4/Color/double；6=shake）。
- 链式 builder：`tween(start,end,dur).delay().ease().repeat(,yoyo).on_complete()`。
- 30+ 缓动（Linear/Sine/.../Elastic/Back/Bounce 的 In/Out/InOut + Custom），`EaseManager` 纯函数（Penner 方程）。
- 特殊：`DelayedCall`、`Shake`、`SetPath`(贝塞尔)、`SetBreakpoint`(Transition 的 PlayFromTo 裁剪)、`smoothStart`(吸收首帧大 dt)。
- `prop_type`：tween 可直接写回节点属性，`on_update` 分发到 setter。

### 11.2 Transition（时间线 = 编排器，不自驱）
- 纯数据 `items: Vec<TransitionItem>` + 运行态 `total_tasks`（引用计数式完成检测）。
- **不自建 update**：`Play()` 把每个 item 翻译成 Tweener 提交到 TweenManager：有 `tween_config` → `tween(start,end,dur).delay(time)`；瞬态帧 → `delayed_call(time)`。倒放 = 逆序 + start/end 互换 + delay 镜像。
- 只支持两点关键帧；多关键帧靠多个 item 串行。嵌套 Transition 递归 + 完成回调递减父计数。

### 11.3 Controller（状态机，纯状态）
- `Controller { name, selected_index, page_ids, page_names, actions }`。
- `set_selected_index` 只做：记 previous、改 index、`parent.apply_controller(this)` 扇出到子节点 Gear + 派发 onChanged。**Controller 不直接改 UI 属性**，全靠 Gear。

### 11.4 Gear（状态→属性映射，纯数据 + 命令式 Apply）
- 每节点 `gears: [Option<Gear>; 10]`：Display/Xy/Size/Look/Color/Animation/Text/Icon/Display2/FontSize。
- 存储 `HashMap<page_id, Value>` + default。`Apply`：查当前页值 → 有 tween 的四重守卫 → 不可插值属性立即设 → kill 旧 tween → 往 TweenManager 提交插值 tween，`on_update` 写回。
- 双向同步：节点属性 setter 调 `update_gear` 回写；`gear_locked` 防循环。
- **HTML 表达**：CSS 自定义伪类 `:l-page(n)`（§4.5）+ `data-controller`/`data-page`。带过渡用 `-l-transition: 0.3s ease`。

### 11.5 Timers
独立通用周期/延时回调（unscaled_dt），与动画解耦。`CallLater`（下一帧）、`AddUpdate`（每帧）。

---

## 12. 资源 / 包系统

### 12.1 双格式（照搬 fgui 思路）
- **编辑期/源**：HTML（结构）+ CSS（样式）+ 资源清单（图/字体/声音）。
- **发布产物**：编译成**单一二进制 blob**（`.pkg.bin`）。体积压到 XML/HTML 的 1/3~1/5、加载无需解析器、少分配。
- 运行时**只认二进制**（含热重载：重新编译 DSL→二进制再热重载二进制）；HTML 解析只在编辑器/工具链，**不进运行时**。

### 12.2 二进制包格式设计（借鉴 fgui _fui）
- Header：魔数 + version + compressed flag。
- 头部 indexTable + `Seek(blockIndex)` 块跳转：组件描述分块，运行时只读需要的块。
- 全局 stringTable + `ReadS(ushort)` 下标：字符串去重。
- 每个 item/child 带 `nextPos` 长度前缀：**前向兼容**。
- 跨资源引用统一 URL（`loom://pkgName#resId`），**存 id 不存内容**。
- 分支(branches)/高清(highResolution) 数组挂同一资源项：多语言/多分辨率作为包内一级概念。

### 12.3 图集
散图 → 图集 → root TextureView + 子 TextureView（只存 UV）。`rotated`/`trim + originalSize + offset` 打包期记录、运行时还原。alpha 分离纹理可选。

### 12.4 引用计数与生命周期（照搬 fgui）
- `TextureView` 自带 `ref_count`，子视图首引用连带 root。
- 渲染组件换纹理自动 AddRef/ReleaseRef。
- 归零 `on_release` 冒泡到资源项 → 通知后端资源管理器（Addressables/YooAsset）卸载。
- `UnloadPolicy`（Destroy/Unload/Custom/None）；`Reload`（卸 native、留壳）移动端必备。

### 12.5 加载与实例化管线（三层分离）
1. `load_package`：只解析描述、建资源项索引（快、可常驻）。
2. `get_item_asset`：按需加载，按类型分发，同步/异步；加载器抽象成 trait，后端注入。
3. `create_object`：工厂 NewObject + 递归 `construct_from_resource`。
- **异步实例化**（大 UI 必须）：先拍平成 `DisplayListItem[]`，再分帧逐项 NewObject + 对象池回填。

### 12.6 扩展机制
照搬 fgui `SetPackageItemExtension`：包内某组件可由用户 Rust struct / Unity 类接管实例化。

---

## 12.7 滚动容器（ScrollPane）

> 游戏 UI 里可滚动容器（背包/聊天/设置/邮件/任务日志）数量远多于虚拟化长列表，且移动端要惯性/回弹/分页/吸附——没这些不可发布。fgui 的 ScrollPane 是挂任意 GComponent 的独立子系统，**虚拟化列表建在它之上**（§13.2）。

**模型**：Container 有一个"可滚动"模式（挂 ScrollPane，非新节点类型）。ScrollPane 持有：
- `content`：一棵子树（实际要滚动的内容）。
- `viewport`：可视矩形（视口）。
- `scroll_type`：Horizontal / Vertical / Both。
- `scroll_pos`：当前滚动偏移（content 相对视口的位移）。

**与布局的衔接**：
- taffy 布局算 `content` 子树的总尺寸（content size）。
- 视口尺寸 = Container 的 measured_size。
- 滚动偏移 `scroll_pos` 是 content 子树根的 transform 偏移（不重布局，只平移）。
- 视口裁剪 = Container 的 `clip_rect`（矩形硬裁剪，§8.5）。
- 滚动时只改 `scroll_pos`（transform）→ 廉价，不触发 taffy solve。

**能力清单**（对照 fgui ScrollPane）：
- 滚动类型（H/V/Both）
- **惯性 + 回弹**（移动端手感灵魂）—— v1
- **滚动条**（GScrollBar，自动显隐/样式）—— v1
- 鼠标滚轮（PC）—— v1
- 吸附 snapToItem（列表对齐到 item）—— v1.x
- 分页滑动 pageMode（引导页/角色选择轮播）—— v1.x
- 下拉刷新 header/footer（onPullDownRelease/onPullUpRelease）—— v1.x

**v1 范围**：基础滚动（惯性 + 回弹 + 滚动条 + 鼠标滚轮）。否则移动端连可滚动设置面板都演示不了。分页/吸附/下拉刷新 v1.x。

**虚拟化建其上**：`<l-list>`（§13.2）的虚拟化复用 ScrollPane 的视口/偏移/裁剪，额外加 slot 复用（§13.2）。不是并列两套。

---

## 13. 动态 UI / 数据模型

> 用户需求：运行时动态生成节点、动画、响应式自适应。这是**完整保留模式场景图 + 命令式 API**，不止列表。

### 13.1 命令式节点 API（v1 主力）
```rust
let c = Container::new();
c.add_child(img); c.remove_child(img); c.set_child_index(...);
node.set_text(...); node.set_position(...); node.set_style(...);
node.add_event_listener(Click, cb);
```
所有操作只置 dirty，帧末统一 solve + 重生成几何。

### 13.2 数据驱动的列表虚拟化（`<l-list>`，建在 ScrollPane 之上）

**槽（slot）复用模型**：核心维护**固定数量的可视槽**（槽数 = 可视窗口 + 少量缓冲，fgui 同款）。数据滚动时，核心把数据项**映射到槽**（item index → slot_id）：同一 slot 这一帧是 item 5、下一帧是 item 6，**slot_id 稳定，NodeId 变**。

- RenderNode 带 `slot_id`（§8.6 公共头）。后端 diff 时**按 slot_id 复用 GameObject**（同 slot 只更新内容：mesh/文本/纹理/数据），不销毁不重建 → 零 GC 抖动。
- **两个身份正交**：`NodeId` = 逻辑身份（事件/命中/核心侧对象），`slot_id` = 渲染复用身份（后端 GO 池 key）。普通节点 `slot_id = None`，后端按 NodeId 管；只有虚拟化列表 item 有 slot_id。
- **slot 概念在核心**：item→slot 映射是布局/数据逻辑，核心是真相源；后端只接收"slot_id + 内容"被动复用（跨 FFI 必须核心主导，不像 fgui 的 GObjectPool 在 C# 后端）。
- 列表声明模板 + 数据源；引擎传数据，核心展开到槽。**虚拟化建在 ScrollPane（§X）之上**，不是并列。

### 13.3 数据绑定（v1 简版，后期增强）
v1：命令式 API + 数据驱动列表。后期：声明式绑定（`data-bind:text="user.name"`），数据变化自动刷新 + 重布局。

### 13.4 响应式重布局
所有动态变化（resize / safe-area / 数据变 / 增删节点）→ 置 dirty → 下帧 taffy solve。布局天然支持动态。

### 13.5 性能对策（运行时 + FFI）
- 别每帧重建整棵 DSL；传结构化增量。
- 只 relayout 变化子树。
- 缓存：命中按帧、纹理/材质按 key、mesh 按 dirty、GameObject 镜像按 node_id 复用池。

---

## 14. FFI 与 Unity 后端

### 14.1 方案：csbindgen（已调研，推荐）
csbindgen 是为 Unity/IL2CPP 设计的主流绑定生成器（Cysharp 自家 MagicPhysX/NativeCompressions 全平台验证）。
- Rust 端 `#[no_mangle] extern "C"` + `csbindgen` 生成 C# `[DllImport]`。
- `csharp_use_function_pointer(false)` 切 Mono 模式（IL2CPP 友好）；`csharp_dll_name_if` 处理 iOS `__Internal`。
- `[GroupedNativeMethods]` 的 context 指针模式适合"持有 Stage 句柄"。

### 14.2 IL2CPP 必须注意的坑
- **回调必须 `static` + `[MonoPInvokeCallback]`**：IL2CPP 下 instance delegate 直接崩。影响 Rust→C# 回调（文本测量、NativeHost bounds 查询、事件）。
- **iOS**：静态库 + `[DllImport("__Internal")]`。
- **string 永远走 UTF-8 `byte*`**。
- **内存所有权严格隔离**：跨边界传 POD/指针/扁平 buffer。
- **P/Invoke 在 IL2CPP + Mono 都可用**，高频调用要控制 marshalling：用扁平数组（pin 或拷贝）。

### 14.3 跨边界的数据与内存模型

**两块 buffer，per-frame arena，C# tick 内拷完**：

```
每帧 FFI 传两块：
1. RenderNode 公共头 SOA 数组（定长字段并行存储）：
   node_ids[], parent_ids[], slot_ids[], visible[], alpha[], grayed[],
   color_tints[], transforms[], blends[], clips[], sort_keys[], payload_tags[]
   —— 定长，C# unsafe 直接读，无逐字段 marshalling 税。
2. 旁挂数据 arena（变长 payload 内容）：
   mesh 顶点/索引、TextLayout、stencil ref 等，按 payload_tag 分区。
   每个 payload 在 arena 里有 (offset, len)，公共头存此引用。
```

**内存模型**：
- 两块都是 **Rust 侧 per-frame arena**。C# 在 `tick` 返回前**拷贝完**到自己的预分配托管 buffer（拷贝而非 pin——避免 Rust 下帧 reset 时 C# 还在读的生命周期纠缠）。
- Rust 下帧开头 **reset arena**（不释放、复用）→ 零分配。C# 侧 buffer 也预分配复用 → 零 GC 压力（Unity 非分代 Boehm GC，每帧 `new` 会卡帧，必须避免）。
- **mesh 仅 dirty 时进 arena**：不 dirty 的帧该节点 payload 标"沿用上帧"，C# 不重传 mesh。
- **拷贝 vs pin**：选拷贝。代价是每帧一次 memcpy；几百~上千节点的 SOA 头是几十 KB，memcpy 可忽略；mesh dirty 才传。pin 的生命周期纠缠不值。
- **升级路径**：v1 用"全集 SOA + 后端 diff"。SOA 定长头 + 拷贝，成本是 O(可见节点数) memcpy，实测大概率够；不够再升级"变更列表"（核心已用 dirty 跟踪，能产出）——v1 不做，留路径。

**其它跨边界数据**：
- **Stage 句柄**：C# 持 opaque 指针（`IntPtr`）。
- **输入事件**：C# 每帧把采集的输入（指针/键/触摸/IME）打包成扁平数组传核心。
- **回调（Rust→C#）**：文本测量、事件回调——必须 static + MonoPInvokeCallback。
- **纹理**：核心只认 `TexId`；C# 上传纹理后注册 id↔Texture2D 映射。

### 14.4 Unity 后端职责
1. MonoBehaviour 驱动：每帧 `tick(input, dt)` → 取 `render_nodes` → 同步 GameObject 镜像。
2. **GameObject 镜像池**：`node_id → GameObject`，diff 渲染树增删复用；每节点 `MeshFilter+MeshRenderer`。
3. **同步**：上传 mesh 到 MeshFilter（核心给的几何）；按 `(shader+flags+blend+texture)` 从 `MaterialManager` 取/建 Material 设到 MeshRenderer；设 transform、`sortingOrder`、blend/stencil state、clip uniform。
4. **NativeHost**：`is_native_host` 节点放用户 GameObject（粒子/3D/自定义），按布局 transform/clip 放置，不画自有 mesh。这是 Unity 特效接入点。
5. **输入采集**：Unity 新/旧输入系统 → 扁平事件（含 IME character）。
6. **资源加载**：Addressables/YooAsset → 纹理上传 → 注册 TexId。
7. **坐标翻转**：Unity 屏幕左下原点 ↔ 核心左上原点。
8. **世界空间 UI**：根 panel GameObject 可放世界空间 + 摄像机（等价 fgui UIPanel world space）。

> 后端的 `MeshFilter+MeshRenderer+MaterialManager+sortingOrder` 层就是 fgui 的 NGraphics/DisplayObject/MaterialManager 在我们架构里的等价物，但**几何数据来自核心**，后端不生成几何。

### 14.5 构建管线
- Rust 交叉编译产出多平台原生库（`.dll`/`.so`/`.dylib`/iOS `.a`/Android `.so`）。
- 放 Unity `Plugins/`，配 Platform/CPU。
- csbindgen 生成 C# 绑定源码纳入 Unity 工程。

### 14.6 GameObject 镜像的生命周期与性能

**所有权与真相源**：Rust 核心拥有场景图（NodeId 树）+ 全部渲染状态，是**真相源**；Unity 后端拥有 **GameObject 镜像**（`Dictionary<NodeId, GameObject>`），是**派生缓存**。Rust **绝不**创建/销毁 Unity 对象，后端负责。

**每帧同步是"脏增量"，不是全量重刷**：
- 核心每帧产出可见节点的 `RenderNode` 集合；后端 diff 镜像：新增 NodeId → 建 GameObject；消失 → 销毁；都在但状态变 → 更新；状态没变 → **no-op（廉价相等判断）**。
- 静态 UI 每帧同步成本≈0（只有相等判断）。贵的 mesh 重生成**只在 dirty 时**。
- v1 传"可见节点全集 + 廉价 diff"；v2 可优化为"变更列表"（核心已用 dirty flag 跟踪）。
- 真正每帧开销是 **Unity 自身遍历 MeshRenderer 做剔除/批合/提交**——任何 GameObject UI 都有此成本，靠材质复用 + FairyBatching 缓解（fgui 同款，够用）。纯 2D 重 UI 性能不够 → 升级 §17 v2 的 SRP 混合渲染。

**句柄：Rust 不持任何 Unity 句柄**。
- 所有 GameObject（自绘节点的 + NativeHost 的用户 GameObject）都由**后端拥有**，后端维护 `NodeId → GameObject` 映射。
- Rust 只持 `NodeId`（整数）。每帧渲染树带 `node_id` + payload（§8.6）；后端据此查自己的映射定位/放置 GameObject。
- NativeHost 的用户 GameObject 由用户经**后端 API** 注册（`BindNativeHost(nodeId, userGO)`），Rust 完全不参与、永不解引用。

**注意区分两件事**（勿混）：
1. **渲染树数据传递**：FFI 传的是**完整渲染树**（§14.3 的 SOA + arena，含 transform/mesh/clip 等全部状态），不是"只传 NodeId"。
2. **Unity 对象引用隔离**：Rust **不持、不解引用**任何 Unity 对象（GameObject/Texture/Material）。跨 FFI 只传 NodeId/TexId 等整数 id + 数据 buffer；Unity 对象引用全在后端，由后端按 id 映射。NativeHost 的用户 GO 也是后端拥有。

**生命周期/回收**：
- 节点 Dispose → 下帧不在渲染树 → 后端销毁其 GameObject（一帧延迟可接受；或核心显式发"已移除 NodeId 列表"立即清理）。
- **对象池**：虚拟列表回收的 item，后端**复用** GameObject 而非销毁/重建（渲染树标 `recyclable` 提示），避免 GC 抖动。
- NativeHost Dispose：核心通知后端 **detach（不销毁，用户拥有该 GameObject）**。
- **无 double-free / use-after-free**：Rust 只持整数/不透明句柄、从不解引用 Unity 对象；两侧各管各的对象，靠 diff + 句柄通信。

---

## 15. 更新循环（每帧管线）

```
引擎 LateUpdate / 固定 update:
  1. gather_input()                  ← 后端采集指针/键/触摸/IME，扁平数组注入
  2. stage.tick(input, dt):
     a. Timers.update(dt)
     b. TweenManager.update(dt)      ← 唯一动画时钟；tweener 回调写节点属性（置 dirty）
     c. layout dirty → taffy solve（子树）→ 写 measured_size/layout_rect
        （含文本/NativeHost 的 MeasureFunc 回调，可能回调后端）
     d. 命中测试（按帧缓存）→ 事件路由（capture + bubble）→ 业务回调（可能再置 dirty）
        （若事件改了布局，回 c 再 solve 一次）
     e. 渲染 pass: DFS 整树
        - mesh dirty → 重生成几何（MeshFactory）
        - 累积 alpha/grayed/裁剪栈（save/restore）
        - FairyBatching 重排 → 分配 sort_key
        - 收集 RenderNode
     f. 输出 Vec<RenderNode>（按 sort_key 排序）
  3. 后端消费 render_nodes → 同步 GameObject 镜像 → Unity 提交渲染
```
关键：动画/事件可能改属性触发再布局，管线要允许"事件后重 solve"一轮。NativeHost 的 MeasureFunc 会跨 FFI 回调后端查询 bounds。

---

## 16. Rust workspace 代码结构（建议）

```
loomgui/
├── loomgui_core/           # 引擎无关核心（纯库，可单测）
│   ├── parse/              # HTML(scraper) + CSS(cssparser) + selectors
│   ├── style/              # cascade 子集 → ResolvedStyle
│   ├── layout/             # taffy 集成 + MeasureFunc
│   ├── scene/              # Node 树、Container、各叶子类型、NativeHost
│   ├── render/             # VertexBuffer, MeshFactory*, FairyBatching, clip stack, RenderNode
│   ├── text/               # rustybuzz + ttf-parser + linebreak + bidi → TextLayout
│   ├── event/              # 命中、bubble/capture、capture_touch、drag、focus
│   ├── anim/               # TweenManager, Transition, Controller, Gear, Timers
│   ├── asset/              # 包格式、TextureView、refcount、load/instantiate
│   └── stage.rs            # Stage: tick(input,dt) → render_nodes
├── loomgui_ffi_c/          # C ABI 导出（extern "C" + 手写薄包装）
├── loomgui_unity/          # csbindgen 生成 C# 绑定 + Unity 后端（GameObject 镜像/MaterialManager）
├── loomgui_editor/         # 后期：编辑器（Web/Tauri，WASM 调同一核心）
└── tests/                  # 核心单测 + 快照测试（DSL→render_nodes）
```
核心可编译为 WASM（给编辑器）和 C ABI（给引擎），同一份代码。

### 16.1 依赖库重量评估

| 库 | 用途 | 重量 | 运行时需要? | 替代/策略 |
|---|---|---|---|---|
| `scraper`（+`html5ever`+`selectors`） | HTML 解析 + 选择器匹配 | 重（Servo 全家桶） | 否（运行时用二进制包） | **保留**（浏览器级健壮，白拿）。**feature-gate 进 `parse` feature，只编进编辑器/WASM，运行时不带** |
| `cssparser` | CSS 声明解析 | 中轻 | 否 | 可手写围栏子集解析器进一步减重；或保留 |
| `taffy` | flexbox 布局 | 中（纯 Rust） | **是** | 无更轻的正确替代（morphorm 旧/不活跃；yoga 需 C++）。保留 |
| `rustybuzz` | 文本 shaping | 中（纯 Rust） | 是（多脚本） | 仅 CJK+Latin 可用更简单 shaper；需阿语/连字则保留 |
| `ttf-parser` | 字体度量 | 轻 | 是 | 保留 |
| `unicode-linebreak` / `unicode-bidi` | 换行 / BiDi | 轻 | 是 | 保留 |
| `fontdue` | 字形光栅化 | 中 | 否（v1） | v1 后端原生渲染不需要；自绘光栅化才上 |

**结论**：解析层（scraper 全家桶）重，但运行时**完全不需要**（用二进制包，§12）→ **feature-gate 进 `parse` feature，只编进编辑器/WASM，游戏运行时二进制里没有 parser**。既然不进运行时，scraper 的健壮性白拿，保留即可（无需换 `tl`）。

---

## 17. 分阶段路线图

### v0 核心骨架（验证架构）
- loomgui_core: Stage.tick 框架、Node/Container 基础、scraper+cssparser 解析最小子集、taffy 接入、纯 Rust 单测。
- 一个"HTML → taffy 布局 → 打印 layout_rect"的端到端通路（不渲染）。

### v1 最小可用（Unity 上能跑）
- 渲染：贴图 quad + 纯文本 + 硬矩形裁剪 + FairyBatching 重排 + 绘制顺序；**Unity 后端 GameObject 镜像 + MaterialManager + 提交**。
- 文本：rustybuzz 测量/断行 + 后端据 TextLayout 生成 mesh。
- 事件：命中 + click/hover + 基本拖拽 + **UI 输入消费（is_pointer_on_ui）**。
- **滚动容器**：基础滚动（惯性 + 回弹 + 滚动条 + 鼠标滚轮，§12.7）。
- **参考分辨率缩放**：设计稿等比缩放（§7.3 补）。
- FFI：csbindgen 通路 + SOA+arena 渲染树同步（§14.3）。
- 资源：二进制包加载（最小）+ 图集 TextureView + refcount。
- 围栏：§4 子集的稳定子集。

### v1.x 增强
- 富文本 + 内联对象；九宫格/平铺/填充；软裁剪/模板遮罩/paintingMode；动画(GTween)+Transition+Controller+Gear；列表虚拟化（建在 ScrollPane 上）；滚动分页/吸附/下拉刷新；动态节点 API 完整化；自定义控件扩展；**IME 完整链路 + 软键盘（移动端）**；**字体 fallback 链**；**NativeHost（Unity 特效接入）**。

> **关于多窗口/弹窗/模态/Tooltip/Popup**：不进核心。Stage 就是 Stage、一个根；要多个独立 UI 层级（HUD/弹窗/Tooltip）就多 new 几个 Stage，由上层/用户组合。窗口基类、模态遮罩、Popup 栈这些是上层封装，不在框架核心——保持核心简单。fgui 的 GRoot/Window 是它的封装选择，不照搬。

### v2
- Godot 后端（镜像成 Control/Node2D 或 RenderingServer canvas item）；编辑器；grid 布局；高级滤镜/shader。
- **SRP 混合渲染**（Unity）：自绘节点用自定义 SRP RendererFeature 批合绘制（少 draw call），NativeHost/特效仍是 GameObject——性能 + Unity 集成兼得。渲染树契约不变。

---

## 18. 风险与开放问题

| # | 风险/问题 | 说明 | 缓解 |
|---|---|---|---|
| R1 | **后端工作量** | Unity 后端要写 GameObject 镜像池 + MaterialManager + 裁剪/遮罩/blend + NativeHost——等价 fgui 的 NGraphics 层，是最大工作块。 | 增量：v1 只 quad+文本+硬裁剪；特性渐进加；几何来自核心，后端只同步，比 fgui 轻。 |
| R2 | **Unity IL2CPP FFI** | 回调必须 static+MonoPInvokeCallback；iOS 静态库；高频 marshalling；NativeHost/MeasureFunc 回调。 | csbindgen + 扁平 buffer + 严守所有权边界。 |
| R3 | **文本测量一致性** | 测量(rustybuzz)与渲染(引擎字体)必须同源。 | 测量/断行全在 Rust；后端关自动换行按 run 渲染。 |
| R4 | **性能：GameObject 数量** | 每节点一个 GameObject，复杂 UI 的 GameObject 数/draw call 靠 Unity 批合。 | MaterialManager 材质复用 + FairyBatching 重排；纯 2D 重子树后期 mesh 合并优化。 |
| R5 | **HTML/CSS 围栏边界** | 子集太宽难实现/不稳，太窄不实用。 | v1 起步更窄，按真实需求扩。 |
| R6 | **stencil 层数** | 位掩码左移，最多 ~8 层模板遮罩嵌套。 | 文档标注上限；超限降级。 |
| R7 | **从"立即推送"到"帧末一致"** | fgui 用户习惯改 A 立即让 B 变；我们统一帧末 solve。 | 文档明确；事件后允许再 solve 一轮。 |
| R8 | **GameObject 镜像同步正确性** | diff 渲染树增删复用 GameObject，易出 reuse/泄漏 bug。 | node_id 池 + 脏粒度 + 单测；参考 fgui DisplayObject 生命周期。 |
| O1 | **Controller/Gear 的 CSS 表达** | `:l-page(n)` 自定义伪类 + data 属性是否够表达全部 gear？ | v1 先支持常用 6 种（Display/Look/Xy/Size/Color/Text）。 |
| O2 | ~~包格式 vs 直接 HTML~~ **已定：运行时只走二进制包** | 运行时调试不需 HTML；热重载 = 重新编译 DSL→二进制再热重载二进制 | **运行时只支持二进制包**；HTML 解析（scraper 全家桶）只在编辑器/工具链，永不进运行时。也使 §16.1 的 parser feature-gate 天然成立。 |
| O3 | **异步实例化粒度** | 分帧阈值如何定？ | 实测调参，暴露配置。 |
| O4 | **Unity 渲染提交方式** | v1 MeshRenderer；v2 SRP？ | v1 用 MeshRenderer（每节点 GameObject，集成最好、最快落地）。v2 升级为 **SRP 混合**：自绘节点在自定义 SRP RendererFeature 里批合绘制（少 draw call），NativeHost/特效仍是 GameObject——渲染树契约不变，只换后端执行策略。 |
| O5 | **NativeHost 的裁剪** | 用户 GameObject 上的 Unity 粒子/3D 如何被 UI 裁剪/遮罩？ | v1 只支持矩形裁剪（设一个裁剪用 render texture / shader clip）；复杂遮罩后期。 |

---

## 19. 附录：与 FairyGUI 的对照（照搬 / 替换）

| 子系统 | 照搬 | 替换 / 升级 |
|---|---|---|
| 对象模型 | Container 唯一持 children；叶子类型映射；sortingOrder 双轨；三类脏标记；pivotAsAnchor | 核心单 Node（无 GameObject）；DisplayObject 等价物搬到后端（GameObject 镜像）；几何生成在核心（fgui 在后端） |
| 布局 | —— | Relations 整套 → taffy flexbox；尺寸字段→taffy size 约束 |
| 渲染 | VertexBuffer+MeshFactory 全套；rotated UV；MaterialFlags/BlendMode/ShaderId；FairyBatching；裁剪四机制；MaterialManager；VertexMatrix | 几何生成移入 Rust 核心；后端只镜像+上传；扁平 draw-call 流→渲染树 |
| 批合 | FairyBatching 保序重排+AABB 不相交；批合根=裁剪/遮罩边界；局部批合 | sort_key 由核心算（含 clip 上下文） |
| 事件 | DOM 三阶段；命中=布局AABB+逆序；capture_touch；click 阈值；rollOver diff；Tab 导航 | 双层 bridge 合一（核心单 Node）；InputProvider trait；触摸槽数 N |
| 动画 | GTween 单时钟；Transition=编排器；Controller 纯状态；Gear 状态→属性+tween；Timers | Tweener/Gear 用 enum/struct；回调用 ListenerId |
| 资源 | 双格式；包结构；TextureView+refcount；分支/高清；异步实例化；SetPackageItemExtension | 源格式用 HTML/CSS；包格式自定二进制 |
| 文本 | TextField 测量+渲染共享；富文本标记子集；BMFont/DynamicFont；内联对象参与断行 | 测量/断行→rustybuzz+ttf-parser+linebreak+bidi；后端只渲染 run |
| FFI | —— | Rust 核心 + csbindgen（fgui 无此问题，纯 C#） |
| **Unity 集成** | DisplayObject=GameObject 路线（每节点 GameObject） | 渲染树镜像 GameObject；NativeHost 节点接 Unity 特效；世界空间 UI |

---

## 20. 已知缺口登记（已识别、待展开设计）

> 这些是审查中识别出、但本轮设计未展开的项。**v1 不做，但必须记着**——避免"later means never"。每条标了归属阶段、为什么需要、对照 fgui、待定问题。后续回来逐条设计。

| # | 缺口 | 阶段 | 为什么需要 | 对照 fgui | 待定问题 |
|---|---|---|---|---|---|
| G1 | **IME 完整链路** | v1.x | 中日文输入：候选词窗口跟光标、composing 下划线、取消。亚洲市场上线 blocker | `InputTextField._composing` + WebGLTextInput | 核心产 caret 屏幕坐标给后端？组合文本如何作为下划线 run 渲染？WebGL IME 单独地狱级 |
| G2 | **软键盘（移动端）** | v1.x | 登录/改名/聊天必备，弹起还要顶起输入框避遮挡 | `Stage.OpenKeyboard`/`keyboardType`/`hideInput` | 软键盘高度 insets 复用 safe-area 机制？iOS/Android 弹出+收字 v1.x |
| G3 | **字体 fallback 链** | v1.x | 拉丁+CJK+emoji 混排没回退会显示方块；rustybuzz 不做回退 | `FontManager.Fallback` + 多名字体 | 核心维护 FontStack，按 cluster cmap 覆盖检测拆 run 到不同字体 |
| G4 | **NativeHost（Unity 特效接入）** | v1.x | UI 特效（粒子/3D）作为节点子物体；世界空间 UI | UIPanel 挂 3D 内容 | 用户 GO 经 `BindNativeHost` 注册；裁剪 v1 只矩形（O5）；MeasureFunc 改后端 push 尺寸非核心 query（M1） |
| G5 | **多窗口/弹窗/模态/Tooltip/Popup** | 不进核心 | 上层封装，用户多 new Stage 组合 | GRoot/Window/PopupStack | **已定：不照搬进核心**。保持 Stage 简单 |
| G6 | **DragDrop（替身图标拖放）** | v1.x | 背包拖道具到快捷栏、卡牌拖场上（区别于节点自移 draggable） | `DragDropManager` | 独立于节点 draggable 的子系统 |
| G7 | **手势（Gesture）** | v1.x | 长按/双指缩放/旋转/滑动（地图缩放、卡牌预览） | `Gesture/` 目录 | 建在 §10 事件之上 |
| G8 | **运行时本地化（translation）** | v1.x | 一份包多份语言表，按 key 替换文本（区别于 branch 资源级切换） | `TranslationHelper` | branch（资源级）vs translation（文本级）分工 |
| G9 | **音效与 UI 事件绑定** | v1.x | 点击/hover/获奖配音，商业 UI 标配 | `UIConfig.buttonSound`/`PlayOneShotSound` | 核心定义事件→音效映射，后端播放 |
| G10 | **性能统计（Stats）** | v1.x | 调试期必备：node 数/render node 数/本帧 mesh 重生成/draw 数 | `Stats.cs` | 核心 debug stats API，零成本 |
| G11 | **通用对象池（GObjectPool）** | v1.x | 飘字/伤害数字/聊天气泡频繁创建销毁要池化，不只列表 | `GObjectPool` | 核心级策略，池生命周期与 Dispose 语义 |
| G12 | **grid 布局** | v1.x | 背包格/技能盘/装备槽经典 grid，flex+wrap 硬凑别扭 | —— | taffy 已支持 grid，围栏放开即可 |
| G13 | **轻量 CSS transition** | v1.x | hover/press 属性微变化用 `transition:0.2s` 比每次 GTween 简洁 | —— | 映射到 GTween，与 Gear 的 `-l-transition` 统一 |
| G14 | **世界空间 UI** | v2 | 3D 场景血条/商店招牌/交互提示 | UIPanel RenderMode WorldSpace | render mode 枚举、命中坐标转换、多 panel 输入仲裁；NodeTransform 的 VertexMatrix 启用 |
| G15 | **SRP 混合渲染** | v2 | 自绘节点 SRP 批合（少 draw call）+ NativeHost 仍是 GameObject | —— | 渲染树契约不变，只换后端执行策略；混合渲染顺序统一 |
| G16 | **Godot 后端** | v2 | 第二引擎 | —— | 镜像成 Control/Node2D 还是 RenderingServer canvas item 待定 |
| G17 | **编辑器** | v2 | DSL 可视化产出（v1 不做，HTML 谁写是项目级风险 R9） | FairyGUI 编辑器 | Web/Tauri + WASM 调同一核心；最小只读预览可能提前到 v1 |

> **审查中已修复的缺口**（不再在此表）：滚动容器（§12.7，进 v1）、UI 输入消费（§10.6，进 v1）、参考分辨率缩放（§7.4，进 v1）。

---

## 21. 下一步

本文档经评审定稿后，按 §17 路线图拆分**实现计划**（每阶段拆成可独立验证的任务：解析子集→布局→渲染最小闭环→文本→事件→FFI→Unity 后端(GameObject 镜像)→资源→动画→...）。本轮止于设计文档，不写实现。
