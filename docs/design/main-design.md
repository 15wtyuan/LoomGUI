# Yio 主设计

> 跨引擎游戏 UI 框架。Rust 核心（引擎无关纯库）+ 多引擎后端（Unity 首发，Godot 等），标准 HTML/CSS 子集作设计期 DSL，类型化对象树作运行时 API，自绘渲染。
>
> **关联权威契约**：[fence.md](fence.md)（围栏）、[public-api.md](public-api.md)（公共 API 终态契约）、[projection-layer.md](projection-layer.md)（C# 投影层机制）。本文定总体架构与渲染管线；公共 API 以 public-api.md 为准。

## 1. 目标与非目标

### 1.1 目标

- **G1 编辑一次，多引擎一致**：同一份 HTML/资源包，在 Unity 及后续引擎上布局/文本/几何一致。
- **G2 标准 Web 语义**：HTML 围栏遵循标准 HTML/CSS 语义（Block/Flex/Inline），AI 读代码能正确预测渲染结果。
- **G3 类型化对象树**：运行时 API 是类型化的 Node 对象树（Container/Button/Slider/...），不是全局句柄 + 命令式 stage 调用。
- **G4 运行时动态**：UI 在运行时可任意增删改节点、跑动画、响应数据变化。
- **G5 渲染质量**：自绘、批合、遮罩/裁剪、九宫格、富文本；可挂引擎特效、世界空间 UI。
- **G6 可扩展**：标准控件 + 用户自定义业务组件（Web Components 约定）共存。

### 1.2 非目标

- 不做完整浏览器引擎（无完整 IFC、无 float、无 grid）。
- 不做 Unity UGUI/UIToolkit 兼容层。
- 编辑器单独项目，本文只定 DSL 规范、运行时 API 契约与渲染管线。

**对标取舍**（参考实现 FairyGUI/RmlUi/Unity UI Toolkit，镜像在 `temp/`）：不照搬 fgui 的绝对定位中心模型、Gear/Controller DSL、命令式 tween、全局单例与 data 挂载点；不照搬 RmlUi 的低层 DOM 操作感；采用 UI Toolkit 的 Style/Transform/Geometry 三分。

---

## 2. 总体架构

### 2.1 分层

```text
标准 HTML/CSS 子集（设计期 DSL，人/编辑器/工具链/AI 读写）
        │ pack + validate（打包期验证围栏，拒绝不支持的语法）
        ▼
不可变 UITemplate / Package（.pkg.bin + 图集）
        │ instantiate（克隆模板 → 类型化对象树）
        ▼
类型化语义对象树（Node / Container / Button / Slider / ...）
  - 公共 API：UIContext / Get<T> / 事件 / typed Style / 生命周期
        │ computed style（cascade + 伪类 rematch）
        ▼
布局、滚动、文本等内部 Behavior Strategy
  - Block/Flex/Overflow/Scroll 策略切换，不改变对象类型
        │ frame model
        ▼
渲染树（Vec<RenderNode>，意图化契约）
        │ FFI（SOA 扁平数组，引擎中立）
        ▼
引擎后端（Unity GameObject+MeshRenderer / Godot Node2D+canvas_item）
```

> **「单向」指每帧数据流，不指模块依赖**：core 内部各模块（style/scene/layout/render/input/text/scroll/list）共享以 `Scene`（类型化节点树 + per-node 状态表）为中心的读写，模块间存在双向依赖（如选择器引擎遍历 Scene、输入驱动控件编辑原语）。上图箭头是每帧的数据流契约——顺序正确性由 CI 门锁定（§16）；不要把本图读成「模块 import 必须单向」的分层律。

### 2.2 关键边界

- **公共语义层**：类型化 Node 对象树，是游戏业务程序员的唯一 API 表面。
- **内部行为层**：布局策略、滚动物理、文本排版、渲染状态计算。使用 Strategy/State/Bridge/Pool 等模式，不暴露给公共 API。
- **FFI 缝界**：SOA 扁平数组传渲染树 + 事件回传。NodeId 不出现在公共 API。
- **引擎后端**：输入采集、渲染树→原生对象镜像、资源加载。
- **不跨越的**：公共层不知道 GameObject/CanvasItem；后端不解析 HTML/CSS、不独立算布局、不生成几何。
- **fence→core 单向类型归属**：fence 破例单向产 core 的选择器/rule 类型（fence 依赖 core，无适配层），但不产 core 节点树类型——fence 解析停在 IrTree，IrTree→TemplateNode 桥归 packer（fence 保持纯解析器）。

### 2.3 架构原则

> **公共层暴露语义和意图；内部层实现变化。只有业务真正拥有决策权的策略才进入公共 API。**

- Composite：Node/Container 对象树。
- Abstract Factory：根据稳定 HTML 语义签名（base 标签按 tag；控件/列表按 `role`）创建控件。
- Strategy + State：CSS 在不改变对象类型的前提下切换 Block/Flex、Overflow 等行为。策略只持算法，不持节点状态。
- Observer + 路由链：控件语义事件与捕获/冒泡事件。
- Bridge/Adapter：隔离 core、FFI 和具体引擎后端。
- Object Pool：ListView 按模板分别复用实例。
- Identity Map：同一个内部节点始终对应同一个公共 Node 对象。

---

## 3. HTML/CSS 围栏

> **权威清单 = `docs/design/fence.md`**（真相源是可执行测试 `crates/fence/src/schema/ + crates/fence/tests/`）。本节只写设计哲学与原则。

### 3.1 设计哲学：标准 HTML 语义 + AI 强先验

围栏是面向游戏 UI、能够完整兑现语义的标准 HTML 子集。不是假装支持整个浏览器，也不是四个标签的极小集。

**首要判据**：AI 读 HTML 能否正确预测渲染结果。所有围栏决策的第一判据。

**用标准 HTML 元素**：AI 训练数据海量、浏览器原生渲染。不自创框架 Widget 标签（如 `<scroll-view>`）——已有的标准 HTML/CSS 能力（如 `overflow`）不用自定义标签重复。

**标准布局语义**：
- `div` 默认 `display:block`（标准浏览器默认）；`button/img` 默认 inline（必须放进 flex 容器，见 fence §6.5）；`span` 是文本级行内元素（**默认归 rich_text_block**——含文字时走 inline flow 整体测量 text+padding，而非 inline→flex 容器；显式 `display:flex` 才留 flex，见坑 202）；`template` 默认 `display:none`；`slot` 透明继承父级。
- 控件与列表无专属标签——作者在 `<div>` 上写 WAI-ARIA `role` 表达（`role=slider`/`role=list`/...），视觉部件用 `data-slot`（`data-slot=fill`/`thumb`）。详见 [fence.md](fence.md) §2.3。
- `display:flex` 默认 `flex-direction:row`（标准 CSS 默认）。
- 需要纵向堆叠明确写 `display:flex; flex-direction:column`。
- `display:block/flex/none` 选择内部布局 Strategy，**不改变节点类型**。
- `box-sizing`：Yio **content-box 定版**（CSS 规范初始值——AI 按标准先验预测渲染，#116 拍板，否决 08-24 的 border-box 文案与 #115 的反转提案）；`padding adds to the set width/height`（`width:420px + padding:22px` 渲染 464 外框），作者声明被 fence 硬拒并给减法指引（`css_resolve.rs`）。唯一例外：**根节点 border box = 视口**（solve 期把 Stage 钉入的 root_size 连同 `box_sizing=BorderBox` 一起覆写根 taffy style——浏览器 ICB 同构；root 自身 padding 内缩而非外溢，作者对 root 的显式尺寸声明不参与）。全屏页惯用法 = `width:100vw; height:100vh` + 零 padding，内缩交给子级。

### 3.2 围栏元素

围栏 15 标签 = 8 shell + 7 runtime（真相源 = [fence.md](fence.md) §2）。控件与列表无专属标签，用 `role` 表达：

| 类别 | 元素 | 公共类型/语义 |
|---|---|---|
| 文档与样式 | `html/head/body/title/meta/style/link[rel=stylesheet]/script` | 打包和 authoring 元数据，不进入实时树 |
| 结构 | `div` | `Container`（block 默认） |
| 文本 | `span` | `TextElement` |
| 操作 | `button` | `Button` |
| 图片 | `img` | `Image` |
| 链接 | `a` | `Link`（富文本内链接，仅 rich-text-block 上下文合法，#74） |
| 控件 | `div role=...` | `Slider`/`Toggle`/`RadioButton`/`TextField`/`TextArea`/`NumberField`/`ProgressBar`/`Dropdown`（见 [fence.md](fence.md) §2.3） |
| 列表 | `div role=list` / `div role=listitem` / `div role=option` | `ListView`/`ListItem`/`OptionItem` |
| 复合控件 | `div role=tablist` / `div role=tab` | `TabList`/`Tab`（panel `role=tabpanel` 不分派，走 div→Container，靠 `aria-controls` 跨树关联） |
| 层级列表 | `div role=tree` / `div role=treeitem` | `Tree`/`TreeItem`（直接嵌套声明，无 group 包装层；branch 折叠/leaf） |
| 模板 | `template` | 惰性 `UITemplate`，不进入实时树 |
| 内容投影 | `slot` | Custom Element 的标准 Slot |
| 自定义元素 | `tag-name`（含 hyphen） | `CustomElement`（R3 注册验证） |

### 3.3 稳定语义签名

> **节点类型由稳定 HTML 语义签名决定：base 标签按 tag；控件/列表按 WAI-ARIA `role` + `aria-*`。CSS 永远不决定类型。**

完整签名表见 [fence.md](fence.md) §3.1。`resolve_semantic(tag, role, aria_multiline)`：`role` 优先于 tag，未识别的 role 回退到 tag 映射：

