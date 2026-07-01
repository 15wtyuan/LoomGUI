# Task 8 Report: Unity DynamicTreeDemo + 重编 .dll + push

## 状态：完成（待家里机 PlayMode 验收）

## 做了什么

T8 是 v1.3+ 动态树重构的末项（Unity 侧收尾）。T1-T7 已完成 Rust 侧（代际 NodeId + slotmap + 动态 API + 9 FFI 导出 + csbindgen 生成 C# 绑定）。T8 完成 Unity 侧 3 件事：

1. **LoomStage.cs 加 9 个 P/Invoke 封装**（转调 T7 csbindgen 生成的 `Native.loomgui_stage_*`）。
2. **新建 DynamicTreeDemo.cs**（三场景演示动态建树）。
3. **重编 release .dll**（含 9 新 FFI 函数）+ commit + push 到 main。

## LoomStage 9 封装签名

文件：`loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`（行 184-268）

照既有 P/Invoke 风格（`FindNodeById`/`LoadHtml`）：UTF-8 字节 + `fixed (byte* p = bytes)` 钉住 + `(nuint)len`，null-stage 守卫。create_root/create_node 返 `uint`（`uint.MaxValue`=0xFFFF_FFFF=失败）；其余返 `int`（0=ok，-1=err）。

```csharp
public uint CreateRoot(string kind, string css)   // 建根；kind∈{div/l-container/button/img/span}
public uint CreateNode(string kind, string css)   // 建游离节点（不挂父）
public int AppendChild(uint parent, uint child)   // 挂子到末尾；child 必须无父
public int InsertBefore(uint parent, uint child, uint refId)  // refId=0xFFFF_FFFF→末尾追加
public int RemoveChild(uint parent, uint child)   // 摘子（不删，可重挂）
public int RemoveNode(uint node)                  // 删节点（递归删子+清 anim/scroll/tween）；恒返 0
public int SetText(uint node, string text)        // Text 节点 content；非 Text→-1
public int SetSrc(uint node, string src)          // Image 节点 src；非 Image→-1
public int SetStyle(uint node, string css)        // 改 base_style；下帧 rematch 生效
```

**关键点**：T7 csbindgen 已把 9 函数生成到 `Native` 类（`loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs`，gitignore 不入库但 Unity 侧用）。LoomStage.cs 直接调 `Native.loomgui_stage_*`，无需自定义 DllImport（同既有 SetNodeDisabled/FindNodeById 风格）。

## DynamicTreeDemo 三场景

文件：`loomgui_unity/Assets/LoomGUI/Examples/DynamicTreeDemo.cs`（新建）

挂在带 LoomStage 的 GameObject 上。LoomStage.Awake 先用 inline _html/_css 建最小 scene（单空根 div，满足 create_root 等的 scene 前置——`Stage::create_root` 需 `self.scene.as_mut().ok_or("no scene")`），本脚本 Start 再用 9 动态 API 在其上纯动态建 UI：

- **场景1 层级骨架**：`CreateRoot("div", ...)` 建 stage 根（1920×1080 深蓝底）+ `CreateNode("div", ...)` 建 layer 容器（flex column 居中）+ `AppendChild` 挂载。
- **场景2 锚点挂载**：`CreateNode` 建 panel(400×300 白底) + title(span, 24px) + icon(img, 64×64) + icon2(img, 48×48)，`AppendChild` 挂 panel→layer、title/icon→panel；`InsertBefore(icon2, title)` 验子序 API；`SetText(title, "背包")` + `SetSrc(icon, "item_001.png")` + `SetSrc(icon2, "item_002.png")` 填内容。
- **场景3 SetStyle 改样式**：按 Space 切 panel 底色（白↔灰），`SetStyle(panel, "background:#eeeeee")` 增量改 base_style，下帧 rematch 从 base 重算 → 渲染自动生效。

用返回的 NodeId 句柄（不硬编码 0——slotmap idx 从 1 起，首节点 NodeId 非 0）。失败（0xFFFF_FFFF/-1）记 LogError 不中断（便于部分验收）。

## .dll 重编

- 命令：`cargo build -p loomgui_ffi_c --release`（成功，0 错误 0 警告）。
- 拷贝：`target/release/loomgui_ffi_c.dll` → `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`（覆盖）。
- **大小**：1802752 → 1838592 字节（+35840，含 9 新 FFI 函数）。
- **路径**：`loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`（注意：brief 写 `loomgui_core.dll`，**实际 .dll 名是 `loomgui_ffi_c.dll`**，照既有 .dll 名不变）。
- **入库**：gitignore 有 `!**/Plugins/**/*.dll` 例外，.dll 入库。
- **验证**：`cargo test -p loomgui_ffi_c --release dynamic_tree_api_ffi_round_trip` PASS（9 函数经 FFI 建/改/删节点全通）。

## commit + push

