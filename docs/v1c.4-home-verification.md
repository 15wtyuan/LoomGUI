# v1c.4 家里机验收文档

> 本机（无 Unity）已完成 v1c.4 全部实现 + final review Ready。core+ffi `cargo test` **core 162 + ffi 28 全绿**；**C# 代码本机未编译/未跑**（家里机验）。本文档供晚上家里机对着测。
> spec：`docs/superpowers/specs/2026-06-24-v1c.4-click-design.md`
> plan：`docs/superpowers/plans/2026-06-24-v1c.4-click.md`

## 0. v1c.4 是什么

click 全对齐 fgui + v1c 收尾：`click_test`（Click 目标=按下叶非当前 hit，per-axis 阈值 mouse10/touch50）+ 双击 clickCount 1→2→1（350ms+位置+同键）+ Move>50 取消 + Canceled（不发 Click+reset）+ CancelClick API + stopImmediatePropagation（纯 C#）+ Stationary hover 跟随（静止光标下元素动→hover 刷新，fgui 改进）。

**关键行为变化（v1c.3→v1c.4，验收须知情）**：
- **Click 目标从「当前命中」改为「按下叶」**：按下按钮、漂移到相邻元素（位移<阈值）再松手 → 仍 click 按下的按钮（照 fgui「点按缩放」语义）。v1c.3 是「down_node==hit」严格同节点。**无现存业务破坏**（interact sample 不测漂移 click），但语义更宽松。
- **双击**：同位置同键 350ms 内两次 click → `EventContext.clickCount==2` / `isDoubleClick==true`（fgui 无 onDoubleClick 事件，消费侧读 clickCount）。
- **Move>50 取消 click**：按下后拖动 >50px 再松手 → 不触发 click（拖拽防误触）。
- **Canceled（触摸）**：系统取消触摸（如电话打断）→ Up 仍发、不发 Click、clickCount 重置（偏离 fgui quirk，spec §0.6）。
- **Stationary hover**：光标不动、元素动画移入其下 → :hover 刷新（v1c.3 不刷新）。

## 1. 前置状态

- 分支：直接在 main（v1c.3 验收后已 merge，本 v1c.4 也在 main）
- core+ffi 全绿：core 162 + ffi 28（含 3 新 abi_tests：version v1c.4 / sizeof 20+16+Canceled==3 / cancel_click 两帧无 Click）
- `.dll` 已重编 commit（v1c.4，1694208B，含 cancel_click FFI + tick dt + version v1c.4）
- final review：Ready（6 跨 task 集成点全清），无 Critical/Important
- 已 push origin/main（HEAD `db33b24`）

## 2. 拉代码

```bash
git fetch origin && git checkout main && git pull origin main
```
> **pull 前关 Unity**（坑 10：Unity 开着锁 `.dll`）。pull 后重开。

## 3. 打开 Unity

- Unity Hub 打开 `loomgui_unity`（Unity 6.5）
- `.dll` 已 commit（`Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`），无需重编
- **`LoomGUIBindings.cs` 会自动再生**（csbindgen build.rs），含新 `loomgui_stage_cancel_click`。若没再生（旧文件残留），删 `Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs` 触发 Unity reimport
- 等 Unity reimport 完

## 4. EditMode 测试（Test Runner）— ⚠️ 先填 font_path

`Window → General → Test Runner → EditMode → Run All`，应 **16 测全绿**（5 既有 + 9 回填 + 2 新）。

### 4.1 【必做先决】填 BuildStage font_path

`LoomEventHandlerTests.cs` 的 `BuildStage()` helper 当前 font_path 是**占位 null**（本机无 Unity 写不出真路径）。家里机填真路径：

```csharp
// BuildStage() 内，把 fontPathBytes = null 占位改为：
string fontPath = System.IO.Path.Combine(Application.streamingAssetsPath, "DejaVuSans.ttf");
// 或用 LoomStage 既有字体资产路径（看 LoomStage._font 怎么加载的，照搬）
byte[] fontPathBytes = System.Text.Encoding.UTF8.GetBytes(fontPath);
```

> **不填则 9 个 BuildStage 测全失败**（`Assert.IsTrue(stagePtr != null)` guard 会拦——这正是它存在的意义，坑 40）。填完跑 16 测。

