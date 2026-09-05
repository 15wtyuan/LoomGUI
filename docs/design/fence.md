# Yio 围栏（Fence）权威清单

> **单一真相源**：`crates/fence/src/schema/` 下的 Rust const 注册表（machine-readable）。本文档为人类可读副本，以代码为准。
>
> **防漂移门**：`cargo test -p yio_fence`——改围栏后必跑。

---

## 1. 设计哲学

### 1.1 标准 HTML 语义 + AI 强先验

围栏是一个面向游戏 UI、能够完整兑现语义的标准 HTML/CSS 子集。所有围栏决策的第一判据：AI 读 HTML 能否正确预测渲染结果。

AI 对标准 HTML/CSS 有海量训练数据先验。因此围栏只用标准 HTML 元素和 CSS 属性，不自创框架标签（如 `<scroll-view>`）——已有的标准能力（如 `overflow`）不自定义标签重复发明。

### 1.2 标签决定类型，CSS 赋予行为

这是围栏的核心原则：

- **标签 + 不可变结构属性决定稳定对象类型**。`<button>` 永远是 Button，`<div role="slider">` 永远是 Slider。
- **CSS（class、伪类、computed style）永远不改变对象类型**。`display:flex` 选择内部布局 Strategy，`overflow:auto` 选择滚动 Strategy。策略切换不重建节点、不丢状态。
- 这条原则让围栏设计在许多设计模式（Strategy、State）上自然落地，后续工作变得顺畅。

### 1.3 失败策略：明确报错，不静默降级

围栏外输入在打包期明确报错。围栏外 CSS 不静默忽略——写什么得到什么，围栏外即失败。

流水线一次性收集所有 diagnostic 再输出（collect-all），不 fail-fast。这对 AI 辅助创作至关重要：一轮修完所有错误比多轮对话高效得多。

---

## 2. 围栏元素

> **标签取舍判据**（历史标签下线决策的通用规则，供未来扩标签时复用）：浏览器有不可样式化原生 UI 的标签（`input`/`select`/`progress`...）→ 砍，改 role 驱动；已被 role 完整取代的纯结构标签（`ul`/`li`/`label`...）→ 砍；保留 = 无 role 等价物（`div`/`span`/`img`/`template`/`slot`）或 CSS 完全可控且 AI 先验极强（`button`）。

### 2.1 文档壳标签（8 个，不进运行时树）

| 标签 | 用途 |
|---|---|
| `html` | 文档根，解析时消费 |
| `head` | 元数据容器 |
| `body` | 内容根，子元素提升为模板根 |
| `title` | 文档标题 |
| `meta` | 元数据 |
| `style` | 内联 CSS（打包期消费） |
| `link` | 外部 CSS 引用（`rel=stylesheet`） |
| `script` | 脚本（围栏外，打包期报错或跳过） |

这些标签在 `tree_builder` 阶段被消费，不产生运行时对象。其中两个承载 CSS 来源语义：

- **`<style>`**：内联 CSS，整块文本交阶段 4.5 解析。
- **`<link rel="stylesheet" href="...">`**：外部 CSS。href 相对所在 HTML 文件解析（页面与组件文件同规则），文件内容并入同一解析管线——规则、`@keyframes`、诊断与内联完全同待遇；读取失败 → `FenceStylesheetNotFound` error（定位在 `<link>` 标签，报错带解析后的完整路径）。其余 rel 值（icon 等）按文档壳语义静默消费。CSS 内不支持 `@import`（与 `<style>` 内出现未知 at-rule 同待遇：报错）。

**相对路径基准总则**——三处引用各按浏览器语义取基准，全部词法解析、打包期归一为 workspace 相对 sprite key：

| 引用 | 基准 |
|---|---|
| `<img src>` | 相对所在 HTML 文件（页面文件或组件文件；组件在 `components/` 子目录，引用外层资源要多爬层级） |
| `<link rel="stylesheet">` 的 href | 相对所在 HTML 文件 |
| CSS 内 `url()`（如 background-image） | 内联 `<style>` 相对所在 HTML 文件；外部 CSS 相对**该 CSS 文件**（打包期改写成等价的 HTML 相对路径后统一归一） |

### 2.2 运行时标签（7 个）

下表是完整的运行时标签注册表。控件与列表没有专属标签——作者在 `<div>` 上写 WAI-ARIA `role` 表达（spec §2.2），如 `<div role="slider">`、`<div role="list">`。列含义：

- **SemanticKind**：打包期标注的稳定语义类型（base 标签按 tag、控件/列表按 `role`）。
- **Display**：不写 CSS `display` 时的默认显示值。**注意：Yio 运行时不实现 CSS inline flow**——`Inline` 标签运行时被当作 block-level flex（撑满父宽、竖向堆叠），与浏览器的横向 inline 行为不同。为防止 AI 先验错误，inline 布局 box（button/img）**必须放进 flex 容器**，不能裸放在 block 容器里，否则打包报错（见阶段 6.5）。文本级 `span`/`slot` 豁免（其行内混排走 rich-text 模型；但阶段 6.4 的混排检查仍把 slot 按 block 级对待——两次保守判定同因：rich-text 标志打包期烘焙）。
- **Category**：HTML 分类简化为四值——Block（块级结构）、Phrasing（行内文本级）、Void（自闭合）、Transparent（透明，继承父级）。
- **ContentModel**：允许的子内容——None（无子内容）、Text（仅文本）、Phrasing（行内元素+文本）、Flow（任意）、Transparent（继承父级）、Only([...])（仅列出的子标签）。

| 标签 | SemanticKind | Display | Category | ContentModel | Void |
|---|---|---|---|---|---|
| `div` | Container | Block | Block | Flow | |
| `span` | TextElement | Inline | Phrasing | Phrasing | |
| `button` | Button | Inline | Phrasing | Flow | |
| `img` | Image | Inline | Void | None | ✓ |
| `a` | Link | Inline | Phrasing | Phrasing | |
| `template` | Template | None | Phrasing | Flow | |
| `slot` | Slot | Inline | Transparent | Transparent | |

两个高频踩点：

- **`<slot>` 在模板结构校验里按块级对待**（阶段 6.4 的混排检查）：slot 不与文本/`span` 混排在同一块容器的直接子里。slot 的 fallback 文本放进投影内容（使用处给子），或让 slot 与文本分属不同容器；`<span>` 只包 slot 不夹文本是合法包装。根因是 rich-text inline flow 标志在打包期烘焙，模板期无法预知投影进来的内容是行内还是块级，保守判定防止标志烘错。
- **没有 `<br>`**：多行文案拆多个块级容器。围栏刻意不做文本内换行标签——一切换行都是结构。

**`<a>`（Link，#74）三条围栏规则**——富文本内链接，折叠渲染模型（子树折进父 rich inline flow）只在 rich 上下文成立，故全部打包期拦截（不静默降级成普通 span）：

1. **仅 rich-text-block 上下文合法**：直接父须是非 flex 的 `span`（span 自身是 rich 候选）或 rich-text-block 容器；flex 容器 / 裸 block 容器 / `slot` / `template` 里出现 → `FenceLinkOutsideRich`。
2. **`href` 必填非空**：缺失或 trim 后为空 → `FenceLinkHrefRequired`。href 是 opaque 标识符（无 URI 解析语义，框架不解析不 OpenURL，原样回传给游戏），空链接是不可交互的死元素。
3. **子内容只许文本与嵌 `<span>`**：`img-in-a`、`a-in-a` 及其它元素子 → `FenceLinkInvalidChild`。

`<a>` 打包期烙 UA 默认样式：蓝（#0000EE）+ 下划线，作者 CSS 可覆盖（含 `:hover` 等伪类）。键盘聚焦/Enter 激活 deferred 至键盘导航项。运行时点击走既有 `Clicked` 语义事件、`Href` 只读回传（公共 API 见 public-api.md §7.1）。

### 2.3 控件与列表：role 驱动

控件与列表没有专属标签。作者在 `<div>`（或其他 base 标签）上写 WAI-ARIA `role` 表达，打包期 `resolve_semantic` 把 `role` 映射到对应 `SemanticKind`：

| role | SemanticKind | 必需子结构（打包期校验，阶段 6.8） |
|---|---|---|
| `combobox` | Dropdown | `role=listbox` 子（内含 `role=option`）+ `data-slot=value` 子（选中值显示区） |
| `listbox` | Container | ≥1 `role=option` 子 |
| `option` | OptionItem | — |
| `slider` | Slider | `data-slot=thumb` 子 |
| `spinbutton` | NumberField | — |
| `switch` | Toggle | — |
| `radio` | RadioButton | — |
| `progressbar` | ProgressBar | `data-slot=fill` 子 |
| `textbox` | TextField（默认）/ TextArea（`aria-multiline=true`） | — |
| `list` | ListView | `role=listitem` 子（或 `template > role=listitem` 蓝图） |
| `listitem` | ListItem | — |
| `tablist` | TabList | `role=tab` 子（panel 靠 `aria-controls` 关联，非 role）。可选 `data-activation="manual"`（值域 `manual` \| `automatic`，缺省 automatic；写坏值报 `FenceBadAttrValue`——语义消费的 data-* 在消费位校验）：manual = 方向键只移焦点、Enter/Space 才提交选中；automatic = 方向键即时选中且焦点跟随 |
| `tab` | Tab | — |
| `tree` | Tree | `role=treeitem` 子（任意嵌套深度——嵌套声明 = treeitem 内直接嵌 treeitem，围栏不认 group 包装层）。单选树（焦点移动即选中，APG 单选树模型）；键盘 APG Tree View 核心档：Up/Down 可见项间移动、Right 展开/进首子项、Left 折叠/回父项、Home/End 首末可见项、Enter/Space 激活（选中 + branch 折叠展开互切）。typeahead（APG optional）defer |
| `treeitem` | TreeItem | 有直接 `role=treeitem` 子 = branch（展开/折叠态，`aria-expanded="true"` 烘焙初值、缺省折叠；无 aria-expanded 的 branch 是合法折叠态）；无 = leaf。选中态从所属 Tree.selected 派生（`aria-selected` synth），初始选中 = 首个 `aria-selected="true"` 条目（无则首项）。branch 的 label 走子元素（wrapper div）——branch 宿主嵌套条目为 block 子，纯文本 label 会触发 `FenceMixedInlineBlock`；leaf 纯文本天然合法。样式钩子：`[aria-selected="true"]`、`[aria-expanded="true"]`、`[aria-level="N"]`（层级结构派生，顶层=1，缩进样式用） |
| `tabpanel` | Container（`div` tag 回退，非 role 分派） | — |
| `dialog` | Container（`div` tag 回退，非 role 分派——模态弹层容器） | — |

