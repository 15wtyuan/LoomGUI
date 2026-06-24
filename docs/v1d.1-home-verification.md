# v1d.1 家里机验收文档

> 本机（无 Unity）已完成 v1d.1 全部实现 + per-task review + final whole-branch review Ready。core+ffi `cargo test` **core 185 + ffi 30 = 215 全绿**；**C# 代码本机未编译/未跑**（家里机验）。本文档供家里机对着测。
> spec：`docs/superpowers/specs/2026-06-24-v1d.1-drag-longpress-safearea-design.md`
> plan：`docs/superpowers/plans/2026-06-24-v1d.1-drag-longpress-safearea.md`

## 0. v1d.1 是什么

drag + longpress + safe-area，检测全在 core，机制/阈值镜像 fgui（已核实源码）：

- **drag**（opt-in）：HTML `draggable="true"` 属性 → core 状态机按下+移动超阈值(mouse 2/touch 10，per-axis OR)→ `DragStart`（取消 click）→ `DragMove` → Up/Canceled `DragEnd`。drag_target=最近 draggable 祖先（leaf 优先）。**只发事件不跟手**（跟手留 v1d.3 transform）。
- **longpress**（universal）：任何节点按住 1.5s（位移≤50px、未 Up）→ `LongPress` 一次。**与 click 独立**（不自动取消，fgui 语义）。
- **safe-area**：`LoomStage` 读 `Screen.safeArea` 把 design viewport shrink-to-fit 进 safe 区（内容避刘海，safe 区外 letterbox）；`LoomInputCollector.ScreenToDesign` 用同一变换的逐项逆（触控↔渲染对齐）。core 零改。

**关键行为变化（v1c.4→v1d.1，验收须知情）**：
- **drag opt-in**：只有 `draggable="true"` 的节点（或其祖先）才发 drag 事件；普通节点按下拖动无 drag（走原 click 逻辑）。
- **drag 取消 click**：draggable 节点拖动起 → 后续 Up **不发 Click**（drag-start 置 click_cancelled）。阈值内（mouse<2/touch<10）拖动 + Up → 仍正常 Click（drag 不破坏 click 容忍）。
- **longpress 不取消 click**：按住 1.5s 发 LongPress 后松手 → **Click 照发**（独立，业务要消费调 `CancelTouch`）。
- **longpress universal**：任何节点（含普通按钮）按住 1.5s 都发 LongPress——无 listener 时 C# 跳过（零成本）。
- **safe-area 默认 on**：`_safeArea=true`。**无刘海屏 `Screen.safeArea==全屏` → 零回归**（与 v1c.4 行为一致）；有刘海屏内容缩进 safe 区。
- **⚠️ M6 行为变化（spec §5.1 本意，非回归 bug）**：设计 aspect ≠ 屏 aspect 时，内容从 v1c 的 **per-axis stretch（拉伸填满）** 改为 **shrink-to-fit + letterbox（等比居中留白）**。v1c 实际上 render 用 uniform sf、input 用 per-axis stretch 本就不一致（latent bug），v1d.1 统一为 uniform sf。若某场景之前看起来"拉伸"现在"正确居中留白"，那是修复生效。

## 1. 前置状态

- 分支：直接在 main（v1c.4 验收后已 merge，v1d.1 也在 main）
- core+ffi 全绿：core 185 + ffi 30 = 215（含 v1d.1 新测：drag 9 + longpress 6 + pkg v4 2 + abi drag/longpress 端到端 2 + ScreenToDesign 含 notched round-trip）
- `.dll` 已重编 commit（v1d.1，1694720B，含 drag/longpress 检测 + version v1d.1）
- final review：Ready（8 跨 task 集成点全清），无 Critical/Important 阻塞
- **未 push**——本步骤先 `git pull`（家里机拉的是已 push 的；若本机还没 push，见 §8）

## 2. 拉代码

```bash
git fetch origin && git checkout main && git pull origin main
```
> **pull 前关 Unity**（坑 10：Unity 开着锁 `.dll`）。pull 后重开。