- `<div>` → `Container`；`<span>` → `TextElement`；`<button>` → `Button`；`<img>` → `Image`；`<a>` → `Link`
- `<div role="slider">` → `Slider`；`<div role="switch">` → `Toggle`；`<div role="radio">` → `RadioButton`
- `<div role="textbox">` → `TextField`（默认）；`<div role="textbox" aria-multiline="true">` → `TextArea`
- `<div role="spinbutton">` → `NumberField`；`<div role="progressbar">` → `ProgressBar`
- `<div role="combobox">` → `Dropdown`；`<div role="option">` → `OptionItem`
- `<div role="list">` → `ListView`；`<div role="listitem">` → `ListItem`
- `<div role="tablist">` → `TabList`；`<button role="tab">` / `<div role="tab">` → `Tab`（panel `role=tabpanel` 不分派，走 div→Container）
- `<div role="tree">` → `Tree`；`<div role="treeitem">` → `TreeItem`（条目直接嵌套：treeitem 内嵌 treeitem）
- `<template>` → `Template`；`<slot>` → `Slot`
- 含 `-` 的标签名 → `CustomElement`（R3 注册验证）

`role` + `aria-multiline` 是打包期确定类型的不变量，实例化后不能改成另一种控件类型。普通动态状态（`checked/selected/disabled`，以 `aria-checked`/`aria-expanded` 等表达）可变。控件初始值放 ARIA（`aria-valuenow`/`aria-checked`/...）或 `data-*`（`data-step`/`data-name`）属性。

### 3.4 WAI-ARIA 复合控件

控件与列表用白名单内的标准 WAI-ARIA Pattern（`role=slider`/`role=list`/`role=combobox`/...），不使用 `data-widget` 或 `data-controller`，也不自创私有 role 名。控件视觉部件用 `data-slot`（`data-slot=fill`/`thumb`）表达——ARIA 把 progressbar/slider 当原子控件，内部构造无 ARIA 语义，`data-*` 是 HTML 为私有扩展预留的标准机制：

```html
<div role="slider" aria-valuenow="50" class="slider">
    <div data-slot="fill"></div>
    <div data-slot="thumb"></div>
</div>
```

框架负责输入导航、`aria-selected`/`aria-checked`/`aria-expanded` 状态同步。打包器验证 role 组合与必需子结构（fence §6.8）+ ARIA 关系。

首个落地的复合控件是 **TabList**（M3，2026-08）。tab 高亮靠 `[aria-selected="true"]` 属性选择器，该属性是双向语义：作者在 HTML 里声明的 `aria-selected="true"` 是**初始选中种子**（打包期派生进 selected_index，多重声明取首个、无则 0），运行时该属性反转为只读合成值（从父 TabList.selected_index 跨节点合成——Tab 无 ControlState，像 OptionItem 从父 Dropdown 派生选中态）。panel 显隐靠 `aria-controls="panelX"` ↔ `id="panelX"` 跨树关联（panel 非 tablist 子节点，`RoleInfo.attrs` 通用属性仓（attrs β）存 linkage 字符串，`sync_control_visuals` 每帧用作用域安全查找（组件实例内解析，多实例不串）定位 panel）。panel 显隐由框架切 display：非激活强制 `display:none`（复用剪枝）；激活回落作者声明的 display 值。

### 3.5 失败策略

围栏外输入明确失败，不静默降级：

- 围栏外标签、属性、CSS 属性或属性值 → 打包期报错。
- 不支持的 `role` 或 ARIA 属性值 → 打包期报错。
- `display:grid` 在真正实现前留在围栏外，不能降级成 Flex。
- 围栏外 CSS 不静默忽略。

### 3.6 围栏治理（防漂移）

单一真相源 = machine-readable schema（标签、属性、结构属性、CSS 值、运行时类型、后端需求）。解析器、打包器、绑定生成器、文档和测试不得各维护一份白名单。

防漂移门：`cargo test -p yio_fence`——改围栏后必跑。

---

## 4. 公共对象模型

### 4.1 对象层级

```text
Node
├── Container（子树 = 用户内容，运行时可编排）
│   ├── AbsolutePanel（语法糖，子节点自动 absolute）
│   ├── TextElement（span）
│   ├── Button（button）
│   ├── ListView（role=list）/ ListItem（role=listitem）
│   ├── OptionItem（role=option，从属 Dropdown）
│   ├── Slot / CustomElement
├── TextNode / Image（叶子：内容 / 绘制）
└── 控件（叶子：子树 = 控件构造，公共 API 不暴露编排）
    ├── TextField（role=textbox）/ TextArea（role=textbox + aria-multiline）
    ├── NumberField（role=spinbutton）
    ├── Slider（role=slider）/ ProgressBar（role=progressbar）
    ├── Toggle（role=switch）/ RadioButton（role=radio）
    └── Dropdown（role=combobox）
```

- **Container vs Node 叶子的划线**：Container = 子树是「用户内容」（运行时可编排）；Node 叶子 = 子树是「控件构造」（设计期写定，框架管理，公共 API 不暴露编排）。控件仍是 `: Node` 叶子（不是 Container）——role 化改的是**谁写结构**（框架注入 → 作者写），没改**谁管结构**（仍是框架）。`OptionItem`/`ListItem` 是 Container（装用户内容），其从属关系在文档说明，不做类型层次强化。详见 [public-api.md](public-api.md) §2。
- `Container` 才暴露子节点增删；叶子类没有 `AddChild()`。
- `Button` 可包含图标和文本，因此属于容器。
- 公共对象持有稳定身份，内部句柄（NodeId）不暴露。
- **无 Panel/Component 类型**：作用域是运行时标记（`IsScopeRoot`），非类型；`Instantiate` 返回模板根真实类型。完整层级与划线见 [public-api.md](public-api.md) §2。

### 4.2 顶层上下文

`UIContext` 是显式顶层实例，拥有 Package、根节点、焦点、输入、时钟和后端连接。允许同进程存在多个独立上下文。**UIContext 是「获取而非创建」**——无公共构造，由引擎集成层创建并驱动；业务程序员从集成层获取一个已跑起来的 UIContext（见 [public-api.md](public-api.md) §11.3）。

```csharp
UIContext ui = backend.Context;                  // 由集成层提供，非 new
UIPackage game = ui.LoadPackage("game-ui", bytes);
Container home = game.Instantiate("views/home"); // 返回模板根真实类型
ui.Root.AddChild(home);
```

### 4.3 ID 与查询

标准 HTML `id` 是业务代码的结构契约。查找在组件实例作用域内递归，不穿透嵌套组件边界：

```csharp
Button start = home.Get<Button>("start");      // 缺失/类型不匹配 → UIContractException
home.TryGet<Button>("optional", out var btn);   // 可选
IReadOnlyList<Button> actions = home.Query<Button>(); // 零到多个
```

- 同一模板作用域内重复 ID 在打包期报错。
- 组件实例和 List item 模板实例各自拥有独立 ID 作用域（Shadow DOM 风格）。

### 4.4 生命周期

- `RemoveFromParent()` 只摘树。对象可重挂，属性、状态和监听器保留。
- `Dispose()` 才递归销毁子树、内部资源和事件订阅。
- 已销毁对象上的任何操作抛 `ObjectDisposedException`。
- detached 对象仍属于原 `UIContext`，不能跨 Context 挂载。

### 4.5 树操作

以对象为主语：`Parent`、`Children`、`ChildCount`、`AddChild`、`InsertChild`、`RemoveChild`。动态创建是次要逃生口：

```csharp
Container panel = ui.Create<Container>(); // canonical <div>
Button button = ui.Create<Button>();       // canonical <button>
panel.AddChild(button);
```

### 4.6 后续：强类型 View

结构稳定后可生成强类型 View：

```csharp
HomeView home = game.Views.Home.Instantiate();
home.Start.Clicked += OnStart;
home.Templates.MailItem;
home.Styles.Compact;
```

---

## 5. 样式

### 5.1 三条路径

1. authored HTML/CSS 是主要布局来源。
2. class 用于离散状态切换。
3. typed `Style` 用于运行时数值变化。

```csharp
panel.Classes.Add(HomeStyles.Compact);       // 生成的 StyleClass token
panel.Style.Width = Length.Px(320);
panel.Style.OverflowY = Overflow.Auto;
```

### 5.2 项目 class

项目 class 不能穷举成框架 enum。生成器从项目 CSS 产生 `StyleClass` token；无生成代码时保留 `AddClass("compact")` 和 raw style 逃生口。

### 5.3 CSS Cascade

- Specificity：标准 CSS tuple a-b-c（`inline > id > class > tag`）。
- 属性选择器与伪类同归 class 级（b）。
- **CSS 规则表进包（不 bake 丢）**：逻辑层运行时大量用 CSS（`Classes.Add/Replace`、`StyleSheet.Add`、class 切换驱动动画），规则表必须活到运行时，否则对设计期未带该 class 的节点 `Classes.Add` 会失效。cascade 引擎是 core 的运行时唯一真相源；fence 只把 `<style>` 解析成规则表。
- 运行时 rematch 处理伪类 + class + Style override 变化，每帧从 `base_style` 重算基线（`base_style` = 每帧 cascade 基线，非首帧缓存）。
- 运行时样式 = `base_style + 命中动态规则的合并`。动态规则含 `<style>` 打包规则与运行时 `StyleSheet.Add` 注入的全局规则（同 specificity 后注入赢，语义见 fence.md §5.4）；含 `var()` 的声明延后到 var 环境解析后应用（环境 = 祖先链 `--*` 声明合并，运行时 `SetVar` 为最高优先级层；解析失败该声明跳过、不抛异常）。

### 5.4 组件样式边界（Shadow DOM 风格）

