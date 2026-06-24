# v1d.2 家里机验收文档

> 本机（无 Unity）已完成 v1d.2 全部实现 + per-task review + final Ready。core+ffi `cargo test` 全绿；**C# 代码本机未编译/未跑**（家里机验）。本文档供家里机对着测。
> spec：`docs/superpowers/specs/2026-06-24-v1d.2-keyboard-focus-design.md`
> plan：`docs/superpowers/plans/2026-06-24-v1d.2-keyboard-focus.md`

## 0. v1d.2 是什么

键盘 keydown/up + 焦点（tabindex opt-in）+ click-to-focus + Tab/Shift+Tab 导航 + `:focus` 伪类 + FocusIn/FocusOut。检测全在 core，C# 只路由（复用既有 `borrow_events` 通道 + 新增 key 事件枚举）。key 事件复用 `EventRecord` 流（EVT 12-15）。

- **keydown/up**：有焦点才发（无焦点丢弃）。键码走 `EventRecord.touch_id` 复用、modifiers 走 `EventRecord.pad[0] @6`。
- **焦点 tabindex opt-in**：只有带 `tabindex` 属性的节点可聚焦 / 进 Tab 链（默认不可聚焦）。
- **click-to-focus**：pointer-down 命中 `tabindex>=0` 节点 → 自动聚焦（发 FocusIn）。
- **Tab 导航**：Tab/Shift+Tab 按 tabindex 序遍历（正整数升序后 0 组），wrap。Tab 被消费不发 keydown。
- **`:focus` 伪类**：聚焦节点的 `:focus` 规则匹配（每帧 rematch）。
- **FocusIn/Out**：走 BubbleRoute（验祖先链，见 §4.2 T7 测）。
- **request_focus**：编程聚焦（强制，含 `tabindex=None/-1`）；disabled 拒。

**关键行为变化（v1d.1→v1d.2，验收须知情）**：
- **tabindex opt-in**：默认所有节点不可聚焦（v1d.1 无焦点概念）。加 `tabindex` 属性才可聚焦/进 Tab 链。
- **click-to-focus**：点 `tabindex>=0` 节点自动聚焦发 FocusIn；点不可聚焦节点不夺焦（保持原焦点）。
- **keydown 需焦点**：无焦点按键 → 丢弃（无 KeyDown 事件）。先 click-to-focus 或 request_focus。
- **Tab 被消费**：按 Tab 触发导航时**不发 keydown**（导航优先消费）。
- **`:focus` 规则生效**：每帧 rematch（聚焦变化 → 样式跟随）。

## 1. 前置状态

- 分支：直接在 main（v1d.1 验收后已 merge，v1d.2 也在 main）
- core+ffi 全绿（含 v1d.2 新测：tab 链 / key 事件 / focus / click-to-focus / `:focus` / pkg v5 / abi key/tab/request_focus）
- `.dll` 已重编 commit（v1d.2，含 `KeyEvent` struct + 3 新 FFI `set_key_input`/`request_focus`/`focused_node` + version v1d.2）
- final review：Ready，无 Critical/Important 阻塞
- **未 push**——本步骤先 `git pull`（家里机拉的是已 push 的；若本机还没 push，见 §8）

## 2. 拉代码

```bash
git fetch origin && git checkout main && git pull origin main
```
> **pull 前关 Unity**（坑 10：Unity 开着锁 `.dll`）。pull 后重开。

## 3. 打开 Unity

- Unity Hub 打开 `loomgui_unity`（Unity 6.5）
- `.dll` 已 commit（`Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`），无需重编
- **新 FFI 函数**（`loomgui_stage_set_key_input` / `loomgui_stage_request_focus` / `loomgui_stage_focused_node`）+ **新 struct**（`KeyEvent`）→ csbindgen reimport 时 **regen `LoomGUIBindings.cs`**
  - 若 csbindgen 报找不到（KeyEvent / 新 fn 缺）：**删 `LoomGUIBindings.cs` 触发 reimport**
  - 确认 `.dll` 是 v1d.2（`md5sum` / 文件大小对比 commit）
- 等 Unity reimport 完

## 4. EditMode 测试（Test Runner）— ⚠️ 先填 font_path

`Window → General → Test Runner → EditMode → Run All`，应全绿。

### 4.1 【必做先决】填 BuildStage font_path（LoomEventHandlerTests）

`LoomEventHandlerTests.cs` 的 `BuildStage()` helper 当前 `font_path` 是**占位 null**（本机无 Unity 写不出真路径，v1c.4 既有 TODO）。家里机填真路径（照 `docs/v1d.1-home-verification.md §4.1`）：

