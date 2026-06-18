# LoomGUI 文档

跨引擎游戏 UI 框架。Rust 核心 + 多引擎后端（Unity 首发），HTML/CSS 子集 DSL，taffy flexbox，自绘渲染。

## 文档结构

| 目录 | 内容 | 何时读 |
|---|---|---|
| [`design/`](design/) | **主设计**（单一真相源，最终总设计，无版本/迭代噪音） | 理解"设计成什么样" |
| [`roadmap/`](roadmap/) | 实现范围（按版本，v1/v1.x/v2 干什么） | 理解"先做什么" |
| [`decisions/`](decisions/) | 架构决策记录（ADR，每个决策的"为什么"） | 理解"为什么这么定" |
| [`review/`](review/) | 对抗性审查归档（三轮 subagent review） | 追溯决策来源 |

## 入口

- **开发依据**：[`design/00-main-design.md`](design/00-main-design.md) —— 唯一真相源
- **v1 做什么**：[`roadmap/v1-scope.md`](roadmap/v1-scope.md)
- **为什么这么定**：[`decisions/README.md`](decisions/README.md)

## 维护原则

- 主文档**干净**：只写最终设计，不写版本（v1/v1.x）、不写迭代历史（"从 X 改成 Y"）。更新时也保持干净。
- 版本范围进 `roadmap/`，迭代历史进 `decisions/`（ADR）。
- 主文档是后续开发的**唯一依据**。