控件初始值属性分三类（围栏禁止 `<div>` 上出现 plain 控件属性）：

- **progressbar**：`aria-valuenow`/`aria-valuemin`/`aria-valuemax`（缺省 min=0/max=100）。填充比例按 ARIA 语义 `(value-min)/(max-min)`（min=0 时与 value/max 等价）；运行时合成 `aria-valuemin` 镜像（改 min 后属性选择器同拍生效）。
- **spinbutton**（NumberField）：`aria-valuenow`/`aria-valuemin`/`aria-valuemax` + `data-step`。
- **textbox/textarea/spinbutton** 共用编辑初始值：`aria-placeholder`（占位文本）、`data-maxlength`、`aria-readonly`；textbox/textarea 的初始**值**来自元素文本内容（ARIA 无 textbox-value 属性）。
- **radio** 分组用 `data-name`：ARIA 无「radio 组名」属性（标准靠 radiogroup 容器/相邻性表达），用 `data-*` 私有承载；同 `data-name` 自动互斥。

Dropdown 选中值显示：**必需** `data-slot=value` 直接子（内嵌 TextNode）——框架把当前选中项文本写进去；漏写 = `FenceMissingControlChild` 打包期报错（不再静默）。Dropdown 无内置箭头（作者自画）。

`switch` / `radio` 无框架槽位（没有 knob slot）：框架只切换 `aria-checked` 状态、不驱动子节点几何——视觉状态全在 CSS 状态选择器（含旋钮位移：`[role="switch"][aria-checked="true"] .knob { transform: translateX(...) }`，rematch 状态变化自动重评）。

`progressbar` 的 indeterminate：初始 `aria-valuenow` 缺席 = indeterminate（ARIA 语义，运行时 API 可翻转）；运行时合成 `aria-indeterminate` 供 CSS 属性选择器（`[role="progressbar"][aria-indeterminate="true"] [data-slot="fill"] { … }`）。indeterminate 期间框架对 fill 让权——清掉 determinate 时代写入的 inline `width`，fill 几何全归作者 CSS（keyframes marquee 等）；determinate 期间框架每帧写 fill 的 `width:%`（inline 语义，占据该属性）。

**运行时合成属性不参与打包期 CSS 命中**：`aria-indeterminate`（运行时合成）与 `aria-checked` 的 `true` 态（运行时翻转，作者手写的初始 `false` 态除外）在打包期 HTML 里不存在——§6.7 的子节点 CSS 命中门只匹配作者手写属性，若 fill/旋钮**只有** `[aria-indeterminate="true"] [data-slot=fill]` / `[aria-checked="true"] .knob` 一类态规则，会判「无命中」报 `FenceControlChildWithoutCss`。子部件须另有能命中打包期 HTML 的基础规则（如 `[data-slot="fill"] { … }` / `.knob { … }`），运行时态规则叠加其上。

### 2.4 自定义元素

标签名含 `-`（如 `<my-widget>`）识别为 CustomElement（`SemanticKind::CustomElement`），**display 默认 Block（同 div）**——连字符标签不在 TAGS 注册表，须在 css_resolve 显式铺默认，漏铺会落到 taffy Flex Row（模板根无显式宽时被内容收缩，浏览器块级根则撑满）。围栏放行含 hyphen 的标签名通过 Fence Gate；**注册验证在打包器**（R3 已落地）：每个 package dir 下 `components/<tag>.html` 即该标签的注册（Package 注册表承担 `customElements.define()` 角色，main-design §7.4），打包期见 hyphen 标签即查注册表展开（slot 投影 + 展开域锚定规则），未注册 → `UnregisteredCustomElement` 打包错误。`<slot>` 只在组件模板内合法（页面级 `<slot>` 打包错误）；无效 slot（light 子的 `slot` 属性无对应位 / 无默认 slot 却有游离子）同样打包期报错。

**组件隔离模型（Shadow DOM 式硬墙，三面同源）**——组件是自包含单元：

- **样式墙**：组件实例子树的 CSS 宇宙 = 组件文件自己的 `<style>` / `<link>` 外部 CSS；页面规则不进组件子树，组件规则不出组件。作用域语义三条（preview 浏览器近似与 core 同口径，改写单实现在打包器 `preview_comp_style`）：**①作用域 = host 子树含模板根**——组件规则的选择器原样匹配，单复合选择器（`.tip`）直接命中模板根自身，根类规则合法且生效（#95：preview 旧 JS 前缀只会拼后代选择器、根类规则整条死，已收编 Rust 修掉）；**②host 本体不吃组件规则**——host 节点归外层页面作用域（同 DOM shadow 规则不样式化 host），宿主标签开头的链（`my-widget.is-press .slot`）以 host 为链首祖先下探、可给子树定样式，裸宿主选择器（`my-widget {}`）不落 host 上；**③非 `@keyframes` 的 at-rule（`@media` 等）整段拒绝**——preview 同步丢弃（fail-closed，不产「预览能看、构建报错」的假象）。页面与组件要共享视觉（design token、公共控件样式），用同一份外部 CSS 文件在两边分别引入——同一文件、两个作用域各自生效。
- **投影归属**：light 子（`<my-widget><span slot="x">…</span></my-widget>` 里的 span）投影进组件后归组件作用域——给投影内容定样式写在组件文件里，或用内容属性（如 img 的 width/height）自带。
- **校验自包含**：组件文件独立过全围栏校验（注册期）。组件内控件（`role=slider` 等）必须由组件自己的 CSS 命中，页面 CSS 救不了（阶段 6.7 按组件文件自身的规则集跑）。
- **id 墙**：运行时 `Get<T>("id")` 不穿透组件实例边界（经 host 节点两跳进组件）。

**组件打包错误/警告全集**（packer 侧合成码，独立于 §7 的 fence DiagnosticCode 表）：`UnregisteredCustomElement`（未注册）、`DuplicateComponentName`（注册表重名）、`ComponentNameRequiresHyphen`（注册名无连字符）、`ComponentMultipleRoots`（组件模板必须**单一根元素**，多根报错、不静默产森林）、`ComponentKeyframesNameCollision`、展开环（组件引用成环）、无效 slot / slot 嵌套、同展开域 id 撞车（light 子 id 与组件模板 id 在同一展开作用域内必须唯一；跨实例重复合法）。

**组件 @keyframes 两级裁决**：打包期——同名同内容静默去重（同组件多实例必然同名）；同名异内容宿主胜 + warning。运行时——keyframes 全局按名表，后实例化的组件覆盖同名。

**已知降级**：手工 file:// 双击打开 HTML 已退役（预览脚本改由 `yio preview` server 注入，HTML 源零 `<script>` 引用）——一切预览（含组件页）走 `yio preview` 工作台。

---

## 3. 稳定语义签名

对象类型由不可变签名决定，实例化后不能改变。

### 3.1 签名 = tag + role

Base 标签按 tag 映射；控件/列表按 `role` 映射（`role` 优先于 tag）。

| 签名 | SemanticKind |
|---|---|
| `div` | Container |
| `span` | TextElement |
| `button` | Button |
| `img` | Image |
| `div role=slider` | Slider |
| `div role=spinbutton` | NumberField |
| `div role=switch` | Toggle |
| `div role=radio` | RadioButton |
| `div role=textbox` | TextField |
| `div role=textbox aria-multiline=true` | TextArea |
| `div role=combobox` | Dropdown |
| `div role=option` | OptionItem |
| `div role=progressbar` | ProgressBar |
| `div role=list` | ListView |
| `div role=listitem` | ListItem |
| `div role=tablist` | TabList |
| `div role=tab`（或 `button role=tab`） | Tab |
| `div role=tabpanel` | Container（panel 靠 `aria-controls` 关联，非 role 分派） |
| `div role=dialog` | Container（模态弹层容器，非 role 分派） |
| `template` | Template |
| `slot` | Slot |
| `tag-name`（含 hyphen） | CustomElement |

`resolve_semantic(tag, role, aria_multiline)`：`role` 优先，未识别的 role 回退到 tag 映射。CSS（class/伪类/computed style）永远不改变 SemanticKind。

### 3.2 CSS 不改变类型

`display:block/flex/none` 选择布局 Strategy，`overflow:auto/scroll` 选择滚动 Strategy——都是行为切换，不重建节点、不丢状态、不改 SemanticKind。

**交互原语路由**：视口平移/滚动（大内容小视口）用 `overflow:auto` 滚动容器——拖拽跟手、滚轮、惯性、边界钳制、滚动取消点击全自带，作者零拖拽数学；运行时 Drag 事件 API 是对象拖拽（标题栏、拖道具入格）的低层积木，`Delta` 为逐 Move 增量（累计偏移由 StartPosition+Position 推导）。文本拖选按指针类型分流：鼠标拖文本控件（含 readonly，disabled 不拦）= 扩展选区（占据手势、抑制祖先拖滚，滚走滚轮/滚动条）；触摸拖 = 照旧视口 pan——浏览器逐字对齐，触屏选区走后续长按通道。

---

## 4. 属性围栏

### 4.1 全局属性（所有元素接受）