- 模板内部选择器只作用于模板内部。
- 父组件普通选择器不穿透边界。
- 标准可继承属性和 CSS 自定义属性 `--*` 跨边界传递。
- 运行时注入（`StyleSheet.Add`）不受本墙约束：全局规则字面匹配可及组件展开内部（程序员工具非 AI 编辑面，public-api §10.2 明示）。
- `::part(name)`（#57 已交付）是打包期页面规则穿本墙的唯一许可通道：compound 前缀匹配 host、part 匹配展开子树内带 `part` 属性的目标节点（单层不递归，语义细目见 fence.md 选择器节）。运行时注入则本就不受本墙约束（上一条）。
- 组件 host 节点打三重标记（CSS 隔离 + 查找边界 + host 归属）：对后代 host 是 CSS 与查找边界；host 本体归外层页面作用域（页面规则可样式化 host、组件内部规则不落 host——同 DOM shadow DOM 不样式化 host，当前无 `:host`）。同模板多实例的 scoped 规则按 scope 各自包装，不按 selector 文本去重（按 selector 去重会误丢不同组件的同名 class 规则）。

---

## 6. 标准控件

控件用 `<div role="...">` 表达（§3.3）；ARIA 属性提供初始值，C# 属性表示实时状态。用户输入和代码修改走同一状态通道。视觉部件用 `data-slot`（如 slider 的 `data-slot=thumb`、progressbar 的 `data-slot=fill`）。

| HTML（role） | 公共类型 | 主要实时 API |
|---|---|---|
| `button` | `Button` | `Disabled`, `Clicked` |
| `div role=textbox` | `TextField` | `Value`, `Placeholder`, `ReadOnly`, `ValueChanged`, `Submitted` |
| `div role=textbox aria-multiline=true` | `TextArea` | `Value`, `Placeholder`, `Selection`, `ReadOnly`, `Disabled`, `ValueChanged` |
| `div role=spinbutton` | `NumberField` | `Value`, `Min`, `Max`, `Step`, `Disabled`, `ValueChanged` |
| `div role=slider` | `Slider` | `Value`, `Min`, `Max`, `Step`, `Disabled`, `ValueChanged`, `ChangeCommitted` |
| `div role=switch` | `Toggle` | `IsChecked`, `Disabled`, `CheckedChanged` |
| `div role=radio` | `RadioButton` | `IsChecked`, `Name`, `Disabled`, `CheckedChanged` |
| `div role=combobox` | `Dropdown` | `SelectedIndex`, `SelectedValue`, `Disabled`, `SelectionChanged` |
| `div role=tablist` | `TabList` | `SelectedIndex`, `Disabled`, `SelectionChanged`（panel 靠 `aria-controls` 关联，`role=tab` 是 `Tab` 容器节点，无独立控件 API） |
| `div role=tree` | `Tree` | `SelectedItem`, `ExpandAll`/`CollapseAll`, `SelectionChanged`（单选、焦点移动即选中；`role=treeitem` 是 `TreeItem` 容器节点：`IsBranch`/`Expanded`/`Level`/`Select`/`ExpandedChanged`） |
| `div role=progressbar` | `ProgressBar` | `Value`, `Max`, `IsIndeterminate` |

伪类 `:checked/:disabled/:focus` 匹配实时状态；Toggle/RadioButton 也可用属性选择器 `[aria-checked="true"]` 表达选中态。RadioButton 同 `name`（或 `data-name`）组框架自动互斥（只新选中项触发 `CheckedChanged`）；按 name 聚合的 RadioGroup 是逻辑层积木，作用域边界由 `IsScopeRoot` 标记决定。控件数值（Slider/NumberField/ProgressBar）用 `float`。完整控件契约见 [public-api.md](public-api.md) §7。

`ValueChanged` 表示实时变化；`ChangeCommitted` 表示拖动结束、回车或失焦确认。所有控件仍保留通用路由事件（`node.On<PointerDownEvent>(...)`）。

### 6.1 状态→视觉单向桥

core 读 `ControlState`，按 role/`data-slot` 定位**作者写的**子节点写 inline override——inline 是 HTML 语义最高优先级（> 动态规则 > base_style），作者 CSS 改不了状态驱动的几何：

- Progress/Slider 的 `data-slot=fill`：inline `width:%`。
- Slider 的 `data-slot=thumb`：inline user_transform 定位（可滑距离 = slider 宽 − thumb 宽，垂直居中；transform 是渲染/命中层，不触发 solve）。
- Toggle/RadioButton 无几何映射：走 `[aria-checked="true"]` 属性选择器。
- Dropdown open → `role=listbox` 子树 display；selected → `data-slot=value` 内嵌 TextNode 的文本。

`aria-*` 运行时合成总原则：aria 值不双存（防打包期初始值与运行时实时值双源），运行时从 ControlState 合成；派发提示类 aria 在 fence 阶段用完即弃、不进 pkg；唯一例外 `aria-controls`（不可从状态派生，作为纯数据随模板迁移）。

### 6.2 TextField 编辑内核

编辑状态以 UTF-8 字节偏移表达且恒落字符边界；选区 = `[min(anchor,cursor), max]`。`max_length` 按 UTF-8 字符数计（HTML maxlength 语义）、校验先于变更（超额干净拒绝、不删用户选区）、改小不追溯裁剪已有文本。输入 sanitize 滤控制字符（TextArea 保留 `\n`）。readonly 拦输入/删除/粘贴但不拦复制，编程 setter 仍可写；单行 readonly 框 Enter 仍发 `Submitted`。

掩码显示（`-webkit-text-security`）下显示串与 value 是两个字节空间，光标命中/几何经按字符数换算。IME composition 不另开 buffer：组合串拼进显示串并返回字节区间（下划线/光标几何/候选窗定位的统一真相源），commit 复用 insert 语义（选区删除/sanitize/max_length 一致）；NumberField 预编辑期不过滤、commit 时才过滤。NumberField 的 value 以文本形式存储，输入期只过滤非法字符，数值约束（clamp + step 量化 + 文本回写）在读写门执行；初始值取 `aria-valuenow/valuemin/valuemax` 与 `data-step`。

---

## 7. 模板、组件与复用

### 7.1 UITemplate

每个独立 HTML 资产都编译为不可变 `UITemplate`。界面、弹窗、业务组件和列表项只是模板被使用时扮演的角色。

### 7.2 内联模板

```html
<div role="list" id="mails">
    <template id="normal-mail">
        <div role="listitem" class="mail"><span id="title"></span></div>
    </template>
</div>
```

内联 `<template>` 只属于当前组件。打包期验证 list item template 根是 `role=listitem`。

### 7.3 包级共享模板

独立 `templates/mail-item.html` 可被多个界面引用：

```csharp
UITemplate item = common.GetTemplate("templates/mail-item");
```

模板资产只编译、缓存一份；每次实例化生成独立对象树、状态、事件和 ID 作用域。模板与实例化产物的关系同 Unity prefab：卸载模板不影响已实例化的活节点（独立副本）。

### 7.4 用户业务 Custom Elements

框架基础能力不得发明自定义标签。只有 HTML 没有对应概念的用户业务组件，才使用标准 Web Components 约定：

```html
<game-item-card id="sword" rarity="legendary">
    <button slot="action">装备</button>
</game-item-card>
```

- 名称必须包含 `-`。
- Package 注册表承担 `customElements.define()` 的角色。
- 标准 `<slot>` 提供内容投影。
- 未注册元素、无效 slot 在打包期报错。

---

## 8. ListView

声明使用 `role=list` + `role=listitem` + `<template>`：

```html
<div role="list" id="mails">
    <template id="normal-mail"><div role="listitem">...</div></template>
    <template id="reward-mail"><div role="listitem">...</div></template>
</div>
```

```csharp
ListView mails = view.Get<ListView>("mails");
UITemplate normal = view.GetTemplate("normal-mail");
UITemplate reward = view.GetTemplate("reward-mail");

mails.ItemCount = data.Count;
mails.TemplateSelector = index => data[index].HasReward ? reward : normal;
mails.BindItem = (item, index) => {
    item.Get<TextElement>("title").TextContent = data[index].Title;
};
```

契约：
- `role=list` → `ListView`，`role=listitem` → `ListItem`。
- 虚拟化是运行时实现决策（不进 HTML）；首次设 `ItemCount`/`ItemTemplate`/`BindItem` 即数据驱动 + 清空设计期 listitem。静态/数据驱动强制互斥（越界抛 `UIContractException`）。
- item 模板来源优先级：显式 `ItemTemplate`/`TemplateSelector` > 设计期 `<template id>` > 第一个 listitem 兜底。未设且 list 下单个 `<template>` 自动用、多个 `<template>` 抛 `UIContractException`。
- `TemplateSelector` 是纯 `Func<int, UITemplate>`；用户 `view.GetTemplate("name")` 取 template 后塞 lambda 闭包按 index 选，框架不自动收集。
- `TemplateSelector` 返回 `UITemplate` 对象，不返回字符串。
- 严格派：selector 设了即全权——每个 index 必须返回 UITemplate（null 抛 `UIContractException`）；与 `ItemTemplate` 同设 selector 赢（`ItemTemplate` 为默认蓝图）。selector 求值在投影层完成后按区间批量推送（core 零回调）；换 selector/`Notify*` 后受影响区间重推，模板变了的项 park 旧蓝图 slot、下帧以正确蓝图重新物化。
- ListView 按模板分别池化；spacer 估高按蓝图均值分化。
- 虚拟化、可见区、测量补偿、content size 和后端 reuse key 全部是内部实现。内部不变量：slot 永驻 list 不 detach（park = inline `display:none`；unpark = 清 override，让 cascade 回落作者真实 display）；`notify_*` 一律 park/shift 复用、不重建；`reuse_key` 出生即定永不旋转、场景级命名空间（多 ListView 同页不撞 key）。虚拟化判据（可测）：render node 数与 slot 数不随 `ItemCount` 增长。
- 滚动来源两模式：list 自身 `overflow:auto`（自滚，用自身 viewport）或祖先滚动容器（扣祖先偏移）。
- ul 为 flex-row+wrap 时自动按行虚拟化（行内全量、spacer 全宽独占行）。
- 已知限制：bind 滞后一帧——新进可见区的 item 第一帧显示模板原样/上一复用者内容，快速滚动会出现一帧旧内容（接受此代价）。

