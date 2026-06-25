# LoomGUI UI 框架设计评审报告

## 1. 总体结论

LoomGUI 当前的 UI 框架设计方向是正确的，整体架构没有跑偏。它已经具备一个跨引擎 UI 框架应有的核心骨架：Rust core 负责确定性逻辑，Unity 后端负责引擎接入与镜像渲染，两者边界清晰。

当前最大风险不是“设计方向错误”，而是后续复杂度会集中压到几条关键链路上：DSL 围栏、inline/package 一致性、FFI 契约、Unity 自动化验证、transform/scroll/dirty 性能债务。

本次评审按要求不把正在施工中的 v1d.1 半成品作为负面结论。

## 2. 架构评价

LoomGUI 的核心设计是合理的：

- Rust core 保持引擎无关，负责解析、样式、布局、文本测量、命中、状态、事件生成和渲染意图。
- Unity 后端只负责输入采集、字体/纹理加载、FrameBlob 解析、GameObject 镜像、Material/Mesh 提交和 C# 事件路由。
- `.pkg.bin` 作为 Rust-internal 包格式，C# 不解析，降低了跨语言维护成本。
- FrameBlob/EventRecord 才是真正跨 FFI 的契约，这个边界划分是对的。
- 设计文档、roadmap、spec 分层比较清楚，决策来源可追溯。

整体看，这是一个“core 权威 + 后端镜像”的架构，而不是 Unity 插件里塞一堆业务逻辑的方案。这个方向适合未来扩展到 Godot、WASM editor 或其它后端。

## 3. 设计亮点

### 3.1 HTML/CSS 子集作为 AI DSL 是有战略价值的

LoomGUI 选择 HTML/CSS 子集，不是为了做浏览器，而是为了让 AI 能读、写、预测 UI。这一点非常关键。相比 JSON/XML 私有格式，HTML/CSS 对 AI 更友好，也更适合用文本驱动 UI 生成。

这个产品动机是成立的。

### 3.2 v1 范围控制比较健康

v1 没有一开始就铺开 Controller、Gear、Transition、富文本、虚拟列表、复杂输入控件，而是先收敛到：

- 按钮
- 文本
- 图片
- 基础布局
- 基础事件
- 打包器
- 图集
- Unity 渲染后端
- 后续滚动和自适应

这个切法是务实的，能先证明架构成立。

### 3.3 事件系统路线基本正确

当前事件系统从 hover/active、click，到 bubble/capture、多触摸、CancelClick、stopImmediatePropagation，整体是在向 FairyGUI 的成熟语义靠拢。

Rust core 负责命中和状态机，C# 负责业务侧事件路由，这个分工是合理的。

### 3.4 渲染树作为“意图契约”很重要

RenderNode / FrameBlob 描述的是要画什么、顺序是什么、裁剪上下文是什么、纹理和 mesh 是什么，而不是规定 Unity 必须怎么画。这一点是跨引擎的根基。

## 4. 主要风险

### 4.1 HTML/CSS 先验和 LoomGUI 语义存在张力

LoomGUI 不是浏览器：

- `div` 默认是 flex column
- 不支持 inline flow
- 文本换行不完全等同浏览器
- margin/gap 行为需要强约束
- 很多 CSS 属性在围栏外

这不是错误，但必须有强围栏验证器。否则 AI 很容易写出浏览器可渲染、LoomGUI 不成立的页面。

结论：围栏验证器不是辅助工具，而是这个 DSL 能否成立的核心基础设施。

### 4.2 inline 路径和 package 路径必须保持一致

开发期 inline，生产期 package。如果两者行为不同，调试体验会变差，也会削弱测试可信度。

目前需要特别关注：

- inline 是否提取动态伪类规则
- package 是否和 inline 渲染黄金等价
- 运行时状态样式是否两条路径一致

建议把 inline/package 等价测试继续作为强门槛。

### 4.3 FrameBlob 性能债务需要按计划偿还

v1 阶段全量或半全量同步可以接受，但后续必须兑现：

- `Unchanged`
- dirty diff
- ArrayPool
- 静态帧低成本同步
- 冷帧/换页帧预算验证

否则 500 节点演示可以过，但真实项目会开始出现性能压力。

### 4.4 transform / scroll 会显著提高坐标复杂度

当前 flatten 到根 GO、绝对 design 坐标的方案很务实，帮助 v1a/v1b 快速稳定。但 v1d 后会引入：

- transform
- world_to_local 命中
- scroll content offset
- 动画 transform
- safe-area 输入映射

这些会要求更严格的坐标矩阵契约。这里不宜用 Unity 侧临时偏移补丁，否则会破坏 core 权威。

### 4.5 Unity 自动化测试仍需增强

目前 Unity 侧仍有一些“家里机补路径/手动跑”的流程痕迹。短期可以理解，但长期会拖慢回归。

建议尽快做到：

- 自动定位字体资源
- 测试缺环境时明确 `Ignore`
- 能跑的 EditMode 测试不要依赖手工改代码
- Rust/C# ABI 常量和结构尺寸持续锁定

## 5. 当前完成度判断

排除 v1d.1 正在施工的内容后，当前完成度较好：

- Rust core 主线已经比较扎实。
- package / atlas / CJK / mesh merge / event v1c 主链路已经成型。
- Unity 后端已经能作为真实接入层，而不是纯 mock。
- 文档和 spec 的维护意识较强。

已观察到：

- `cargo build --workspace` 通过。
- `loomgui_core` 测试通过。
- `loomgui_pkg` 测试通过。
- workspace 全量测试当前受 v1d.1 施工影响，不作为本报告负面项。

## 6. 建议优先级

1. **保持 inline/package 行为一致。**  
   这是开发体验和测试可信度的基础。

2. **尽快补强围栏验证器。**  
   AI DSL 的成败很大程度取决于它。

3. **把 Unity 测试从“人工流程”变成“可重复流程”。**  
   尤其是字体路径、Stage 构造、EventHandler 路由测试。

4. **v1d 的 transform/scroll 保持 core 权威。**  
   不要把坐标、滚动、拖拽语义散落到 Unity 侧。

5. **v1e 兑现性能债务。**  
   dirty/Unchanged/ArrayPool/冷帧预算需要成为正式验收项。

## 7. 最终评价

LoomGUI 的设计是健康的，当前没有看到方向性跑偏。它最强的地方是：目标清楚、边界清楚、v1 范围克制、设计文档和实现路线能互相解释。

后续真正的挑战不是“能不能继续加功能”，而是能否在功能增加后继续守住契约一致性和自动化验证。如果这两点守住，LoomGUI 有继续成长为完整跨引擎 UI 框架的基础。