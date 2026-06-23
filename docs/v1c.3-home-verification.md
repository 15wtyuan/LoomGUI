# v1c.3 家里机验收文档

> 本机（无 Unity）已完成 v1c.3 全部实现 + final review Ready。core+ffi `cargo test --workspace` **180 测全绿**；**C# 代码本机未编译/未跑**（家里机验）。本文档供晚上家里机对着测。
> spec：`docs/superpowers/specs/2026-06-23-v1c.3-multi-touch-design.md`
> plan：`docs/superpowers/plans/2026-06-23-v1c.3-multi-touch.md`

## 0. v1c.3 是什么

多触摸：单指针 `PointerState` → 固定 5 槽（slot0=鼠标 touch_id=-1，slot1-4=触摸）+ CaptureTouch/touch monitor + Move 对齐 fgui（无 monitor 不产）+ active/hovered 全局 union recompute（多指合并）+ EventRecord 加 touch_id。click 增强（双击/downTargets 兜底）defer v1c.4。

**关键行为变化（v1c.2→v1c.3，验收须知情）**：
- **Move 无 monitor 不产事件**（鼠标/触摸统一）。v1c.2 鼠标 Move 在命中元素上产事件沿链 bubble；v1c.3 不产（除非 capture）。**无现存业务破坏**（interact sample 无 Move listener），但若你写过依赖鼠标 Move 的代码会静默失效。
- **active/hovered 多指合并**：任一指命中元素或祖先 → `:hover`；任一指按下命中链 → `:active`。
- **RollOver/Out per-touch + hovered 全局 union 双语义**：A 指移出 X 但 B 指还在 X → X 收 RollOut 但 `:hover` 仍 true（描述不同事实，正常）。

## 1. 前置状态

- 分支：直接在 main（v1c.2 验收后已 merge，本 v1c.3 也在 main）
- core+ffi 全绿：147 core + 3 snapshot + 24 ffi_c + 3 pkg + 3 pack = 180 测
- `.dll` 已重编 commit（v1c.3，md5 `b96e2efd`，含 add/remove_touch_monitor + EventRecord 20B + PointerKind repr(u8) + version v1c.3）
- final review：Ready（5 跨 task 集成点全清），无 Critical/Important
- `LoomGUIPointerEvent.cs`（手补 C# 镜像）已升 v1c.3 layout（kind/button/pad0/pad1/touch_id/x/y = 16B）

## 2. 拉代码

```bash
git fetch origin
git checkout main
git pull origin main
```
> **pull 前关 Unity**（坑 10：Unity 开着锁 `.dll`）。pull 后重开。

## 3. 打开 Unity

- Unity Hub 打开 `loomgui_unity`（Unity 6.5）
- `.dll` 已 commit（`Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`），无需重编
- **`LoomGUIBindings.cs` 会自动再生**（csbindgen build.rs 跑），含 `loomgui_stage_add_touch_monitor` / `remove_touch_monitor`。若没再生（旧文件残留），删 `Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs` 触发 Unity reimport
- 等 Unity reimport 完

## 4. EditMode 测试（Test Runner）

`Window → General → Test Runner → EditMode → Run All`

### 4.1 既有测（直接跑，应绿）

- `LoomEventHandlerTests`：v1c.2 既有（EventContext 池、EventBridge 多播、DispatchPending_Routes / NoListener_NoOp / MultipleEvents 等）
- **注意**：v1c.2 的 4 个路由测骨架若上次家里机已补 handle 去 Ignore，仍应绿（v1c.3 没动 BubbleRoute 的 capture/bubble 路由本质，只加了 cap/bub 各消费 _touchCapture）

### 4.2 v1c.3 capture 测骨架（Assert.Ignore，需补 handle）

`LoomEventHandlerTests.cs` 里 v1c.3 新增的 capture 测骨架（`CaptureTouch_SetsFlag_...` / `Move_NoMonitor_NoDispatch` / `MultiTouch_DistinctTouchId` 等）当前 `Assert.Ignore`。家里机补 `BuildHandlerWithChain` helper（v1c.2 已有此模式）+ 去 Ignore 后跑：