### 4.2 既有 5 测（直接绿）
`DispatchPending_Routes / NoListener_NoOp / MultipleEvents / EventContext_Pool / EventBridge_AddMultipleCallbacks`——clickCount=0 默认不破坏。

### 4.3 v1c.4 回填 9 测 + 2 新测
- **9 回填**（原 Assert.Ignore 骨架，已填真断言）：BubbleRoute 祖先链 / StopPropagation 止冒泡 / RollOver 直派 / AddCapture 先于 Target / DelegateRemove / CaptureTouch / Move 直派 / MultiTouch touchId / RemoveTouchMonitor。
- **2 新**：`StopImmediate_StopsSiblingListenersOnSameNode`（同节点两 listener，第一个 StopImmediate→第二个不触发）、`DoubleClick_ClickCount_ReachesEventContext`（clickCount=2→ctx.clickCount=2 + isDoubleClick）。

## 5. PlayMode 验收（interact sample）

### 5.1 配置 LoomStage（同 v1c.3）
LoomStage GameObject Inspector：`_usePackage=true`、`_pkgFile=loom_interact.pkg.bin`、`_fontFile`+`_font` 拖字体、挂 `LoomInteractDemo`。

### 5.2 鼠标回归（v1c.1-v1c.3 不破）
hover/click/active/disabled 四项照旧（详见 v1c.3 验收文档 §5.2）。

### 5.3 v1c.4 新验：双击 + Move 取消（鼠标）

interact sample 的按钮 onClick listener 可加日志读 `ctx.clickCount`/`ctx.isDoubleClick`（若 demo 没加，临时在 `LoomInteractDemo` click 回调加 `Debug.Log($"click count={ctx.clickCount}")`）：

1. **双击 → count=2**：同位置快速双击按钮（350ms 内）→ Console 见 `click count=2`；慢击（>350ms）→ `count=1`。
2. **三击循环 1→2→1**：快速三击 → count 序列 1, 2, 1（非 1,2,3）。
3. **Move>50 取消**：按住按钮，拖动 >50px 再松回原位 → **无 click**（Console 无 click 日志，或 count 不增）。拖 <50px 松手 → 正常 click。

### 5.4 v1c.4 新验：CancelTouch API（鼠标拖拽场景）

`LoomInteractDemo` 若加了拖拽 demo 调 `ctx.CaptureTouch()` + `handler.CancelTouch(touchId)`（拖拽开始取消待 click，防拖拽完误触发 click）：按住拖动后松手 → **无 click**（CancelTouch 取消了）。若 demo 没加，业务侧手动验：任意 Down 后调 `_eventHandler.CancelTouch(-1)`，再 Up → 无 click。

### 5.5 v1c.4 新验：Stationary hover 跟随

需一个会动的元素（动画/布局变化移到静止光标下）：
1. 鼠标悬停空白处不动。
2. 触发一个元素动画/布局变化，使其移入光标下方。
3. → 该元素应 `:hover` 变色（v1c.3 不变色，v1c.4 刷新）。

> 若 sample 无动画元素，临时验：`Update` 里改某元素 CSS 位置移到光标下，看 :hover 是否跟随。core 单测 `stationary_cursor_hover_follows_moved_element` 已覆盖语义。

### 5.6 v1c.4 新验：触摸 Canceled（需触屏/模拟）

**触屏设备**：系统级取消触摸（如多指冲突/电话打断，InputSystem TouchPhase.Canceled）。
**模拟**：Player Settings → Input System → Touch Simulation，或代码注入 `TouchPhase.Canceled`。

验：触摸 Down → 触发 Canceled → Console 见 Up 日志、**无 click**、后续同位置快速点击 count=1（Canceled reset 了双击窗口，spec §0.6 偏离）。

> 触摸 Canceled 难稳定复现，**无条件验记为待补非阻塞**（core 单测 `canceled_emits_up_skips_click` + `canceled_resets_click_count` 已覆盖）。

### 5.7 v1c.4 新验：stopImmediatePropagation

业务侧两 listener 同节点同事件，第一个调 `ctx.StopImmediatePropagation()` → 第二个**不触发**（StopPropagation 只止冒泡，第二个仍跑；StopImmediate 同节点也止）。core 不涉，纯 C#，EditMode `StopImmediate_StopsSiblingListenersOnSameNode` 测已覆盖，PlayMode 可选验。