| 属性 | 用途 |
|---|---|
| `id` | 同展开域内唯一标识（打包期校验；组件多实例之间同 id 合法） |
| `class` | CSS 类选择器目标 |
| `style` | 行内 CSS（Fence Gate 校验属性名 + 关键字值） |
| `slot` | 投影到父组件的具名 slot |
| `hidden` | 隐藏元素 |
| `tabindex` | 焦点顺序 |
| `draggable` | 拖拽使能（值域 `true` \| `false`，缺省 `false`；浏览器枚举值 `auto` 的原生拖拽语义在自绘引擎无对应物，报 `FenceBadAttrValue`）。`true` = 节点成为 drag_target 候选，pointer-down 后按位移阈值启动 DragStart/DragMove/DragEnd 事件链 |
| `type` | 通用类型提示（`input[type]` 结构分派已退役，现作普通全局属性透传） |
| `role` | WAI-ARIA 角色（白名单校验：注册表 12 role + `textbox` + `tabpanel` + `dialog`，未知值报 `FenceUnknownRole`——拼错的 role 静默回退成基础标签类型会跳过全部控件校验，故拒绝） |
| `aria-*` | WAI-ARIA 状态/属性（打包期校验 IdRef 关系） |
| `data-*` | 自定义数据属性（透传，不做结构验证） |
| `--*` | `--` 前缀属性（透传，可被 `[attr]` 选择器匹配；**不是** var() 解析来源——var 来源三模型见 §5.4：样式表规则 / 行内 `style` / 运行时 SetVar） |
| `part` | 组件内部节点暴露给页面 `::part()` 选择器的名字（Web Components 同名机制，#57）。语义只对组件展开域内节点生效：页面级裸 `part` 属性可写但不参与 `::part()` 匹配（无 shadow 边界可穿）；`[part="x"]` 属性选择器**不**等价 `::part(x)`——前者按字面匹配任何节点，后者是穿墙通道 |

### 4.2 结构属性（影响类型/核心行为，Fence Gate 校验值域）

当前运行时标签无专属结构属性（控件语义由全局 `role` 驱动，见 §2.3）。

### 4.3 内容属性（初始值透传，Fence Gate 校验属性名）

| 元素 | 内容属性 |
|---|---|
| `img` | `src`, `alt`, `width`, `height` |
| `button` | `disabled`（布尔属性 presence = true；映射运行时 disabled 态——点击抑制 / `:disabled` 伪类 / 指针不给手型，与运行时 disabled API 同一语义） |
| `slot` | `name` |

**语义限定内容属性**（按 resolve 后的控件语义放行，不按字面 tag）：`div role=option` 可带 `value`（镜像原生 `<option value>` 语义；运行时 `SelectedValue`/`OptionItem.Value` 优先取它，缺席回落该项文本）。普通 `div` 带 `value` 仍报 `FenceUnknownAttr`。

控件初始值不走内容属性——role 驱动控件把初始值放 ARIA（`aria-valuenow`/`aria-checked`/...）或 `data-*`（`data-step`/`data-name`）里（见 §2.3；option 的 `value` 是例外，见上）。

---

## 5. CSS 围栏

CSS 在围栏中以三个正交维度建模。每个 CSS 属性声明的结局有三档：**支持**（正常解析进运行时）；**围栏外**（属性名/值不在白名单 → error，诊断附围栏内替代写法引导）；**围栏内但用法致预览 ≠ 运行时**（warning 不阻断，见 §7 warning 清单）。

### 5.1 属性白名单（CssPropSpec）

每个 CSS 属性注册为一个 `CssPropSpec`，包含属性名、默认值、是否继承、值解析器类型。围栏只认注册表中的属性名，未注册属性名打包期报错。

当前注册的 CSS 属性按功能分组：

**尺寸**

`width`, `height`, `min-width`, `min-height`, `max-width`, `max-height` — 值域 Length / Percent / Auto / Viewport（`vw`/`vh`/`vmin`/`vmax`，分母 = Stage root_size 画布而非父容器；分辨率适配的重排语言，见 main-design §11.5）。

**布局**

- `display`（`block` / `flex` / `none` / `inline`，**不含 `grid`**）
- `flex-direction`（默认 `row`——标准 CSS 默认），`flex-wrap`（`wrap`/`nowrap`；`wrap-reverse` 围栏拒绝），`flex-grow`, `flex-shrink`, `flex-basis`, `gap`, `row-gap`, `column-gap`
- `justify-content`, `align-items`, `align-content`, `align-self`
- `order`, `aspect-ratio`
- `z-index`（层叠序，整数 / 负数合法 / 默认 0，`auto` 不收——缺省即 0）：管盖上关系，**不改 flex 排列**（那是 `order`，两者正交）。**画序 = CSS painting order 的 stacking context 全局分层（#96/#100 修齐，两端一致）**：画序不是逐父兄弟排序，而是每个 stacking context 内部四段——负 z 的 SC 沉底 → static 内容（树序）→ z0 层（树序：`position:relative/absolute` 未声明 z 的元素 + **opacity<1 / transform / filter 的元素**，后者按 CSS Color 规范「当作 z-index:0 的 positioned 元素」绘制）→ 正 z（升序、同值树序）。关键推论：**嵌套深度不设层**——static 子树里的 opacity/transform/filter/定位+声明 z 后代会被上提到所属 SC 的对应层（static 顶栏里的半透明图标浮在 absolute z0 底图之上、同顶栏的纯文本沉在底下——浏览器正是这么画的，#100 实证）；SC 内部再按同算法递归（「子树整体移动」：opacity 组里的 z:5 子不越过组边界）。**声明位硬门（error，#101）**：z-index 只能声明在定位元素（`position:relative/absolute`）或 flex item 上——非定位、非 flex item 的元素上声明 z-index 被围栏拒绝：浏览器对该声明视而不见（元素留在 static 绘制层）、core 恒生效（运行时直改 z 的 fgui 血统语义），同一份 HTML 预览与运行时画序不同 = 预览说谎；围栏里写不出来的分歧够不着（运行时 API 直改 z 不受影响，那是 API 层非围栏层）。配套 **warning（#101）**：同父兄弟 static 与 positioned（或声明 z）混排、static 侧无显式 z——CSS painting order 里 positioned 元素恒画在 static 内容之上（与树序无关），漏声明靠「碰巧画对」正是 #96/#100 的成因；显式声明画序意图（static 侧补 `position:relative; z-index:0`，同视觉）消警。跨端安全的叠放写法不变：非底图内容显式 `position:relative` + `z-index`。

**定位**

- `position`（`absolute` / `relative` / `static`；初始值 `static`）
- `top`, `right`, `bottom`, `left`
- **absolute 包含块 = 最近 positioned 祖先**（声明了 `relative`/`absolute` 的最近祖先，浏览器语义）；无 positioned 祖先时 = 初始包含块（视口）。已知限制：inset 全 `auto` 的 absolute 保持直接父的静态位置（浏览器 hypothetical-box 语义不做）；overflow 裁剪链仍随 DOM 祖先（含块在 overflow 祖先之上的逃逸裁剪不实现）——游戏 UI 罕见形态

**盒模型**

`padding-top/right/bottom/left`（px/视口单位/env()）, `margin-top/right/bottom/left`

> **UA 默认 / noop**：`button` UA 默认 `text-align:center` + `justify-content:center` + `align-items:center`（内容居中，`css_resolve.rs`）；`resize`（`none`/`both`/`horizontal`/`vertical`）fence 接受但 core 不消费（noop，避免 textarea 报 prop 名错）。

**边框**

`border-color`, `border-style`（`none` / `solid` / `dashed` / `dotted` / `double`，默认 `none`）, `border-radius`, `border-image-slice`

> **border 两条运行时限制**：① `border-color` 四边共享单色——同元素四边异色时后写者覆盖、四边全变（静默错渲染，非报错）；② 彩色边框环与 background-image/gradient 互斥——有图/渐变时边框不画（静默）。

**背景**

`background-color`, `background-image`, `background-size`（`cover` / `contain` / `100%` / `stretch`）, `background-repeat`（`repeat` / `no-repeat` / `repeat-x` / `repeat-y`，默认 `repeat`）, `background-clip`, `-webkit-background-clip`

**渐变**（`background-image` / `background` 值域，core `parse_gradient` 单一真相源）

- `linear-gradient([angle|to-dir], stop[, stop]*)`：角度 `<N>deg`（0=to top 顺时针，任意值）或 4 正向关键字（`to top/right/bottom/left`）；缺省 `to bottom`。stop = `color [pos%]`，1..=8 个（超出打包期报错）；色支持 hex/`rgb()`/`rgba()`/`transparent`；位置缺省按 CSS 规则烘（首 0% / 末 100% / 中间相邻中点）。
- `radial-gradient([shape||size][at pos], stop[, stop]*)`：shape = `circle`/`ellipse`（缺省 ellipse）；size = `closest-side`/`farthest-side`/`closest-corner`/`farthest-corner`/1~2 个长度（单长度=正圆、双长度=椭圆；缺省 farthest-corner）；`at cx cy`（`%`/px，支持负值；缺省 `50% 50%`）。stop 同 linear。
- 越界（`conic-gradient` / `repeating-*` / 角点方向 `to top right` / 命名色 / >8 stops / 坏语法）→ 打包期 `FenceBadCssValue`（inline 与 `<style>` 规则都报——`<style>` 侧有渐变值探针）。
- 渲染：program=6/7 per-fragment shader（premultiplied alpha 插值；`background-color` 垫底合成）；`background-clip:text` 文本渐变共享同一套采样数学。

**视觉**

`opacity`, `box-shadow`, `pointer-events`, `transform`, `transform-origin`, `filter`, `clip-path`

> **transform 原点**：`transform-origin` 在围栏——`<length|%>` × 2 + 位置关键字（`left`/`center`/`right`/`top`/`bottom`；单值时 y 缺省 `center`）。缺省 `50% 50%`（= 元素几何中心）。百分比相对自身布局尺寸，**延迟解析**（存描述符、世界矩阵累计期按当帧尺寸解析）。绕非中心点旋转（连线、指针）直接声明 origin，不再需要「中点定位 + 换算」绕路。

