# LoomGUI 围栏（Fence）权威清单

> **单一真相源**：`loomgui_core/tests/fence_contract.rs`（可执行围栏契约测试）。本文档为人类可读副本，**以测试为准**。文档与测试不一致时，测试赢。
>
> **维护规则**：见 §4。改代码加/改围栏属性 → 必须同步改 `fence_contract.rs` 测试 → 同步本文档副本。CI/本机 build 前跑 `cargo test -p loomgui_core fence_contract` 是围栏契约的门。
>
> **AI 可预测性口径**（design §1.1）：写什么得到什么，围栏外即失败或静默忽略——但"静默忽略"行为本身必须被测试锁定，不可靠"推测"。

---

## 0. 核实方法论（为什么本清单可信）

每条围栏规则分两类标注：

- **【实证】**：有 `fence_contract.rs` 测试断言锁定行为。`fence_contract.rs` 建成后，本标注生效。
- **【推断·待测】**：靠源码 grep + taffy 默认值推断，`fence_contract.rs` 尚未覆盖。实现期补测试后转【实证】。

**反例（已纠正的推断错误）**：
- `position:relative` 曾被误判"代码无 match = 不支持"。实际 taffy 0.5 `Style::DEFAULT.position = Position::Relative`（taffy style/mod.rs:311），LoomGUI 不碰 position 字段 → **所有节点默认 Relative**，写不写行为一致。靠 taffy 默认生效，非显式映射。教训：**"搜索无 match"≠"不支持"，可能是底层默认**。必须查依赖默认值 + 补测试。

---

## 1. 元素标签围栏

**白名单**（`FENCE_TAGS`，parse/dom.rs:29）：

| 标签 | 映射 NodeKind | 出处 |
|---|---|---|
| `div` | Container | scene/node.rs:278 |
| `span` | Text（内容取 `el.text`） | scene/node.rs:283 |
| `img` | Image（src 取 `el.attrs["src"]`） | scene/node.rs:280 |
| `button` | Button | scene/node.rs:279 |

**围栏外标签**：一律**报错**（不降级、不静默忽略）。parse/dom.rs:63-68。
- 【实证】`rejects_fence_out_element`（dom.rs:203）：`<video>`/`<input>`/`<b>` 等被拒。
- 【实证】`fence_tags_all_accepted`（dom.rs:216）：白名单标签全接受。

**裸文本规则**：Container/Button 内裸文本自动生成 Text 子节点（scene/node.rs:304-316）。行内混排（元素内"文本+元素+文本"）报错（dom.rs:31 注释）。

---

## 2. CSS 属性围栏

`apply_decl`（style/mapping.rs:294）match prop，支持项返回 `true`，末尾 `_ => false`（mapping.rs:558，装饰属性静默忽略）。

### 2.1 布局属性（进 taffy_style）

| 属性 | 值约束 | 出处 | 标注 |
|---|---|---|---|
| `display` | `none`/其他→Flex（无 grid） | mapping.rs:425-431 | 【实证】 |
| `flex-direction` | row/row-reverse/column/column-reverse | mapping.rs:385-393 | 【实证】 |
| `flex-wrap` | wrap/nowrap | mapping.rs:394-400 | 【实证】 |
| `gap` | 四值展开取前两 | mapping.rs:377-384 | 【实证】 |
| `row-gap`/`column-gap` | gap longhand | （mapping.rs:377 同字段） | 【推断·待测】 |
| `justify-content` | flex-start/center/flex-end/space-between/space-around/space-evenly | mapping.rs:401-404 | 【实证】 |
| `align-items` | flex-start/center/flex-end/stretch/baseline | mapping.rs:405-408 | 【实证】 |
| `align-self` | 同 align-items 值 | mapping.rs:409-412 | 【实证】 |
| `flex-grow` | number | mapping.rs:413-416 | 【实证】 |
| `flex-shrink` | number | mapping.rs:417-420 | 【实证】 |
| `flex-basis` | dimension | mapping.rs:421-424 | 【实证】 |
| `order` | integer | mapping.rs:543-549 | 【实证】 |
| `width`/`height` | px/%/auto | mapping.rs:297-304 | 【实证】 |
| `min-width`/`min-height` | px/% | mapping.rs:305-312 | 【实证】 |
| `max-width`/`max-height` | px/% | mapping.rs:313-320 | 【实证】 |
| `padding` | 1-4 值 px（仅 px） | mapping.rs:321-330 | 【实证】 |
| `margin` | 1-4 值 px/%/auto | mapping.rs:331-340 | 【实证】 |
| `border`/`border-width` | 简写只取宽度（color/style 丢） | mapping.rs:341-352 | 【实证·待测简写 color 丢弃】 |
| `aspect-ratio` | number | mapping.rs:537-542 | 【实证】 |

### 2.2 视觉属性