## 3. 打开 Unity

- Unity Hub 打开 `loomgui_unity`（Unity 6.5）
- `.dll` 已 commit（`Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`，1694720B），无需重编
- **无新 FFI 函数**（drag/longpress 走既有 `borrow_events` 通道）→ csbindgen **不需 regen**；Unity reimport 即可
- 等 Unity reimport 完

## 4. EditMode 测试（Test Runner）— ⚠️ 先填 font_path

`Window → General → Test Runner → EditMode → Run All`，应全绿。

### 4.1 【必做先决】填 BuildStage font_path（LoomEventHandlerTests）

`LoomEventHandlerTests.cs` 的 `BuildStage()` helper 当前 `font_path` 是**占位 null**（本机无 Unity 写不出真路径，v1c.4 既有 TODO）。家里机填真路径（照 `docs/v1c.4-home-verification.md §4.1`）：

```csharp
// BuildStage() 内，fontPathBytes = null 占位改为：
string fontPath = System.IO.Path.Combine(Application.streamingAssetsPath, "DejaVuSans.ttf");
byte[] fontPathBytes = System.Text.Encoding.UTF8.GetBytes(fontPath);
```

> **不填则 LoomEventHandlerTests 全失败**（`Assert.IsTrue(stagePtr != null)` guard 拦）。填完跑。

### 4.2 LoomEventHandlerTests（18 测）

- **16 既有**（v1c.4 回填 9 + 新 2 + 5 基础）：照 v1c.4 验收，clickCount=0 默认不破坏。
- **2 新（T7）**：
  - `DragStart_BubbleRoute_ReachesAncestors`——child(2) DragStart → child+parent(1)+root(0) 都收（bubble）。
  - `LongPress_BubbleRoute_ReachesAncestors`——child(2) LongPress → 祖先链都收。

### 4.3 LoomInputCollectorTests（ScreenToDesign，T8 改+新）

- **3 个 safe==full 测**（`MapsCorrectly`/`TopLeftScreen_IsTopLeftDesign`/`BottomScreen_IsBottomDesign`，rootSize 改 aspect-matched `(200,100)`，sf=1）。
- **1 新（T8 关键）**：`ScreenToDesign_NotchedSafeArea_RoundTrip`——notched safe area（screenSize 400×800，rootSize 200×400，area (40,0,320,800)）下 6 点（4 角+中心+刘海缘）forward→inverse 往返误差 <0.001。**这是 safe-area 触控↔渲染对齐的硬门**（T8 Critical fix 的回归测）。

## 5. PlayMode 验收（interact sample + 临时改）

interact sample 当前**无 draggable 元素 / 无 longpress listener**，需临时加（照 v1c.4 临时改 demo 的模式）。

### 5.1 配置 LoomStage（同 v1c.4）

LoomStage GameObject Inspector：`_usePackage=true`、`_pkgFile=loom_interact.pkg.bin`、`_fontFile`+`_font` 拖字体、挂 `LoomInteractDemo`。**`_safeArea` 默认 true**（v1d.1 新增 Inspector 字段）。

### 5.2 drag 验收

临时在 sample HTML 加一个 `draggable="true"` 元素（如 `<div class="drag" draggable="true">拖我</div>`），并在 `LoomInteractDemo` 注册 listener：

```csharp
uint dragId = _stage.FindNodeById("drag");  // 或推断 build 序 id
h.AddListener(dragId, EventType.DragStart, ctx => Debug.Log($"[drag] Start pos=({ctx.x},{ctx.y})"));
h.AddListener(dragId, EventType.DragMove,  ctx => Debug.Log($"[drag] Move pos=({ctx.x},{ctx.y})"));
h.AddListener(dragId, EventType.DragEnd,   ctx => Debug.Log($"[drag] End pos=({ctx.x},{ctx.y})"));
```

