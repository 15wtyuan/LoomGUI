# v other — Editor 工作流 设计

> **状态**：brainstorm 收敛稿，待 review → writing-plans。
> **日期**：2026-06-30
> **roadmap 依据**：`docs/roadmap/roadmap.md` §2（v other），本 spec 对其做了三处修订（见 §1.3）。
> **调研依据**：`temp/open-design/` 源码（nexu-io/open-design 0.10/0.11）+ 实测 `od project import` + 实装 stable 通道桌面 app。

---

## 1. 目标与范围

### 1.1 目标

让设计师在 **open-design 桌面 app** 里用 AI 生成 **LoomGUI 围栏合规**的 UI（HTML+CSS），导出后经 `loomgui_pkg` 打包成 `.pkg.bin`，Unity 加载渲染。整个流程设计师**不碰 Rust / 命令行打包**，只跑一个 init 脚本 + 在 open-design 里用自然语言生成。

### 1.2 不做（YAGNI）

- **不自建编辑器壳**：复用 open-design 桌面 app，不改其源码（roadmap §2 决策）。
- **不提供正式 design-system**：每个 UI 的风格由设计师 prompt 驱动，LoomGUI 只管围栏规则。design-system 仅作**测试夹具**存在于 `samples/`（§4）。
- **不写独立 grep linter**：`lint-artifact.ts` 是 open-design 硬编码函数非扩展点（调研确认），围栏把关由 `loomgui_pkg` 打包期验证（FENCE_TAGS，坑 57）+ skill 引导 AI 自检承担。不造第二个验证器。
- **不做 loomgui_pkg 二进制分发**：loomgui_pkg 随整个 LoomGUI 仓库版本走，skill 调源码 `cargo build`。设计师拿固定版本跑，更新 = 整仓库更新（含重新 build 的二进制）。
- **不做 WASM 零偏差预览**：roadmap §3.4 的 v2 事项，本期近似预览（open-design Chromium iframe）+ skill 教"信围栏别信预览不可信项"。

### 1.3 对 roadmap §2 的修订

| roadmap §2 原文 | 本 spec 修订 | 依据 |
|---|---|---|
| §2.1 `DESIGN.md` 写死围栏规则 | 围栏规则**不进 DESIGN.md**，放工作区 `CLAUDE.md`（按 harness） | DESIGN.md 是给 AI 的风格 prose，混入硬约束会污染；open-design 调用的 harness（Claude Code 等）自动读 cwd 的 CLAUDE.md |
| §2.2 加 open-design `lint-artifact.ts` 扩展点做 grep linter | **砍掉** grep linter 层 | `lint-artifact.ts` 是硬编码函数不是扩展点；围栏把关靠 loomgui_pkg 打包验证 + skill 自检 |
| §2.3 skill 独立于 open-design | 保留，且 skill **封装 loomgui_pkg**（验证器+打包器进 skill，不向设计师暴露） | 设计师只看到"生成→自动验证→打包"闭环 |
| §2.4 打包桥 | 保留，即 loomgui_pkg CLI（已存在），skill 直接调 | 无新建 |

---

## 2. 架构总览

### 2.1 两个目录

```
LoomGUI/
├── editor/                              ← 模板源：init 脚本 + rules + skill 模板
│   ├── init.mjs                         ← Node 单文件注入脚本
│   ├── rules/                           ← 各 harness 围栏规则模板（带标签包裹）
│   │   ├── claude/CLAUDE.md.tmpl
│   │   ├── opencode/AGENTS.md.tmpl      ← opencode 用 AGENTS.md
│   │   └── codex/AGENTS.md.tmpl         ← codex 用 AGENTS.md
│   └── skill/loomgui-editor/            ← skill 模板（封装 loomgui_pkg）
│       ├── SKILL.md
│       ├── references/
│       │   ├── fence.md                 ← 围栏规则细节（CLAUDE.md.tmpl 的全文展开）
│       │   └── preview-trust.md         ← 预览可信清单（v1-scope §2.1）
│       └── tools/
│           └── pack.mjs                 ← 调 loomgui_pkg：定位仓库根 → cargo build → CLI
│
└── samples/                             ← UI 样例家 + editor 测试场（合并）
    ├── v1-showcase/                     ← 已验收基线（不动，Unity 打包源）
    ├── ai-output/                       ← editor 工作流 AI 新生成
    ├── design-systems/loomgui/          ← 测试夹具（暗色 dashboard，仅测试用）
    │   ├── DESIGN.md                    ← 纯风格（无围栏规则）
    │   ├── tokens.css
    │   └── components.html              ← 围栏内组件样例（flex/gap，非 grid/margin）
    ├── CLAUDE.md                        ← 【生成物】init 注入，gitignore
    └── .claude/skills/loomgui-editor/   ← 【生成物】init 注入，gitignore
```

