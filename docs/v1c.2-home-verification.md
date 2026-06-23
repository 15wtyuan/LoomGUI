# v1c.2 家里机验收文档

> 本机（无 Unity）已完成 v1c.2 全部实现 + final review Ready。core `cargo test --workspace` 164 测全绿；**C# 代码本机未编译/未跑**（家里机验）。本文档供晚上家里机对着测。
> spec：`docs/superpowers/specs/2026-06-23-v1c.2-event-bubbling-design.md`
> plan：`docs/superpowers/plans/2026-06-23-v1c.2-event-bubbling.md`

## 0. 前置状态

- 分支：`v1c.2`（领先 main 10 commit：spec/design/plan 3 + T1-T6 6 实现 + 注释 fix 1）
- core 全绿：134 core + 3 snapshot + 21 ffi_c + 3 pkg + 3 pack = 164 测
- `.dll` 已重编 commit（v1c.2，含 `node_parent` + `hover_diff`），md5 `f27e0e82...`
- final review：spec ✅ 9/9 + 跨 task 一致 ✅ + 必修 Minor 已修（`ad453bb`）
- 主设计已修订（`00-main-design.md` §10.2/§6.3/§15：路由降级业务侧）

## 1. 拉代码

push 后家里机：
```bash
git fetch origin
git checkout v1c.2
git pull origin v1c.2
```
> 若已 merge main 则 `git checkout main && git pull`。**pull 前关 Unity**（坑 10：Unity 开着锁 `.dll`）。

## 2. 打开 Unity

- Unity Hub 打开 `loomgui_unity`（Unity 6.5）
- `.dll` 已 commit（`Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`），无需重编
- 等 Unity reimport 完（首次拉代码会 reimport .cs/.meta）

## 3. EditMode 测试（Test Runner）

`Window → General → Test Runner → EditMode → Run All`

### 3.1 既有测（直接跑，应绿）

- `LoomEventHandlerTests`：`EventContext_Pool_ReusesInstances`（池复用 AreSame）、`EventBridge_AddMultipleCallbacks_AllInvoked`（多播 + Remove）
- 既有 v1c.1 测（DispatchPending_Routes / NoListener_NoOp / MultipleEvents 等，无 handle 依赖）

### 3.2 4 个路由测骨架（Assert.Ignore，需补 handle 装载）

`LoomEventHandlerTests.cs` 里 4 个路由测当前 `Assert.Ignore("家里机：需 Stage handle + scene...")`。家里机补 `BuildStage` helper + 去 Ignore 后跑：

**补 helper（示例，放 LoomEventHandlerTests.cs）**：
```csharp
    // 手搓 root(0)>parent(1)>child(2) 三 Container，load_package，返 handler（已 SetHandle）
    static LoomEventHandler BuildHandlerWithChain(out LoomStage stage) {
        // 用 LoomStage（场景里挂的，或 new GameObject 加 LoomStage）
        // _usePackage=true + 手搓 root>parent>child 包 load_package
        // 或：直接 new StageHandle + 手搓 scene（参考 ffi_c abi_tests 的 Scene::build 风格，但 C# 侧需走 load_package）
        // ... 家里机按既有 LoomStage 测的 setup 风格补
        stage = ...;
        return stage.EventHandler;  // LoomStage 暴露 _eventHandler
    }
```
> 家里机按既有 `LoomStage`/`LoomInputCollector` 测的 setup 风格补（手搓 root>parent>child 的 `.pkg.bin` 或 inline HTML，load 后 `SetHandle` 自动调）。

**去 Ignore + 跑验**：
- `BubbleRoute_ReachesAllAncestors`：6 hits（3 capture root→child + 3 bubble child→root）；phase 序 Capture(root)→...→Target(child)→Bubble(parent)→Bubble(root)
- `StopPropagation_BreaksBubbleButNotCapture`：capture 跑完；child bubble StopPropagation → parent/root bubble 不收
- `RollOver_DirectDispatch_NoBubble`：RollOver(child) 只 child 收，parent/root 不收（直派不沿链）
- `AddCapture_FiresInCapturePhaseBeforeTarget`：root AddCapture → capture 阶段先于 child Target
- `DelegateRemove_StopsReceiving`：RemoveListener 后 cb 不再调

骨架注释已写断言意图，照注释填。

## 4. PlayMode 验收（interact sample）

### 4.1 配置 LoomStage

场景里 LoomStage GameObject，Inspector：
- `_usePackage = true`
- `_pkgFile = loom_interact.pkg.bin`（StreamingAssets，T5 重打 3817 bytes）
- `_fontFile = DejaVuSans.ttf`（或 CJK 字体若 sample 含中文）
- `_font` 拖字体资产（EnsureFont 兜底）
- 场景挂 `LoomInteractDemo`（T5 新建 driver，注册 bubble/stop/capture listener）

