# LoomGUI 文档

跨引擎游戏 UI 框架。Rust 核心 + 多引擎后端（Unity 首发），HTML/CSS 子集 DSL，taffy flexbox，自绘渲染。

## 文档结构

| 目录 | 内容 | 何时读 |
|---|---|---|
| [`design/`](design/) | [主设计](design/main-design.md)（项目设计真相源）+ [围栏权威](design/fence.md) | 理解"设计成什么样、怎么实现" |
| [`roadmap/`](roadmap/) | [路线图](roadmap/roadmap.md)——v1 已交付 + v1.x/v other/v2 路线 + 机制草稿 | 理解"做了什么、接下来做什么、defer 了什么" |
| [`pitfalls.md`](pitfalls.md) | 踩坑全库 + 依赖 API 适配 | 开工前查"具体怎么干 + 坑在哪" |

## 入口

- **开发依据**：[`design/main-design.md`](design/main-design.md) —— 项目设计真相源
- **围栏属性**：[`design/fence.md`](design/fence.md) —— 权威清单（真相源 `fence_contract.rs` 测试）
- **路线/范围/机制草稿**：[`roadmap/roadmap.md`](roadmap/roadmap.md)
- **踩坑/API 适配**：[`pitfalls.md`](pitfalls.md)
- **AI 工作约束**：根 [`CLAUDE.md`](../CLAUDE.md)

## 维护原则

- **主设计 = 项目设计真相源**：只写设计意图 + 契约。不写机制实现细节（在 roadmap 草稿），不写迭代历史，不写版本标注。
- **决策理由**体现在主设计各章节的设计说明里；历史决策追溯见 git。
- **围栏属性权威 = `fence.md`**（真相源 `fence_contract.rs` 测试）；主设计 §3 只写哲学/原则，不重复属性表。
- **踩坑 + 依赖 API 适配**进 `pitfalls.md`（编号递增，写法：症状/根因/解决/教训）。
- **AI 工作约束 + 高价值可复用经验**进根 `CLAUDE.md`（踩坑不进 CLAUDE.md，进 pitfalls.md）。
- **v1.x/v2 机制草稿**进 `roadmap/roadmap.md` §5（等实现验证后"毕业"回主文档）。
