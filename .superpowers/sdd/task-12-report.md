# v1d.5 家里机 PlayMode 验收清单（ScrollPane）

> sample：`LoomScrollDemo`（挂 LoomStage 同 GO）。HTML/CSS 见 `LoomScrollDemo.SampleHtml`/`SampleCss`（50 item 垂直列表 + 1 draggable + 1 嵌套水平 scroll）。
> Rust 侧 T1-T11 已全完成 + .dll v1d.5 重编 commit (5609854)。core+ffi cargo test --workspace 356 测全绿。
> **C# 本机不编译（无 Unity）**——家里机 PlayMode 验。.dll 已是 v1d.5（Plugins/ 里）。

## 必验 9 点（spec §10.3）

- [ ] **拖拽跟手**：垂直拖 scroll-area 内容随指 1:1 移动（无滞后）
- [ ] **Up 惯性**：甩出后内容滑行减速（修后 v²→|v|，v~1000px/s → ~1.4s 滑行，手感不偏长）
- [ ] **边界回弹**：拖到顶/底越界 → cubicOut 0.3s 弹回边界（不卡死在外）
- [ ] **滚轮滚动**：鼠标在 scroll-area 内滚 → 内容滚（CollectWheel 新旧输入双路径）
- [ ] **grip 拖动**：点右侧 thumb 滑块拖 → 直接定位 scroll（perc→scroll_pos）
- [ ] **overflow:auto 不显条 / overflow:scroll 始终显条**：改 sample overflow 验（auto 无溢出无 thumb；scroll 无溢出仍 thumb）
- [ ] **嵌套轴锁**：外垂直 scroll-area 内嵌 `.nested-hscroll`（overflow-x:scroll）→ 各滚各的（外垂直拖不触发内水平，反之亦然）
- [ ] **scroll-vs-draggable 仲裁**：`.item-drag`(draggable=true) 可拖（drag 阈值 2/10 先达赢）；空白处可滚（scroll 阈值 8/20）；scroll-start 取消 click
- [ ] **编程 SetScrollPos + 零回归 + 500 stress**：demo 自动演示 SetScrollPos(y=300 animated / y=800 instant / y=0 animated)；既有 v1d.4 tween sample 渲染/交互不变；500 节点场景滚动无卡顿

## 文件清单（T12）

- `loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIWheelEvent.cs`（**新**，手补 C# 镜像，坑 35；对齐 Rust `#[repr(C)]` 16B）
- `loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`（+`SetScrollPos` wrapper + `StagePtr`/`DesignSize`/`UseSafeArea` internal 访问器 + LateUpdate 调 CollectWheel）
- `loomgui_unity/Assets/LoomGUI/Runtime/LoomInputCollector.cs`（+`CollectWheel` 新旧输入双路径 + ScreenToDesign 复用 + 栈 `&ev`）
- `loomgui_unity/Assets/LoomGUI/Runtime/LoomScrollDemo.cs`（**新**，PlayMode sample）

## 待办（家里机）

- [ ] **.meta 补 commit**（坑 13）：`LoomGUIWheelEvent.cs.meta` + `LoomScrollDemo.cs.meta` 本机无 Unity 未生成 → 家里机开 Unity 生成后 commit
- [ ] PlayMode 跑 9 点验收；有问题反馈本机修
- [ ] inertia 手感（修后 |v| 法）：若仍偏长/偏短，回查 scroll.rs `begin_inertia` 公式 + `INERTIA_DIST_COEFF=0.4`