刷新 API：`RefreshItem(index)`、`RefreshItems()`、`NotifyInserted/Removed/Moved`。完整契约见 [public-api.md](public-api.md) §8。

---

## 9. 事件

### 9.1 控件语义事件

```csharp
button.Clicked += OnStart;
slider.ValueChanged += OnVolumeChanged;
```

### 9.2 类型化路由事件

所有节点同时提供类型化路由事件（捕获 → 目标 → 冒泡）：

```csharp
node.On<PointerDownEvent>(OnPointerDown);
```

- `Target` 与 `CurrentTarget` 都是公共 `Node`。
- 节点 `Dispose()` 自动清理其订阅。
- `RemoveFromParent()` 不清理订阅。
- 内部后端事件不得泄漏 NodeId 或 FFI 结构。

### 9.3 命中测试

命中按等效绘制顺序逆序（后画的先命中）。命中几何：**点**经 world matrix 逆变换投到节点本地轴对齐 box 判定（transform 生效，旋转节点命中精确，非宽松 AABB）。`pointer-events:none` 跳过自身但继续测子（CSS 语义，不屏蔽子树）；class 规则的 `pointer-events` 与 inline 同源进级联终值（rematch 把 `style.touchable` 回写 `interaction.touchable`）。**stage 文档根不可命中**（`create_root` 建的宿主容器恒 touchable=false）——根铺满画布且可命中时「点到空白处」命中根，多 Stage 输入路由据此饿死指针下全部底层 Stage；overlay 类 Stage 的页面根应声明 `pointer-events:none`（交互面板再 `auto`）收窄命中面。disabled 节点仍参与命中与 hover diff（`:disabled` 需要 hover 反馈），但 Down/Up/Click/drag/longpress 全抑制；命中落在 disabled 子节点时祖先链 active 同步截断。优先级序：scrollbar grip > open dropdown popup > 正常内容。

**软件指针形态（#93）**：鼠标主指槽每帧命中 → `Stage::cursor_intent`（箭头/手型/隐藏三态），宿主订阅 `CursorIntentChanged` 驱动 `Cursor.SetCursor`（去抖缓存，纹理消费侧注册）。判定沿命中节点祖先链叶→根单遍（rich 内联命中细化到 source 后上溯宿主控件——浏览器模型里「指针下的元素」是宿主而非文字节点）：先遇到的作者 `cursor` 声明（围栏定为不继承，最近者生效）或 pressable 控件定型；disabled/不可命中宿主给箭头并截断。HTML 布尔属性 `disabled`（button）经 pkg 烘入、instantiate 映射 `NodeFlags::DISABLED`——与运行时 disabled API 同一语义源。

### 9.4 拖拽与滚动仲裁

拖拽与滚动通过阈值赛跑仲裁，先达者赢（同指针位互斥清除另一方候选）。轴锁让出：单轴滚动容器遇主轴正交的更大手势时让出，并把候选提升到下一个可滚祖先。scroll 启动即取消待决 click/longpress。

### 9.5 引擎输入桥

核心定义 `InputProvider` trait（指针/键/触摸/IME character），后端实现并每帧注入。坐标核心左上原点；翻转在后端根一次性做。

### 9.6 UI 输入消费

```csharp
bool hit = ui.IsPointerOnUI;
```

极简：核心命中后存当前指针命中的节点，暴露事实查询。不做消费策略/consume 标志/每指针数组。

### 9.7 输入模型语义

- 多指：固定槽位模型——鼠标占主槽常驻，触摸按 fingerId 占槽、超槽丢弃；鼠标与触摸可同帧共存。
- hover 双语义：`RollOver/RollOut` 事件按单指针进出判定、不冒泡（按祖先链 diff 逐节点直派——从父进子父不 RollOut，对齐 CSS `:hover` 祖先语义）；`:hover` 伪类是全部活跃指针命中链的 union。多指下两者可能不一致。
- `PointerMove` 是 monitor-gated：仅指针被 capture（touch monitor）后才产生 Move 事件流。
- capture 阶段不检 `StopPropagation`（root→target 全程跑），bubble 前预检；target 节点在 capture 末尾与 bubble 开头各收一次。
- keydown 无焦点即丢弃（核心无全局键盘概念）。Tab/Shift+Tab 自动焦点链导航内置（tabindex 正整数升序先于 0 组、DOM 序、链尾 wrap、Tab 被导航消费不发 keydown）；方向键/手柄焦点导航不做——是逻辑层积木。
- 焦点语义：pointer-down 命中可聚焦节点（tabindex ≥ 0 且非 disabled）自动聚焦；点不可聚焦区域清焦点（对齐 DOM 点空白 blur）；编程 `Focus()` 是强制语义（不查 tabindex，仅 disabled 拒）；`FocusIn/FocusOut` 只发焦点节点本身、不沿祖先链。

---

## 10. 文本与 Inline Formatting

### 10.1 正常 HTML 子树

删除旧 `display:block` RichText desugar 暗号和特殊公共 `RichText` 类型。富文本就是正常 HTML 子树（`<div>` + `<span>` + `<img>` + 裸文本）：

```html
<div id="description">
    对敌人造成 <span id="damage">120</span> 点伤害
    <img src="fire.png" alt="火焰">
    <span id="details">详情</span>
</div>
```

### 10.2 公共对象树

公共树保留 `TextNode/TextElement/Image` 的 ID、样式和事件。事件归属：span 内文字命中 span（事件挂 span）；rich-block 直接文本命中 TextNode 自身；Image 命中自身。

### 10.3 内部文本布局

内部文本布局将最近 Inline Formatting Context 编译成 TextRun、ImageRun，用于统一换行、baseline、测量与几何构建。

- 裸文本形成叶子 `TextNode`。
- inline 元素（`span`）是语义容器。
- `div` 建立文本 block。
- `TextContent` 与 DOM 一样，用纯文本替换当前全部子内容。
- 修改 inline 子树只使最近文本上下文的测量失效（失效粒度 = 最近 Inline Formatting Context）。
- `rich_text_block` 是运行时单向可翻转 flag：打包期烙印；运行时显式 `display:flex`（inline override 或命中动态规则）把 rich 折叠翻回 flex 容器布局；flex→block 不回标。

公共语义树与内部布局/渲染树可以不同。

### 10.4 文本测量

taffy 对"尺寸取决于内容"的节点回调 `MeasureFunc(known_dimensions) -> measured_size`：给定约束宽返回 `(text_width, text_height)`。必须廉价、无副作用（auto-size/shrink 反复调用）。

自绘字体地基：ttf-parser 取 outline，光栅成**单通道 SDF** 存 etagere 图集——一份固定源尺寸的 SDF 供所有目标字号共享（字号不进字形缓存键），shader 用屏幕空间导数重建边缘；文字效果（描边/多层阴影/发光/模糊）在同一 fragment pass 按 SDF 参数块合成，无需逐效果重光栅。spread 必须覆盖最大效果宽度，之外距离饱和、效果被硬切。MSDF 不引入：CJK 圆滑曲线用不上锐角修复，且不为此引 C++ 依赖。字体缺字回退：主字体按 fallback 链逐个 probe、首个含该字者补上（行度量走主字体，字形度量走提供方字体）。

换行遵 CSS 换行控制属性（`white-space` 空白折叠 × 自动换行 × 源换行保留三轴 + `word-break` / `overflow-wrap` / `text-wrap`，值域真相源在 fence.md）：超长无空格串（URL/数字串）默认**不拆**——独占一行横向溢出（浏览器 `overflow-wrap: normal` 语义），显式 `overflow-wrap: break-word` 才逐字拆行。CJK 断行自动避头尾（行首不出句读/闭括号、行尾不出开括号——断点调整式禁则，无悬挂标点）。文本控件（TextField 系）的空白语义冻结为 pre 系（空格/换行原样保留）：空格折叠会破坏光标字节↔布局 1:1 映射；换行开关仍尊重声明。

---

## 11. 布局层

### 11.1 布局策略

`display:block/flex/none` 选择内部布局 Strategy：

- **Block**：标准块级布局（子元素垂直堆叠，margin collapse）。
- **Flex**：flexbox（标准 CSS flexbox 规范子集）。默认 `flex-direction:row`（标准 CSS）。
- **None**：`display:none`，不参与布局和渲染。

布局策略切换不改变节点类型。策略只持算法，不持节点状态。

### 11.2 taffy 集成

场景图 Container 树 ↔ taffy 节点树一一对应。增删 Container 同步增删 taffy 节点；改 style 同步改 taffy style 并标记子树 layout dirty。

taffy 树跨帧持久（`Scene.layout_cache`）：每帧 solve 做「期望态 diff」——style/measure context 值比较短路（稳态帧零脏标）、结构变更走 set_children/remove，靠 taffy 自带 dirty 上溯 + 布局缓存跳过干净子树（替代每帧全量重建）。「每帧一次 solve」不变量不变：solve 仍每帧调用，内部对无变更子树短路。正确性由差分守卫测试保障（随机操作序列下增量结果与全重建逐节点 rect 全等）。

taffy 0.12 同时支持 Flex 和 Block 布局算法。统一走 `compute_layout_with_measure`（内部按节点 `taffy_style.display = Display::Block/Flex` 分派；不再分别调 `compute_block_layout`/`compute_flexbox_layout`）。裸 block 默认标签（当前围栏里 `div`）和显式 `display:block` 都设 `Display::Block`；inline 标签和显式 `display:flex` 设 `Display::Flex`（inline 走 Flex Row）。

### 11.3 尺寸模型 → 映射

