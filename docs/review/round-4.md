# 第四轮对抗性审查归档

> 三视角并行：**AI 可预测性** / **文本与测量层** / **FFI 与帧管线**。
> 本轮特色：第三轮落地"AI 可预测性"首要准则后，验证主文档是否真正贯彻；并深压文本/FFI 两个硬技术轴。
> 30 条归并为 T1-T30，处理结论见下。主文档/ADR 的修订已落 `docs/design/00-main-design.md` 与 `docs/roadmap/v1-scope.md`。

---

## HIGH（8 条，全改）

| # | 问题 | 处理 |
|---|---|---|
| **T1** | `gear_locked` 重入模型写错：文档说 Apply 时置位跨帧保住让 on_update 跳过回写——但 fgui(`GearXY.cs:108-134`) 是 on_update **同栈帧** set→write→clear，跨帧时 locked=0 守卫不触发 → tween 中间值污染存储。且"深度计数区分哪个 gear"错（per-Node u8 区分不了 10 个 gear）。 | §6.3 `Cell<u8>`→`Cell<bool>`；§11.4 重写为"同步同栈帧守卫、跨帧 on_update 自重置、bool 够用"，对齐 fgui。 |
| **T2** | `Unchanged`(per-NodeId) 与 slot 复用(per-slot_id) 身份轴冲突 → 虚拟列表 slot 换 NodeId 时若发 Unchanged → 后端沿用旧 item 内容 → 花屏。 | §8.7/§13.2/§14.3/§14.6 统一为 **reuse_key**(slot_id 优先否则 node_id)；核心强制不变量：slot→node 映射变化时必发真实 payload。 |
| **T3** | "browser-compatible 垂直堆叠"是假命题：AI 对 div 的真先验是 block flow（文本行内流），`<div>HP: <img> 100</div>` Chrome 一行、LoomGUI 三行。称 browser-compatible 反误导 AI。 | §4.1 删 browser-compatible 框定，正面立规"div 永远 flex 容器、只装 flex item、混排进 `<l-rich>`"；§4.2 编译错误变规则执法。 |
| **T4** | margin collapse 未记录：flex margin 不折叠，AI 写堆叠子项 margin 控间距 → Chrome 折叠(30px) vs LoomGUI 求和(50px)，Chrome 预览骗 AI。 | §5.4 加"margin 不折叠、子项间距用 gap"。 |
| **T5** | 字体度量归一化契约缺失：fgui(`DynamicFont.cs:70-71`) 用 `fontSize*1.25` 估算，是单引擎内偷懒；我们跨引擎 ttf-parser vs Unity hinted advance 会偏。但全套归一化契约(advance 权威/关 hinting)对 v1 单后端是纯负担。 | **降级为标注**：§9.1 标 v1 单后端简化假设 + v1.x(Godot 接入)待定升级归一化契约。不现在上。 |
| **T6** | TextLayout SOA 缺顶点装配规则：schema 无 glyph 位置公式，两后端各自发明累加规则 → 必然发散。 | §9.2 改 **glyph 存绝对 x/y**（核心已累加+已应用 text-align），后端 `quad_min=(x+bearing_x,y+bearing_y)` 零累加；advance 变咨询值；per-glyph font_id 预留。 |
| **T7** | 缺 text dirty 位：内容变但 box 不变时(10→09)可能发 Unchanged → 静默陈旧文本。fgui `_textChanged` 独立于 `_meshDirty`。 | §6.3 DirtyFlags 加 `text` 位；§8.7 Text 节点 `text_dirty\|\|mesh_dirty` 必发 Text 变体；§15 管线加 text dirty 步。 |
| **T8** | `:l-page(n)` 自创伪类无 AI 训练先验（猜不出 specificity/索引），而 `[data-page="n"]` AI 烂熟。 | §4.5/§5.2/§5.3/§5.5 + v1-scope 全改 `[data-page="n"]` 属性选择器，data-page 挂 Controller 元素、标准 cascade。 |

## MED（改 / 降原则 / 登记）