> **box-shadow 完整语法**：`[inset] <ox> <oy> [blur] [spread] <color>`，逗号分隔多层（括号深度感知切层）；`inset` 位置任意；至少 2 个数值（偏移）；负 blur 静默 clamp 为 0；层叠序按 CSS——先声明的画在最上。层数硬限：inset 最多 8 层、outer 最多 4 层（渲染层合成 id 的编码容量，超限层会撞相邻编码区错渲染）——超限整条声明打包期 `FenceBadCssValue`（inline 走 `apply_decl` 返 false、`<style>` 规则走共享值域门，单一真相源 = core 解析器）。

- `clip-path`（**不继承**，#52 shape mask，fence 子集 = basic shapes 两形）：`none` / `circle(<length|%> [at <length|%> <length|%>])` / `polygon(<x> <y>, ...)`（3..=16 点，`%`/px）。circle 半径 `%` 按 `sqrt(w²+h²)/√2` 解析（CSS 精确语义——`circle(50%)` 在正方形盒恰内切）；polygon 点 `%` 各按宽/高。`at` 位置缺省 `50% 50%`（居心）。域外形态（`ellipse()` / `inset()` / `closest-side` / `farthest-side` / fill-rule 前缀 / geometry-box 关键字 / 裸 `circle()` / 负半径）→ `FenceBadCssValue`（单一真相源 = core `parse_clip_path`）。声明即 clipper：裁自身绘制 + 子树（与 overflow 独立，web 原义），命中测试同形（被裁区域点击穿透）。同元素 `overflow:hidden` + `clip-path` = 交集；`overflow:scroll/auto` + `clip-path` → `FenceClipPathScrollCombo` 硬错；裁剪链（overflow + clip-path 沿祖先链）深度 > 4 → `FenceClipChainTooDeep` 硬错（后端 clip 槽定长）。fill-rule 不收——实现走 crossing number，对简单（非自交）多边形与 web nonzero 一致。硬边语义（SDF/crossing，无羽化——soft clip 是 #113 deferred）。

- `cursor`（**不继承**，#93）：值集 `auto` / `default` / `none` / `pointer`。桌面端指针 affordance 入口——`auto`（缺省）= UA 默认行为：hover 到 pressable 控件（button/tab/toggle/radio/slider/dropdown/option 与 `<a>` 链接）运行时出手型**意图**（0/1/2 数值经 `CursorIntentChanged` 交后端渲染；Unity driver 缺省渲染系统光标，手型贴图由消费侧 `SetCursorTexture` 注册——见 projection-layer.md §3.3），其余系统箭头（纯 runtime 决策，不经样式通道、无需作者声明）；命中细化到控件内文字/内联子后沿祖先链上溯宿主控件判定（悬停按钮文字同样手型），disabled/不可命中控件不给手型并截断。显式声明恒压 UA 行为——`pointer` 手型（标非控件可点区）、`default` 箭头（把可点控件压回箭头）、`none` 元素级隐藏指针（配合游戏自绘光标）。文本输入类（TextField 等）的 I-beam 暂不在 UA 默认内。预览侧浏览器原生支持 `cursor`，无单侧缺口。

**文本**

`color`（继承）, `font-size`（继承；px/视口单位——响应式字号 `font-size:2vmin`，`%`/`em`/`rem` 拒）, `font-family`（继承）, `font-weight`（继承）, `text-align`（继承）, `line-height`（继承）, `letter-spacing`（继承）, `white-space`（继承）, `text-shadow`（继承）, `-webkit-text-stroke`（继承）, `font-effect`（继承，Yio 私有扩展）

- `text-decoration`（**不继承**，#74）：值集 `none` / `underline`（`line-through`/`overline` 围栏拒绝）。作用于 rich inline 流的 run——`span`/`a` 声明均可；`<a>` 打包期烙 UA 默认 `underline`，作者声明覆盖。

**换行控制**（#73，均继承）

- `white-space`：`normal` / `nowrap` / `pre` / `pre-wrap` / `pre-line` 全集——空白折叠 × 自动换行 × 源换行保留 三轴组合（`normal` 折叠+换行；`nowrap` 折叠+不换行；`pre` 保留+不换行+`\n` 断行；`pre-wrap` 保留+换行+`\n` 断行；`pre-line` 折叠空格+换行+`\n` 断行）。
- `overflow-wrap`：`normal`（超长词溢出不拆，浏览器一致）/ `break-word`（词独行仍超行宽才逐字拆）。
- `word-break`：`normal` / `break-all`（任意字符间可断，拉丁词也逐字）/ `keep-all`（CJK 词内不断，只退到空格边界断）。
- `text-wrap`：`normal` / `nowrap`（关自动换行，`\n` 强制断行与 white-space 空白语义保留）。CSS Text 4 的 `balance` / `stable` / `pretty` 不在围栏——写了报 `FenceBadCssValue`（标题居中场景用 `text-align`）。
- **禁则（kinsoku）**：CJK 断行自动避头尾——行首不出句读/闭括号（`。，、；：？！）】」』…` 等），行尾不出开括号（`（【「『` 等），违规断点自动左/右移（悬挂标点不做）。
- 文本控件（TextField/TextArea/NumberField）的空白语义冻结为 pre 系（空格/换行原样保留），换行开关仍尊重声明——空格折叠会破坏光标字节↔布局 1:1 映射（CSS UA 对 input/textarea 同为 pre 系）。

**文本控件私有属性**（CSS 用控件伪元素表达——`::placeholder`/`::selection` 一类控件伪元素不在围栏子集（唯一伪元素是 `::part`，见选择器节），故平铺 prop；`None` = render 回退到缺省色）

- `caret-color`（继承，`::selection` 光标色；缺省回退 `color`）
- `selection-background` / `selection-color`（`::selection` 选区背景/文字色；缺省蓝半透 / 白）
- `placeholder-color`（继承，`::placeholder` 占位符色；缺省把 `color` alpha 折半，对齐浏览器 `::placeholder` UA 默认 ~opacity 0.5）
- `-webkit-text-security`（继承，password 类输入掩码显示：`disc` `●` / `circle` `○` / `square` `■` / `none`；作用于文本控件显示串，`value` 不变——Chrome 同名属性，HTML 预览同步掩码）

**动画**

`animation`——`<name> <duration> [easing] [iteration-count|infinite] [fill-mode] [direction] [play-state] [delay]` 简写。对齐 public-api.md「动画定义全在 CSS」终态契约：fence 校验拼写错误并解析存值（逗号多声明 → 多个 `AnimationSpec` bake 进 `base_style.animation`，委托 core `parse_animation` 共用同一解析器防双源语义漂移）。8 个长划子属性（`animation-name`/`animation-duration`/`animation-timing-function`/`animation-delay`/`animation-iteration-count`/`animation-direction`/`animation-fill-mode`/`animation-play-state`）已加入——**单值**子集（逗号列表不收，多动画用简写）：写入既有简写 spec 的对应字段，无简写时创建惰性 spec（`animation-name` 到位才启播，CSS「无 name 不播」）；`animation-name: none` 清空；长划在简写后出现则改字段、简写在长划后出现则整体替换；runtime 驱动（@keyframes 表查询 + KeyframePlayer 时间轴）**M2 已交付**（见 main-design §13）。`transition`——`<prop?> <dur> <ease?> <delay?>` 简写，逗号多 spec 解析存值（bake 进 `base_style.transition`，core transition 引擎消费）；ease 关键字按 §5.2 对齐表映射。**transition 可动通道全集（#10 起）**：`background-color` / `color` / `opacity` / `transform` / `width` / `height` / `flex-grow` / `box-shadow`。layout 通道（width/height）端点要求**同域显式值**——`px↔px` / `%↔%` / `vw↔vw`（vw/vh/vmin/vmax 同族各自动）；`auto` 与异域混合（如 `px↔%`）不可动画：`@keyframes` 停靠点端点 parse 期硬拒，元素静态可见端点（inline + 结构匹配 class 规则含伪类变体）打包期扫描硬拒（`FenceLayoutTransitionEndpoint` error），运行时 `add_class` 组合出的动态端点由 core 兜底——不建 tween 直接跳到新值 + `EVT_TRANSITION_SNAP` 警告事件。box-shadow 过渡按 css-backgrounds-3 语义：列表长度不匹配时短列表补「透明色零长空阴影」逐对平滑插值（新增层透明淡入），配对 inset 标志不匹配 → 整表离散跳变（同浏览器）。

`@keyframes <name> { <stop> { decls } ... }` at-rule——`<style>` 内定义命名关键帧。stop 选择器子集：`from` / `to` / `<N>%`（0..=100 整数）；逗号多 stop（`0%,100%{...}`）按 CSS 语义展开为多条 stop（共享同声明块）。stop 声明块内可写 `animation-timing-function`（**per-stop timing**，CSS 标准语义：作用于本 stop 到下一 stop 的区段；未写回落 spec 级 timing）。transform 的 translate 分量收**百分比形**（`translateX(-50%)` / `translate(50%, 10px)`——相对自身布局尺寸，存储描述符、采样写入期解析；scale/rotate 无百分比概念照旧数值）。**停靠点可动属性全集（#10 起）**：`opacity` / `transform` / `background-color` / `color` / `width` / `height` / `flex-grow` / `box-shadow`——width/height 值须显式长度域（px/%/vw/vh/vmin/vmax，auto 拒）且同一 rule 内同域（`FenceLayoutTransitionEndpoint` error），box-shadow 收多层列表（插值语义同 transition 侧）。其他 at-rule（`@media` / `@font-face` 等）不在围栏子集，整块丢弃 + 诊断。

`/* @yio-hook <name> */` 注释锦点——写在 keyframes 的 stop 声明块内或块间（如 `from{...}/* @yio-hook start */ to{...}` 或 `from{/* @yio-hook start */ ...} to{...}`），挂在该 stop 上。合法锦点注释保留为内部 marker 供 stop 解析，普通注释照常移除；纯文本 `@yio-hook`（非注释上下文）不识别。运行时 player 跨越该 stop 百分比时 emit `AnimationHookEvent`，C# 经 `Animation.OnHook(name)` 或 `On<AnimationHookEvent>` 路由（见 public-api §9.4）。