```csharp
// BuildStage() 内，fontPathBytes = null 占位改为：
string fontPath = System.IO.Path.Combine(Application.streamingAssetsPath, "DejaVuSans.ttf");
byte[] fontPathBytes = System.Text.Encoding.UTF8.GetBytes(fontPath);
```

> **不填则 LoomEventHandlerTests 全失败**（`Assert.IsTrue(stagePtr != null)` guard 拦）。填完跑。

### 4.2 LoomEventHandlerTests（20 测）

- **18 既有**（v1c.2-v1c.4 16 + v1d.1 drag/longpress 2）：照 v1c.4/v1d.1 验收，不破坏。
  - > 注：T7 未改动任何既有 `new LoomEvent { ... }` 字面量——`StructLayout.Sequential` 内存布局由字段声明序决定，与 object initializer 是否写 `modifiers` 无关；省略字段默认 0（非 key 事件正确）。
- **2 新（T7）**：
  - `KeyDown_BubbleRoute_ReachesAncestors`——child(2) KeyDown（`touch_id=13`=Return 复用 key_code，`modifiers=0`）→ child + parent(1) + root(0) 都收（bubble）。
  - `FocusIn_BubbleRoute_ReachesAncestors`——child(2) FocusIn → child + parent(1) + root(0) 都收（bubble）。

## 5. PlayMode 验收（interact sample + 临时改）

interact sample 当前**无 tabindex 元素 / 无 key·focus listener**，需临时加（照 v1c.4/v1d.1 临时改 demo 的模式）。

### 5.1 配置 LoomStage（同 v1d.1）

LoomStage GameObject Inspector：`_usePackage=true`、`_pkgFile=loom_interact.pkg.bin`、`_fontFile`+`_font` 拖字体、挂 `LoomInteractDemo`。`_safeArea` 默认 true（v1d.1）。

### 5.2 tabindex + click-to-focus

临时在 sample HTML 加可聚焦元素（如 `<button class="foc" tabindex="0">可聚焦</button>`），并注册 listener：

```csharp
uint focId = _stage.FindNodeById("foc");  // 或推断 build 序 id
h.AddListener(focId, EventType.FocusIn, ctx => Debug.Log($"[focus] in node={ctx.target}"));
h.AddListener(focId, EventType.FocusOut, ctx => Debug.Log($"[focus] out node={ctx.target}"));
```

1. **click-to-focus**：鼠标点 `tabindex` 节点 → Console 见 `[focus] in`（pointer-down 命中可聚焦 → 自动聚焦）。
2. **不可聚焦不夺焦**：点普通按钮（无 tabindex）→ 无 focus 事件（保持原焦点，不夺焦）。
3. **FocusIn bubble**：若 parent 也注册 FocusIn listener → parent 也收（bubble，见 §4.2 测）。

### 5.3 Tab 导航

多个 `tabindex` 节点（如 `tabindex="0"`、`tabindex="1"`、`tabindex="2"`），按 Tab 键 → 焦点按序移动（`:focus` 样式变化可见）。Shift+Tab 反向。序遍完 wrap 回首。

1. **Tab 正向**：焦点按 tabindex 升序移动（正整数 1→2→... 后 tabindex=0 组）。
2. **Shift+Tab 反向**：焦点逆向。
3. **wrap**：末节点按 Tab → 回首节点。

### 5.4 keydown/up

聚焦后（先 click-to-focus 或 Tab 聚到某节点）注册 listener，按 Enter / 字母键：

```csharp
h.AddListener(focId, EventType.KeyDown, ctx => Debug.Log($"[key] down code={ctx.keyCode} mod={ctx.modifiers}"));
h.AddListener(focId, EventType.KeyUp,   ctx => Debug.Log($"[key] up code={ctx.keyCode}"));
```

1. **聚焦后按键**：Console 见 `[key] down code=<KeyCode>`（`ctx.keyCode` = 该键）。
2. **无焦点按键**：丢焦点后按键 → 无 KeyDown（丢弃，验需焦点）。
3. **Tab 不发 keydown**：按 Tab 只触发导航（§5.3），不进 keydown listener（Tab 被导航消费）。
4. **keyCode/modifiers 透传**：按 Return → `ctx.keyCode=13`；按 Shift+X → `ctx.modifiers` 含 Shift 位。

### 5.5 `:focus` 伪类

CSS 加 `:focus` 规则 + 聚焦元素：

```css
.btn:focus { background-color: blue; }
```

