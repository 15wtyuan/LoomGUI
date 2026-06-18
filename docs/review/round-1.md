# 第一轮对抗性审查（v2 → v2.1/v2.2 修订）

> 3 视角并行：架构自洽性 / 性能与可行性 / 完整性与对照。
> 结论已沉淀进主文档（`docs/design/00-main-design.md`）和 ADR（`docs/decisions/`）。本文件为原始审查记录归档。

## 🔴 找出并已修复的问题

### 1. 文本 mesh 归属矛盾 → ADR-004
原说"几何全在核心"但动态字体 UV 在 Unity。修复：文本 mesh 在后端生成，核心只产 TextLayout。

### 2. RenderNode 契约硬化 → ADR-003/006
原扁平 struct 装不下 stencil 跨节点协议、九宫格、paintingMode RT、slot 复用。修复：公共头+enum payload、Mask 意图化、Material key 补 mask_context、eraser 显式化。

### 3. 虚拟化 slot 复用语义 → §13.2
原"全集 diff + 对象池"矛盾。修复：核心维护可视槽，slot_id 稳定 NodeId 变，后端按 slot 复用。

### 4. FFI 内存模型 → ADR-007
原"扁平 buffer"含糊。修复：SOA 公共头 + 按类型多 arena + C# 拷贝 + Unchanged 变体。

### 5. 应用层缺口（滚动/输入消费/参考分辨率）→ §10.6/§7.4/§12.7
原文档"渲染引擎设计到 8 成、UI 框架只到 5 成"。修复：滚动容器、is_pointer_on_ui、参考分辨率进 v1。

## 🟡 已登记 v1.x+ 的缺口

IME 完整链路、软键盘（移动端）、字体 fallback 链、NativeHost、多窗口/弹窗/模态（不进核心，多 Stage 组合）、DragDrop、手势、运行时本地化（translation）、音效绑定、性能统计 Stats、通用对象池、grid、CSS transition、世界空间 UI、SRP 混合、Godot 后端、编辑器。

## 🟢 范围收敛
- NativeHost 移出 v1（v1.x，预留位）
- 窗口系统不进核心
- v1 补滚动+输入消费+参考分辨率
- Gears 收敛 6 种（v1.x 再做 Controller/Gear 整套）

## 总评
地基方向对，渲染/FFI/对象模型想得透。但"渲染引擎 8 成、UI 框架 5 成"——应用层与跨平台硬需求大段空白。两处契约幻觉（文本 UV、RenderNode 欠拟合）须动工前解。
