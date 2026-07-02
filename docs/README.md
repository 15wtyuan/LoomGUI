# LoomGUI 文档

跨引擎游戏 UI 框架。Rust 核心 + 多引擎后端（Unity 首发），HTML/CSS 子集 DSL，taffy flexbox，自绘渲染。

## 文档结构

| 目录 | 内容 | 何时读 |
|---|---|---|
| [`design/`](design/) | **主设计**——设计意图 + v1 精确契约（当前实现真相源）+ [围栏权威](design/fence.md) | 理解"设计成什么样、怎么实现" |
| [`roadmap/`](roadmap/) | [路线图](roadmap/roadmap.md)——v1 已交付 + v1.x/v other/v2 路线 + 机制草稿 | 理解"做了什么、接下来做什么、defer 了什么" |

## 入口

- **开发依据**：[`design/00-main-design.md`](design/00-main-design.md) —— 当前实现真相源
- **围栏属性**：[`design/fence.md`](design/fence.md) —— 权威清单（真相源 `fence_contract.rs` 测试）
- **路线/范围/机制草稿**：[`roadmap/roadmap.md`](roadmap/roadmap.md)

## 维护原则

- **主文档 = 当前实现真相源**：只写设计意图 + v1 精确契约。不写 v1.x/v2 的机制实现细节（在 roadmap 草稿），不写迭代历史。
- **决策理由**体现在主设计各章节的设计说明里（不单设 ADR 目录）；历史决策追溯见 git。
- **围栏属性权威 = `fence.md`**（真相源 `fence_contract.rs` 测试）；主设计 §4 只写哲学/原则，不重复属性表。
- **v1.x/v2 机制草稿**进 `roadmap/roadmap.md` §5（等实现验证后"毕业"回主文档）。
