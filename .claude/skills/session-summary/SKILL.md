---
name: session-summary
description: >
  Use when the user says "summarize this session", "总结到 skill", "更新 knowledge-reference",
  "更新 skill", or after completing a significant LoomGUI feature or bugfix and wanting to persist
  the session's learnings (pitfalls, dependency API adaptations, mechanisms, debug techniques) into
  the project knowledge base. 把当前 session 的 LoomGUI 经验总结进 knowledge-reference skill（设计契约变化才同步 docs）。
---

# LoomGUI Session Summary

将当前会话中与 LoomGUI 相关的经验（踩坑、依赖 API 适配、机制实现、调试技巧）总结进项目知识库 `knowledge-reference` skill。设计契约变化才同步 docs。

## Process

### 0. 简洁原则

**每条控制在 3-5 行。** 症状/根因/解决/教训各一句话。不写冗长叙述、不重复已有内容（先 grep 确认）、不列举显而易见的细节。这些文档是给 AI 看的索引，不是技术博客。

### 1. Review Session Context

从当前会话提取 LoomGUI 相关工作：
- 改了哪些文件/模块？
- 解决了什么问题？
- 踩了什么坑（尤其**依赖 API 与草稿/plan 不符**、**AI 可预测性约束违背**）？
- 有哪些新机制/调试技巧/已知问题变化？

### 2. Read knowledge-reference

读 `.claude/skills/knowledge-reference/SKILL.md` 当前内容（尤其 §3 API / §4 约束 / §5 坑 / §7 ledger），**先 grep 确认要加的东西没重复**。

### 3. Categorize Findings

| 发现类型 | 目标 | 插入位置 |
|---|---|---|
| 依赖 API 适配（taffy/ttf-parser/cssparser/scraper 等版本差异、草稿与实际不符） | knowledge-reference | §3（新子节） |
| 新踩坑（症状+根因+解决+教训） | knowledge-reference | §5（编号递增） |
| 新机制/层实现细节 | knowledge-reference | §2 对应层 |
| AI 可预测性约束变化/新增 | knowledge-reference | §4 |
| 调试/验证新技巧 | knowledge-reference | §6 |
| 已知问题/ledger 变化（v0 占位消除/新 defer 项） | knowledge-reference | §7 |
| **设计契约变化**（新围栏 CSS 属性、新 Node 字段、架构调整、新主文档章节） | `docs/design/00-main-design.md` | 对应章节 |
| 范围/defer 变化 | `docs/roadmap/` | roadmap.md（已合并：范围/路线/机制草稿） |

**§2 机制 vs §3 API 判据**：§2 = 数据结构/层职责/契约（怎么组织的）；§3 = crate 版本签名差异（草稿与实际不符）。同一发现两侧都涉及时，归到主要教训侧——如「taffy 不消费视觉属性」是 §3 API 边界认知，「ResolvedStyle 持哪些字段」是 §2 机制。

### 4. Update knowledge-reference

**原则**：精简、索引化。每条 3-5 行。先 grep 确认无重复。

**新坑格式**（§5，编号递增）：
```markdown
### 坑 N：[标题]

**症状**：具体表现。
**根因**：技术原因。
**解决**：修复方式。
**教训**：可复用经验。
```

**新 API 适配**（§3，新子节）：
```markdown
### 3.X [crate 版本]（模块文件）
- **实际 API**：正确签名/返回类型。
- 与草稿/旧版的差异点。
```

**新机制**（§2 对应层）：
```markdown
### 2.X [层名]（主文档 §X）
- 机制要点 + 关键数据结构/契约。
```

**约束变化**（§4）：追加或修订约束条目，带主文档 § 出处。

**ledger 更新**（§7）：v0 占位项消除 → 从「v0 占位」移除或标注已修；新 defer 项追加到对应分组。

**不要**：
- 重复已有内容（先 grep）
- 写冗长调试过程（坑只写症状/根因/解决/教训）
- 改动非 LoomGUI 相关章节
- 把设计契约写进 skill（设计进 docs/design，skill 只写实操）

### 5. （可选）更新 docs

**仅当本 session 改了设计契约**（新围栏 CSS 属性、新 Node 字段、架构调整、新主文档章节）才更新 `docs/design/00-main-design.md`。范围/defer 变化更新 `docs/roadmap/`。**纯实现/踩坑不进 docs**（进 skill）。

**围栏属性判据**：属性已在围栏列表（`docs/design/fence.md`，真相源 `fence_contract.rs`）→ 加实现支持**不触发** docs；属性**新增**到围栏列表 → 触发 docs（同步 fence.md + 主文档 §4）。

### 6. Commit

```bash
git add .claude/skills/knowledge-reference/SKILL.md
# 若改了 docs:
# git add docs/design/00-main-design.md docs/roadmap/
git commit -m "docs(skill): 总结 session — <一句话主题>"
```

## knowledge-reference 章节索引

- §1 架构（workspace / 数据流 / 渲染树契约）
- §2 各层机制（parse / style / scene / text / layout / render / stage）
- §3 依赖 API 适配（taffy 0.5 / ttf-parser 0.20 / cssparser 0.34 / scraper 0.19）
- §4 AI 可预测性核心约束（8 条）
- §5 v0 踩坑记录（坑 1-N，编号递增）
- §6 调试/验证技巧
- §7 已知问题/未完成（v0 ledger + defer 表）

## 常见误判

| 误判 | 纠正 |
|---|---|
| 把实现细节写进 docs/design | 设计契约才进 docs；实操/踩坑进 skill §2/§3/§5 |
| 重复已有坑 | 先 grep knowledge-reference §5 标题 |
| 坑写成长篇调试流水账 | 只写症状/根因/解决/教训，≤5 行 |
| 漏掉依赖 API 适配 | 这是最易重复踩的（plan 草稿常与 crate 实际不符），每次踩必进 §3 |
| session 结束不总结 | 经验丢失 = 下次重复踩；完成功能/修复即触发本 skill |