## 6. 回归 v1c.1 / v1c.2 / v1c.3

- 既有 interact 3 按钮（hover/active/disabled）仍工作
- v1c.2 bubble/capture/stop 路由 + v1c.3 capture demo + 多指（若有触屏）仍工作
- click 单击行为：同节点位移<阈值 → click（v1c.3 等价；v1c.4 仅阈值改 per-axis，对角更宽松，肉眼难辨）

## 7. 风险点 + 排查

| 风险 | 排查 |
|---|---|
| **9 BuildStage 测失败（stagePtr null）** | BuildStage font_path 未填（§4.1）。填真字体路径 |
| EditMode 编译错 `loomgui_stage_cancel_click` 找不到 | `LoomGUIBindings.cs` 未再生——删文件触发 reimport（csbindgen 加新 FFI） |
| `LoomEvent` sizeof != 20 | `Marshal.SizeOf<LoomEvent>()` 应 20（clickCount @offset5 + 2 pad + touch_id@8）。LoomEventHandler.cs `DispatchOne` 用 `Marshal.SizeOf<LoomEvent>()` 自动跟；手搓 fixture 注意识 |
| 双击 count 永远 1 | time_s 没累积——确认 LoomStage 传 `Time.unscaledDeltaTime`（非 deltaTime）给 tick；或两次 click 间隔 >350ms |
| click 漂移不触发（困惑） | v1c.4 Click 目标=按下叶，**位移<阈值才 click**（mouse 10/touch 50）。拖太远（>阈值）不 click 是设计 |
| 鼠标拖拽完误触发 click | 业务侧拖拽开始没调 `CancelTouch`——加 `handler.CancelTouch(touchId)` |
| `.dll` 锁（坑 10） | pull 前关 Unity |
| 全不渲 + Console 干净 | md5sum 对比 `.dll`（stale）——本机已 commit v1c.4 .dll |
| Canceled 测不出 | 触摸 Canceled 难复现，core 单测已覆盖，记为待补 |

## 8. 报坑流程

家里机报坑 → 本机修：
1. core 改：`cargo test -p loomgui_core` 验 → `cargo build -p loomgui_ffi_c --release` + `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/` + commit .dll
2. C# 改：静态核（本机无 Unity 不编译）
3. `git push origin main`
4. 家里机 `git pull` → 再验

## 9. commit 序列（v1c.4，main 上）

```
db33b24 fix(v1c.4-T8): BuildStage null-guard 用 IntPtr.Zero 比较（Assert.IsNotNull 对指针装箱无效）
6e88ffe test(v1c.4-T8): C# 路由测回填 — BuildStage helper + 9 骨架填实 + StopImmediate/双击测
c3c538e feat(v1c.4-T7): C# stopImmediatePropagation
519a44c feat(v1c.4-T6): C# clickCount 镜像 + Canceled 输入 + dt/CancelClick 接线
185bbb6 feat(v1c.4-T5): FFI advance_time(dt) + cancel_click + version v1c.4 + .dll 重编
09a886e feat(v1c.4-T4): core stationary hover 跟随（fgui 改进）
86c7f4b feat(v1c.4-T3): core Canceled(PointerKind=3) + cancel_click API
17db6a3 feat(v1c.4-T2): core 双击 clickCount(1→2→1) + Move>50 取消 + time_s
afb0f40 feat(v1c.4-T1): core click_test + per-axis 阈值(mouse10/touch50) + down_targets
```
（main 上另有 `a709020` plan / `73ac456` spec）

## 10. 验收结论

- EditMode 16 测全绿（**先填 BuildStage font_path**）+ PlayMode（双击 count 1→2→1 / Move>50 取消 / CancelTouch / stationary hover / 触摸 Canceled 若有条件）+ 回归 v1c.1-v1c.3 不破 → v1c.4 通过
- 通过后：session-summary 已做（编码侧经验进 knowledge-reference §2.18/坑 39/40 + design §10.3）
- 报坑：按 §8 流程回本机修

## 11. v1d 预告（下一波）

v1c 已收尾。v1d 候选（spec §11 defer）：transform world_to_local 命中（旋转/缩放/嵌套 transform 下精确命中）、onKeyDown/Up/onMouseWheel 路由（键盘/滚轮输入）、broadcast 子树广播。待 v1c.4 验过后 brainstorming 定范围。