- **CaptureTouch_AddsMonitor_MoveDispatched**：Down + CaptureTouch → 后续 Move → monitor 收
- **CaptureTouch_CapAndBub_BothAdded**：cap+bub 阶段各调 CaptureTouch → 加 2 monitor（照 fgui 两消费模型）
- **Move_NoMonitor_NoDispatch**：无 capture 的 Move EventRecord → 无 listener 调（**v1c.3 新行为**）
- **MultiTouch_DistinctTouchId**：两 touch_id 各自 dispatch 不串
- **Up_NoDouble_WhenHitIsMonitor**：Up 去重（monitor==hit 只一次）

骨架注释已写断言意图，照注释填。

## 5. PlayMode 验收（interact sample）

### 5.1 配置 LoomStage（同 v1c.2）

场景里 LoomStage GameObject，Inspector：
- `_usePackage = true`
- `_pkgFile = loom_interact.pkg.bin`（StreamingAssets）
- `_fontFile` + `_font` 拖字体资产
- 场景挂 `LoomInteractDemo`（v1c.3 加了 capture demo listener）

### 5.2 鼠标回归（v1c.2 不破）

1. **hover button → 祖先链 :hover 变色**：鼠标移到内层 btn → `.btn:hover` 变色（v1c.1/v1c.2 行为不变）
2. **click button → bubble**：click btn → Console 见 `btn click`（临时注释 StopPropagation 验 outer 也收）
3. **active**：按下 btn → `.btn:active` 变色；松开归零
4. **disabled**：disabled 按钮半透 + 点击无响应

### 5.3 v1c.3 新验：capture demo（鼠标）

`LoomInteractDemo` 的 capture demo：Down 在 btn → `ctx.CaptureTouch()` + Move listener。

1. **鼠标 Down 在 btn → capture**：Console 见 `[interact] btn Down touch=-1 → capture`（鼠标 touch_id=-1）
2. **拖出 btn 仍收 Move**：按住鼠标左键拖出 btn 到 outer 区 → Console 持续见 `[interact] btn Move (capture 中) touch=-1 pos=(...)`（capture 后手指/鼠标移出仍收 Move——fgui 语义）
3. **松开后不再收 Move**：松开左键 → Console 见 btn Up，后续移动不再有 `[interact] btn Move`（Up 清 monitor）

> **若 Down 不 capture 后拖出无 Move 日志**：说明核心 Move 无 monitor 不产生效（正确）；capture 后才有（正确）。

### 5.4 v1c.3 新验：多指（需触屏或 Input System 模拟）

**触屏设备**：直接两指操作。
**无触屏桌面**：Player Settings → Input System Package → Behavior → `Simulate Touch Input` = true（用鼠标模拟）。或 Input System 的 Touchscreen Simulation。

1. **两指 Down 不同按钮**：两指分别按 btn1/btn2 → Console 见两条 `[interact] ... Down touch=<fingerId>`，fingerId 不同
2. **is_pointer_on_ui 任一指**：任一指在 UI 上 → `stage.IsPointerOnUI()` = true
3. **多指 active 合并**：两指按两按钮 → 两按钮都 `:active`（若 sample 有多按钮布局）；松一指 → 剩余仍 active
4. **多指 hover 合并**：两指悬停两元素 → 两元素都 `:hover`

> 多指验依赖触屏/模拟，**若无条件验，记为家里机待补，非阻塞**（core 已有 9 个多指单测覆盖语义）。

### 5.5 空帧 hover 保持

- 鼠标悬停 btn 后不动 → 无 Move 事件 → hover 保持（`.btn:hover` 不闪）
- Console 无 spurious RollOut

### 5.6 nodeId 风险（同 v1c.2，首查项）

`LoomInteractDemo.cs` 的 `OuterId=4, BtnId=5` 仍是推断 build 序。**若 Console 无 `[interact]` 日志**：临时加 `Debug.Log($"hit target={ctx.target}")` 对比实际 nodeId，调常量。

## 6. 回归 v1c.1 / v1c.2

- 既有 interact 3 按钮（hover 变色 / active 变色 / disabled 半透）仍工作
- v1c.2 的 bubble/capture/stop 路由（4 路由测若家里机已补）仍绿
- `:hover/:active/:disabled` 伪类正常（hovered/active 改全局 union recompute，单指语义等价）