**:nth-child(An+B|odd|even|N) 选择器**——参数化伪类，`<style>` 规则选择器接受。括号内 An+B 语法（`2n+1`/`2n`/`odd`/`even`/`<N>`）解析为 `NthChildExpr{a,b}`，命中条件 = 子序号 i 满足 `i = a*k + b`（1-based）。常配合 `animation` 简写实现错峰入场（delay 作简写第 2 个 time token，如 `.nav-card:nth-child(N){animation:fadeIn .4s .05s both}`——错峰单值也可用 `animation-delay` 长划单独声明）。语法越界（无括号/缺 `)`/坏参数）→ 选择器不匹配；组合子 `>` `+` `~` 仍越界（注意 `+`/`-` 在 `:nth-child(...)` 括号内是 An+B 合法语法，不判为组合子）。

> **⚠️ 虚拟化列表禁止 `:nth-child`**：虚拟化 `<ul>`（`role=list`）的 parked slot 留挂 ul 子树（`display:none`），按 CSS 仍计入 child count。`:nth-child` 的序数包含 parked slot → item 序号不可控。用 item-index / `data-*` 属性 + 属性选择器替代（如 `[data-index="0"]`）。

**::part(name) 选择器（#57）**——唯一进围栏的伪元素：页面规则穿组件内容墙命中组件内部带 `part="name"` 的节点。语法 `prefix::part(name)`：compound 前缀（class/tag/伪类等）匹配**组件 host**（host 归页面域，可带 class/id），`::part(name)` 匹配 host 展开子树内带该 part 属性的**目标节点**。约束：必须位于最后一个 compound 的结尾（伪元素后不可再缀，`X::part(a) Y` 越界）；**一层不递归**——嵌套组件内部节点的 host 是嵌套组件自身，外层 `::part()` 不下钻（跨层要组件作者显式中转，`exportparts` 不在子集）；specificity 按 web（part 名属性级 + 伪元素元素级，`.card::part(title)` = (0,2,1)）。运行时注入（`StyleSheet.Add`）同语法可用——运行时规则本就字面穿墙，`::part` 在该通道只是普通匹配臂。预览（浏览器平铺 DOM 无 shadow）由 preview server 把 `::part(n)` 重写为平铺等价 `[part="n"]` 后代选择器，保真不缺样式。**作者规则：part 目标节点的可覆盖属性（color 等）不要写进行内 style**——行内声明按 CSS 语义压过一切 class 含 `::part`（页面侧怎么写都赢不了）；组件默认视觉全走 `<style>` 类，行内只放结构性布局。

**溢出**

`overflow-x`, `overflow-y`

### 5.2 值校验（CssValueParser）

每个属性的值通过对应的解析器校验。主要类型：

| 解析器 | 校验方式 |
|---|---|
| `Keyword([...])` | 必须是列出的关键字之一 |
| `Length` | 长度值（px 或视口单位——#110 全长度属性放开） |
| `LengthPercent` | 长度或百分比（长度含视口单位） |
| `LengthPercentAuto` | 长度、百分比或 auto（长度含视口单位） |
| `Viewport` | `vw` / `vh` / `vmin` / `vmax`（分母 = 画布对应维；#110 起全长度属性收：尺寸族 + inset/margin/padding/gap 族 + `flex-basis` + `font-size` + `letter-spacing` + `border-radius`） |
| `Env` | `env(safe-area-inset-top/right/bottom/left)`（长度 token 的第四种来源，全长度属性可用；值 = 画布伸进屏幕 unsafe 区的深度，design px——fit 模式贴物理边 → 真实 inset，letterbox → 恒 0（黑边已让位不重复避让）。safe-area 之外的 env() 名是 `FenceBadCssValue`） |
| `Color` | 颜色值（围栏仅校验属性名，值格式交 core `parse_color`：`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa` hex、`rgb()`/`rgba()` 函数式） |
| `Number` | 数字 |
| `Integer` | 整数 |
| `Overflow` | `visible` / `hidden` / `scroll` / `auto` |
| `Transform` | translate / rotate / scale |
| `Filter` | grayscale / brightness / contrast / saturate / hue-rotate / invert / sepia |
| `BoxShadow` | `[inset] ox oy [blur] [spread] color`，逗号分隔多层（首层最贴盒子） |
| `TextShadow` | `ox oy [blur] color` |
| `Transition` | `property duration easing delay` 简写→`TransitionSpec`（逗号多 spec；ease 按下面对齐表） |
| `Animation` | `<name> <duration> [easing/count/fill/direction/play-state/delay]` 简写→`AnimationSpec`（逗号多声明；ease 按下面对齐表） |
| `BackgroundImage` | `background-image` 值域：`none` / `url()` / `linear-gradient()` / `radial-gradient()`（渐变子集交 core `parse_gradient` 探针） |
| `TextEffect` | `glow(w color)` / `blur(w)` |
| `TextStroke` | `width color` |
| `BackgroundClipText` | `text` 触发渐变字形 |
| `BorderRadius` | 1-4 值 px/%/视口单位 + `/` 垂直值 |
| `FourSidedPx` | 1-4 值 px（九宫格等） |
| `FourSidedMargin` | 1-4 值 px/%/auto |
| `Raw` | 原样存储，不校验 |

关键字值校验在 `css_resolve` 阶段进行。非关键字值由 `apply_decl` 的值解析逻辑处理，解析失败也产生 `FenceBadCssValue` diagnostic。

**长度形态门（单位强制）**：长度族声明（`Length`/`LengthPercent`/`LengthPercentAuto`/`BorderRadius` 属性 + `padding`/`margin` Box 简写）打包期逐 token 校验单位——**非零长度必须带单位，仅 `0` 可裸写**（`FenceBadCssValue`）。理由：core 的 `parse_four`/`parse_lp` 对裸数字一律当 px（运行时生效），浏览器则整条丢弃——`padding: 14px 6 16px 6` 这类声明预览与运行时静默分叉（#95 skill-slot 实证）。单位集按域开放（与 core 通道一致，见上表 Viewport/Env 行）：px-only 族（padding/gap/font-size/letter-spacing）收 `px`/视口单位；尺寸族/inset/margin 收 `px`/`%`/视口单位/`auto`；border-radius 收 `px`/`%`/视口单位。`env(safe-area-inset-*)` 四值作为长度 token 域无关全收（见上表 Env 行）。longhand 只收单值（多 token 浏览器无效、core 只取首值）。**注意**：`em` 全域不在围栏（core `parse_px` 不收）——浏览器生效、运行时丢弃的反向分歧同样拦（showcase 曾用 `letter-spacing:-0.02em` 实证，已换算 px）。

**缓动函数全集**（真相源 = core 的 `parse_ease`/`css_ease_keyword`，fence/core 共用同一解析器防漂移）。

- **CSS 标准**：`linear` / `ease` / `ease-in` / `ease-out` / `ease-in-out`（映射到 CSS Easing Functions L1 的**精确 bezier 等价**——早期 Quad/Cubic 幂函数近似已废除，缺省 timing = `ease`）；`cubic-bezier(x1,y1,x2,y2)`（x∈[0,1] 约束，y 可越界表 overshoot）；`step-start` / `step-end`（单步 steps；`steps(n,…)` 多步形不收）。
- **yio 超集 keyword**（非 CSS 标准，游戏 UI 刚需，命名照 CSS keyword 惯例）：`ease-{in,out,in-out}-back` / `ease-{in,out,in-out}-elastic` / `ease-{in,out,in-out}-bounce`（固定系数；参数化 elastic 不收，要参数用 cubic-bezier 近似）。
- **运行时专属**（不进 DSL）：`quad-*`/`cubic-*` 幂函数族在运行时 API（ScrollTo 等）可用，DSL 侧不收。
- 非法值（未知 keyword / bezier x 越界 / 坏参数个数）→ `FenceBadCssValue`。

### 5.3 简写展开（ShorthandSpec）

| 简写 | 展开为 | 展开方式 |
|---|---|---|
| `padding` | padding-top/right/bottom/left | Box（四边） |
| `margin` | margin-top/right/bottom/left | Box（四边） |
| `inset` | top/right/bottom/left | Box（四边，#110 进围栏；值域同 longhand） |
| `overflow` | overflow-x, overflow-y | Replicate（双轴同设） |
| `border` | border-color | BorderShorthand |
| `border-width` | — | Box |
| `border-top/right/bottom/left` | — | FallThrough（单边） |
| `background` | — | BackgroundShorthand |
| `flex` | flex-grow, flex-shrink, flex-basis | FallThrough |

### 5.4 自定义属性与 var()（#11）

CSS 自定义属性 `--x` 与 `var()` 引用按 web 语义收窄进围栏。**来源模型（三源）**：
①`<style>` 规则声明（`.theme { --accent: #f00 }`）②行内 `style="--accent: #f00"`
③运行时 `Style.SetVar`（C#，最高优先级）。HTML 属性 `--*`（§4.1 表）是纯数据透传
（可被 `[attr]` 选择器匹配），**不是** var() 解析来源——web 先验里行内 style 才是。

- **声明**：`--` 前缀 + 名字（字母/数字/`-`/`_`），值近乎自由（CSS 规范行为：custom
  prop 值不做关键字校验），仅做 `var()` 形状校验（括号配平、引用名合法 `--` 前缀）。
- **消费**：`var(--x)` / `var(--x, fallback)` 可出现在**任意属性的值位**——含 var()
  的值打包期跳过字面值校验（终值运行时才定），仅做形状校验；fallback 可嵌 var()
  （递归解析）。继承沿祖先链、跨组件边界（main-design「`--*` 跨边界传递」）。
- **解析语义**（core `style::vars`，CSS computed-value 收窄）：同节点先按级联选出每
  名胜出声明、再对胜出集递归解析引用（声明顺序不影响引用可见性）；声明点解析
  （`--a: var(--b)` 在声明 `--a` 的节点环境里解析，结果随继承传播）。优先级：规则
  声明 < 行内 style < 运行时 SetVar。