| CSS | 布局算法 |
|---|---|
| `width/height`(px/%) | `size` |
| `min/max` | `min_size`/`max_size` |
| `flex-basis` / `flex-grow/shrink` | 同名（flex 模式） |
| `flex-direction/wrap/gap` / `justify/align-*` | 同名（flex 模式） |
| `padding/border-width/margin` | `padding`/`border`/`margin` |
| `position:relative`+insets | `Relative`+`inset`（视觉偏移，不影响兄弟布局）。注：taffy `Style::DEFAULT.position` 已是 `Relative`，显式写 `position:relative` 为 no-op |
| `position:absolute` | taffy `Absolute` + inset（脱离流） |
| 内容自适应（文本/图片） | `MeasureFunc` 回调（§10.4） |

### 11.4 响应式与异形屏

- **resize**：屏幕尺寸变 → 根节点 size 变 → 整树 solve。
- **safe-area**：在后端根解决——`Screen.safeArea` 矩形作适配框（letterbox 在其内 contain 居中；Fit 模式画布从它起算，内容不进刘海），渲染与输入映射消费同一组适配变换（单源 = core 数学）；核心不感知 safe-area，无 CSS 级避让机制（不做 `env()`；变量注入通道是 #11 落地后的演进方向）。
- **动态内容/数据变化**：改文本/增删子节点 → 置 dirty → 下帧 solve。

### 11.5 分辨率适配（参考分辨率 / 长宽比 / safe-area）

设计稿 1080×1920 在 1440×2560 整体等比放大只是适配的平凡半边（均匀缩放）；**真问题是长宽比不匹配**。模型分三层：

- **配置正主 = workspace**：`yio.workspace.json` 的 `design {w,h}` + `match_mode` 由打包器透传进 `yio.runtime.json`，引擎集成层（C# Driver）读产物；Inspector 字段是 manifest 缺项时的 fallback。设计分辨率是设计师事实，活在 AI 可编辑的文本空间，不活在 Unity 场景手填。
- **策略数学 = core**（`yio_compute_adaptation` 纯函数，全引擎共享同一份）：三模式枚举——
  - `letterbox`（默认，contain）：root 锁设计分辨率，取较小缩放比，safe 区内居中留黑边。布局永远按设计稿排，最可预测。
  - `fit-width` / `fit-height`：拆黑边重排——锁一维锚（宽或高 = 设计稿），另一维 root 直接取屏幕换算值，`Stage.set_root_size` 喂核心下帧重排（flex/% / vw-vh 声明流动）。px 不变形（缩放仍均匀），无黑边无裁切。
- **重排语言 = 围栏视口单位 + env()**：`vw`/`vh`/`vmin`/`vmax`（分母 = root_size 画布，区别于 `%` 相对父容器）与 `env(safe-area-inset-*)` 同为**延迟长度**（`DeferredLength`），进 `ResolvedStyle.viewport` 平行字段（taffy CompactLength 装不下第四种 tag）。消费分两路：几何通道 solve 建树期按当帧 root/safe 换算覆写 taffy 副本；视觉通道（font-size/letter-spacing 是继承属性）在继承传播的 tree-order 走查里先解析成 px 再向下传（父的 resolved px 是子的继承源）。收哪些通道见 fence.md §5.2（#110 起全长度属性：尺寸/inset/margin/padding/gap 族 + flex-basis + font-size + letter-spacing + border-radius）。

safe-area 语义（#110 定案，web viewport-fit=cover 模型）：**fit 模式 root 贴物理全屏**（scale 按物理宽/高算，unsafe 带被 root 覆盖、背景满铺贴边），元素用 `env(safe-area-inset-*)` 自行避让；letterbox 仍以 safe 矩形为 contain 框——root 全在 safe 内，env() 恒 0（黑边已让位、不重复避让）。三模式同一条公式「env() = root 伸进 unsafe 屏区的深度（design px）」，由宿主按 adapt 结果 + 屏幕 safe 矩形经 `yio_stage_set_safe_area` 注入（core 单源换算，跨引擎不重写）。叠加顺序：适配算 scale/root → safe inset 注入 → 布局 → 渲染根变换 + 输入逆映射消费同一组 scale/offset（单源 = core 数学，集成层不自己重推）。

跨引擎契约：模式枚举是 FFI u32 ABI（只增不改）；未来 Godot 后端复用同一 `yio_compute_adaptation`，保三引擎适配行为逐像素一致。

### 11.6 滚动

任意 `Container` 通过标准 `overflow:auto/scroll` 获得滚动行为，对象类型保持不变（§3 设计哲学：CSS 赋予能力，不改变类型）。

```css
#inventory { overflow-y: auto; }
```

内部 Overflow Strategy 可以在 Visible、Clip、AutoScroll 和 Scroll 间切换；`ScrollState` 独立保存。非滚动态调用滚动 API 遵循 DOM，位置被钳制或不产生视觉滚动。

**惯性回弹物理**：ScrollPane 自维护可变 target 的 tween，content size 变化时按状态补偿 start、不突变。不走 GTween（content 异步变化时 GTween 的固定 end 会跳变）。tick 分两段：`advance_all`（惯性/回弹物理推进）在 solve 前消费指针输入并推进滚动位置；`refresh_content_sizes`（内容尺寸刷新）在 solve 后、compute_world_transforms 前。

能力：滚动类型、惯性+回弹、滚动条、鼠标滚轮。分页/吸附/下拉刷新后期。嵌套滚轮透传（内层滚到边界自动交外层）未做。合成滚动条 thumb 的 design rect 含祖先滚动补偿（`ancestor_scroll_offset`——后代节点的 world_matrix 注入 T(-祖先.scroll_pos)，thumb 是根级追加行不享此通道，须手工补；嵌套滚动首个用例 #52 shape-mask 页）。

anchoring 豁免：虚拟列表内容回填引起的几何变化走 clamp 但不清滚动 tween（回填 ≠ 内容突变，惯性应继续）。

---

## 12. 渲染层（自绘，渲染树契约）

> **核心原则**：渲染树契约描述**渲染意图**（画什么/遮罩意图/绘制顺序），**不规定**引擎实现机制。后端各自选择。

### 12.1 坐标系

核心唯一真相源：左上原点，y 向下。后端根 Stage 做翻转（如 Unity flips y）。

### 12.2 几何生成

非文本几何（图片 quad/形状/九宫格/填充）在 Rust 核心生成（确定性、跨引擎一致）。文本 mesh 同样在核心生成（核心自绘字体，v1.6+）。九宫格不变量：四角不缩放、四边单轴拉伸、中心双轴；容器边长小于两侧 slice 之和时按比例收缩防角重叠；slice 与圆角共存时四角为圆角扇形。

### 12.3 DrawState

核心不算材质对象，只算 draw 所需状态：
- `DrawFlags`(u32)：`Clipped|Grayed` 等。
- `BlendMode`：Normal 等基础。
- `ProgramId` 全集（真相源 = render 代码注释）：0 纯色/图直通；1 文本（SDF）；2 bg-color×图合成（BG_COMPOSITE——bg-color 在图之下，shader 按 source-over 合成，单 quad、无合成 RenderNode）；3 filter；4 filter+bg 合成；5 box-shadow SDF；6/7 渐变 per-fragment（7 = 渐变×filter）。
- 后端按 `(program+flags+blend+texture+mask_context)` 维护 DrawState 缓存。

### 12.4 批合（FairyBatching）

两元素能并入同批 ⟺ DrawState 相同（AABB 不相交则可重排聚拢；同 DrawState 相交仍可合）。DFS 遇裁剪器（overflow 裁剪或 clip-path 声明，§12.5）的 Container 强制其为 BatchingRoot；批合收集不下钻进 root 子树。core 显式合并 mesh → 真 N→1 draw call。mesh 合并锚：合并产物的 `node_id` 取 batch 内最小原始 id（后端 GO 复用防抖动）。排除项：控件节点（后端要建交互实体——控件排除点是「加新控件类型」的 dispatch 位置之一）、文本节点、非纯平移 transform、box-shadow 合成层，均不参与重排合并。

合并批的增量语义（#109 起）：**批不跨渲染显隐、不跨 world-space 挂载**（合批键含这两维——merged 行的 GO 归属/显隐是整批属性，混批会互相绑架）。合并批**携带 anchor 的世界平移矩阵**且定级按**整批合并 payload 的哈希**：纯平移（滚动/Transform 位移）在 payload 顶点里不可见（位置编码在矩阵，顶点是局部系），批必须靠矩阵轴捕捉——同质批纯平移 → Header 级（只挪 GO 不重传 mesh），成员几何/批成员集变化 → Full 重传；稳态帧仍全批 Skip。

### 12.5 裁剪/遮罩

**多 entry clip 栈**（#52，web clip 栈模型）：核心产 clip 表——每个 `mask_context` 对应**整条**祖先裁剪链（每条 entry = 一个裁剪器的测试），后代的有效裁剪 = 链上全部 entry 逐条应用取交集，**不坍缩**。`mask_context` 仍是批合边界（裁剪器开新 context）。链深上限 4（后端 clip uniform 槽定长）：authored 超深 fence 打包期拒（`FenceClipChainTooDeep`），运行时 CSS 越界 core warn-once 丢最内层（少裁不黑屏）。核心给「意图」（box-local 几何 + 裁剪器世界矩阵逆）；后端自选实现（Unity 走片元 discard：design 坐标映回裁剪器局部系后按 entry kind 测 rect/圆角 SDF/circle SDF/polygon crossing）。

**entry 几何 = clipper box-local + 局部系逆矩阵**：几何存裁剪器 border box 局部坐标（(0,0) = 左上），`inv_frame` = 裁剪器世界（design 系）矩阵逆。消费端把点映回裁剪器局部系再测形状——共享祖先的 transform/滚动在映射中自动消解；**裁剪器自身 transform 旋转时裁剪形随之旋转**（web 语义：clip 定义在裁剪器局部系——预览一致性的硬要求）。子代的 transform 不动祖先裁剪形（各 entry 独立测试，天然正确）。