| 属性 | 值约束 | 出处 | 标注 |
|---|---|---|---|
| `background-color` | #rrggbb hex | mapping.rs:444-447 | 【实证】 |
| `background-image` | url("path") | mapping.rs:448-452 | 【实证】 |
| `background-size` | 仅 cover/contain/100%（拒两值如 `100% 50%`） | mapping.rs:453-460 | 【实证】 |
| `border-color` | #rrggbb hex | mapping.rs:461-464 | 【实证】 |
| `border-radius` | px/% 1-4 值 + `/` 垂直值 | mapping.rs:353-376 | 【实证】 |
| `opacity` | 0-1 | mapping.rs:465-473 | 【实证】 |
| `overflow` | visible/hidden/scroll/auto（双轴同设） | mapping.rs:474-481 | 【实证】 |
| `overflow-x`/`overflow-y` | longhand | mapping.rs:482-494 | 【实证】 |
| `color` | #rrggbb hex | mapping.rs:495-500 | 【实证】 |
| `font-size` | px（拒 %） | mapping.rs:501-504 | 【实证】 |
| `font-family` | 原样存储 | mapping.rs:505-508 | 【实证】 |
| `font-weight` | u16 数字 | mapping.rs:509-512 | 【实证】 |
| `text-align` | left/center/right | mapping.rs:513-520 | 【实证】 |
| `line-height` | px 或裸数字 | mapping.rs:521-528 | 【实证】 |
| `letter-spacing` | px | mapping.rs:529-532 | 【实证】 |
| `white-space` | 仅识别 nowrap | mapping.rs:533-536 | 【实证】 |
| `transform` | translate(px,px)/rotate(deg)/scale(num[,num]) | mapping.rs:554-556 | 【实证】 |
| `pointer-events` | auto/none | mapping.rs:549-553 | 【实证】 |

### 2.3 v1.x 扩展属性（v1 围栏冻结子集未列，代码已实现）

| 属性 | 值约束 | 出处 | v1.x 版本 | 标注 |
|---|---|---|---|---|
| `filter` | grayscale/brightness/contrast/saturate/hue-rotate/invert/sepia（颜色矩阵，不认 blur/drop-shadow） | mapping.rs:432-436, color_filter.rs | v1.3 | 【实证·待测 blur 拒】 |
| `border-image-slice` | 1-4 值 px/%（九宫格） | mapping.rs:437-443 | v1.3 | 【实证】 |

### 2.4 围栏外 CSS 属性（写了不生效/静默忽略，必须测试锁定）

| 属性 | 实际行为 | 标注 |
|---|---|---|
| `position:relative` | 靠 taffy 默认 Relative 生效，写不写一致（无 inset 偏移） | 【推断·待测】 |
| `position:absolute/fixed/sticky` | 静默忽略，position 保持默认 Relative，**不脱离流** | 【实证】 |
| `display:grid` | 非 none 落 Flex，grid 布局不生效 | 【实证】 |
| `float` | 静默忽略 | 【实证】 |
| `align-content` | 无 handler，静默忽略 | 【实证】 |
| `cursor` | 静默忽略 | 【实证】 |
| `clip-path` | 静默忽略 | 【实证】 |
| `background-position` | 静默忽略 | 【实证】 |
| `background-repeat` | 静默忽略 | 【实证】 |
| `transform-origin` | 硬编码 center，自定义静默忽略 | 【实证】 |
| `transform: skew()/matrix()` | 显式跳过（mapping.rs:278） | 【实证】 |
| `font-style` | 无 handler，静默忽略 | 【实证】 |
| `border-style`（dashed/dotted） | 简写只取宽度，style 丢 | 【实证】 |
| `@media` | AtRuleParser 拒（parse/css.rs:58-63） | 【实证】 |

---

## 3. 选择器围栏

### 3.1 支持

| 选择器 | 出处 | 标注 |
|---|---|---|
| 标签 `div` | selector.rs:77-78 | 【实证】 |
| 类 `.btn` | selector.rs:79,94-95 | 【实证】 |
| ID `#main` | selector.rs:79,95-96 | 【实证】 |
| 后代 `div span` | selector.rs:18-60,238-253 | 【实证】 |
| 子代 `div > span` | selector.rs:23-36,228-237 | 【实证】 |
| 分组 `.a,.b` | css.rs:121-133（展开多 Rule） | 【实证】 |

### 3.2 伪类

| 伪类 | 出处 | 标注 |
|---|---|---|
| `:hover` | selector.rs:132, dynamic.rs:108-109 | 【实证】 |
| `:active` | selector.rs:133, dynamic.rs:111-112 | 【实证】 |
| `:disabled` | selector.rs:134, dynamic.rs:114-115 | 【实证】 |
| `:focus` | selector.rs:135, dynamic.rs:117-118（v1d.2 修复） | 【实证】 |

### 3.3 围栏外选择器（静默忽略）

