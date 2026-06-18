# LoomGUI 文档

跨引擎游戏 UI 框架。Rust 核心 + 多引擎后端（Unity 首发），HTML/CSS 子集 DSL，taffy flexbox，自绘渲染。

## 文档结构

| 目录 | 内容 | 何时读 |
|---|---|---|
| [`design/`](design/) | **主设计**——设计意图 + v1 精确契约（当前实现真相源） | 理解"设计成什么样、v1 怎么实现" |
| [`roadmap/`](roadmap/) | 实现范围：[`v1-scope.md`](roadmap/v1-scope.md)（v1 干什么）+ [`v1x-deferred.md`](roadmap/v1x-deferred.md)（v1.x/v2 机制草稿） | 理解"先做什么、defer 了什么" |
| [`decisions/`](decisions/) | 架构决策记录（ADR，每个决策的"为什么"） | 理解"为什么这么定" |
| [`review/`](review/) | 对抗性审查归档（五轮 subagent review） | 追溯决策来源 |

## 入口

- **开发依据**：[`design/00-main-design.md`](design/00-main-design.md) —— v1 实现真相源
- **v1 做什么**：[`roadmap/v1-scope.md`](roadmap/v1-scope.md)
- **v1.x defer 了什么**：[`roadmap/v1x-deferred.md`](roadmap/v1x-deferred.md)
- **为什么这么定**：[`decisions/README.md`](decisions/README.md)

## 维护原则

- **主文档 = 当前实现真相源**：只写设计意图 + v1 精确契约。不写 v1.x/v2 的机制实现细节（那些在实现期才定），不写迭代历史。
- **v1.x/v2 机制**进 `roadmap/v1x-deferred.md`（机制草稿，等实现验证后"毕业"回主文档）。
- **版本范围**进 `roadmap/`，**决策理由**进 `decisions/`（ADR），**审查记录**进 `review/`。
- 主文档是 v1 开发的**唯一依据**；v1.x 机制以 `v1x-deferred.md` 为起点、实现时再精化。