**裁剪器判定与测试项**：`overflow` 非 Visible（rect 测试 + 自身 `border-radius` 圆角）**或** `clip-path` 声明（shape 测试，声明即 clipper——裁自身绘制 + 子树，web 原义）。同元素 `overflow:hidden` + `clip-path` = 单 entry 双测试并存（交集）。**祖先链圆角随 entry 传播**（祖先 rounded rect entry 作用于后代角——web 行为）。命中测试同语义（`clip_gate_passed` 逐裁剪器判定，与渲染共用几何函数——画出来裁的点即命不中的点；border-radius 感知命中 = 浏览器行为）。`overflow:scroll/auto` + `clip-path` fence 硬拒（shape 裁滚动视口无语义）。soft clip（羽化逐像素 alpha）未做（#113 deferred）。

**浮层机制**（通用模式，open dropdown 的 `role=listbox` 子树与 scrollbar thumb 共用）：跳出正常 DFS，render 末尾追加独立 DFS、sort_key 续 post-merge 最大值、mask 重赋脱离祖先裁剪链；命中层对应前置。画序单源 `scene::stacking::paint_order`（stacking context 全局分层，CSS Appendix E 语义：static 子树里的 opacity<1/transform/filter/定位+声明 z 后代会上提到所属 SC 的对应层，#96/#100）——主 DFS、浮层追加、hit 逆序三消费点共用同一份序，绘制与命中一致性由构造保证，无「多处手抄同步」面。

**world-space 挂载与裁剪互斥**（#109 C8，v1）：挂载子树的渲染行在挂载根处脱离祖先 clip 链（mask 清 0）——clip 平面定义在屏幕系，行顶点已 re-base 到挂载根局部系（随业务 3D 容器变换），屏幕系裁剪框对它无意义。挂载子树内声明 overflow 裁剪在挂载登记时被拒（防「声明的裁剪静默失效」）；完整形态（clip 挪 design 系使挂载内可裁剪）见 deferred 票。

### 12.6 RenderNode 契约

```rust
struct RenderNode {
    node_id: u64,                     // NodeId 位型（idx:32+gen:24+tag:8），build 直填 n.id.0
    parent_id: Option<u64>,
    mount_root_id: u32,               // world-space 挂载槽位（#109 C8）：0 = 屏幕空间；非 0 = 行顶点已 re-base 到挂载根局部系，后端按槽位路由 SetParent 到业务 3D 容器
    visible: bool,                    // 渲染显隐：运行时 render_hidden 的累积值（CSS visibility:hidden 继承语义——隐藏祖先 = 整子树行 visible=0；与 display:none 正交，不剪子树不动布局）
    alpha: f32,
    // grayed: bool — deferred（灰化禁用节点渲染，待视觉束落地）
    color_tint: [f32; 4],
    world_matrix: Affine2,            // 已累计的 world-space 仿射矩阵（过渡；终态用 NodeTransform）
    blend: BlendMode,
    mask_context: MaskContext,
    sort_key: u32,
    change_level: ChangeLevel,        // Skip=0 / Header=1 / Full=2
    reuse_key: u32,                   // MirrorPool GO 复用键
    effect: EffectBlock,              // 文字效果参数（定长块，v15 起按需进 fat arena）
    shadow_params: [f32; 6],         // box-shadow SDF 参数（半宽/半高/圆角/σ/inset 标志/填充；偏移烘进几何；v15 起按需进 fat arena）
    gradient: GradientParams,         // 渐变参数（program 6/7 消费；v15 起按需进 fat arena）
    payload: NodePayload,
}

enum NodePayload {
    Mesh { verts, uvs, colors, indices, image_path, program, color_matrix },
    // Mask / PaintTarget / NativeHost — 见 roadmap
}
```

> 注：`grayed` 灰化渲染待 visual beam 落地；`world_matrix: Affine2` 为 v1 过渡形态，终态替换为 `NodeTransform`（分解 Position/Scale/Rotation，对齐 public-api.md 三分模型）。

`ChangeLevel::Skip/Header/Full` 表达本帧变化程度。

**增量渲染构建**（#109 A2）：每节点维护**输入指纹**（style 实际改写版本 / layout 宽高量化 / 世界矩阵与累积 alpha 全量 / 文本布局代数 / anim 烘 mesh 通道 / 渲染显隐 / 挂载归属+槽位+量化原点 / 资源代数 / rich 折叠位 / 图 src），命中则**整段复用上帧产物**（含合成层与配对追踪）；在场节点集签名变化（增删/换父/显隐翻转/浮层开合）→ 缓存整表清空兜底；控件壳永不缓存。**纯平移不进 payload 顶点**（位置编码在矩阵、顶点是局部系）——凡按 payload 哈希判变更的路径必须单独覆盖平移轴（合并批的矩阵轴见 §12.4）。高频运行时 setter 须同值幂等（同值写不触指纹失效——否则逐帧 Set 的调用模式会打爆全缓存）。

合成 `node_id` 命名空间：`NodeId` u64 位型 = index(32) + generation(24) + tag 字节(8)。真实节点 tag 恒 0；文本跨页子页、scrollbar thumb、TextField 合成层、box-shadow inset/outer 层各占 tag 字节区段——合成 id 与真 id 靠 tag 位型天然区分，无碰撞可能（区段分配真相源 = render 代码注释）。这是跨层 ABI 契约（MirrorPool 按 id 建 GO、命中按 tag 解码）。渲染 blob 不是节点状态查询通道：批合会让空/透明节点从 blob 消失，按 id 查状态（world matrix/sort_key/visible）必须走独立 FFI getter。

---

## 13. 动画（单时钟）

动画系统分两层：**功能层**（M2 已交付，功能完整）+ **引擎终态**（M2.5，触发判据明确，见 roadmap milestones）。本节描述 M2 实现现状；带「M2.5」标记的条目是 deferred 的引擎终态。

### 13.1 单一时钟

整个核心只有一个动画时钟。每帧 tick 前段（§16 step b/c）并列推进两个写入者（`render` 读 `NodeAnim` 时 `anim.unwrap_or(style)` 天然优先于 base style）：

```
b. TweenManager.update(dt)        ← transition 的 opacity/bg_color/text_color 先写
c. KeyframePlayer.update(dt)      ← animation 的 transform/opacity/bg_color/text_color 后写（同通道覆盖）
```

写入顺序即优先级——天然实现 CSS **animation 优先于 transition**，无需占用标记。ScrollPane 物理是唯一例外（自维护 tween）。

### 13.2 KeyframePlayer（路线甲：独立 player，不翻译成 Tween 序列）

`@keyframes` 表（组件级打包、Scene 级查找）+ `AnimationSpec`（fence `animation` 简写解析）驱动独立的 `KeyframePlayer`，slotmap 稳定句柄（`PlayerKey: u64`）。player 推进时间轴（delay/iteration/direction/fill/ease + per-segment timing-function + TRS lerp），写 `NodeAnim` 的 transform/opacity/bg_color/text_color 四通道，不翻译成 Tween 序列。player 本体是纯时间轴状态机：不持场景引用、不发事件，推进只返回纯数据帧，写 `NodeAnim` 的副作用由 tick 集成层执行；elapsed 是唯一时间源头（iteration/progress 均从它推导）。

**tick step m `sync_animation_players`**（rematch 后、solve 前）检测 computed `animation` 声明变化启停 class 触发的 player：新增 name → 启 player；消失 → 停；参数变 → 重启。`node.Play("name")`（程序化）走 FFI 立即建 player，不等 rematch。

**`Container.RestartAnimations()`（声明式动画运行时重启）**：清子树内全部 class 触发 player（按通道所有权回收，幸存 player 的通道不误清；programmatic player 不动），下一帧 step m 依 `base_style.animation` 声明原样重建（delay 重计、backwards/both 立即写首帧）。与销毁重实例化的差别：节点身份/滚动位置/控件值/事件订阅全保留。

**写入通道**：transform 走 TRS 分解的单复合通道（transition 与 player 都可写，中途接管以当前值为 start）；`opacity/bg_color/text_color` player 与 tween 都可能写，player 后写覆盖。

**fill-mode 完成态**：`forwards`/`both` → Completed 态不回收，每帧持续写末值（直到声明消失/Stop）；`none`/`backwards`（默认）→ 回收 player，通道回 None，下帧 tween/base 接管。**backwards/both 首帧 backwards fill**：启动时立即算一次首帧值写 NodeAnim，不等下帧 update，避免 delay 期间闪 base。

**多 animation 并存**：`animation: fadeIn .4s, spin 2s` → 一节点 N 个 player；同通道冲突时后声明的赢（CSS 标准，列表后者优先），player 按 `animation: Vec` 顺序写。

**ITERATION CSS 语义**：最后一次 iteration 结束的完成帧**只发 END，不发 ITERATION**（对齐浏览器 `animationiteration` 不因最后一次迭代触发）；非完成的 iteration 边界跨越才发 ITERATION。

**Animation 句柄 L3 全套**（见 [public-api.md](public-api.md) §9）：`Node.Play(name)` 返回 `Animation` 句柄，事件 `AnimationStart`/`End`/`Iteration`/`Key`/`Hook` + `TransitionEnd` 经 `borrow_events` 双路由（全局 `On<T>` + 句柄 `player_key` 私有回调）。

