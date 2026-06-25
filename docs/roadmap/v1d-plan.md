# LoomGUI v1d 实施路线（滚动 + 键盘交互 + transform/动画 + 响应式）

> **判据**：v1d = 各轮 spec 明确标 `v1d`/`v1d+` 的**全部项**，照单全收，一个不漏。始终 v1.x（Controller/Gear/Transition）不进。
> **本文件是 v1d 的带勾选清单**——治"defer-defer、东西遗失"。每完成一子轮在 §5 打勾。子轮内精确设计走各自 brainstorming→spec→plan（`docs/superpowers/specs/`、`docs/superpowers/plans/`）。
> **依据**：v1-scope §1/§3/§4、v1x-deferred、v1a-spec:36/56/205、v1c.1:446-450、v1c.2:329-331、v1c.3:346、v1c.4:313。

## 0. v1d 目标

清掉 v1c 各轮 defer 到 v1d 的全部交互/动画/transform/响应式项，关 v1 验收 **#2 可滚动容器** + **#4 自适应分辨率（safe-area）**。v1d 完 → v1 验收全过 → **v1 ship** → v1e（压测/打磨）/ v1.x（Controller/Gear/虚拟化/…）。

## 1. v1d 全量项（12，defer 出处可溯）

| # | 项 | defer 出处 | 归属子轮 |
|---|---|---|---|
| 1 | 拖拽 drag（状态机+事件） | v1c.1:448, v1c.3:346 | v1d.1 |
| 2 | 拖拽/滚动仲裁 | v1c.1:448, v1c.3:346 | v1d.1(drag-vs-click) + v1d.5(嵌套 scroll) |
| 3 | 键盘 onKeyDown/Up | v1c.1:450(v1d+), v1c.2:330, v1c.3:346 | v1d.2 |
| 4 | 滚轮 onMouseWheel | v1c.1:450(v1d+), v1c.2:330 | v1d.5（随消费者滚动） |
| 5 | IME/字符输入（G5 最小 PC 字符级） | v1c.1:450(v1d+/G5) | ⚠️ §2 待定 |
| 6 | 焦点/Tab 导航 + `:focus` | v1c.1:449 | v1d.2 |
| 7 | transform 字段 + world_to_local 命中 + 渲染 | v1c.1:446, v1c.2:331, v1c.3:346 | v1d.3 |
| 8 | GTween 时钟（基础 tween） | v1c.1:447 | v1d.4 |
| 9 | ScrollPane（惯性/回弹/滚动条/自带 tween） | v1c.1:448, v1c.3:346, v1a:56 | v1d.5 |
| 10 | 手势仲裁 | v1c.3:346(v1d+) | v1d.5 |
| 11 | 长按 onLongPress（holdTime） | v1c.4:313, knowledge§626 | v1d.1 |
| 12 | safe-area（响应式） | v1a:36,56 | v1d.1（尾巴任务） |

## 2. 排除 / 待定（透明列出，可推翻）

- **broadcast 子树广播** → **v1.x**（v1c.2:329, design:457）。前置缺 added/removedToStage 事件、无消费者。
- **AncestorChain 池化 / invalidation set 伪类重匹配** → **v1e perf**，性能优化非功能 defer。
- **⚠️ IME/字符输入（#5）待定**：spec 标 v1d+/G5，但唯一消费者 TextInput 是 v1.x（同 broadcast **无消费者**）。**当前默认 defer 随 TextInput（v1.x）**。若要走 (a) 在 v1d.2 接 char-input plumbing，改 §1 表 #5 归属为 v1d.2。

## 3. 5 子轮分解

原则：新输入类型跟首个消费者同轮（键盘→焦点、滚轮→滚动）；transform 与 GTween 分开（测试面不同：render/hit vs 引擎）；drag 是 scroll 的前置。

**依赖**：仅 `v1d.5 ← v1d.1`（drag 是 touch scroll 前置）。v1d.2 / v1d.3 / v1d.4 **互相独立**，可任意序。ScrollPane 自带 tween（v1-scope §1），**不依赖 GTween(v1d.4)**。v1d.4 建议 v1d.3 后做（tween 能动 rotate/scale 更值，非硬依赖）。