| # | 问题 | 处理 |
|---|---|---|
| **T9** | ScrollPane 物理没在 §15 管线定位 → 滚动/命中 1 帧滞后或双写。v1 含滚动。 | §15 加 (2b') ScrollPane 物理步；§2.6 单时钟软化"单 tick 入口内有序分步"；§12.7 定 tick 时机+禁 GTween tween scroll_pos。 |
| **T10** | NativeHost 尺寸 push 没管线步 → 用上帧陈旧 intrinsic size。 | §15 加 (1.5) drain 步；§14.4 加 push 时机约束（tick 前完成）。 |
| **T12** | "批合不跨 Erase"是期望非强制；两遍 DFS sort_key 规则没写死 → 批合重排可能把 Content 移过 Erase → 漏出遮罩。照搬 fgui。 | §8.5 改**结构强制** BatchingRoot（不下钻进 root 子树）；§8.8 钉死 `Erase.sort_key=max(子树Content)+1`、重排区间 `[Write+1,Erase-1]`；§8.7 改引用。 |
| **T14** | C# buffer 大小策略未定 → 冷帧/换页帧 GC 风险；v1 验收只覆盖静态帧。 | §14.3 定 `ArrayPool.Rent(本帧实际大小)` 池化租用、2ms 预算；v1 §4 加冷帧/换页帧验收。 |
| **T15** | Chrome 预览系统性骗 AI 布局（margin/换行），"偏差可控"是假话。 | v1 §2.1 加 Chrome 预览可信/不可信清单（不进主文档）。 |
| **T17** | line-height 推导未定义。复杂度过高，不钉算法。 | §9.2 补**原则一句**：line-height 生效并烤进 Line.height，后端不重套；公式实现期对 Chrome 调。 |
| **T22** | CJK 换行/kinsoku/white-space 模型未定。同 T17 不钉算法。 | §9.1 补**原则一句**：white-space 生效、换行以核心为准、kinsoku 实现期对 Chrome 调、围栏文档记已知 divergence。 |
| **T16** | RichText 行内对象测量可能循环（允许 %/flex 时）。 | §7.2 明确行内对象**纯 intrinsic 尺寸**（px 或纹理像素），%/flex 编译错误。 |
| **T18** | 行内混排禁令理由没讲清"为什么"。 | 随 T3 一起改：§4.1/§4.2 正面立规"混排进 l-rich"。 |
| **T20** | row-gap/column-gap 砍了但 gap 在，AI 常写 longhand。 | §4.4/v1 §2 支持 longhand（映射同 taffy 字段）。 |
| **T21** | position:relative+insets 映射静默。 | §6.4 加映射行（Relative+inset 视觉偏移）。 |
| **T11** | 命中测试 1 帧滞后+缓存语义未声明。 | §10.1/§15 加命中语义（本帧布局命中、事件回调改的布局延下帧）。 |
| **T13** | 可选扩展列(offset指针) 与 FFI"定长/拷贝/不追指针"矛盾；`Option<VertexMatrix>` 已在 v1。 | §8.9/§14.3 统一为"arena 内相对偏移 u32"+原子拷贝+feature_flags 不变量。 |
| **T19** | -l-slice vs border-image-slice 双拼法。 | §4.4 定 border-image-slice canonical，砍 -l-slice。 |
| **T23** | 文本测量缓存缺失 → 性能悬崖。v1 naive 够用。 | **登记不实现**：v1 §7 标"实现期撞墙再加 cache"。 |
| **T30** | 消失节点检测算法/复用键；feature_flags 变化必发真实 payload。 | 随 T2(diff 算法/复用键)、T13(feature_flags 不变量) 一起改完。 |

## LOW（批量改）

| # | 处理 |
|---|---|
| **T24** | §5.2 cascade 来源 → 标准 specificity tuple (a-b-c)，属性选择器/伪类归 b 级。 |
| **T25** | 继承白名单补 `visibility`。 |
| **T26** | §4.1 写明 `-l-*`(真 CSS 扩展) vs `data-*`(状态) 拆分原则。 |
| **T27** | §9.2 钉死 v1 `cluster`=源码点索引 1:1，警告 v1.x 光标别基于 1:1。 |
| **T28** | §9.1 枚举 BiDi 切的代价（CJK+emoji→tofu、组合符号→错位），不再称"干净切"。 |
| **T29** | `x_off/y_off`→`bearing_x/bearing_y`（随 T6 落）。 |

---

## 本轮关键产出

1. **AI 可预测性落地验证**：T3/T4/T8/T15 把"首要准则"从口号变成可执行规则——删假 browser-compatible、补 margin divergence、`:l-page`→`[data-page]`、Chrome 预览可信清单。AI 现在能从 HTML 预测渲染，且知道自己会在哪几条上被骗。
2. **文本层契约化**：T6（顶点装配）+T7（text dirty）把文本从"断言"变"可用 schema"；T5/T17/T22 按复杂度门槛降级，避免过度设计卡住产品。
3. **FFI 契约硬化**：T2（reuse_key）+T12（批合强制）+T13（扩展列）+T14（buffer 池化）把"会咬人的"契约钉死。
4. **机制纠正**：T1（gear_locked）+T9（ScrollPane 时钟）是文档把机制写错/写漏，纯纠正。

## Ponytail 取舍记录

- **T5 字体归一化契约**：全套契约对 v1 单后端纯负担 → 降标注，v1.x Godot 接入再评估。
- **T17 line-height / T22 kinsoku 公式**：复杂度过高 → 不钉算法，留原则。
- **T23 测量缓存**：v1 naive 重算够用 → 登记不实现。
