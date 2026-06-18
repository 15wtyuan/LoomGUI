# 第三轮对抗性审查（v2.3，新视角 → 主文档重构）

> 3 新视角：DSL/解析/样式层 / 行为正确性 / 跨引擎一致性与长期演进。
> 避免重复挖已知坑。结论已沉淀进主文档和 ADR。本轮触发主文档重构（去版本/迭代噪音）。

## 🔴 架构级（动工前定）

### 1. 渲染契约 Unity-flavored → ADR-003
- 坐标系：`y=height-y` 翻转是 Unity 假设（Godot 本就左上 y 下，不用翻）；翻转点没定。
- stencil：深度耦合 GPU stencil + Material ref（Godot 2D 用 canvas_group，4.5 才加 stencil）。
- 概念名泄漏：MaterialManager/sortingOrder/MeshRenderer 在契约章节。
- → 契约从"Unity 机制描述"重构为"渲染意图描述"。Mask 意图、DrawState、根 Stage 一次性 flip。

### 2. Godot 镜像目标待定 → ADR-015
"Control/Node2D/RenderingServer 待定"反推 v1 契约。→ 拍板 Node2D+canvas_item 自绘，否决 Control。

## 🔴 DSL/样式层（v1 可用性）

### 3. cascade 缺继承 → ADR-010
不实现继承 → 每个 span 重复写字体 → DSL 不可用。→ 补继承白名单 + 打包期展开。

### 4. 运行时伪类 + 样式 dirty 缺失 → ADR-011
`:hover/:active` 是 v1 保留项，但需运行时 selectors 求值 + 样式重算循环，DirtyFlags 无 style 档。→ 加 style dirty 档 + Element 适配器。

### 5. 打包器编译边界被高估 → §5.5
打包器不能编译运行时状态伪类成 flat style。→ 产物分静态（base_style）+ 动态规则表。

## 🔴 行为正确性 → ADR-012/013/014

### 6. 事件后"再 solve 一轮"能死循环
具体场景：click 改 height 裁出子元素→mouseleave→改回→mouseenter…→ 无终止。→ 改"事件布局改动延迟下帧 solve"。

### 7. transform vs layout 二义性 → 命中/动画断裂
GTween 改 transform 还是 layout style 没定义；命中用 layout_rect 还是 +transform 没定义。→ transform 不进 taffy；命中=layout_rect 经累计 transform 的 AABB。

### 8. 命中按 children 序 vs 绘制按重排 → 视觉/命中错位
sortingOrder 没回写 children 序，点视觉上层命中下层。→ 命中按等效绘制顺序逆序。

### 9. 拖拽 vs 滚动触摸仲裁缺失
列表拖 item vs 滚列表两都想 capture，无仲裁=bug。→ 阈值+先达者赢+双向退让（照搬 fgui）。

## 🟡 其它
- click 判定：Stage 绝对坐标 + Move 中取消 + down_targets 断裂沿祖先回退（§10.3）。
- ScrollPane 惯性不走 GTween（content 异步变化跳变）→ ADR-013。
- Gear reentrancy：Cell<bool> 单值不安全 → ADR-014（深度计数）。
- 包格式只有前向兼容，无反向/版本协商 → §12.2（formatVersion + 迁移器）。
- RenderNode 契约无版本化 → ADR-016。

## 🟢 澄清
- CSS 值→taffy 映射层完全没设计 → §5.4。
- HTML 嵌套混排→Node 映射规则缺失 → §4.2。
- 忽略策略分级（display:grid/position:absolute 应编译期报错）→ §4.1。
- 文本 font_id 跨引擎一致性 → ADR-004（字体资产契约，同一 ttf）。
- 跨引擎视觉回归测试缺失 → §16（normalized draw list diff）。

## 总评
v2.3 渲染/FFI/范围已硬，但这轮新视角挖出三组之前没碰的真问题：DSL/样式两道空承重墙、行为正确性死循环和命中错位、跨引擎契约 Unity-flavored。最该重视的是 ADR-003（契约意图化）+ ADR-015（Godot 拍板）——修正窗口是现在（v1 契约定型前）。触发主文档重构。