1. 聚焦 `.btn`（click-to-focus 或 Tab）→ 背景变蓝（`:focus` 规则匹配，每帧 rematch）。
2. 离焦 → 背景回原色。

### 5.6 request_focus（编程聚焦）

代码调 `Native.loomgui_stage_request_focus(stageHandle, nodeId)`：

```csharp
Native.loomgui_stage_request_focus((StageHandle*)stagePtr, targetId);
// 下一个 tick 后焦点变化（FocusIn 发）
```

1. `request_focus` → 下 tick 焦点 = `targetId`（含 `tabindex=None/-1` 也能强制聚焦）。
2. **disabled 拒**：`request_focus` 一个 disabled 节点 → 聚焦失败（`focused_node` 不变）。

## 6. 回归 v1c.1-v1d.1

- 既有 interact 3 按钮（hover/active/disabled）仍工作
- v1c.2 bubble/capture/stop 路由 + v1c.3 capture demo + 多指仍工作
- v1c.4 click 单击/双击 count / Move>50 取消 / CancelTouch / 触摸 Canceled / StopImmediate 仍工作
- v1d.1 drag opt-in / longpress / safe-area 不破
- **tabindex 无属性的节点行为不变**（不可聚焦，照旧——v1d.1 语义）

## 7. 风险点 + 排查

| 风险 | 排查 |
|---|---|
| **csbindgen 报 `KeyEvent` / 新 FFI 找不到** | 删 `LoomGUIBindings.cs` 触发 reimport；确认 `.dll` 是 v1d.2（`md5sum` / 大小对比 commit） |
| **Tab 不触发导航** | `CollectKeys` 未调（`LoomStage.LateUpdate` 漏 `loomgui_stage_set_key_input`）；或 `ENABLE_INPUT_SYSTEM` 宏下 `Keyboard.current` null |
| **keydown 永不发** | 无焦点（先 click-to-focus 或 `request_focus`）；或键不在 KeyList 白名单；或 Tab 被导航消费（非 bug） |
| **`:focus` 不生效** | rematch 未跑（tick 管线）；或 `.btn:focus` 规则没进 dynamic（`extract_dynamic_rules` 漏 `pseudo_focus`） |
| **pkg.bin v5 被拒** | 旧 `.pkg.bin` 是 v4——用新打包器重打 interact sample（v1d.2 NodeBlock 加了 tabindex flags） |
| **LoomEvent 字段错位** | `StructLayout.Sequential` 内存布局由声明序定，与 object initializer 是否写 `modifiers` 无关——既有字面量省略 `modifiers` 默认 0 正确（T7 未改既有字面量） |
| **`.dll` 锁（坑 10）** | pull 前关 Unity |
| **全不渲 + Console 干净** | `md5sum` 对比 `.dll`（stale）——本机已 commit v1d.2 .dll（含 KeyEvent + 3 FFI + version v1d.2） |
| **EditMode 全失败（stagePtr null）** | BuildStage `font_path` 未填（§4.1） |

## 8. 报坑流程

家里机报坑 → 本机修：

1. core 改：`cargo test -p loomgui_core` 验 → 若改 FFI/pkg/struct：`cargo test -p loomgui_ffi_c` + `cargo build -p loomgui_ffi_c --release` + `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/` + commit `.dll`
2. C# 改：静态核（本机无 Unity 不编译）
3. `git push origin main`
4. 家里机 `git pull` → 再验

## 9. commit 序列（v1d.2，main 上）

```
（final review 后填——含 T1 node tabindex 字段 + T2 HTML tabindex 解析 + T3 core 焦点/tab 链/key 事件 + T4 pkg v5 + T5 FFI set_key_input/request_focus/focused_node + KeyEvent + T6 C# EventType KeyDown/KeyUp/FocusIn/FocusOut + EventContext.keyCode/modifiers + T7 C# EditMode 测 + 本验收文档）
```

## 10. 验收结论

- EditMode 全绿（**先填 BuildStage font_path**）含 T7 KeyDown/FocusIn BubbleRoute（2 新，共 20 测）+ PlayMode（tabindex opt-in / click-to-focus / Tab 导航 + wrap / keydown 需焦点 + keyCode·modifiers 透传 / Tab 被消费 / `:focus` rematch / request_focus 编程聚焦 + disabled 拒）+ 回归 v1c.1-v1d.1 不破 → v1d.2 通过
- 通过后：session-summary（编码侧经验进 knowledge-reference + design 同步）
- 报坑：按 §8 流程回本机修