- **环与缺失（分层 fail-loud）**：引用环（`--a ↔ --b` 互指）与引用链断——运行时该
  custom prop 为 invalid，消费声明**跳过**（有 fallback 用 fallback）、不抛异常（主题
  系统不能变运行时炸弹），跳过事件 warn-once 进运行时 warnings；**打包期**对同块
  （`<style>`）/同元素（style 属性）静态可见的环发 `FenceCustomPropCycle` warning
  （非阻断——不同选择器命中的声明可能落在不同节点、运行时不成环）。目标缺失本身
  合法不告警（运行时 SetVar 可注入，打包期无法预判）。
- **运行时注入**（`UIContext.StyleSheet.Add`）：解析同围栏子集（选择器 + 声明），
  **at-rule 一律拒**（含 `@keyframes`——运行时动画注入不在本通道）→ `UIStyleException`
  带行列。注入规则是**全局规则**：与模板 CSS 同 cascade 优先级（同 specificity 后
  注入赢）、跨作用域命中——**打包期组件内容墙不约束运行时注入**（字面匹配可及组件
  展开内部节点；程序员工具非 AI 编辑面，public-api §10.2）。

---

## 6. 六阶段流水线（+ `<style>` 解析）

围栏验证是一条六阶段流水线，输入 HTML 字符串，输出 `ParsedTemplate`（IrTree + ResolvedStyle 数组 + Diagnostic 数组 + 引用精灵列表 + `dynamic_rules` 规则表 + `keyframes` 关键帧表）。

### 阶段 1+2：Tokenize + Tree Build

- html5gum 0.8 WHATWG tokenizer 词法分析。选型：纯 WHATWG tokenizer（非 tree builder），tree builder 自建、`emit_error` 收集为 Diagnostic 而非静默恢复——html5ever 被否因其静默错误恢复与「不静默降级」原则冲突。
- **要求显式闭合标签**：不实现 HTML 隐式闭合（浏览器容忍省略 `</p>`/`</li>`），未闭合 → `UnclosedTag`——对带浏览器先验的 AI 作者，显式闭合低摩擦且确定性。
- 构建中间表示 `IrTree`（元素节点 IrElement / 文本节点 Text / 注释 / Doctype）。
- 每个 IrNode 携带字节偏移 Span，用于后续 diagnostic 定位。
- 文档壳标签在此阶段被消费。

### 阶段 3：Fence Gate（逐元素校验）

对每个元素检查：

- **标签名**：不在注册表中、不是壳标签、不含 `-` → `FenceUnknownTag`
- **属性名**：全局属性放行；结构属性按值域校验（`FenceBadAttrValue`）；内容属性按白名单校验；未识别 → `FenceUnknownAttr`
- **行内 CSS 属性名**：不在 `CSS_PROPS` 且不在 `CSS_SHORTHANDS` → `FenceUnknownCssProp`

不做跨元素检查（那是阶段 5 的职责）。

### 阶段 4：CSS Resolve（行内样式解析）

对每个元素的 `style` 属性：

- 逐条声明解析 `prop: value`。
- 属性名校验（同阶段 3 但在 resolve 上下文）。
- 关键字值域校验（`Keyword([...])` 类型属性）。
- 值解析校验（`apply_decl` 返回 false 时报 `FenceBadCssValue`）。
- 应用 DisplayDefault（来自 schema）：Block → `DisplayMode::Block`，Inline → `DisplayMode::Flex` + `flex-direction:row`，None → `DisplayMode::None`。
- **`flex-direction` 默认 `row`**（标准 CSS 默认值）。如果未显式设置且 display 为 flex，强制覆盖为 Row。

产出 `Vec<ResolvedStyle>`（每节点一个，按 node-index 对齐）。

### 阶段 4.5：`<style>` 块解析

- 解析 `<style>` 标签的文本内容为 `DynamicRule` 选择器规则表（class/tag/id/后代空格/子代 `>`（#114）/属性选择器/伪类 + specificity）。
- 解析 `<link rel="stylesheet">` 引入的外部 CSS——同一解析器、同一待遇（规则 + `@keyframes` + 诊断 file 落 CSS 路径）；CSS 内 `url()` 相对该 CSS 文件，解析前改写成等价的 HTML 相对路径（见 §2.1 路径基准总则）。
- 解析 `@keyframes` at-rule 为 `KeyframesRule` 表（`from`/`to`/`N%` stop 选择器）。
- 产出存入 `ParsedTemplate.dynamic_rules` + `ParsedTemplate.keyframes`。
- 其他 at-rule（`@media` 等）丢弃 + 诊断。
- **选择器解析器是手搓的**（零选择器依赖）：cssparser 只是分词器——不产选择器 AST、不算 specificity（prelude 只是一串不透明 token），要 AST 须再叠 `selectors`/`scraper`，连带拖进 fence 不需要的 HTML 解析器。spike 实证后放弃接入，手搓直产 core 类型（组合子集：class/tag/id/后代/子代/属性/参数化伪类）。

### 阶段 5：Structural（跨元素结构校验）

- **Content Model**：子节点的 Category 必须被父节点的 ContentModel 允许。如 `<div>` 中放任何 Flow 内容合法，但 `<span>` 中放 `<div>`（Block inside Phrasing）报错。
- **文本内容**：ContentModel 为 `None` 或 `Only([...])` 的元素不接受文本子节点（空白文本节点跳过）。
- **ID 唯一性**：同一模板作用域内重复 `id` → `DuplicateId`。
- **Deferred 验证**（后续阶段新增）：ARIA 关系（`aria-controls` / `aria-labelledby` 的 IdRef 目标存在）。
  （`label[for]` 与 `ul/ol` 内 `template` 根校验随 `label`/`ul`/`ol`/`li` 标签下线而移除——列表结构契约改由阶段 6.8 按 `role` 校验。）

### 阶段 6：Annotate（语义类型填充）

对每个元素调用 `resolve_semantic(tag, role, aria_multiline)`，填充 `IrElement.semantic`。`role` 优先于 tag；这是确定性的：同样的 tag + role 永远产生同样的 SemanticKind。

### 阶段 6.4：rich-text-block 分类 + mixed 报错

**根因**：Yio 运行时只在一种上下文里实现浏览器式 inline flow——**rich-text-block**：`display:block` 容器且其直接子**全是 inline 级**（text / `span`(TextElement) / `img`(Image)）。runtime 把这些 inline 子拍平成 `RichRun` 走 `measure_rich_text`（见 main-design 文本模型），与浏览器预览一致。若 block 容器的直接子**既有 inline 级又有 block 级**（`<div><span>x</span><div>y</div></div>`），inline flow 不可定义（一部分要横排流、一部分要纵向堆叠，同一 formatting context 无解）。本阶段在打包期分类 + 拦下这种混合。

**规则**：每个 `display:block` 容器（tag 默认或 inline style 烘入；**非 flex**——flex 子是 flex item，走 flex 排版）的直接子（过滤纯空白文本后）：
- 全是 inline 级且 ≥1 个 → 标 rich-text-block（记进 `ParsedTemplate.rich_text_blocks` 的 ir_idx 集合，packer bridge 据此烘 flag）
- inline 级 + block 级混合 → `FenceMixedInlineBlock` error，打包失败。错误信息教学两种改法（二选一）：
  1. inline 子裹进一个子 `<div>`（让外层直接子全 block）。
  2. 容器改 `display:flex`（让所有子变 flex item）。
- 全 block 子 / 空子 / 仅纯空白 → 不分类

**inline 级集合**：`IrText`（非纯空白）/ `span`(TextElement) / `img`(Image)。其余元素子（`div`(Container) / 控件 / `template` / 自定义元素）为 block 级。`button` 是控件非 phrasing，**不进 inline 级**。

**显式 flex 的 span 是 block 子**：声明了 `display:flex` 的 `span`（inline style 或静态可判定的 class 规则）在父容器的分类里算 **block 级**——浏览器 `display:flex` 同时改内外显示类型（外层块级），且运行时它被豁免出 rich 折叠、自身是 flex 容器。若仍按 inline 折进父的 rich 流，slot 投影进该 span 的块级内容会被整棵折成一行（或被防御性跳过而隐身）。因此：flex-span + 装饰空白（无其他 inline 子）的父容器不标 rich、不报错；text + flex-span 混排按 mixed 报错（与 div 子同款）。

**slot 不进纯 span**：无显式 `display:flex` 的 span 直接包 `<slot>` → `FenceSlotInInlineContext` error。投影进 slot 的 light 子落在 inline 上下文，块级子无法按自身 display 参与宿主布局（浏览器里 slotted 节点在 slot 位置正常参与布局）。slot 须放在 `div` 或显式 flex 的 span 里。

**空白折叠**：纯空白文本子（缩进/换行，块间装饰空白）视为**中性**——既不算 inline 也不算 block。否则任何缩进的 block 模板都会被误判成 mixed。与浏览器块间空白折叠语义一致。

**display 判定**：复用阶段 6.5 的 parent-display 判定 helper（inline style + tag 默认 + 单 compound 静态可判定选择器——class / id / 静态属性如 `[role="tablist"]`；状态属性选择器（aria-checked 等）与多 compound 后代/子代规则保守放行）——两阶段对「parent 是 block 还是 flex」结论一致，避免 showcase 的 `.row { display:flex }` 等布局被误判。

**须在 6.5 之前跑**：6.5 的 img 豁免读本阶段产出的 `rich_text_blocks`。

### 阶段 6.5：inline 元素布局上下文检查

**根因**：taffy 0.12 不支持 CSS inline flow（inline 元素自动横排换行）。Yio 只在一种上下文里让 inline 元素和浏览器一致：**flex 容器内**——inline 元素是 flex item，按 flex 规则排（两边行为相同）。