### 4.2 进 PlayMode，Console 看 `[interact]` 日志

验 spec §8.3 五条：

1. **hover button → 祖先链 diff 正确**：鼠标移到内层 btn → Console 见 `btn RollOver` + `outer RollOver`（祖先链进子，父新进 hover）；**外层不 RollOut**（v1c.1 bug 已修）。鼠标移出 btn 到 outer 区 → btn RollOut，outer 保持。
2. **click button → bubble 到外层**（临时注释 `LoomInteractDemo` 里 btn 的 `ctx.StopPropagation()`）：click btn → Console 见 `btn click` + `outer bubble 收到 click`。
3. **btn StopPropagation → 外层不收**（恢复 StopPropagation）：click btn → Console 只见 `btn click` + `outer capture`（capture 跑），无 `outer bubble`。
4. **外层 AddCapture → capture 先于 btn**：Console 日志序 `outer capture` 在 `btn click`（target）前。
5. **回归伪类**：hover btn → `.btn:hover` 变色；按下 → `.btn:active` 变色；disabled 按钮（`.btn.disabled`）半透 + 点击无响应。

### 4.3 nodeId 风险（首查项）

`LoomInteractDemo.cs` 的 `OuterId=4, BtnId=5` 是推断 build 序。**若 Console 无 `[interact]` 日志**：
- 临时在 listener 加 `UnityEngine.Debug.Log($"hit target={ctx.target}");`
- 对比实际 nodeId，调 `OuterId`/`BtnId` 常量
- scene 结构：root(0) > [btn1(1), btn2(2), btn3.disabled(3), outer(4) > btn(5)]（推断序，disabled 不影响序）

## 5. 回归 v1c.1

- 既有 interact 3 按钮（hover 变色 / active 变色 / disabled 半透）仍工作
- `stress500`（若家里机跑）不崩
- v1c.1 伪类 `:hover/:active/:disabled` 正常（hovered 状态祖先链未动，rematch 路径不变）

## 6. 风险点 + 排查

| 风险 | 排查 |
|---|---|
| nodeId 推断偏移（Console 无日志） | Debug.Log ctx.target 对比，调常量（§4.3） |
| csbindgen 绑定名 | `LoomGUIBindings.cs:131` 应是 `loomgui_node_parent`（无前缀）；LoomEventHandler.cs:168 调用名匹配。若 Unity 重跑 build.rs 覆盖，内容应一致 |
| `.dll` 锁（坑 10） | pull 前关 Unity；pull 后重开 |
| `Marshal.PtrToStructure` | 桌面 Mono OK；IL2CPP 移动端对齐坑（spec §14.3），v1c.2 桌面优先非阻塞 |
| 全不渲 + Console 干净 | md5sum 对比 `.dll`（坑 10 stale .dll）——本机已 commit v1c.2 .dll，家里机 pull 即新 |

## 7. 报坑流程

家里机报坑 → 本机修：
1. core 改：`cargo test -p loomgui_core` 验 → `cargo build -p loomgui_ffi_c --release` + `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/` + commit .dll
2. C# 改：静态核（本机无 Unity 不编译）
3. `git push origin v1c.2`
4. 家里机 `git pull` → 再验

## 8. commit 序列（v1c.2 branch）

```
ad453bb docs(v1c.2): hover_chain_idempotent 注释同步（final review 必修）
a4c57a0 chore(v1c.2-T6): release .dll v1c.2
2934700 feat(v1c.2-T5): LoomStage SetHandle + interact sample
7ef0a4e fix(v1c.2-T4): BubbleRoute 测骨架补 AddCapture
200cd74 feat(v1c.2-T4): C# bubble/capture 路由
c744858 fix(v1c.2-T3): EventContext 池测 + EventBridge public
2628886 feat(v1c.2-T3): C# EventContext + EventBridge
fc93851 feat(v1c.2-T2): FFI node_parent + version v1c.2
bbbdd2f fix(v1c.2-T1): 恢复 Move 每次 emit
d7e04eb feat(v1c.2-T1): core hover_diff 祖先链 diff
```
（main 上另有 `ffc56f3` plan / `bad774b` design 修订 / `1c03bcf` spec）

## 9. 验收结论

- EditMode 全绿（含 4 路由测补 handle）+ PlayMode 5 条全过 + 回归 v1c.1 不破 → v1c.2 通过
- 通过后：merge `v1c.2` → `main`，`git push origin main`
- 报坑：按 §7 流程回本机修