### 2.2 设计师工作流（正式）

1. 设计师 clone/拿到 LoomGUI 仓库（固定版本）。
2. 跑 `node editor/init.mjs`，交互输入：
   - 目标工作区路径（设计师自己的项目目录，如 `D:\my-game-ui`）
   - 输出目录路径（pkg.bin 落哪，通常 Unity 的 `StreamingAssets/`）
   - harness 选择（claude / opencode / codex）
3. 脚本把 `rules/<harness>/` + `skill/loomgui-editor/` **拷贝**进目标工作区：
   - `CLAUDE.md`（或 `AGENTS.md`）：增量合并（§3.2），围栏规则用标签包裹。
   - `.claude/skills/loomgui-editor/`（或对应 harness 的 skill 目录）：skill 全套。
   - skill 里的 `pack.mjs` 已写入 LoomGUI 仓库根路径（脚本定位 `editor/` 上层）。
4. 设计师在 open-design 桌面 app 里 `od project import <目标工作区>`（或 UI 导入）。
5. open-design 调用 harness（Claude Code 等），harness **自动读工作区的 CLAUDE.md**（围栏规则）+ 发现 `.claude/skills/loomgui-editor/`。
6. 设计师用自然语言描述要什么 UI，AI 按 CLAUDE.md 围栏生成 HTML+CSS，落工作区。
7. skill 引导 AI 跑 `pack.mjs` 验证+打包：违规 → loomgui_pkg 非零退出 + 报错 → AI 自纠；合规 → 产出 `.pkg.bin` + atlas 到输出目录。
8. Unity 加载 `.pkg.bin` 渲染（家里机验收）。

### 2.3 自测工作流（本机）

`samples/` 自身当被注入的测试工作区（问题 A 决策）：

1. 跑 `node editor/init.mjs`，工作区路径填 `samples/`。
2. 脚本注入 `samples/CLAUDE.md` + `samples/.claude/skills/loomgui-editor/`（生成物，gitignore）。
3. open-design import `samples/`。
4. AI 生成进 `samples/ai-output/`，与 `v1-showcase/` 基线对照验收。
5. `samples/v1-showcase/` 仍是 Unity 打包源（路径不变，`cargo run -p loomgui_pkg -- samples/v1-showcase/index.html ...`）。

---

## 3. 组件设计

### 3.1 `editor/init.mjs`（Node 单文件注入脚本）

**职责**：交互式把围栏规则 + skill 注入目标工作区。

**输入**（readline 交互）：
- 目标工作区绝对路径
- 输出目录绝对路径（pkg.bin 落点）
- harness 选择（claude / opencode / codex）

**行为**：
1. `LOOMGUI_ROOT` = 脚本所在目录的上一层（`editor/` 在仓库根下，`init.mjs` 在 `editor/` 里，故 `path.resolve(__dirname, '..')`）。
2. 按 harness 选 `rules/<harness>/<file>.tmpl`，增量合并到目标工作区的对应规则文件（§3.2）。
3. 把 `skill/loomgui-editor/` 整目录拷到目标工作区的 harness skill 目录：
   - claude → `<workspace>/.claude/skills/loomgui-editor/`
   - opencode/codex → 对应 harness 的 skill 发现路径（调研确认：opencode/codex 亦读 cwd 的 skill 目录，具体路径实现期查 harness 文档）