在 **block 容器**里（裸 `<div>` 等），Yio 把 inline 标签当 block-level（撑满 + 竖排），和浏览器的 inline 行为（收缩 + 横排）必然不一致。放任这种写法会让 AI 按浏览器先验预期横排、运行时却竖排 → 渲染不可预测 → 返工。

**规则**：inline 布局 box（button/img）若**直接在 block 容器里**（parent 是 block、元素自己未显式 `display:block`）→ `FenceInlineElementInBlockContext` error，打包失败。错误信息教学两种改法（二选一）：
1. 父容器加 `display:flex`（多元素横排加 `flex-wrap:wrap`）。
2. 元素显式 `display:block`（有意当块级撑满）。

**豁免**：
- `span`（TextElement）/`slot`：文本级或结构占位，其行内混排走 rich-text 模型，不是 flex 能修的。
- 元素显式 `display:block`：作者有意当块级（撑满），浏览器也撑满，两边一致。
- parent 是 flex（inline style / tag 默认 / `<style>` 规则声明 `display:flex`——单 compound 静态可判定选择器：class / id / 静态属性）。
- **parent 是 rich-text-block**（阶段 6.4 判定）时 `img` 豁免：img 作为 inline run 走 rich-text inline flow，与浏览器一致。`button` 不豁免——button 是控件非 phrasing，不进 inline 级集合。

**parent display 判定**：stage 4 css_resolve 只烘 inline style + tag 默认 display；`<style>` class 规则的 display 在 dynamic_rules（运行时 rematch）。检查合并两个来源判定 parent 是 block 还是 flex：匹配用单 compound 的**静态可判定**选择器（class / id / 静态属性如 `[role="tablist"]`、`[data-slot="fill"]`；运行时可变状态属性——aria-checked / aria-expanded / hidden / tabindex 等——与伪类属条件化命中，不做静态判定）；多 compound（后代/子代）声明 flex 时保守放行（避免假阳性）。display:block 的子元素显式块级判定同源。

### 阶段 6.7：控件 CSS 命中校验

**根因**：Yio 控件（role 驱动：`progressbar`/`slider`/`switch`/`radio`/`textbox`/`spinbutton`/`combobox`）**不带 UA 默认样式**——core 刻意保持纯净，不开「框架自带样式源」先例。写了控件却没匹配的 CSS 规则 = 运行时渲染空白。浏览器会套自己的 UA 样式表，预览看着正常，打包进 Yio 却空——作者无法从预览察觉。本检查在打包期拦下，明确告诉作者差异。

**规则**：受校验控件（`role` 在控件 role 白名单）若**无任何 `<style>` 规则的选择器命中它本身** → `FenceControlWithoutCss` error，打包失败。**必需子节点（§6.8 契约表）的每个实例同样须被命中**，任一无命中 → `FenceControlChildWithoutCss` error——本体命中只证明作者在样式控件，不证明子部件被样式（thumb 无 background = 可拖不可见的隐形滑块头）；`option`/`listitem` 多实例逐个查（存在一个被命中的不算过），template 蓝图内的 listitem 同样查。tag / class / id / 后代 / 属性选择器落地在该节点都算命中；伪类（`:hover` 等）不门控（带状态规则同样表明作者在样式控件）。**只有完全无命中才报错**。

**选择器匹配**：复用 stage 4.5 解析出的 `dynamic_rules`，按 tag/class/id/attr 字面对照 IrElement 判定（fence-local，不依赖运行时 Node）。Descendant 沿祖先链逐层尝试；Child（`>`，#114）限直接父——直父不命中即整链不命中（与 core `match_element_with_state` 同款语义）。`+` `~` 仍拒。

**教学文案**：指出控件无内置默认样式，再按 role 给出修复指引（`data-slot` 子节点型：progressbar/slider 引导为控件本身 + `data-slot=fill`/`thumb` 子配 CSS；switch/radio 引导 `[aria-checked]` 属性选择器；combobox 引导控件本身 + `role=listbox`/`role=option` 子；textbox/spinbutton 引导 background/border + caret-color）。

### 阶段 6.7b：控件结构 CSS 契约校验

**根因**：命中校验只证明「作者在样式这个控件」，不证明**结构声明齐全**。控件的运行时行为对作者 CSS 有结构性依赖——最典型是 Dropdown 弹层：`role=listbox` 必须 `position:absolute` 脱流（缺了展开会把容器撑开）、combobox 本体必须 `position:relative` 做锚点（缺了弹层参照外层 containing block，core 的视口翻转定位数学失效）。这类缺陷在浏览器预览和命中校验里都看不出来（视觉规则照常命中），到 PlayMode 才显形——正是「围栏外输入打包期报错，不静默降级」要拦的类别。

**规则**：表驱动（`crates/fence/src/control_css_check.rs` 的 `STRUCTURE_CSS_CONTRACTS`，单一真相源）。当前契约：

- `combobox`：命中它的规则须声明 `position:relative`；其子树内 `role=listbox` 被命中的规则须声明 `position:absolute`。

任一缺失 → `FenceControlStructureCss` error，打包失败，教学文案附可直接抄的标准写法（`position:relative` 锚点 + `display:none; position:absolute; left:0; top:100%` 弹层）。后续控件的结构契约（如 slider fill/thumb 的绝对定位依赖）在核实后于表内扩展。

### 阶段 6.8：控件结构契约校验（必需子角色）

**根因**：role 化重构后（§2.2）控件结构由**作者写**，不再由框架运行时注入。作者可能漏写必需子角色（`<div role="slider">` 缺 `data-slot=thumb`、`<div role="combobox">` 缺 `role=listbox`）。把这种保证留到运行时 = 拿确定性换自由度，违背「围栏外输入打包期报错」的项目原则。本 pass 在 Annotate 之后严格拦截。

**规则**：带必需子角色的控件（见 §2.3 表）若**直接子节点**中缺对应 role / data-slot → `FenceMissingControlChild` error，打包失败。契约：

- `combobox` → 直接子含 `role=listbox`（listbox 再要求含 `role=option`，递归校验）+ `data-slot=value`（选中值显示区——运行时把选中 option 文本写进它内嵌 TextNode，漏写 = 选中值静默无显示）
- `listbox` → 直接子含 ≥1 个 `role=option`
- `slider` → 直接子含 `data-slot=thumb`
- `progressbar` → 直接子含 `data-slot=fill`
- `list` → 直接子含 `role=listitem`
- `tablist` → 直接子含 `role=tab`（panel 靠 `aria-controls` 跨树关联，不在此校验）
- `tree` → 直接子含 `role=treeitem`（嵌套条目直接嵌在 treeitem 内，无 group 包装层）

`textbox`/`spinbutton`/`switch`/`radio`/`option`/`listitem`/`tab`/`tabpanel` 无必需子角色（不校验）。

**tabpanel 不得手写内联 `display:none`**：TabList 运行时切面板 = 激活面板 unset inline display 回落作者样式（显隐所有权归控件，激活态布局归作者）——作者内联 `display:none` 烙进打包期 base_style 后 unset 清不掉，激活面板永久不可见（静默坏，无运行时症状）。非激活面板的初始隐藏由控件运行时首帧负责，作者不可（也无需）手写；手写即 `FenceTabpanelHiddenByAuthor` error。

**直接子字面**：校验只看**直接子节点**，与 §2.2 结构字面对齐——把必需子角色嵌进 wrapper div（如 `slider > div.wrap > data-slot=thumb`）不算满足契约，仍报 error。唯一例外是 **`list` 的 template 蓝图模式**：数据驱动 ListView 把 item 蓝图写在 `<template>` 子节点里（运行时克隆产 slot），`role=list > template > role=listitem` 视同满足 list→listitem 契约（template 的首个元素子节点被当成 listitem 检查）。

**教学文案**：诊断 message 按 role 给出作者应写的完整结构（如 combobox 引导 `role=listbox` 子 + `role=option` 孙；slider 引导 `data-slot=thumb` 子），取代旧 `.yio-*` 「照着填」的提示载体。

### 流水线特性

- **Collect-all**：所有阶段的 diagnostic 汇总到一个 `Vec<Diagnostic>` 输出，不 fail-fast。
- **Diagnostic 结构**：每条包含 severity（Error/Warning）、code（DiagnosticCode）、message、SourceLocation（文件名/行/列/源文本）、notes（Help/Note/Related）。
- **LineMap**：预计算的行偏移表，O(log n) 偏移到行列转换。

---

## 7. DiagnosticCode 完整清单