| 选择器 | 出处 | 标注 |
|---|---|---|
| `:nth-child`/`:nth-of-type`/`:first-child` | selector.rs:136 `_ => {}` | 【推断·待测】 |
| `:not()` | 同上 | 【推断·待测】 |
| 属性选择器 `[attr]` | selector.rs:76-111 不解析 `[]` | 【推断·待测】 |
| 通配符 `*` | 走 FENCE_TAGS 报错 | 【实证】 |
| 相邻兄弟 `+` / 后续兄弟 `~` | selector.rs:21-60 不认 | 【推断·待测】 |

---

## 4. 维护机制

### 4.1 单一真相源

`loomgui_core/tests/fence_contract.rs` 是围栏契约的可执行真相源。它显式枚举：
- **支持项**：写进去断言生效（映射出非默认值 / 期望布局结果）。
- **围栏外项**：写进去断言不改变布局 / 被忽略（如 `position:absolute` 不脱离流、`display:grid` 落 Flex、`float` 无效）。

本 `fence.md` 是测试的人类可读副本。**两者不一致时测试赢**；文档过时仅是可读性问题，不致命。

### 4.2 改围栏时的流程

**新增围栏属性**（如 v1.x 加新 CSS 支持）：
1. `apply_decl` 加 match arm。
2. `fence_contract.rs` 加"支持"断言（写进去 → 期望生效）。
3. 本 `fence.md` §2 对应表补一行，标注【实证】。
4. 若是 v1.x 新增，roadmap §1.2 同步补。

**新增围栏外禁令**（明确某属性不该写）：
1. `fence_contract.rs` 加"围栏外静默忽略"断言（写进去 → 期望不改变布局）。
2. 本 `fence.md` §2.4 补一行，标注【实证】。
3. editor 的 CLAUDE.md.tmpl / fence.md 副本同步"禁写"清单。

**改 arm 行为**：
1. 测试 fail（行为变了）。
2. 评估：是 bug 修复（同步测试+文档）还是契约变更（design 主文档也要改）。
3. 同步测试 + 本文档。

**【推断·待测】→【实证】**：实现期补 `fence_contract.rs` 对应断言后，把本表格标注改成【实证】。

### 4.3 防漂移门

`cargo test -p loomgui_core fence_contract` 必须在以下时机跑：
- 本机编码机 build .dll 前。
- 任何 `apply_decl` / `FENCE_TAGS` / selector 解析的改动后。
- PR/合并前。

测试 fail = 围栏契约被破坏，必须修复或显式更新契约（同步文档）。

### 4.4 不做什么（YAGNI）

- **不从代码自动生成 fence.md**：match arm 提取不出"围栏外项"和"值约束"和"预览可信清单"，自动生成只能覆盖一部分，混合维护更乱。
- **不把 match 重构成数组驱动**：CSS 属性的值约束（如 background-size 只认 cover/contain/100%）塞不进数组，过度工程。
- 测试即文档是最轻的防漂移手段。

---

## 5. 围栏副本分发

围栏清单的消费者有三处，都引用本 `fence.md`（或其子集）为源，不各自维护：

| 消费者 | 位置 | 内容 |
|---|---|---|
| editor 围栏规则（注入设计师工作区） | `editor/rules/claude/CLAUDE.md.tmpl` + `editor/skill/loomgui-editor/references/fence.md` | 围栏清单 + 预览可信清单（roadmap §1.3） |
| v1 范围冻结 | `docs/roadmap/roadmap.md` §1.2 | 引用 fence.md + 标注 v1 冻结子集 / v1.x 扩展 |
| 设计契约 | `docs/design/main-design.md` | 引用 fence.md 为围栏权威源 |

**同步规则**：改 fence.md → 检查三处消费者是否需同步。editor 的 CLAUDE.md.tmpl 是注入给设计师的，过时会让 AI 生成违规 UI，优先同步。

---

## 6. 预览可信清单

open-design Chromium iframe 预览 ≠ taffy 渲染。AI 须分清：

**可信**（Chrome ≈ LoomGUI）：flex 轴/方向、显式 `display:flex`、`gap` 间距、颜色、opacity、border、图片、px 尺寸、`background-image`/`background-size`（标准 CSS，Chrome 原生）。

**不可信**（Chrome ≠ LoomGUI，别按预览调）：
- **margin 控间距**：Chrome（block flow）折叠 margin、LoomGUI（flex）求和不折叠。**子项间距用 `gap`**，别用 margin。
- **文本换行/像素级**：Chrome 文本引擎 vs LoomGUI（unicode-linebreak），换行点/塞文本宽度会偏。
- **`position:absolute`**：Chrome 脱离流、LoomGUI 不脱离（围栏外静默忽略）。预览会骗 AI。
- **`display:grid`**：Chrome 渲染 grid、LoomGUI 落 Flex。预览会骗 AI。
- **`@media` 响应式**：Chrome 响应、LoomGUI 用参考分辨率缩放不响应 @media。

**口径**：不可信项"信围栏规则，别信预览"。
