---
name: session-summary
description: >
  Use when the user says "summarize this session", "总结到 skill", "更新踩坑",
  "总结到 CLAUDE.md", or after completing a significant LoomGUI feature or bugfix and wanting to persist
  the session's learnings into the project knowledge base. 把当前 session 的 LoomGUI 经验分类沉淀：
  踩坑进 docs/pitfalls.md，高价值原则/调试/偏好进 CLAUDE.md。
---

# LoomGUI Session Summary

将当前会话中与 LoomGUI 相关的经验分类沉淀进项目知识库。设计契约变化才同步 `docs/design/main-design.md`。

## 两个去处（判据：是否高价值可复用 + 不希望 AI 遗忘）

| 类型 | 去向 | 判据 |
|---|---|---|
| **踩坑**（症状/根因/解决/教训） | `docs/pitfalls.md` §2 | 具体 bug 踩坑，编号递增（坑 100+） |
| **依赖 API 适配**（crate 版本签名差异、草稿与实际不符） | `docs/pitfalls.md` §1 | 具体 crate API 差异 |
| **设计契约变化**（新围栏 CSS 属性、新 Node 字段、架构调整） | `docs/design/main-design.md` | 改了契约才动；纯实现不进 |
| **范围/路线变化** | `docs/roadmap/roadmap.md` | 范围/defer/路线调整 |
| **高价值原则/设计哲学/调试技巧/用户偏好** | `CLAUDE.md` | **必须高价值、可复用、不希望 AI 遗忘**才放——不是什么都放 |

## Process

### 1. Review Session Context

从当前会话提取 LoomGUI 相关工作：
- 改了哪些文件/模块？
- 解决了什么问题？
- 踩了什么坑（尤其**依赖 API 与草稿/plan 不符**、**AI 可预测性约束违背**）？
- 有哪些新机制/调试技巧/原则变化？

### 2. 读现状 + grep 防重复

- 读 `docs/pitfalls.md` §2 坑标题（先 grep 确认要加的坑没重复）
- 读 `CLAUDE.md` 当前内容（确认要加的原则/调试不在里面）

### 3. 分类沉淀

**踩坑** → `docs/pitfalls.md` §2 末尾，编号递增：
```markdown
### 坑 N：[标题]

**症状**：具体表现。
**根因**：技术原因。
**解决**：修复方式。
**教训**：可复用经验。
```
每条 3-5 行，不写冗长调试流水账。

**依赖 API 适配** → `docs/pitfalls.md` §1 对应 crate 子节（或新子节）。

**设计契约变化** → `docs/design/main-design.md` 对应章节（围栏属性变化还要同步 `fence.md` + `fence_contract.rs` 测试）。

**高价值原则/调试/偏好** → `CLAUDE.md`。**严格判据**：
- ✅ 放：AI 下次会重犯的错（如 stale .dll、改 parse-time 必重打 pkg）、跨任务可复用的方法论（如"草稿常不符，查 crate 源码"）、设计哲学约束（AI 可预测性 8 条）、高价值调试路径（dump_*.rs）
- ❌ 不放：一次性实现细节、具体函数签名、版本号快照、单 feature 流程——这些进 pitfalls.md 或靠代码/spec
- 边界：拿不准的进 pitfalls.md，CLAUDE.md 宁缺毋滥（它是 AI 每次都读的，太长会稀释）

### 4. Commit

```bash
git add docs/pitfalls.md CLAUDE.md
# 若改了设计契约：
# git add docs/design/main-design.md docs/design/fence.md
git commit -m "docs: 总结 session — <一句话主题>"
```

## 常见误判

| 误判 | 纠正 |
|---|---|
| 把实现细节写进 main-design.md | 设计契约才进 docs/design；实操/踩坑进 pitfalls.md |
| 什么都往 CLAUDE.md 塞 | CLAUDE.md 只放高价值可复用；踩坑进 pitfalls.md，实现细节靠代码 |
| 重复已有坑 | 先 grep pitfalls.md §2 坑标题 |
| 坑写成长篇调试流水账 | 只写症状/根因/解决/教训，≤5 行 |
| 漏掉依赖 API 适配 | 这是最易重复踩的（plan 草稿常与 crate 实际不符），每次踩必进 pitfalls.md §1 |
| session 结束不总结 | 经验丢失 = 下次重复踩；完成功能/修复即触发本 skill |