| 子轮 | 内容 | 覆盖 # | 依赖 | 状态 |
|---|---|---|---|---|
| **v1d.1** | 拖拽 + 长按 + safe-area：drag 状态机(down→阈值→start/move/end) + drag-vs-click 仲裁 + onLongPress holdTime + safe-area(insets→root padding) | 1,2,11,12 | v1c click 基座 | ☐ |
| **v1d.2** | 键盘 + 焦点 + Tab + `:focus`：keydown/up 事件通道 + focused node / tab 序 / focus·blur 事件 / `:focus` 伪类重匹配 | 3,6 | — | ☐ |
| **v1d.3** | transform 命中+渲染：NodeTransform scale/rotation 激活(现死码) + CSS `transform` 解析 + world_to_local 命中 + blob 序列化 | 7 | — | ☐ |
| **v1d.4** | GTween 时钟 + 基础 tween：复用 tick dt 的 clock + easing 子集 + TweenManager / tween API | 8 | —（建议 v1d.3 后） | ☐ |
| **v1d.5** | ScrollPane + 滚动条 + 滚轮 + 手势仲裁：content offset/clip/轴锁/bounds/惯性回弹(自带 tween) + 滚动条 render G14 + wheel 滚动 + 嵌套/手势仲裁 | 4,9,10,2 | v1d.1 | ☐ |

## 4. 子轮目标速览（精确设计见各自 spec）

### v1d.1 — 拖拽 + 长按 + safe-area
- **drag 状态机**：Down 记起点 → Move 超 drag 阈值（与 click 阈值区分）→ DragStart → DragMove → Up → DragEnd；drag-vs-click 仲裁（Move>50 cancel click 已有基础）。
- **onLongPress**：Down 后 holdTime（照 fgui）到 → 发 LongPress（期间未 Move/Up）。
- **safe-area**：读 Screen.safeArea insets → root/指定节点 padding/margin，异形屏不挡。
- **不范围**：ScrollPane 逻辑（v1d.5）、手势（v1d.5）。
- **fgui 对照**（brainstorming 期研究）：DragManager、touch holdTime/downFrame。

### v1d.2 — 键盘 + 焦点
- **keydown/up**：新事件类型进 EventRecord/FFI/C# collector（Unity 新/旧输入系统键盘）。
- **focus**：focused node（Scene/Node 字段）+ tab 序（DOM 序 / focusable 标记）+ focus/blur 事件 + `:focus` 伪类（rematch）。
- **不范围**：IME char（§2 待定）、TextInput（v1.x）。

### v1d.3 — transform 命中 + 渲染
- 激活 `NodeTransform.scale_x/scale_y/rotation`（现死码）+ pivot。
- CSS `transform: translate/rotate/scale` 解析（**新增围栏属性** → 同步 v1-scope §2 + 主文档 §4.4）。
- world_to_local：命中累计逆矩阵；render blob 序列化 scale/rotation。
- **不范围**：3D/透视（v1.x VertexMatrix）、skew。

### v1d.4 — GTween 时钟 + 基础 tween
- **clock**：复用已接线 tick dt（v1c.4 unscaledDeltaTime；评估是否需 scaled dt 双参数）。
- TweenManager + `tween(start,end,dur,easing)` + delay + onComplete；easing 子集（Linear/Quad/Cubic/Back/…）。
- **不范围**：Transition/Controller/Gear 编排（v1.x）、路径 tween。

### v1d.5 — ScrollPane + 滚动条 + 滚轮 + 手势仲裁
- **滚动容器**：content offset + clip（现 clip_rect）+ 轴锁 + bounds + 惯性/回弹（**自带 target tween，不走 GTween**，v1-scope §1）。
- **滚动条** render（G14）；**wheel** 滚动（#4 输入通道本轮接）。
- **嵌套 ScrollPane / drag-vs-scroll 仲裁**（#2，消费 v1d.1 drag + v1c.3 多指）。
- **不范围**：虚拟化 `<l-list>`（v1.x）、分页/吸附/下拉刷新（v1.x）。

## 5. 进度勾选

- [ ] **v1d.1** 拖拽 + 长按 + safe-area
  - [ ] #1 drag 状态机+事件
  - [ ] #2 drag-vs-click 仲裁
  - [ ] #11 onLongPress
  - [ ] #12 safe-area
- [ ] **v1d.2** 键盘 + 焦点 + Tab + `:focus`
  - [ ] #3 keydown/up
  - [ ] #6 focus/Tab/`:focus`
- [ ] **v1d.3** transform 命中+渲染
  - [ ] #7 scale/rotation 激活 + CSS `transform` 解析 + world_to_local + blob
- [x] **v1d.4** GTween 时钟 + 基础 tween
  - [x] #8 clock + TweenManager + easing
- [ ] **v1d.5** ScrollPane + 滚动条 + 滚轮 + 手势仲裁
  - [ ] #9 ScrollPane 核心
  - [ ] #4 滚轮
  - [ ] #10 手势仲裁
  - [ ] #2 嵌套仲裁

## 6. v1d 完成判定

- §5 全勾（#5 IME 按 §2 默认 defer，或改 v1d.2）。
- v1 验收 **#2**（可滚动容器：惯性+回弹+滚动条）+ **#4**（safe-area 自适应）过。
- → **v1 ship**，进 v1e（冷帧/换页帧 FFI≤2ms 压测 + FairyBatching 实机）或 v1.x。