1. **DragStart/Move/End**：按下该元素拖动（mouse >2px / touch >10px）→ Console 见三条日志。
2. **drag 取消 click**：拖动起后松手 → **无 Click 日志**（drag-start 已取消 click）。阈值内（mouse<2px）微动 + 松手 → 正常 Click。
3. **非 draggable 无 drag**：拖动普通按钮 → 无 drag 日志（仅既有 hover/click）。
4. **阈值 per-axis**：mouse 对角 (1,1) 不发 DragStart（per-axis |1|≤2），(3,0) 发。
5. **祖先 draggable**：若 draggable 容器内有非 draggable 子，按子拖动 → DragStart@容器（drag_target=最近 draggable 祖先）。

### 5.3 longpress 验收

任意节点（含普通按钮）加 longpress listener：

```csharp
h.AddListener(someId, EventType.LongPress, ctx => Debug.Log($"[longpress] fired node={ctx.target}"));
```

1. **按住 1.5s**：按下不动 1.5s → Console 见 `[longpress] fired`（**一次**，不重复）。
2. **Move>50 取消**：按下后拖动 >50px → 1.5s 后**不发** LongPress。
3. **<1.5s 松手**：按下 <1.5s 松开 → 无 LongPress。
4. **独立 click**：按住 1.5s 发 LongPress 后松手 → **Click 照发**（独立）。
5. **disabled 不发**：disabled 节点按住 → 无 LongPress。

### 5.4 safe-area 验收（需 Device Simulator / 刘海模拟）

**Device Simulator**：Window → General → Device Simulator，选一个有刘海的设备（如 iPhone 带 notch）。或代码 override `Screen.safeArea`。

1. **内容避刘海**：UI 内容应缩进 safe 区，刘海处 letterbox（空白/不渲染内容）。
2. **触控↔渲染对齐（T8 Critical fix 硬验）**：**tap 一个可见按钮 → 它响应**（这是变换数学的真实验证；符号恒等已验，实机确认触控落点 = 渲染点）。
3. **关 `_safeArea`**：Inspector 关 `_safeArea` → 内容回全屏（v1c 行为，可能进刘海区）。
4. **无刘海屏零回归**：普通 Game 视图（无刘海）→ 内容与 v1c.4 完全一致。

> ⚠️ **M6 留意**：若某场景设计 aspect ≠ 屏 aspect，内容现**居中 letterbox**（v1c 是 stretch 拉伸）。这是 spec §5.1 本意 + 修 v1c latent 不一致，非 bug。
> ⚠️ **I1 留意（v1d.x）**：Device Simulator 同分辨率旋转（safeArea 变但 width/height 不变）→ 内容可能不重流（ConfigureTransforms 只在 Screen.width/height 变时重调）。多数刘海变化伴随分辨率变化，低风险。

## 6. 回归 v1c.1 / v1c.2 / v1c.3 / v1c.4

- 既有 interact 3 按钮（hover/active/disabled）仍工作
- v1c.2 bubble/capture/stop 路由 + v1c.3 capture demo + 多指仍工作
- v1c.4 click 单击/双击 count 1→2→1 / Move>50 取消 / CancelTouch / 触摸 Canceled / StopImmediate 仍工作
- click 行为对 draggable 节点：阈值内正常 Click，超阈值 drag（不发 Click）

## 7. 风险点 + 排查