- commit hash：`b2c9582`（main 分支）
- commit msg：`feat(unity): DynamicTreeDemo + LoomStage 动态 API 封装 + 重编 dll (T8)` + 正文 + `Co-Authored-By: Claude`
- push：`7f6940a..b2c9582 main -> main`（成功）
- 文件：3 files changed, 187 insertions（LoomStage.cs 修改 + DynamicTreeDemo.cs 新建 + .dll 重编）

## 待家里机 PlayMode 验收项

家里机 `git pull` 后 Unity PlayMode 验：

1. **DynamicTreeDemo 场景**：挂 DynamicTreeDemo + LoomStage 同 GO。LoomStage inline _html/_css 设最小 scene（单空根 div）。PlayMode 跑 → 动态建的 stage 根 + layer + panel + title("背包") + 2 icon 正常显示，样式对（深蓝底、白底 panel、24px 标题、64/48px 图占位）。按 Space 切 panel 白↔灰底（SetStyle 增量改样式生效）。
2. **旧 v1-showcase 46 卡零回归**：旧 LoomShowcaseDriver 场景仍正常（LoomStage 新增 9 封装是纯增量，不改既有路径）。
3. **无 console 报错 / 花屏 / 悬空 node 访问异常**：动态建树无悬空 NodeId（slotmap 代际安全）。

## Concerns

1. **DynamicTreeDemo 需配 LoomStage inline scene**：create_root 等 API 需 scene 已建（`Stage::create_root` → `self.scene.as_mut().ok_or("no scene")`）。LoomStage.Awake 已调 LoadHtml/LoadPackage 建场景，故 demo 的 CreateRoot 在 Start 调（Awake 后）即可。家里机验时 LoomStage 的 _html/_css 留默认或设最小 `<div></div>` 即可（demo 不依赖 inline 内容，纯动态建）。
2. **icon 无 atlas 时 fallback 白占位**：SetSrc("item_001.png") 设了 src 但 demo 未绑 atlas，渲染时 tex_id 缺 → MirrorPool fallback 白占位（不阻塞，验动态建树结构 + set_src 调用通即可）。若要显示真图需配 atlas pkg（非本 task 范围）。
3. **brief .dll 名差异**：brief 写 `loomgui_core.dll`，实际 .dll 名是 `loomgui_ffi_c.dll`（crate 名）。照既有 .dll 名保持不变，路径 `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`。
4. **commit msg 首次 PowerShell here-string 泄漏 @**：已 `--amend` 修复，最终 commit `b2c9582` msg 干净。

## Final review fix

final review 发现 2 个 Minor（动态树新场景暴露，符合 spec §5.3"全持久附属同步清"不变量精神）。

### Minor-1：`remove_node` 没清 `scene.focused_node`

删焦点节点后 `focused_node` 悬空，FOCUS_OUT 事件带 stale node_id。

**修复**（`loomgui_core/src/scene/dynamic.rs` `remove_node` 联动清段，anim/scroll/tween 清之后、slotmap remove 之前）：
```rust
if scene.focused_node == Some(id) {
    scene.focused_node = None;
}
```
全局单一焦点，`== Some(id)` 对每个被删节点都做——递归删子时若子是焦点同样清。

**测试**（3 个新单测，全过）：
- `remove_node_clears_focused_node`：删焦点节点 → `focused_node == None`。
- `remove_node_keeps_focused_node_when_other_deleted`：删非焦点 → `focused_node` 不变（指向 root 仍 live）。
- `remove_node_recursion_clears_focused_child`：递归删焦点子（root 删 → grand 是焦点）→ `focused_node == None`。

### Minor-2：`input.rs` grip-dragging 期间 `expect("live node")` 可能 panic

`loomgui_core/src/input.rs` 约 line 585：拖滚动条 thumb 时若 `remove_node` 删容器，下帧此 expect panic。

**修复**（`expect` 改安全 match）：
```rust
let lr = match scene.get(pane) {
    Some(n) => n.layout_rect,
    None => {
        slot.scrolling_pane = None;
        slot.grip_dragging = false;
        continue;  // 中断本次 grip 处理，跳到下一 ev
    }
};
```
- 上下文：grip-dragging 在 `for ev in events {` 的 `PointerKind::Move` 臂内，`slot = &mut self.slots[slot_idx]`。`continue` 跳过本事件剩余 hover/move-dispatch，进入下一 ev。
- 同时清 `grip_dragging`：否则下帧 grip_dragging 仍 true 但 scrolling_pane 已 None，进不了此臂（无副作用，但清了更干净，避免悬空状态）。

### 验证

- `cargo test -p loomgui_core`：**468 passed; 0 failed**（465 既有 + 3 新 focused_node 测试）。
  - `scene::dynamic::tests`：31 passed（含 3 新测试）。
  - fence_contract / snapshot / v1e_dirty 集成测试全过。
- `cargo build --all-targets`：过（1 个 pre-existing 无关 warning：`loomgui_ffi_c` unused import `NodeId`）。

### commit

- hash：`69866fb`（main 分支，未 push）。
- msg：`fix(scene): remove_node 清 focused_node + grip-dragging 防 panic (final review)` + `Co-Authored-By: Claude`。
- 不 push（final fix 一起 push 或等用户）。