**引擎终态（#9 已交付）**：池化 Tween（`TweenManager { active, pool }`，稳定序回收）+ 缓动全集（CSS 标准 keyword 精确 bezier + `cubic-bezier()` + yio 超集 back/elastic/bounce + steps；per-stop `animation-timing-function`）+ 链式 builder（Rust `Stage::tween_builder` / FFI `YioTweenSpec` 单入口 / C# `Node.Tween` fluent，`OnComplete` 走 TweenComplete 事件 tag 路由）+ 插值原语统一（共享 `TweenValue` 定长缓冲 + `lerp_n`）。tween 支持 `repeat`+`yoyo` 多轮（alternate 语义）；keyframes transform 的 translate 分量收百分比形（`LenPct` px+pct 混合描述符，写入期按节点布局尺寸解析——player 保持纯时间轴）。**缺省 timing 全端 = 精确 CSS `ease` bezier(0.25,0.1,0.25,1)**（幂函数近似已废）。Ease/TweenProp 判别值末尾追加纪律（pkg bincode variant index + FFI kind 契约）。

### 13.3 Transition

纯数据 `items: Vec<TransitionSpec>`。class/typed style 变化在下一帧 tick step **k** rematch 生效后，step **l** transition drain 比较 computed style 变化（基线 = 上帧 computed，不含 NodeAnim），把每个 item 翻译成 Tweener 提交 TweenManager；**提交即以 n=0 预写起始值进 NodeAnim**（与 player 的 backwards 首帧立即写同纪律：本帧 solve 读 override 而非级联终点，否则首帧闪现端点值一帧；delay 期间预写兜底 = CSS「延迟期持有旧值」）。与控件状态（Toggle 切换、TabList 切换）正交，由状态变化触发。transition 与 animation 检测独立——animation 播放期间 computed style 不变，transition 不误触发。

### 13.4 opacity 父级累积传播

渲染 DFS 累积：`node_alpha = parent_alpha × own_opacity`（`render/mod.rs::accumulate_alpha`），进子树时把 node_alpha 当作子的 parent_alpha。parent 半透明会按比例衰减子树——近似浏览器 opacity（浏览器走隔离合成层，交叠的半透明子元素两种模型呈现略有差异）。

### 13.5 Timers

独立通用周期/延时回调（unscaled_dt），与动画解耦。`CallLater`（下一帧）、`CallNextFrame`、`OnUpdate`（每帧 recurring，返 IDisposable）。OnUpdate 是逻辑驱动每帧钩子，非动画系统。

### 13.6 Layout 动画与 box-shadow 通道（#10，M3）

player/tween 可动**布局属性**：width / height / flex-grow（端点同域显式值 px↔px / %↔% / vw↔vw；auto 与异域混合是围栏硬拒项，运行时 add_class 组合漏网端 snap + `EVT_TRANSITION_SNAP` 警告事件兜底）+ box-shadow 列表（css-backgrounds-3 语义：短列表补透明零长空阴影逐对插值，配对 inset 不匹配整体离散）。

**不需要 solve 重入**（v0.0.13 前设计稿设想的「prop_type 分层 + layout_dirty + solve 重入」前提已过时）：tween 推进在 tick 序 ①、solve 在 ⑦——layout 属性的动画 override 经 `NodeAnim.width/height/flex_grow` 写入，由 solve 的 taffy 树 sync 消费，覆写链末位（base → viewport 换算 → anim），「每帧一次 solve」不变量保持。vw/vh 域 override 在 solve sync 期按当帧 root_size 换算——动画中途 resize 自动重解析（继续走完、比例跟画布）。`set_style` 值比较短路保证稳态帧零成本。

三通道共享同一插值底层：CSS `transition`（rematch 检测端点变化）、CSS `@keyframes`（AnimatableProps 扩展，pkg v44）、C# `TweenBuilder`（`.FromPx/.ToPx` 值+域码载荷；box-shadow 走 `FromShadow/ToShadow` 列表载荷，FFI 专用入口）。

---

## 14. 资源 / 包系统

### 14.1 双格式

- **编辑期/源**：HTML（结构）+ CSS（样式）+ 资源清单。
- **发布产物**：编译成**单一二进制 blob**（`.pkg.bin`）+ 图集（`atlas/<name>.png` + `atlas/<name>.atlas.json`，多页 `<name>.<n>.png`）。
- 运行时**只认二进制产物目录**；HTML 解析只在打包器。产物目录是运行期后端与设计期工具的唯一契约面——后端永远不认识工作区/GUI/CLI。

### 14.2 图集（Rust 自绘，打包器产出）

打包器自绘图集（`atlas/<name>.png` + `atlas/<name>.atlas.json`）。`sprite_key` = 图相对工作区根的路径（正斜杠）——用全路径消除裸文件名跨目录撞车；HTML 里 `src` 相对所在 html 文件解析（浏览器语义），打包期归一。核心只持 sprite_key + 图片原始像素尺寸；后端 `SpriteResolver` 据 atlas.json 的 UV 字典取子区 UV。

- **归属校验**（打包期）：HTML 引用的每张图须恰好归属一个 atlas——缺失报错、跨 atlas 重叠报错；atlas 内未被引用的图合法（运行时 `set_src` 动态图标）。
- **图尺寸唯一真相源 = atlas.json**：运行时动态图标不写 HTML，pkg manifest 永远缺它们的尺寸——故 manifest 不存尺寸，启动时把全部 atlas.json 尺寸批量灌入核心。交叉验证刻意单向（不反向要求 atlas 图都被 HTML 引用，否则动态图标全判无用）。
- `border-image-slice` 是 per-element 属性烤进 base_style（同一图不同元素可不同 slice）——图集管 per-image 数据，pkg 管 per-element 数据。
- `<img>` 内在尺寸三档：CSS 声明 > 图集真实像素 > 默认兜底（无尺寸 img 的布局行为）。

### 14.3 运行时 Bootstrap

驱动启动时读 `yio.runtime.json`（声明包/图集/字体列表）→ 加载各 `.pkg.bin` + 图集 → 解析 atlas.json 中每张图的 `orig` 尺寸推入核心 → 初始化 SpriteResolver。

### 14.4 包格式

- Header（20B）：magic（`0x474B504C`，"LPKG" LE）+ formatVersion（u32）+ flags（u32，预留）+ component_count（u32）+ string_count（u32）。
- 组件描述分块，运行时只读需要的块。
- 全局 stringTable 去重。
- 跨资源引用存 id 不存内容。
- 版本协商：Header `formatVersion` + runtime 声明 `min/max_supported_version`。

### 14.5 纹理

核心只持 `TexId`（整数）。图集：一张大纹理 + N 个轻量 TextureView（只存 UV）。子 view 首引用连带 root；归零通知后端可卸载。GPU 生命周期全在后端。

「按包释放纹理」不是本架构的概念：`yio.runtime.json` 的 packages 与 atlases 是 workspace 级平行列表，SpriteResolver 按 (atlasIdx, page) 全局懒缓存、与包注册表解耦（重载同名包不重载纹理），字体是 driver 级注册——`UnloadPackage` 只动模板注册表（上面的归零卸载模型属于核心 TexId 通用层，本架构的 Unity 侧不使用）。

### 14.6 资源宿主与 Stage 实例分离（#109 A3）

字体表/字形图集/包注册表/图尺寸表从 Stage 实例剥离进 **ResourceHost**（Stage 持共享句柄）：单 Stage 行为不变；多 Stage 共存时字体只注册一次（无 font id churn / 图集重光栅），字形图集脏页单点拉取（多后端各拉各清会饿死后拉者）。宿主侧写操作（注册字体/回落族/图尺寸/装包）不经场景 mutation——宿主持单调**代数（generation）**，Stage 每帧 tick 前对账：代数变了才清文本两缓存 + 标脏文本叶子（下帧 solve 重测）。字体字节的所有权在宿主（Stage 换代重注册不留泄漏）。

---

## 15. FFI 与引擎后端

### 15.1 csbindgen

- Rust 端 `#[no_mangle] extern "C"` + `csbindgen` 生成 C# `[DllImport]`。
- `csharp_use_function_pointer(false)` 切 Mono 模式（IL2CPP 友好）。
- `[GroupedNativeMethods]` context 指针模式。

### 15.2 IL2CPP 注意

- 回调必须 `static` + `[MonoPInvokeCallback]`。
- string 永远走 UTF-8 `byte*`。
- 内存所有权严格隔离：跨边界传 POD/指针/扁平 buffer。
- 高频调用用扁平数组（pin 或拷贝）。

### 15.3 跨边界数据（SOA + per-frame arena + 段末增量）

每帧 FFI 传：
1. RenderNode 公共头 SOA **lean 列**（定长字段并行存储；#109 v15 起胖参数块——color_matrix/effect/shadow/gradient——移出列进**按需 fat arena**：字节全零的块不写，行持 1-based 引用 + 子掩码）。
2. 按类型分区的 per-frame arena（mesh 顶点/UV/颜色/索引、path 表、fat arena 等；#52 起 clip 表亦在此族——多 entry 布局：92B 定长 entry（ctx/flags/inv_frame/rect/radii/circle/poly 头）+ 段尾 poly arena 按需存 polygon 点，布局详见 §12.5）。
3. **段末 skip 段**（#109 v15）：定级 Skip 的行不再占 SOA 列，进段末极简条目（node_id + reuse_key + flags；parked keepalive 走此段，flags bit1）——全 Skip 帧的带宽从「每行全列」降到「每行极简条目」。skip 条目的 flags 与 lean 行 visible 列**不同义**（skip = 沿用上帧态，不传递显隐）。
4. ChangeLevel（Skip/Header/Full）：Skip 进段末条目，Header/Full 在 lean 列；Skip/Header 不写 mesh arena。

C# tick 内一次拷完。后端维护双 dict（`_poolByNodeId` + `_poolByReuse`）做 stale-mark-sweep 镜像同步，O(n) 每帧（v15 起为双段遍历：skip 段先走保活，lean 段只余 Header/Full）。

数据形态分工规则：定长 per-node 结构走 SOA 列，变长几何走 arena——arena 索引不反映内容变化、哈希跨 arena 会破坏自包含性；例外是「多数行恒零」的胖参数块（fat arena 按需化省的是带宽不是哈希语义，定级仍由 §12.6 指纹管）。控件 FFI 导出全值操作（clone ControlState → 改字段 → 回写）而非结构视图，控件内部结构演进不动 FFI 面。