4. 写入 `pack.mjs` 的 `LOOMGUI_ROOT`（脚本注入时替换占位符）。
5. 打印"接下来：od project import <workspace>"。

**跨平台**：纯 Node，`node:fs`/`node:path`/`node:readline`，零第三方依赖。open-design 用户必有 Node（app 基于 Node）。

### 3.2 围栏规则增量合并（CLAUDE.md / AGENTS.md）

工作区可能已有规则文件。合并逻辑：

- **不存在** → 直接拷模板全文。
- **存在且无标签** → 在文件末尾追加，用标签包裹：
  ```markdown
  <!-- loomgui-editor-begin -->
  （围栏规则全文）
  <!-- loomgui-editor-end -->
  ```
- **存在且有标签** → 替换 `<begin>..<end>` 之间的内容（支持重复跑 init 更新规则，不碰用户原有部分）。

**围栏规则内容**：权威清单见 `docs/design/fence.md`（单一真相源 = `loomgui_core/tests/fence_contract.rs` 测试）。CLAUDE.md.tmpl 全文 = `editor/skill/loomgui-editor/references/fence.md`（fence.md 副本，注入时拷贝）。

围栏清单要点（详见 fence.md）：
- 元素白名单：`div` / `span`+裸文本 / `img` / `button` / `l-container`。围栏外标签报错（不降级）。
- CSS 布局：`display:flex/none` + flex 全家桶 + `gap`/`row-gap`/`column-gap` + `width/height/min/max` + `padding` + `margin`（但**子项间距用 gap 不用 margin**）+ `border-width` + `aspect-ratio` + `order`。
- CSS 视觉：`background-color`、`background-image`+`background-size`(cover/contain/100%)、`border-radius`、`border`/`border-color`、`opacity`、`overflow`/`overflow-x`/`overflow-y`、`color`、`font-size/font-family/font-weight`、`text-align`、`line-height`、`letter-spacing`、`white-space:nowrap`、`transform`(translate/rotate/scale)、`pointer-events`。
- v1.x 扩展：`filter`(颜色矩阵 7 函数)、`border-image-slice`(九宫格)。
- 伪类：`:hover/:active/:disabled/:focus`。
- 选择器：标签/类/id/后代/子代/分组。
- **围栏外（写了不生效/静默忽略，AI 禁写）**：`display:grid`、`position:absolute/fixed/sticky`、`float`、`align-content`、`cursor`、`clip-path`、`background-position`、`background-repeat`、`transform-origin`、`skew()/matrix()`、`font-style`、`@media`、`:nth-child`、属性选择器、`+`/`~` 组合器。
- **注意 `position:relative`**：靠 taffy 默认 Relative 生效（写不写一致），非显式映射。AI 可写可不写，行为无差。
- **预览可信清单**（fence.md §6）：信 flex/gap/color/px/background-image；**不信** margin 折叠/文本换行像素/`position:absolute`(Chrome 脱离流 LoomGUI 不脱离)/`display:grid`(Chrome 渲染 grid LoomGUI 落 Flex)/`@media`。口径"信围栏规则别信预览"。

**围栏维护机制**（fence.md §4）：单一真相源 = `loomgui_core/tests/fence_contract.rs`（可执行围栏契约）。改 `apply_decl`/`FENCE_TAGS`/selector 必须同步改测试 + fence.md 副本。`cargo test -p loomgui_core fence_contract` 是防漂移门。本 spec 实现期需**新建** `fence_contract.rs`（现状：元素围栏有测试 dom.rs:203，CSS 属性有 37 个 mapping 测试但无"围栏外静默忽略"断言；fence.md 标【推断·待测】项需补测试转【实证】）。

### 3.3 `editor/skill/loomgui-editor/`（skill 模板）

**SKILL.md frontmatter**（最小必填，Claude Code base spec）：
```yaml
---
name: loomgui-editor
description: |
  Generate LoomGUI fence-compliant UI (HTML+CSS) for game dashboards/panels.
  Uses flex-only layout, tag whitelist (div/span/img/button), no grid/absolute.
  After generating, run pack.mjs to validate + pack into .pkg.bin for Unity.
triggers:
  - "loomgui ui"
  - "game dashboard"
  - "游戏 UI 面板"
---
```

