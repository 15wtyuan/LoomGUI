# v1d.2 brainstorm prep（compact 前固化，免重研究）

> v1d.1 已完成 + push（HEAD `b6511e9`）。v1d.2 brainstorm 已开：fgui 研究 + LoomGUI 现状探索完成，范围已提议，**5 个决策待用户拍**（compact 后接着拍 → 写 spec）。
> 范围来源：`docs/roadmap/v1d-plan.md` §3 v1d.2（defer #3 键盘 + #6 焦点/Tab/:focus）。
> 流程：拍决策 → spec `docs/superpowers/specs/2026-06-24-v1d.2-keyboard-focus-design.md` → plan → subagent-driven（不跳过 review）。

## fgui 对照（已核实源码 temp/FairyGUI-unity/）

- **键盘**：key 事件从 `Stage.focus`（聚焦对象）**bubble**，无焦点→Stage 全局（Stage.cs:958-962）。字段：keyCode + character + modifiers(ctrl/shift/alt/cmd)（InputEvent.cs:27/32/37）。`onKeyDown` 在 GObject（GObject.cs:245）。Tab/Enter/Escape **仅 InputTextField 内**处理（非通用，InputTextField.cs:1370-1389）。
- **焦点**：单一全局 `Stage.focus`（一个 DisplayObject，Stage.cs:35/326）。`focusable` bool 默认 true（DisplayObject.cs:919）。**点击自动聚焦**（pointer-down → SetFocus(target)，Stage.cs:1127/1181/1282）。`onFocusIn/onFocusOut` 派发到焦点链祖先（Stage.cs:412-441）。编程 RequestFocus（GObject.cs:1017）。
- **`:focus` 伪类**：fgui **无**（无 CSS）。LoomGUI 有 CSS :hover/:active/:disabled → **加 :focus 是补 fgui 没有的**（非镜像）。
- **Tab 导航**：`DoKeyNavigate(backward)` 按 **`tabStop` bool + DFS 子孙序**遍历（Stage.cs:487-537）——**无数值 tabIndex**。`tabStopChildren` 容器 scope。仅 InputTextField 自动触发。

## LoomGUI 现状（gaps）

- `input.rs`：纯指针（PointerEvent 16B / EventKind 4 var / EventRecord 20B / EVT 0-9）。无键盘。`time_s` 可做按键计时。
- `Node`（node.rs:35-66）：hovered/active/disabled/draggable/touchable bools。**无 focusable/focused/tabindex**。`Scene`（:96-102）**无 focused_node**。
- `dynamic.rs`：pseudo_hover/active/disabled flags（:40-42）+ compound_matches_with_state 门控（:106-118）。**无 pseudo_focus**。rematch_pseudo_classes 每帧跑（:181-225）→ :focus 一旦加 flag+门控自动生效。
- selector parser：认 :hover/:active/:disabled，**不认 :focus**。
- C# `LoomInputCollector.cs`：mouse+touch only，**无键盘**。
- FFI `lib.rs`：set_input(PointerEvent*) only。**无 key 通道**。ABI：EventRecord 20B / PointerEvent 16B。

## v1d.2 提议范围（检测全 core，C# 路由，照 v1d.1 模式）

- **keydown/up**：新 `KeyEvent` 输入结构 + 新 FFI `set_key_input`；keydown/up 进**既有 EventRecord 流**（EVT_KEY_DOWN/UP=10/11，node_id=焦点节点，字段复用带 key_code+modifiers）。
- **focus 状态**：`Scene.focused_node: Option<NodeId>`（单一全局，照 fgui）；`Node.tabindex: i32`（HTML `tabindex` 属性）。
- **click-to-focus**：pointer-down 命中 focusable 节点 → 聚焦（照 fgui+DOM）。
- **Tab 导航**：Tab/Shift+Tab 按 tabindex 序遍历 focusable 节点。
- **:focus 伪类**：pseudo_focus flag + selector `:focus` + compound 门控（rematch 既有每帧跑）。
- **focus/blur 事件**：FocusIn/FocusOut（照 fgui onFocusIn/Out）。
- **不范围**：IME/character（defer 随 TextInput v1.x）、TextInput。

## 5 个待拍决策（提议 + 推荐）

1. **key 事件 ABI**：新 KeyEvent 输入 + keydown/up 进既有 EventRecord 流（event_type 10/11，key_code 复用 touch_id 字段、modifiers 复用 pad[0]，零新输出 ABI、单流）— vs 独立 KeyEventRecord 输出（干净但多 borrow 通道）。**推荐前者**。
2. **focus opt-in 模型**：HTML `tabindex` 属性（DOM 原生，0=DOM序/N=显式序/-1=仅编程）— vs fgui 布尔 tabStop（DFS 序）。**推荐 tabindex**（AI 可预测性）。
3. **click-to-focus**：pointer-down focusable → 自动聚焦。**推荐 yes**。
4. **Tab 导航**：Tab/Shift+Tab 按 tabindex 序遍历。**推荐 yes**（plan #6 在范围）。
5. **modifiers**：KeyEvent 带 ctrl/shift/alt 位掩码（Shift+Tab 至少要 shift）。**推荐 yes**。

## compact 后恢复点

用户拍 5 决策 → 我写 spec → plan → subagent-driven 实现（不跳过 review）。脑中已有 fgui + 现状全貌（见上），无需重研究。