| 风险 | 排查 |
|---|---|
| **LoomEventHandlerTests 全失败（stagePtr null）** | BuildStage font_path 未填（§4.1） |
| **drag 永不触发** | 元素没 `draggable="true"`（drag opt-in）；或 listener 注册的 nodeId 错（用 `FindNodeById` 而非硬编码 build 序） |
| **drag 后误发 Click** | drag-start 应取消 click——若发 Click 是 bug；确认 draggable 元素、确认 Move 超阈值 |
| **longpress 永不触发** | time_s 没累积——确认 LoomStage 传 `Time.unscaledDeltaTime` 给 tick；或按住 <1.5s；或拖动了 >50px |
| **longpress 重复发** | 应只发一次（longpress_fired guard）；重复是 bug |
| **safe-area 内容进刘海** | offX/offY 算错——确认 T8 fix（0fac306）已 pull；`_safeArea=true` |
| **safe-area tap 落点错（触控偏移）** | ScreenToDesign 与 ComputeRootTransform 不一致——T8 Critical fix 回归；跑 `ScreenToDesign_NotchedSafeArea_RoundTrip` EditMode 测 |
| **设计 aspect≠屏 时内容变了（困惑）** | M6：v1c stretch → v1d.1 letterbox 居中（spec §5.1 本意，非 bug） |
| **`.dll` 锁（坑 10）** | pull 前关 Unity |
| **全不渲 + Console 干净** | md5sum 对比 `.dll`（stale）——本机已 commit v1d.1 .dll（1694720B） |
| **csbindgen 报新 FFI 找不到** | 不会——v1d.1 无新 `#[no_mangle] fn`；若报旧错删 `LoomGUIBindings.cs` 触发 reimport |
| **同分辨率旋转 safeArea 不重流** | I1（v1d.x）——临时改屏尺寸触发 resize 重调 |

## 8. 报坑流程

家里机报坑 → 本机修：
1. core 改：`cargo test -p loomgui_core` 验 → 若改 FFI/pkg：`cargo test -p loomgui_ffi_c` + `cargo build -p loomgui_ffi_c --release` + `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/` + commit .dll
2. C# 改：静态核（本机无 Unity 不编译）
3. `git push origin main`
4. 家里机 `git pull` → 再验

## 9. commit 序列（v1d.1，main 上）

```
0fac306 fix(v1d.1-T8): safe-area 变换数学 — 设计 span 居中 safe 区 + ScreenToDesign 逐项逆（修触控↔渲染错位）
acef54f feat(v1d.1-T8): safe-area 根 letterbox — LoomStage Screen.safeArea shrink-to-fit + 输入映射对齐
603dc7f test(v1d.1-T7): C# drag/longpress BubbleRoute 路由测
7556505 feat(v1d.1-T6): C# EventType +DragStart/Move/End/LongPress + DispatchPending BubbleRoute
069e099 feat(v1d.1-T5): FFI version v1d.1 + abi 测 + .dll 重编
5c996f1 feat(v1d.1-T4): core longpress 检测 — tick 计时 1.5s/50px + EVT_LONG_PRESS
3a112bc feat(v1d.1-T3): core drag 检测 — 状态机 + EVT_DRAG_START/MOVE/END
b8148ef feat(v1d.1-T2): pkg.bin v4 — NodeBlock +draggable flags byte
5d95a66 feat(v1d.1-T1): Node.draggable 字段 + HTML draggable 属性解析 + Scene::build 6-tuple
```
（main 上另有 `1020465` plan / `d322859` spec / `ea38bd5` v1d-plan）

## 10. 验收结论

- EditMode 全绿（**先填 BuildStage font_path**）含 T7 drag/longpress bubble + T8 ScreenToDesign（NotchedSafeArea_RoundTrip 硬门）+ PlayMode（drag opt-in/取消 click/阈值 + longpress 1.5s 一次/Move 取消/独立 click/disabled + safe-area 避刘海/触控↔渲染对齐/关 _safeArea 回归）+ 回归 v1c.1-v1c.4 不破 → v1d.1 通过
- 通过后：session-summary 已做（编码侧经验进 knowledge-reference + design 同步）
- **留意 M6**（letterbox 替换 stretch，spec 本意）+ **I1**（同分辨率旋转 safeArea，v1d.x）
- 报坑：按 §8 流程回本机修

## 11. v1d.2 预告（下一波）

v1d.1 收尾。v1d.2 = **键盘 + 焦点 + Tab + `:focus`**（v1d-plan §3）：keydown/up 事件通道 + focused node/tab 序/focus·blur 事件/`:focus` 伪类重匹配。IME/字符输入按 v1d-plan §2 默认 defer 随 TextInput（v1.x）。待 v1d.1 验过后 brainstorming 定范围。