### 15.4 渲染对象镜像生命周期

- Rust 核心拥有场景图 + 渲染状态（真相源）；后端拥有渲染对象镜像（派生缓存）。
- 每帧脏增量同步（四态）：全标 stale → skip 段先走（**parked** keepalive——虚拟列表离场 slot，flags bit1——清 stale 并保持隐藏，留 GO 不渲染；持久池，fgui dormant 模式）→ lean 段遍历（**active** 命中清 stale 并按 change_level 更新；**hidden** = 行 visible 0，运行时渲染显隐——清 stale 但**保留 GO 仅 SetActive(false)**，与世界锚点出屏自动隐藏配套）→ 仍 stale 的（**gone**）销毁。parked keepalive 条目由 build_blob 追加（render 管线不动）。
- world-space 挂载行（mount 槽位非 0）镜像 GO 挂到业务 3D 容器（内层 y-flip，层随容器 → 场景相机渲染 + 深度测试吃 3D 遮挡）；解除挂载时镜像 GO 先行挂回屏幕根再销毁容器。
- 无 double-free/use-after-free：Rust 只持整数 id。

### 15.5 原生库分发

编译产出多平台原生库（`.dll`/`.so`/`.dylib`/iOS `.a`/Android `.so`）。csbindgen 生成 C# 绑定源码。Unity Domain Reload 保护。

---

## 16. 更新循环（每帧管线）

```text
引擎 update（C# 投影层 + Rust 核心，见 projection-layer.md）:
  1. set_input()                       ← 后端采集指针/键/触摸/IME
  2. flush 脏属性回写                   ← C# 投影层：攒批的 Style(css 串)/Transform(数值) 推 Rust（tick 前）
  3. context.tick(dt) — 显式依赖拓扑（真相源 = stage.rs `tick_and_render`；
     CI 门 core/tests/tick_order_gate.rs 锁步骤清单与代码一致，改管线先过门）：
     a.  drain pending_events           ← FFI setter（tick 外调用）产的事件先进本帧
     b.  TweenManager.update            ← 唯一动画时钟（transition 先写）
     c.  KeyframePlayer update_all      ← animation 后写同通道覆盖（写入顺序 = 优先级）
     d.  advance_cursor_blink           ← 光标闪烁 timer
     e.  消费 pending_focus_request      ← 编程聚焦/清焦点
     f.  process 指针输入                ← 多槽命中测试（用上帧 world）+ 拖拽/滚动/点击仲裁
     g.  apply_wheel + scroll advance   ← 惯性/回弹物理
     h.  process_keys                   ← keydown/up（Tab/Shift+Tab 自动焦点链导航内置；方向键/手柄导航不做——逻辑层积木）
     i.  process_text_input             ← UTF-32 字符通道（IME/可打印字符）
     j.  list plan/execute_visible      ← ListView 虚拟化 slot 换绑（solve 前：新 slot 本帧布局）
     k.  rematch                        ← 伪类 :hover/:active/:focus/:disabled/:checked 重 cascade（class/style 变更下帧生效）
     l.  transition drain               ← kill 旧 (node,prop) tween + 提交新 tween + 预写起始值进 NodeAnim（基线 = 上帧 computed）
     m.  sync_animation_players         ← computed animation 声明变化启停 player（rematch 后、solve 前）
     n.  sync_control_visuals           ← 控件态→子 inline_override（solve 前：inline 影响布局）
     o.  solve                          ← Block/Flex 各自算法（每帧一次，帧末一致）
     p.  measure_text_controls          ← 文本控件显示文本测量（需 solve 产出的 layout_rect.w）
     q.  list collect_heights           ← ListView 高度回填（refresh_content_sizes 前）
     r.  refresh_content_sizes          ← scroll content_size 刷新
     s.  compute_world_transforms       ← DFS 累计 world matrix（含 Transform 渲染偏移，不触发 solve）
     t.  build_render_nodes             ← 剪 display:none + 输入指纹增量复用（命中整段复用上帧产物，见 §12.6）+ 批合 + sort_key + 浮层末尾追加 + 挂载行 re-base
     u.  输出 Vec<RenderNode>（SOA lean 列 + 段末 skip 段 + fat arena blob，见 §15.3）
  4. 后端 borrow_frame → MirrorPool 同步镜像；borrow_events → 事件路由 → 业务回调
```

> 本清单有 CI 门（`core/tests/tick_order_gate.rs`）锁定：stage.rs 实际调用序列与登记清单不一致（换序/插入/删除）即红。有意改管线 = 同步更新登记清单，本节描述保持与之一致。

关键：
- **flush 在 tick 前**：C# 投影层攒批的属性写（Style/Transform）在 tick 之前一次性推 Rust，与 set_input 合并过桥。见 [projection-layer.md](projection-layer.md) §2.1。
- **rematch 在 solve 和 compute 之前**——伪类/class/style 变更当帧全部生效。class 切换驱动动画的下帧 rematch + 上帧 computed 做 transition 基线见 [public-api.md](public-api.md) §9.1。
- **hit_test 用上帧 world_transforms**（1 帧延迟）；scroll_pos 同帧进 world。
- **事件回调里改的布局属性延迟到下帧 solve**（避免反馈环）；Geometry 读的是最近完成的 solve（滞后一帧，同 web reflow）。
- **单一动画时钟**：TweenManager.update(dt) + KeyframePlayer.update(dt) 同在 step a 并列推进（transition 先写、animation 后写覆盖，天然 animation 优先于 transition）；OnUpdate 是逻辑驱动每帧钩子（非动画系统）。动画全貌见 §13。
- transform 动画不改布局，不触发 solve（layout 动画 deferred 到 M2.5，见 §13.6）。

---

## 17. 跨引擎扩展

引擎集成层分两层（Spec-4b 落地，commit `8e2df1c..d4c0f28`，branch `spec4b`）：

```
[引擎无关 · C# 共享 · Unity+Godot-C# 复用]
  Public/         UIContext/Node/Button/Style（业务 API，4a 已有）
  Projection/     NodeRegistry/EventDemuxer/EventBus（4a 已有）
  Host/           YioHost        ← stage 宿主 + 每帧驱动序（零 UnityEngine）
                  YioBackend      ← 抽象契约（本 § 三件事）

[Unity 特定 · 各引擎各写]
  UnityYioBackend : YioBackend   ← 持 MirrorPool/MaterialManager/NativeHostManager/SpriteResolver/InputCollector
  YioStageDriver (MonoBehaviour)  ← 瘦宿主：Unity 生命周期 + 资源 IO + 创建 Host/Backend
```

- **YioHost(引擎无关,`Runtime/Host/`)**:持 stage handle (IntPtr) + UIContext + YioBackend。零 `using UnityEngine`。每帧驱动 `Step(dt)` 严格按 §16 五步序:(1) `backend.CollectInput(stage)` → set_input;(2) `UIContext.FlushPendingWrites()` 攒批过桥脏属性（StyleMirror + NodeTransform，标脏不即时）；(2.5) `ctx.DrainPendingBinds()` ListView bind 排空；(3) `yio_stage_tick` FFI;(4) `borrow_frame` FFI → `backend.SyncFrame(stage, framePtr, frameLen)`;(5) `borrow_events` FFI → EventDemuxer → EventBus typed `On<T>` 路由。资源 FFI 引擎中立(RegisterFont / SetImageSizes / SetFallbackFamilies)放此层。`borrow_frame` 的 FFI 调用归 YioHost(产生引擎特定镜像对象的 FFI 仍归引擎无关驱动核心),backend 只消费 blob 做镜像。
- **YioBackend（引擎无关抽象契约，`Runtime/Host/`）**：契约 = 2 个 abstract 方法——`CollectInput(stage)` / `SyncFrame(stage, framePtr, frameLen)`。`set_input` FFI 在 backend（采集引擎特定但 FFI 引擎中立，省一次交互）。资源对象上传（如 Texture2D 上传 atlas 页）是引擎特定实现细节，不进入抽象契约（由 `UnityYioBackend` 内部方法如 `InitSprites`/`SyncFontAtlas` 承担）。
- **UnityYioBackend : YioBackend**：持 MirrorPool + MaterialManager + NativeHostManager + SpriteResolver + InputCollector（零改复用，从退役的 YioStage 搬过来）。NativeHost（GameObject 绑定 3D 模型）作为 UnityYioBackend 额外方法，不进通用契约（Unity 专属概念）。
- **YioStageDriver（Unity MonoBehaviour，瘦宿主）**：Awake 创建 UnityYioBackend（注入 Unity 组件）→ `new YioHost(designSize, backend)` → 读 .ttf/atlas 喂 `host.RegisterFont`/资源 → `ctx.LoadPackage`。Update 调 `host.Step(Time.unscaledDeltaTime)`。保留 Unity 特定（相机 / safeArea / 输入钩子 / 设计分辨率 / NativeHost 根 transform）。

> **YioStage 退役**：v1 的 `YioStage`（业务 API 透传层）在 Spec-4b clean break 整层删，无双壳——业务 API 透传已被 4a UIContext 取代，driver 的 ~10 个生命周期/后端编排调用按上述分层迁移。终态契约里只有 YioHost/YioBackend/UnityYioBackend，无 YioStage。

- **Godot 后端**：镜像成 Node2D + RenderingServer canvas_item 自绘。否决 Control 路线（与核心布局双系统冲突）。遮罩用 canvas_group/clip。复用 YioHost + 整个 Projection + Public，只写 `GodotYioBackend : YioBackend`。
- **SRP 混合渲染**（Unity 增强）：自绘节点用自定义 SRP RendererFeature 批合绘制。
- 新后端只需实现：消费 `Vec<RenderNode>` + 输入注入 + 资源加载。契约引擎中立。