## 7. 风险点 + 排查

| 风险 | 排查 |
|---|---|
| nodeId 推断偏移（无日志） | Debug.Log ctx.target 对比，调 OuterId/BtnId（§5.6） |
| csbindgen 绑定名 | `LoomGUIBindings.cs` 应含 `loomgui_stage_add_touch_monitor`/`remove_touch_monitor`（snake_case，无前缀）。LoomEventHandler.cs / LoomInputCollector.cs 调用名匹配。未再生→删文件触发 reimport |
| `LoomGUIPointerEvent.cs` layout | 应是 v1c.3（kind/button/pad0/pad1/touch_id/x/y = 16B）。若仍 v1c.2（kind/x/y/button）→ set_input 写错布局，触摸/鼠标坐标乱 |
| `PointerKind : byte` | Rust PointerKind 现是 `repr(u8)`（1B）。C# 侧 PointerEvent.kind 是 byte（LoomGUIPointerEvent.cs 已 byte）。若某处声明 int 会 mis-slice |
| `.dll` 锁（坑 10） | pull 前关 Unity |
| `Marshal.SizeOf<LoomEvent>` | 应 20（v1c.2 是 16）。LoomEventHandler.cs 用 `Marshal.SizeOf<LoomEvent>()` 自动跟 |
| 全不渲 + Console 干净 | md5sum 对比 `.dll`（坑 10 stale .dll）——本机已 commit v1c.3 .dll md5 b96e2efd |
| 鼠标 Move 不产事件（困惑） | **这是 v1c.3 设计**（对齐 fgui，无 monitor 不产）。不是 bug。要跟鼠标 Move 须 capture |
| 多指验无触屏 | Player Settings 开 Touch Simulation，或记为待补（core 单测已覆盖） |

## 8. 报坑流程

家里机报坑 → 本机修：
1. core 改：`cargo test -p loomgui_core` 验 → `cargo build -p loomgui_ffi_c --release` + `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/` + commit .dll
2. C# 改：静态核（本机无 Unity 不编译）
3. `git push origin main`
4. 家里机 `git pull` → 再验

## 9. commit 序列（v1c.3，main 上）

```
2beb6e7 docs(v1c.3): 清 is_pointer_on_ui 过时 cur_hit 注释（多指化后）
61e8eb9 chore(v1c.3-T6): release .dll v1c.3
9f9be6d feat(v1c.3-T5): LoomInputCollector 多指采集 + interact capture demo
1906cbc feat(v1c.3-T4): C# LoomEventHandler touch_id + CaptureTouch + Move DirectDispatch
9956c3d docs(v1c.3-T3): 清 sizeof 测过时注释（repr(u8) 后 PointerEvent 16B）
5dd4aa9 fix(v1c.3-T3): PointerKind repr(u8) — PointerEvent 16B（非 20B）
2e1d1bc feat(v1c.3-T3): FFI add/remove_touch_monitor + version v1c.3 + abi_tests
0bd6eac feat(v1c.3-T2): core touch_monitors 派发 + add/remove_touch_monitor
38790cb feat(v1c.3-T1): core 多槽状态机 + EventRecord/PointerEvent 加 touch_id
```
（main 上另有 `5cd4230` plan / `35818da` spec）

## 10. 验收结论

- EditMode 全绿（含 v1c.3 capture 测补 handle）+ PlayMode 鼠标回归 + capture demo + 多指（若有触屏）+ 回归 v1c.1/v1c.2 不破 → v1c.3 通过
- 通过后：`git push origin main`（若未 push）+ session-summary 进知识库
- 报坑：按 §8 流程回本机修

## 11. v1c.4 预告（click 增强，正交）

v1c.3 click 沿用 v1c.2 简化（down_node==hit && <10px）。v1c.4 做（待 v1c.3 验过后启动）：
- 双击（350ms 窗口 + 位置 + 同键）
- downTargets 链兜底（down 目标被移除沿祖先找）
- 缩放容忍
- Move 中超阈值取消 click
- Canceled 跳过 click
- Stationary hover 跟随（元素动后 hover 刷新）