**SKILL.md body**（工作流）：
1. 读工作区 CLAUDE.md 围栏规则（或 references/fence.md）。
2. 按设计师 prompt 生成 HTML+CSS，严守围栏。
3. 生成完跑 `tools/pack.mjs <html> <css> -o <out.pkg.bin>`：
   - 非零退出 = 围栏违规（loomgui_pkg 报错），读 stderr 自纠后重跑。
   - 零退出 = 合规，pkg.bin 已产出。
4. 向设计师报告：产出路径 + Unity 加载说明。

**references/**：
- `fence.md`：围栏规则全文（CLAUDE.md.tmpl 的同源展开，供 skill 直接读）。
- `preview-trust.md`：预览可信清单。

**tools/pack.mjs**：
- 定位 `LOOMGUI_ROOT`（init 注入时写入）。
- `cargo build -p loomgui_pkg --release`（若 `target/release/loomgui_pkg` 不存在或源码更新）。
- 调 `target/release/loomgui_pkg <html> <css> -o <out> -w 1080 -h 1920`。
- 透传 exit code + stderr（AI 据此自纠）。
- **不向设计师暴露 Rust 细节**：设计师/ AI 只见"跑 pack.mjs，成功产出 pkg.bin，失败报围栏错"。

### 3.4 `loomgui_pkg` 复用（不改动）

`loomgui_pkg` 现有 CLI（`loomgui_pkg/src/main.rs`）已是"验证+打包合一"：
- `pack_named()` 调用走 core parse → 围栏验证（FENCE_TAGS）→ 打包。违规 `pack:` 报错 + `ExitCode::FAILURE`。
- skill 的 `pack.mjs` 直接调此 CLI，**不改 loomgui_pkg 源码**。

**版本关联**：loomgui_pkg 改完后 `cargo build`，下次 `pack.mjs` 调用即新版。单一真相源，零复制同步。

---

## 4. `samples/` 合并 editor-test

### 4.1 合并决策

顶层 `samples/` 同时承担：
- **UI 样例家**：`v1-showcase/`（已验收基线，引擎无关 DSL 源）。
- **editor 测试场**：被 init 脚本注入规则+skill，open-design import 验证 editor 工作流。
- **测试夹具**：`design-systems/loomgui/`（暗色 dashboard，锁测试风格）。
- **AI 产出**：`ai-output/`（editor 工作流新生成，与基线对照）。

### 4.2 目录结构

```
samples/
├── v1-showcase/                     ← 提交，Unity 打包源（路径不变）
├── ai-output/                       ← 提交（测试产出可回归对照）
├── design-systems/loomgui/          ← 提交，测试夹具
│   ├── DESIGN.md                    ← 纯风格（暗色 dashboard，无围栏规则）
│   ├── tokens.css                   ← 暗色 token
│   └── components.html              ← 围栏内组件（flex/gap，非 grid/margin）
├── CLAUDE.md                        ← 【gitignore】init 生成
└── .claude/skills/loomgui-editor/   ← 【gitignore】init 生成
```

### 4.3 gitignore 精细处理

`.gitignore` 追加：
```
# editor init 生成物（samples/ 自测注入）
samples/CLAUDE.md
samples/.claude/
```
保留提交：`v1-showcase/`、`ai-output/`、`design-systems/loomgui/`。

### 4.4 design-system 夹具（仅测试）

- **DESIGN.md**：纯风格 prose，按 open-design 9-section 惯例写暗色 dashboard（色板/字体/组件/层级），**不含围栏规则**。open-design picker 选它后注入 system prompt，锁测试风格。
- **tokens.css**：暗色 token（`--bg/--fg/--accent/...`），值取自 v1-showcase 现有色板（`#1a1d2e` 等）。
- **components.html**：围栏内组件样例——**必须用 flex/gap，禁用 grid/margin 间距/:focus/@media**（与 open-design 默认 components.html 反面，是 LoomGUI 围栏正面教材）。Button/Card/List/ScrollPane/Text。

**此夹具不进 init 脚本注入物**（正式交付不提供 design-system），仅存在于 `samples/` 供测试。

---

## 5. 环境整理

### 5.1 `temp/open-design/` 隔离

`temp/open-design/` 是 open-design 源码（只读调研参考），未被 git 跟踪但**未在 .gitignore**，有误提交风险。

`.gitignore` 追加：
```
# open-design 源码（只读调研参考，不入库）
/temp/
```

### 5.2 LoomGUI 根 CLAUDE.md

LoomGUI 仓库根**没有 CLAUDE.md**。本 spec 不新建根 CLAUDE.md（LoomGUI 仓库自身的开发规则不在 v other 范围）。`editor/rules/claude/CLAUDE.md.tmpl` 是**注入给设计师工作区**的，与 LoomGUI 仓库自身的开发规则是两回事。

---

## 6. 验收标准

1. `node editor/init.mjs` 能交互式注入到一个空工作区，生成 `CLAUDE.md`（含标签包裹围栏规则）+ `.claude/skills/loomgui-editor/`。
2. 重复跑 init，`CLAUDE.md` 标签内规则被替换、标签外用户内容不丢。
3. open-design import 注入后的工作区，picker 能看到 `loomgui-editor` skill。
4. AI 在注入工作区生成 UI，跑 `pack.mjs`：
   - 围栏违规 HTML → 非零退出 + 报错，AI 能据错自纠。
   - 围栏合规 HTML → 产出 `.pkg.bin` + atlas。
5. `samples/` 自测：init 注入后 open-design import，选 `design-systems/loomgui` 夹具，AI 生成暗色 dashboard 进 `ai-output/`，与 `v1-showcase/` 风格一致、围栏合规、能打包。
6. `samples/v1-showcase/` Unity 打包路径不变，`loom_showcase.pkg.bin` 仍正常产出。

---

## 7. 未定 / 实现期查

- **opencode/codex 的 skill 发现路径**：实现期查各 harness 文档确认 skill 目录位置（claude 是 `.claude/skills/`，另两个待查）。
- **opencode/codex 的规则文件名**：opencode/codex 用 `AGENTS.md`（调研推测），实现期确认。
- **pack.mjs 的 cargo build 触发策略**：每次都 build（保证最新）vs 检测源码 mtime 增量 build。倾向后者（mtime 比 target 新才 build），实现期定。
- **design-system 夹具能否被 open-design picker 读到**：picker 读 `USER_DESIGN_SYSTEMS_DIR`（app 数据目录），`samples/design-systems/loomgui/` 在 project 内未必被读。**实测**：import `samples/` 后看 picker 出不出 LoomGUI；不出则把夹具也拷一份到 `data/design-systems/`（仅测试机操作，不入正式流程）。

---

## 8. 下一步

review 通过 → writing-plans 拆任务：
1. **写 `loomgui_core/tests/fence_contract.rs`**（围栏契约测试，单一真相源）：枚举支持项（断言生效）+ 围栏外项（断言静默忽略/不脱离流）。把 fence.md 标【推断·待测】的项转【实证】。优先锁定 `position:absolute`/`display:grid`/`float`/`@media` 等关键围栏外行为。
2. 建 `editor/` 骨架（init.mjs + rules 模板 + skill 模板）。
3. 写围栏规则副本（fence.md → `editor/skill/loomgui-editor/references/fence.md` + `editor/rules/claude/CLAUDE.md.tmpl`）。
4. 写 pack.mjs（调 loomgui_pkg，脚本目录定位仓库根 → cargo build）。
5. 建 `samples/design-systems/loomgui/` 夹具 + `samples/ai-output/`。
6. `.gitignore` 追加（/temp/ + samples 生成物）。
7. 同步 `docs/roadmap/v1-scope.md` §2（引用 fence.md，补 v1.x 扩展属性 filter/border-image-slice/:focus，纠正 position:relative/font-style）+ `docs/roadmap/roadmap.md` §2（1.3 三处修订）。
8. 自测：init 注入 samples/ → open-design import → 生成 → 打包 → 对照 v1-showcase。