| Code | 含义 |
|---|---|
| `FenceUnknownTag` | 标签不在围栏注册表中 |
| `FenceUnknownAttr` | 属性不在元素的白名单中 |
| `FenceUnknownCssProp` | CSS 属性名不在围栏中 |
| `FenceBadCssValue` | CSS 值解析失败或不在允许的关键字域内 |
| `FenceBadAttrValue` | 结构属性值不在允许的枚举域内 |
| `FenceClipPathScrollCombo` | `overflow:scroll/auto` 与 `clip-path` 同元素（#52）——shape 裁滚动视口无清晰语义，硬拒 |
| `FenceClipChainTooDeep` | 裁剪链（overflow + clip-path 沿祖先链）深度 > 4（#52）——后端 clip uniform 槽 4 组定长 |
| `DuplicateId` | 同一模板作用域内 ID 重复 |
| `UnclosedTag` | 标签未闭合 |
| `InvalidContentModel` | 子元素不满足父元素的 ContentModel |
| `UnregisteredCustomElement` | 自定义元素未注册（打包器 components/ 注册表校验，R3 已接） |
| `InvalidAriaRelation` | `aria-controls` / `aria-labelledby` 目标不存在 |
| `TokenizerError` | html5gum tokenizer 遇到无法恢复的词法错误 |
| `FenceInlineElementInBlockContext` | inline 布局 box（button/img）裸放在 block 容器里（非 flex）；Yio 无 flex 之外的 inline flow，撑满竖排会和浏览器不一致 |
| `FenceMixedInlineBlock` | `display:block` 容器（非 flex）的直接子既有 inline 级（text/span/img）又有 block 级（div/控件/template）；rich-text inline flow 要求全 inline，混合不可定义。详见阶段 6.4 |
| `FenceSlotInInlineContext` | `<slot>` 位于无显式 `display:flex` 的 span 内。投影进 slot 的 light 子落在 inline 上下文，块级子被折进 rich inline 流（挤成一行/隐身）或按 flex-row hack 横排，无法按自身 display 参与宿主布局；slot 须放在 div 或显式 flex 的 span 里。详见阶段 6.4 |
| `FenceLinkHrefRequired` | `<a>` 缺 `href` 或值为空白（#74）。href 是链接身份标识（opaque，无 URI 解析语义），空链接是不可交互的死元素。详见 §2.2 `<a>` 规则 |
| `FenceLinkOutsideRich` | `<a>` 出现在 rich-text-block 上下文之外（flex 容器 / 裸 block 容器 / `slot` / `template`）（#74）。a 的折叠渲染模型只在 rich inline flow 成立。详见 §2.2 `<a>` 规则 |
| `FenceLinkInvalidChild` | `<a>` 的子元素不是文本/嵌 `span`（`img-in-a`、`a-in-a` 等）（#74）。详见 §2.2 `<a>` 规则 |
| `FenceBorderWithoutStyle` | **warning**：`border-width` 已声明但 `border-style` 缺省（CSS initial=none，浏览器不画边框，Yio 会画）；预览 ≠ 运行时 |
| `FenceBgImageWithoutSize` | **warning**：`background-image` 已声明但 `background-size` 缺省（CSS 默认 auto=原始尺寸，Yio 默认 stretch=拉伸填满）；预览 ≠ 运行时 |
| `FenceControlWithoutCss` | role 驱动控件（`progressbar`/`slider`/`switch`/`radio`/`textbox`/`spinbutton`/`combobox`）无任何 `<style>` 规则命中。控件不带 UA 默认样式，无 CSS = 运行时空白；须为控件及其 `data-slot` 子节点提供 CSS（详见阶段 6.7） |
| `FenceControlChildWithoutCss` | 控件的必需子节点（`data-slot=fill`/`thumb`/`value`、`role=listbox`/`option`/`listitem`/`tab`）无任何规则命中——本体命中不证明子部件被样式（隐形滑块头/隐形列表行）。每个实例都查（详见阶段 6.7） |
| `FenceControlStructureCss` | 控件结构 CSS 契约缺失（当前契约：`combobox` 本体缺 `position:relative`，或其子树内 `role=listbox` 弹层缺 `position:absolute`）。视觉规则命中 ≠ 结构声明齐全，缺锚点/脱流到 PlayMode 才显形；详见阶段 6.7b |
| `FenceMissingControlChild` | role 驱动控件缺必需子角色/slot（`combobox` 缺 `role=listbox`、`listbox` 缺 `role=option`、`slider` 缺 `data-slot=thumb`、`progressbar` 缺 `data-slot=fill`、`list` 缺 `role=listitem`、`tablist` 缺 `role=tab`、`tree` 缺 `role=treeitem`）。控件结构由作者写，漏写 = 运行时半残控件；详见阶段 6.8 |
| `FenceUnknownRole` | `role` 属性值不在 role 注册表（`ROLE_TO_SEMANTIC` + `textbox`/`tabpanel`/`dialog` 例外）。拼错防护：未知 role 若静默回退成基础标签类型，元素会跳过全部控件校验（必需子结构、CSS 命中、结构 CSS），构建绿灯但运行时空白——「不静默降级」原则拒绝此类 |
| `FenceStylesheetNotFound` | `<link rel="stylesheet">` 的外部 CSS 读取失败（href 相对所在 HTML 文件解析，报错带完整路径；静默丢样式是最难排查的降级形态） |
| `FenceDisplayInline` | **warning**：显式 `display: inline` 声明。围栏没有 inline flow——inline 运行时映射为 flex 容器，与浏览器 inline（收缩宽 + 横排流）语义不同，显式声明多半是先验误用 |
| `FenceLayoutTransitionEndpoint` | **error**：layout 动画端点域不一致或含 auto（#10）。transition width/height 的静态可见端点（inline + 结构匹配 class 规则，伪类变体算结构命中；`.a.b` 这类运行时 add_class 组合不在此列）域不齐（px vs % vs vw）或含 `auto`——运行时行为是离散跳变而浏览器平滑过渡，先验分歧打包期硬拒。运行时组合端点由 core `EVT_TRANSITION_SNAP` 警告事件兜底 |
| `FenceTransitionUnsupportedProp` | **warning**：`transition` 声明了运行时不支持的属性。transition 引擎驱动的通道全集 = `TRANSITION_PROPS` 八项（`background-color` / `color` / `opacity` / `transform` / `width` / `height` / `flex-grow` / `box-shadow`，#10 起扩容）；域外属性（margin/filter 等）声明了也不过渡，浏览器先验会翻车（`transition: all` 同理单独提示） |
| `FenceInlineSizing` | **warning**：rich-text inline flow 内的行内元素（span 等）声明 `width`/`height` 族。该类元素无独立盒子，尺寸声明恒无效（与浏览器对 inline 元素一致）。可定尺寸路径：flex item div / `<img>` / 显式 `display:flex` |
| `FencePageRuleProjectedOnly` | **warning**：页面侧纯类规则只可能命中 slot 投射内容。样式墙下页面规则不穿 host 边界，该规则运行时恒为死代码——给投影内容定样式写在组件 `<style>` 里 |
| `FenceComponentRuleOutOfScope` | **warning**：组件 `<style>` 纯类规则在本组件样式宇宙（模板 + 本组件投影内容）外恒无命中——类名只出现在页面 host 外区域或其它组件的投影内容上（组件 CSS 不穿出 host）。跨文件证据版：类名在宇宙内可命中或全库不出现（运行时 `Classes.Add` 挂类）则静默，宁漏报不误报 |
| `FenceSliderThumbPositioned` | **warning**：slider 滑块头（`data-slot="thumb"`）上声明非零定位（`top`/`right`/`bottom`/`left`/`margin` 族）。thumb 位移由控件按 value 全权驱动（水平位移 + 垂直居中，运行时逐帧归零其 inset/margin）——作者定位不生效且叠加双偏移（浏览器预览居中、运行时偏移）。标准写法 `left:0; top:0`（零值锚定）与尺寸/外观声明不受影响 |
| `FenceTabpanelHiddenByAuthor` | `role="tabpanel"` 手写内联 `display:none`。TabList 激活面板靠 unset inline display 回落作者样式——作者内联 display:none 烙进 base_style 后清不掉，激活面板永久隐身（静默坏）。面板初始隐藏归控件运行时首帧，删掉声明即可（详见 §6.8） |

#### 值域门（打包期 error，双路径统一）

行内 `style=""` 与 `<style>` 规则共用同一值域校验（`fence::value_check`）。运行时
`apply_decl` 对下列通道解析失败仍返回 true（宽松吞值）——浏览器先验里合法、运行时
恒无效的值一律打包期报 `FenceBadCssValue`：

- **颜色**：命名色（`red`/`blue`/...）在所有颜色属性上无效；`transparent` 仅 `color`
  接受（core 拦截），其余颜色属性显式清色用 `rgba(0,0,0,0)` / `#00000000`。合法格式：
  `#rgb` / `#rrggbb` / `#rrggbbaa` / `rgb()` / `rgba()`。
- **overflow**（简写与 `-x`/`-y`）：仅 `visible` / `hidden` / `scroll` / `auto`；
  `clip` 等浏览器值运行时静默忽略（等同 visible，无裁剪）。
- **filter**：仅 `grayscale` / `brightness` / `contrast` / `saturate` / `hue-rotate` /
  `invert` / `sepia`；`blur` / `drop-shadow` 等被静默跳过。
- **transform**：仅 `translate(X/Y)` / `rotate` / `scale(X/Y)`；`skew` / `matrix` 等
  被静默跳过（唯一函数时整条声明退化为 identity）。translate 的 px 参数在**静态声明**下
  仍是唯一形态（百分比相对自身尺寸，静态路径打包期无法解析）；**@keyframes stop 内**的
  translate 收百分比形（运行时按节点布局尺寸解析，见 §4 @keyframes 节）。
- `<style>` 规则的 Keyword 值域（此前不校验）与行内路径同门。

---

## 8. 防漂移机制

### 8.1 单一真相源

围栏的所有规则以 `crates/fence/src/schema/` 下的 Rust const 表为唯一真相源：

- `tag.rs` → `TAGS`（7 运行时标签注册表）+ `SHELL_TAGS`（8 壳标签）
- `attr.rs` → 全局属性、结构属性、内容属性定义
- `css.rs` → `CSS_PROPS`（属性白名单）+ `CSS_SHORTHANDS`（简写展开）

解析器、打包器、文档、测试不得各维护一份白名单。

**为什么是 Rust const 表而非外部数据文件**（TOML/JSON）：标签清单相对固定；值约束是 Rust 表达式而非纯数据；避免运行时解析 schema 文件、维护文件格式与新增解析依赖。

### 8.2 防漂移门

```bash
cargo test -p yio_fence                                # 全部围栏测试
cargo test -p yio_fence --test schema_contract         # schema 注册表契约
cargo test -p yio_fence --test doc_schema_sync         # 文档↔schema 交叉校验（防描述层漂移）
cargo test -p yio_fence --test pipeline_integration    # 端到端流水线
```

改围栏后必跑。测试 fail = 围栏契约被破坏。`doc_schema_sync` 从本文档主表解析标签清单与 schema 注册表比对，防止「代码改了文档没跟上」的描述层漂移。

### 8.3 消费者

本文档（`fence.md`）的消费者有三处，都引用本文为源：

| 消费者 | 位置 |
|---|---|
| 设计契约 | `docs/design/main-design.md` §3 |
| 设计师工作区 AI 规则 | 打包器模板 `editor/references/`（yio-editor skill 的渐进披露参考表，scaffold 落各工作区会话根） |

**同步规则**：改 schema 代码 → 检查两处消费者是否需同步。
