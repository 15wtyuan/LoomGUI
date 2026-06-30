# Editor 工作流 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让设计师在 open-design 桌面 app 里用 AI 生成 LoomGUI 围栏合规的 UI，经 loomgui_pkg 打包成 .pkg.bin 供 Unity 加载；并落地围栏权威清单 + 防漂移测试。

**Architecture:** 两条线：(1) 围栏契约——`loomgui_core/tests/fence_contract.rs` 可执行围栏真相源 + `docs/design/fence.md` 人类副本；(2) editor 工作流——`editor/` 模板源（init.mjs 注入脚本 + rules + skill），`samples/` 合并 editor 测试场。设计师跑 init.mjs 注入工作区 → open-design import → AI 生成 → skill 引导跑 pack.mjs（调 loomgui_pkg 验证+打包）→ Unity。

**Tech Stack:** Rust（loomgui_core 测试，taffy 0.5）、Node ESM（init.mjs / pack.mjs，零第三方依赖，Node 22+）、Markdown（fence.md / CLAUDE.md.tmpl / SKILL.md）、HTML+CSS（design-system 夹具）。

## Global Constraints

- **语言**：问答/总结中文，代码/commit 英文（用户只读中文）。
- **两台机串行**：本机唯一编码机（含 Rust build .dll + commit + push），家里机纯 Unity PlayMode 验收。改 Rust 后必重编 .dll 家里机才能测。
- **main 直推**：直接 push main，不建 feature 分支。
- **Ponytail full**：lazy senior dev，最小 diff，YAGNI，无未请求抽象，shortest working diff wins。
- **围栏单一真相源**：`loomgui_core/tests/fence_contract.rs` 是权威，`docs/design/fence.md` 是副本，不一致时测试赢。改 apply_decl/FENCE_TAGS/selector 必须同步测试 + fence.md。
- **Node 零第三方依赖**：init.mjs / pack.mjs 只用 `node:fs`/`node:path`/`node:readline`/`node:child_process`，不引 npm 包。
- **loomgui_pkg 不改源码**：复用现有 CLI（`loomgui_pkg/src/main.rs`），skill 的 pack.mjs 调它的二进制。
- **design-system 仅测试夹具**：`samples/design-systems/loomgui/` 不进 init 注入物，正式交付不提供 design-system。
- **fonts**：loomgui_core 测试用 `tests/fixtures/DejaVuSans.ttf`（仓库内，跨平台一致），ASCII 文本（无 CJK glyph）。
- **commit 规范**：每个 task 末尾 commit，消息英文，结尾不加 Co-authored-by（open-design 仓库规则不适用于本仓库，但本仓库历史 commit 无该 trailer，保持一致）。

---

## File Structure

```
loomgui_core/tests/fence_contract.rs    ← 新建：围栏契约测试（真相源）
docs/design/fence.md                     ← 已存在（brainstorm 期建），本 plan 不改
editor/
├── init.mjs                             ← 新建：Node 注入脚本
├── rules/
│   ├── claude/CLAUDE.md.tmpl            ← 新建：围栏规则模板（标签包裹）
│   ├── opencode/AGENTS.md.tmpl          ← 新建：同内容，opencode 用 AGENTS.md
│   └── codex/AGENTS.md.tmpl             ← 新建：同内容，codex 用 AGENTS.md
└── skill/loomgui-editor/
    ├── SKILL.md                         ← 新建：skill manifest + 工作流
    ├── references/
    │   ├── fence.md                     ← 新建：fence.md 副本（注入用）
    │   └── preview-trust.md             ← 新建：预览可信清单
    └── tools/
        └── pack.mjs                     ← 新建：调 loomgui_pkg（含 LOOMGUI_ROOT 占位符）
samples/
├── ai-output/.gitkeep                   ← 新建：AI 产出目录占位
└── design-systems/loomgui/
    ├── DESIGN.md                        ← 新建：纯风格（暗色 dashboard）
    ├── tokens.css                       ← 新建：暗色 token
    └── components.html                  ← 新建：围栏内组件样例（flex/gap）
.gitignore                               ← 改：追加 /temp/ + samples 生成物
docs/roadmap/v1-scope.md                 ← 已改（brainstorm 期），本 plan 验证
docs/roadmap/roadmap.md                  ← 已改（brainstorm 期），本 plan 验证
```

---

## Task 1: 围栏契约测试 fence_contract.rs（真相源）

**Files:**
- Create: `loomgui_core/tests/fence_contract.rs`

**Interfaces:**
- Consumes: `loomgui_core::parse::dom::parse_html(html: &str) -> Result<ElementTree, String>`、`loomgui_core::style::mapping::apply_decl(style: &mut ResolvedStyle, prop: &str, value: &str) -> bool`、`loomgui_core::style::resolved::ResolvedStyle`（实现 `Default`，字段 `taffy_style: TaffyStyle` 公开）、`loomgui_core::parse::css::parse_css(css: &str) -> Result<StyleSheet, String>`。
- Produces: `loomgui_core/tests/fence_contract.rs`（围栏权威测试，后续 apply_decl 改动必须过此测试）。

**背景**：fence.md 标【推断·待测】的围栏外 CSS 属性（position:absolute / display:grid / float / @media 等）当前无测试锁定行为。本 task 补测试，把它们转【实证】。测试分三类：(A) 元素围栏（围栏外报错）；(B) 支持属性生效（apply_decl 返回 true + 字段变化）；(C) 围栏外属性静默忽略（apply_decl 返回 false + 布局字段不变 / parse_css 不报错但无效）。

- [ ] **Step 1: 写元素围栏测试（A 类）**

创建 `loomgui_core/tests/fence_contract.rs`：

```rust
//! 围栏契约测试 = LoomGUI 围栏权威真相源（docs/design/fence.md 是人类副本）。
//! 三类断言：
//!   A. 元素围栏：围栏外标签报错（parse_html），白名单接受。
//!   B. 支持属性：apply_decl 返回 true + ResolvedStyle 字段变化。
//!   C. 围栏外属性：apply_decl 返回 false + 布局字段不变（静默忽略）。
//! 改 apply_decl / FENCE_TAGS / selector 必须同步本测试 + fence.md。

use loomgui_core::parse::dom::parse_html;
use loomgui_core::style::mapping::apply_decl;
use loomgui_core::style::resolved::ResolvedStyle;

// ── A. 元素围栏 ──────────────────────────────────────────────────

#[test]
fn fence_tags_whitelist_accepted() {
    // FENCE_TAGS = div/span/img/button/l-container，全部应接受。
    for tag in ["div", "span", "img", "button", "l-container"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_ok(), "<{tag}> 应被围栏接受");
    }
}

#[test]
fn fence_out_tags_rejected() {
    // 围栏外标签一律报错，不降级。
    for tag in ["video", "input", "b", "section", "p", "ul"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_err(), "<{tag}> 应被围栏拒绝");
    }
}
```

- [ ] **Step 2: 运行测试验证 A 类通过**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: 2 tests passed（A 类锁的是已实现行为，应直接绿）。

- [ ] **Step 3: 写支持属性测试（B 类）**

追加到 `fence_contract.rs`：

```rust
// ── B. 支持属性生效（apply_decl 返回 true）──────────────────────

#[test]
fn supported_layout_props_return_true() {
    let cases = [
        ("display", "flex"),
        ("flex-direction", "row"),
        ("flex-wrap", "wrap"),
        ("gap", "10px"),
        ("justify-content", "center"),
        ("align-items", "center"),
        ("width", "100px"),
        ("padding", "8px"),
        ("margin", "4px"),
        ("aspect-ratio", "1.5"),
        ("order", "2"),
    ];
    for (prop, val) in cases {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, prop, val), "支持属性 {prop}:{val} 应返回 true");
    }
}

#[test]
fn supported_visual_props_return_true() {
    let cases = [
        ("background-color", "#5fb2c4"),
        ("background-image", "url(\"a.png\")"),
        ("background-size", "cover"),
        ("border-radius", "4px"),
        ("opacity", "0.5"),
        ("overflow", "hidden"),
        ("color", "#e0e0e0"),
        ("font-size", "16px"),
        ("font-weight", "700"),
        ("text-align", "center"),
        ("white-space", "nowrap"),
        ("transform", "rotate(45deg)"),
        ("pointer-events", "none"),
        ("filter", "grayscale(1)"),
        ("border-image-slice", "10"),
    ];
    for (prop, val) in cases {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, prop, val), "支持属性 {prop}:{val} 应返回 true");
    }
}

#[test]
fn background_size_rejects_two_values() {
    // background-size 只认 cover/contain/100%，拒两值如 "100% 50%"。
    let mut s = ResolvedStyle::default();
    assert!(!apply_decl(&mut s, "background-size", "100% 50%"),
        "background-size 两值应被拒（返回 false）");
}

#[test]
fn display_grid_falls_to_flex() {
    // display:grid 走 mapping.rs 非 none 分支 → Flex，返回 true。
    // taffy 无 grid，grid 写了等于 flex，AI 不可预测 → fence.md 标"禁写 grid"。
    let mut s = ResolvedStyle::default();
    let ok = apply_decl(&mut s, "display", "grid");
    assert!(ok, "display:grid 走非 none 分支返回 true（落 Flex）");
}
```

- [ ] **Step 4: 运行测试验证 B 类通过**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: 6 tests passed（A 2 + B 4）。

- [ ] **Step 5: 写围栏外属性静默忽略测试（C 类，关键）**

追加到 `fence_contract.rs`：

```rust
// ── C. 围栏外属性静默忽略（apply_decl 返回 false，布局字段不变）─────
// fence.md §2.4 / §3.3 标【推断·待测】转【实证】的关键项。
// AI 写了以为生效、实际无效 = 不可预测，围栏禁写，测试锁定"无效"行为。

#[test]
fn fence_out_props_return_false() {
    let cases: [(&str, &str); 10] = [
        ("position", "absolute"),
        ("float", "left"),
        ("align-content", "center"),
        ("cursor", "pointer"),
        ("clip-path", "circle(50%)"),
        ("background-position", "center"),
        ("background-repeat", "no-repeat"),
        ("transform-origin", "top left"),
        ("font-style", "italic"),
        ("border-style", "dashed"),
    ];
    for (prop, val) in cases {
        let mut s = ResolvedStyle::default();
        assert!(!apply_decl(&mut s, prop, val),
            "围栏外属性 {prop}:{val} 应返回 false（静默忽略）");
    }
}

#[test]
fn position_absolute_does_not_break_flow() {
    // position:absolute 写了不生效：apply_decl 返回 false，
    // taffy_style.position 保持默认 Relative（不脱离流）。
    // fence.md §0 纠正的"无 match ≠ 不支持"反例的核心锁定。
    let mut s = ResolvedStyle::default();
    let before = s.taffy_style.position;
    let applied = apply_decl(&mut s, "position", "absolute");
    assert!(!applied, "position:absolute 应返回 false（围栏外）");
    assert_eq!(s.taffy_style.position, before,
        "position 字段不变（保持默认 Relative，不脱离流）");
}

#[test]
fn transform_skew_does_not_apply() {
    // transform 只认 translate/rotate/scale，skew 显式跳过（mapping.rs:278）。
    // apply_decl("transform",...) 返回 true（进 match arm），但 transform 字段无变化。
    let mut s1 = ResolvedStyle::default();
    apply_decl(&mut s1, "transform", "skew(10deg,5deg)");
    let s2 = ResolvedStyle::default();
    assert_eq!(s1.transform, s2.transform, "skew 不应改变 transform 字段");
}

#[test]
fn at_rule_media_skipped_by_parser() {
    // @media 被 AtRuleParser 默认拒（parse/css.rs:58-63），整块跳过不报错。
    use loomgui_core::parse::css::parse_css;
    let css = "@media (min-width: 600px) { .a { width: 100px; } }";
    let sheet = parse_css(css).expect("parse_css 不应 panic");
    // @media 块被跳过，sheet 里无 .a 规则。
    assert!(sheet.rules.is_empty(), "@media 块应被跳过，规则不进 StyleSheet");
}
```

- [ ] **Step 6: 运行测试，根据实际行为修正断言**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: 部分可能 fail——【推断·待测】项的实际行为可能和断言不符（这正是要锁定的）。

**处理 fail 原则**（目标是锁定实际行为，不是强加推测）：
- `position_absolute_does_not_break_flow`：若 `s.taffy_style.position` 路径或默认值不对，查 `loomgui_core/src/style/resolved.rs` 的 `ResolvedStyle::default` 实际构造，修正断言到实际默认值。可能需 `use taffy::style::Position;` 比较。
- `at_rule_media_skipped_by_parser`：若 `sheet.rules.is_empty()` 不成立（@media 内规则可能被解析进 rules），查 `parse_css` 实际行为，改成断言"无规则匹配 `.a`"或检查 rules 数量。先 `eprintln!("{:?}", sheet.rules)` 观察实际。
- `transform_skew_does_not_apply`：若 `s.transform` 字段名不对，查 `ResolvedStyle` 的 transform 字段名（可能叫 `local_transform` 或在 `taffy_style` 外），修正。
- 任何 fail：先读对应源码确认实际行为，再改断言。**最终断言必须反映代码实际**。

修正后重跑直到全绿。

- [ ] **Step 7: 全量测试 + commit**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: 全部 passed。

```bash
git add loomgui_core/tests/fence_contract.rs
git commit -m "test(core): fence_contract 围栏契约测试 — 元素/支持/围栏外三类断言锁定"
```

---

## Task 2: 围栏规则副本 + rules 模板（注入物内容源）

**Files:**
- Create: `editor/skill/loomgui-editor/references/fence.md`
- Create: `editor/skill/loomgui-editor/references/preview-trust.md`
- Create: `editor/rules/claude/CLAUDE.md.tmpl`
- Create: `editor/rules/opencode/AGENTS.md.tmpl`
- Create: `editor/rules/codex/AGENTS.md.tmpl`

**Interfaces:**
- Consumes: `docs/design/fence.md`（权威源，brainstorm 期已建）。
- Produces: 围栏规则副本（注入设计师工作区用），三个 harness 的规则模板。

**背景**：fence.md 是项目内权威，但注入给设计师工作区时要一份自包含副本（设计师工作区没有 LoomGUI 仓库的 docs）。CLAUDE.md.tmpl / AGENTS.md.tmpl 是 init.mjs 注入时拷贝的规则文件，带标签包裹（增量合并不覆盖用户已有）。

- [ ] **Step 1: 拷贝 fence.md 为 skill references 副本**

`editor/skill/loomgui-editor/references/fence.md` = `docs/design/fence.md` 的完整副本（含 §1-§6）。直接复制内容，不改。这是注入给设计师工作区的围栏权威副本，skill 的 SKILL.md 会引用它。

```bash
cp docs/design/fence.md editor/skill/loomgui-editor/references/fence.md
```

（若 `editor/skill/loomgui-editor/references/` 目录不存在，先 `mkdir -p`。）

- [ ] **Step 2: 写 preview-trust.md**

创建 `editor/skill/loomgui-editor/references/preview-trust.md`（内容 = fence.md §6 搬迁，自包含）：

```markdown
# 预览可信清单

open-design Chromium iframe 预览 ≠ taffy 渲染。AI 须分清：

## 可信（Chrome ≈ LoomGUI）
flex 轴/方向、显式 `display:flex`、`gap` 间距、颜色、opacity、border、图片、px 尺寸、`background-image`/`background-size`（标准 CSS，Chrome 原生）。

## 不可信（Chrome ≠ LoomGUI，别按预览调）
- **margin 控间距**：Chrome（block flow）折叠 margin、LoomGUI（flex）求和不折叠。**子项间距用 `gap`**，别用 margin。
- **文本换行/像素级**：Chrome 文本引擎 vs LoomGUI（unicode-linebreak），换行点/塞文本宽度会偏。
- **`position:absolute`**：Chrome 脱离流、LoomGUI 不脱离（围栏外静默忽略）。预览会骗 AI。
- **`display:grid`**：Chrome 渲染 grid、LoomGUI 落 Flex。预览会骗 AI。
- **`@media` 响应式**：Chrome 响应、LoomGUI 用参考分辨率缩放不响应 @media。

## 口径
不可信项"信围栏规则，别信预览"。
```

- [ ] **Step 3: 写 claude/CLAUDE.md.tmpl**

创建 `editor/rules/claude/CLAUDE.md.tmpl`。这是注入到设计师工作区 `CLAUDE.md` 的围栏规则，用标签包裹（init.mjs 识别标签做增量合并）。内容精简版围栏（完整版见 references/fence.md）：

```markdown
<!-- loomgui-editor-begin -->
# LoomGUI 围栏规则（硬约束）

生成 HTML+CSS 时严守以下围栏。围栏外写法写了不报错但**不生效**（静默忽略），会导致预览与 Unity 渲染不一致 = 不可预测。完整规则见 skill references/fence.md。

## 元素白名单
只用 `div` / `span`（+裸文本）/ `img` / `button` / `l-container`。其他标签（video/input/p/ul/...）会报错。

## CSS 布局
- `display:flex/none`（**禁 grid**，写了落 Flex 预览会骗你）
- `flex-direction` / `flex-wrap` / `gap` / `row-gap` / `column-gap` / `justify-content` / `align-items` / `align-self` / `flex`(grow/shrink/basis) / `order` / `aspect-ratio`
- `width/height/min/max`(px/%/auto) / `padding` / `margin` / `border-width`
- **子项间距用 `gap`，别用 margin**（Chrome 折叠 margin、LoomGUI 求和不折叠）
- 禁：`position:absolute/fixed/sticky`（写了不脱离流）、`float`、`align-content`

## CSS 视觉
- `background-color` / `background-image`(url) / `background-size`(cover/contain/100%，拒两值)
- `border-radius` / `border`(简写只取宽度) / `border-color` / `opacity`
- `overflow` / `overflow-x` / `overflow-y`
- `color` / `font-size`(px) / `font-family` / `font-weight` / `text-align` / `line-height` / `letter-spacing` / `white-space:nowrap`
- `transform`(translate/rotate/scale，禁 skew/matrix) / `pointer-events`
- `filter`(grayscale/brightness/contrast/saturate/hue-rotate/invert/sepia) / `border-image-slice`(九宫格)
- 禁：`clip-path` / `background-position` / `background-repeat` / `transform-origin` / `font-style` / `cursor`

## 交互/选择器
- 伪类：`:hover` / `:active` / `:disabled` / `:focus`
- 选择器：标签/类/id/后代/子代/分组。禁 `+`/`~`/`*`/属性选择器/`:nth-child`/`:not()`

## position:relative
靠 taffy 默认生效，写不写行为一致（无 inset 偏移）。可写可不写。

## 预览可信清单
信 flex/gap/color/px/background-image；**不信** margin 折叠/文本换行像素/position:absolute/display:grid/@media。口径"信围栏规则别信预览"。详见 references/preview-trust.md。

## 生成完必须跑验证
生成 HTML+CSS 后，跑 `tools/pack.mjs <html> <css> -o <out.pkg.bin>` 验证+打包。非零退出 = 围栏违规，读 stderr 自纠后重跑。
<!-- loomgui-editor-end -->
```

- [ ] **Step 4: 写 opencode + codex 的 AGENTS.md.tmpl**

`editor/rules/opencode/AGENTS.md.tmpl` 和 `editor/rules/codex/AGENTS.md.tmpl` 内容**与 claude/CLAUDE.md.tmpl 完全相同**（围栏规则 harness 无关，只是文件名不同：opencode/codex 用 AGENTS.md）。直接复制：

```bash
cp editor/rules/claude/CLAUDE.md.tmpl editor/rules/opencode/AGENTS.md.tmpl
mkdir -p editor/rules/codex
cp editor/rules/claude/CLAUDE.md.tmpl editor/rules/codex/AGENTS.md.tmpl
```

- [ ] **Step 5: 验证文件结构 + commit**

Run: `ls -R editor/rules/ editor/skill/loomgui-editor/references/`
Expected: 看到 claude/CLAUDE.md.tmpl、opencode/AGENTS.md.tmpl、codex/AGENTS.md.tmpl、references/fence.md、references/preview-trust.md。

```bash
git add editor/
git commit -m "feat(editor): 围栏规则副本 + rules 模板（claude/opencode/codex 三 harness）"
```

---

## Task 3: SKILL.md + pack.mjs（skill 主体）

**Files:**
- Create: `editor/skill/loomgui-editor/SKILL.md`
- Create: `editor/skill/loomgui-editor/tools/pack.mjs`

**Interfaces:**
- Consumes: Task 2 的 references/fence.md、references/preview-trust.md；`loomgui_pkg` CLI（`loomgui_pkg/src/main.rs`，用法 `loomgui_pkg <html> <css> [-o out.pkg.bin] [-w W] [-h H] [-a atlas.png]`）。
- Produces: skill manifest（SKILL.md，open-design picker 发现）+ pack.mjs（调 loomgui_pkg 验证+打包）。
- pack.mjs 的 `LOOMGUI_ROOT` 占位符 `__LOOMGUI_ROOT__` 由 init.mjs 注入时替换（Task 4）。

**背景**：skill 封装 loomgui_pkg，设计师/AI 只见"跑 pack.mjs，成功产出 pkg.bin，失败报围栏错"。pack.mjs 定位 LoomGUI 仓库根（init.mjs 注入时写入绝对路径）→ cargo build → 调 CLI。

- [ ] **Step 1: 写 SKILL.md**

创建 `editor/skill/loomgui-editor/SKILL.md`：

```markdown
---
name: loomgui-editor
description: |
  Generate LoomGUI fence-compliant UI (HTML+CSS) for game dashboards/panels.
  Uses flex-only layout, tag whitelist (div/span/img/button/l-container), no grid/absolute/margin-spacing.
  After generating, run tools/pack.mjs to validate + pack into .pkg.bin for Unity.
triggers:
  - "loomgui ui"
  - "game dashboard"
  - "游戏 UI 面板"
  - "游戏界面"
---

# LoomGUI Editor

生成 LoomGUI 围栏合规的游戏 UI（HTML+CSS），打包成 .pkg.bin 供 Unity 加载。

## 工作流

1. **读围栏规则**：读 `references/fence.md`（围栏硬约束）+ `references/preview-trust.md`（预览可信清单）。围栏是硬约束，违反会导致预览与 Unity 渲染不一致。

2. **按设计师 prompt 生成 HTML+CSS**：
   - 元素只用 `div`/`span`/`img`/`button`/`l-container`。
   - 布局用 flex + `gap`（子项间距用 gap 不用 margin）。
   - 禁 grid/absolute/float/@media/skew 等（详见 fence.md）。
   - 风格由设计师 prompt 决定（颜色/字号/字体自由，只要守围栏）。

3. **生成完跑验证+打包**：
   ```bash
   node tools/pack.mjs <html路径> <css路径> -o <输出.pkg.bin> [-w 1080 -h 1920]
   ```
   - **非零退出 = 围栏违规**（loomgui_pkg 报错）。读 stderr，自纠 HTML/CSS 后重跑。
   - **零退出 = 合规**，.pkg.bin + atlas.png 已产出到指定目录。

4. **报告**：向设计师报告产出路径（.pkg.bin + atlas.png），说明 Unity 加载方式（StreamingAssets/ 下，LoomStage 自动加载）。

## 注意

- **预览不可信项**：open-design 预览是 Chromium iframe，与 LoomGUI（taffy）有分歧。margin 折叠/文本换行/position:absolute/display:grid/@media 别按预览调。详见 references/preview-trust.md。
- **打包器即验证器**：pack.mjs 调的 loomgui_pkg 内含围栏验证（FENCE_TAGS + apply_decl），违规打包期报错。不需要单独的 lint 步骤。
```

- [ ] **Step 2: 写 pack.mjs**

创建 `editor/skill/loomgui-editor/tools/pack.mjs`。`__LOOMGUI_ROOT__` 是占位符，init.mjs 注入时替换成实际仓库根绝对路径。

```javascript
#!/usr/bin/env node
// pack.mjs — 调 loomgui_pkg 验证+打包。封装层，设计师/AI 只见"成功产出 pkg.bin / 失败报围栏错"。
// LOOMGUI_ROOT 由 init.mjs 注入时替换 __LOOMGUI_ROOT__ 占位符。
// 用法：node pack.mjs <html> <css> -o <out.pkg.bin> [-w 1080 -h 1920] [-a atlas.png]

import { execFileSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { join } from "node:path";

const LOOMGUI_ROOT = "__LOOMGUI_ROOT__"; // init.mjs 替换

// 解析命令行参数（与 loomgui_pkg CLI 对齐）。
const args = process.argv.slice(2);
if (args.length < 2) {
  console.error("usage: node pack.mjs <html> <css> -o <out.pkg.bin> [-w 1080] [-h 1920] [-a atlas.png]");
  process.exit(2);
}
const html = args[0];
const css = args[1];
const pkgArgs = [html, css];
for (let i = 2; i < args.length; i++) {
  if (args[i] === "-o" || args[i] === "-w" || args[i] === "-h" || args[i] === "-a" || args[i] === "--atlas-name") {
    pkgArgs.push(args[i], args[i + 1]);
    i++;
  } else {
    console.error(`unknown arg: ${args[i]}`);
    process.exit(2);
  }
}

// 定位 loomgui_pkg 二进制：优先 target/release，不存在或源码更新则 cargo build。
const binPath = join(LOOMGUI_ROOT, "target", "release", "loomgui_pkg" + (process.platform === "win32" ? ".exe" : ""));
const cargoToml = join(LOOMGUI_ROOT, "loomgui_pkg", "Cargo.toml");

function needBuild() {
  if (!existsSync(binPath)) return true;
  // 源码 mtime 比 二进制新 → 重新 build。
  const binMtime = statSync(binPath).mtimeMs;
  for (const src of [join(LOOMGUI_ROOT, "loomgui_pkg", "src", "main.rs"), join(LOOMGUI_ROOT, "loomgui_pkg", "src", "lib.rs"), cargoToml]) {
    if (existsSync(src) && statSync(src).mtimeMs > binMtime) return true;
  }
  return false;
}

if (needBuild()) {
  process.stderr.write("[pack] building loomgui_pkg (release)...\n");
  try {
    execFileSync("cargo", ["build", "-p", "loomgui_pkg", "--release"], { cwd: LOOMGUI_ROOT, stdio: "inherit" });
  } catch (e) {
    console.error("[pack] cargo build failed");
    process.exit(1);
  }
}

// 调 loomgui_pkg CLI，透传 stdout/stderr/exit code。
// 违规 → loomgui_pkg 非零退出 + stderr 报围栏错，AI 据此自纠。
try {
  execFileSync(binPath, pkgArgs, { stdio: "inherit" });
} catch (e) {
  // 非零退出：围栏违规或打包失败。透传已由 stdio:inherit 完成。
  process.exit(e.status ?? 1);
}
```

- [ ] **Step 3: 手动验证 pack.mjs 能跑（用 v1-showcase）**

先手动替换占位符验证逻辑（init.mjs 还没写，先手测）：

```bash
# 临时替换占位符跑一次（验证 cargo build + CLI 调用链）
node -e "const fs=require('fs');const p='editor/skill/loomgui-editor/tools/pack.mjs';let s=fs.readFileSync(p,'utf8');s=s.replace('__LOOMGUI_ROOT__', process.cwd());fs.writeFileSync('/tmp/pack-test.mjs', s);"
node /tmp/pack-test.mjs samples/v1-showcase/index.html samples/v1-showcase/style.css -o /tmp/test.pkg.bin -a loom_showcase.atlas.png
```
Expected: `[pack] building loomgui_pkg (release)...` → cargo build → `wrote /tmp/test.pkg.bin (N bytes) + atlas ...`。退出码 0。

若失败：
- cargo build 失败 → 查 loomgui_pkg 能否独立 build（`cargo build -p loomgui_pkg --release`）。
- 路径错 → 确认 `process.cwd()` 是 LoomGUI 仓库根。

**验证后还原** pack.mjs 的占位符（不要提交替换后的版本）：
```bash
git checkout editor/skill/loomgui-editor/tools/pack.mjs
```

- [ ] **Step 4: commit**

```bash
git add editor/skill/loomgui-editor/SKILL.md editor/skill/loomgui-editor/tools/pack.mjs
git commit -m "feat(editor): SKILL.md + pack.mjs（封装 loomgui_pkg 验证+打包）"
```

---

## Task 4: init.mjs（注入脚本）

**Files:**
- Create: `editor/init.mjs`

**Interfaces:**
- Consumes: Task 2 的 `editor/rules/<harness>/*.tmpl`、Task 3 的 `editor/skill/loomgui-editor/`（含 pack.mjs 的 `__LOOMGUI_ROOT__` 占位符）。
- Produces: 注入后的目标工作区（CLAUDE.md/AGENTS.md 增量合并 + .claude/skills/loomgui-editor/ 拷贝 + pack.mjs 占位符替换）。

**背景**：设计师跑 `node editor/init.mjs`，交互输工作区/输出路径/harness，脚本把围栏规则+skill 拷进目标工作区。LOOMGUI_ROOT = 脚本所在目录上一层（editor/ 在仓库根下）。CLAUDE.md 增量合并用标签包裹（`<!-- loomgui-editor-begin -->..end -->`），不覆盖用户已有内容。

- [ ] **Step 1: 写 init.mjs**

创建 `editor/init.mjs`：

```javascript
#!/usr/bin/env node
// init.mjs — 把 LoomGUI 围栏规则 + skill 注入设计师工作区。
// 交互输入：工作区路径 / 输出路径 / harness（claude/opencode/codex）。
// 零第三方依赖：只用 node:fs / node:path / node:readline。

import { createInterface } from "node:readline/promises";
import { stdin as input, stdout as output } from "node:process";
import {
  existsSync, mkdirSync, readFileSync, writeFileSync, readdirSync, copyFileSync, statSync,
} from "node:fs";
import { join, resolve, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
// LOOMGUI_ROOT = editor/ 的上一层 = 仓库根。
const LOOMGUI_ROOT = resolve(__dirname, "..");

const BEGIN = "<!-- loomgui-editor-begin -->";
const END = "<!-- loomgui-editor-end -->";

// harness → (规则文件名, skill 目录)
const HARNESS = {
  claude: { ruleFile: "CLAUDE.md", ruleDir: join(__dirname, "rules", "claude") },
  opencode: { ruleFile: "AGENTS.md", ruleDir: join(__dirname, "rules", "opencode") },
  codex: { ruleFile: "AGENTS.md", ruleDir: join(__dirname, "rules", "codex") },
};

function ask(rl, q) { return rl.question(q); }

// 增量合并规则文件：无则新建，有则替换标签段（保留用户原有内容）。
function mergeRuleFile(targetPath, tmplContent) {
  const block = `${BEGIN}\n${tmplContent.replace(/^<!-- loomgui-editor-begin -->\n?/, "").replace(/\n?<!-- loomgui-editor-end -->\s*$/, "")}\n${END}\n`;
  // tmplContent 本身已含标签，直接用 tmplContent 作 block。
  const tagged = tmplContent.includes(BEGIN) ? tmplContent : `${BEGIN}\n${tmplContent}\n${END}\n`;
  if (!existsSync(targetPath)) {
    writeFileSync(targetPath, tagged, "utf8");
    return "created";
  }
  const existing = readFileSync(targetPath, "utf8");
  if (!existing.includes(BEGIN)) {
    // 无标签：追加。
    writeFileSync(targetPath, existing.replace(/\n*$/, "\n\n") + tagged, "utf8");
    return "appended";
  }
  // 有标签：替换标签段。
  const re = new RegExp(`${BEGIN}[\\s\\S]*?${END}`, "g");
  const updated = existing.replace(re, tagged.trimEnd());
  writeFileSync(targetPath, updated, "utf8");
  return "updated";
}

// 递归拷贝 skill 目录，pack.mjs 的 __LOOMGUI_ROOT__ 占位符替换成实际路径。
function copySkill(srcDir, destDir) {
  mkdirSync(destDir, { recursive: true });
  for (const entry of readdirSync(srcDir)) {
    const srcPath = join(srcDir, entry);
    const destPath = join(destDir, entry);
    if (statSync(srcPath).isDirectory()) {
      copySkill(srcPath, destPath);
    } else {
      let content = readFileSync(srcPath, "utf8");
      if (entry === "pack.mjs") {
        content = content.replaceAll("__LOOMGUI_ROOT__", LOOMGUI_ROOT.replaceAll("\\", "/"));
      }
      writeFileSync(destPath, content, "utf8");
    }
  }
}

async function main() {
  const rl = createInterface({ input, output });

  const workspace = resolve(await ask(rl, "目标工作区路径（绝对路径）: "));
  const outputDir = resolve(await ask(rl, "pkg.bin 输出目录（绝对路径，如 Unity StreamingAssets）: "));
  console.log("harness 选项: claude / opencode / codex");
  const harness = (await ask(rl, "选择 harness: ")).trim();
  rl.close();

  if (!HARNESS[harness]) {
    console.error(`未知 harness: ${harness}（支持 claude/opencode/codex）`);
    process.exit(2);
  }
  if (!existsSync(workspace)) {
    console.error(`工作区不存在: ${workspace}`);
    process.exit(2);
  }
  const { ruleFile, ruleDir } = HARNESS[harness];
  const tmplPath = join(ruleDir, `${ruleFile}.tmpl`);
  if (!existsSync(tmplPath)) {
    console.error(`规则模板不存在: ${tmplPath}`);
    process.exit(2);
  }

  // ① 注入围栏规则（增量合并）。
  const tmplContent = readFileSync(tmplPath, "utf8");
  const ruleTarget = join(workspace, ruleFile);
  const action = mergeRuleFile(ruleTarget, tmplContent);
  console.log(`[init] ${ruleFile}: ${action}（${ruleTarget}）`);

  // ② 拷贝 skill（pack.mjs 占位符替换）。
  const skillSrc = join(__dirname, "skill", "loomgui-editor");
  // harness 的 skill 发现路径：claude=.claude/skills/，opencode/codex 暂同（实现期查文档，先放 .claude/skills）。
  const skillDest = join(workspace, ".claude", "skills", "loomgui-editor");
  copySkill(skillSrc, skillDest);
  console.log(`[init] skill 注入: ${skillDest}`);

  // ③ 写 outputDir 提示到工作区（skill 需要知道 pkg.bin 落哪）。
  const cfgPath = join(workspace, ".claude", "skills", "loomgui-editor", "config.json");
  mkdirSync(dirname(cfgPath), { recursive: true });
  writeFileSync(cfgPath, JSON.stringify({ output_dir: outputDir, loomgui_root: LOOMGUI_ROOT }, null, 2), "utf8");
  console.log(`[init] config.json: ${cfgPath}（output_dir=${outputDir}）`);

  console.log("\n完成。接下来：");
  console.log(`  1. open-design import 工作区: ${workspace}`);
  console.log(`  2. 在 open-design 里用 AI 生成 UI，skill 会引导跑 pack.mjs 验证+打包`);
  console.log(`  3. pkg.bin 产出到: ${outputDir}`);
}

main().catch((e) => { console.error(e); process.exit(1); });
```

- [ ] **Step 2: 手动测试 init.mjs（注入到临时空目录）**

```bash
mkdir -p /tmp/init-test-ws
echo "# my existing rule" > /tmp/init-test-ws/CLAUDE.md
echo "claude" | node editor/init.mjs 2>&1 | head -20
# 交互输入：工作区=/tmp/init-test-ws，输出=/tmp/init-out，harness=claude
```

实际跑时交互输入三项。或用 printf 喂入：
```bash
printf "/tmp/init-test-ws\n/tmp/init-out\nclaude\n" | node editor/init.mjs
```
Expected:
- `[init] CLAUDE.md: appended（/tmp/init-test-ws/CLAUDE.md）`（已有文件无标签 → 追加）
- `[init] skill 注入: /tmp/init-test-ws/.claude/skills/loomgui-editor`
- `[init] config.json: ...`

验证：
```bash
cat /tmp/init-test-ws/CLAUDE.md
# 应看到原有 "# my existing rule" + 标签包裹的围栏规则
grep "LOOMGUI_ROOT" /tmp/init-test-ws/.claude/skills/loomgui-editor/tools/pack.mjs
# 应看到实际仓库根路径（占位符已替换，不再是 __LOOMGUI_ROOT__）
```

- [ ] **Step 3: 测试重复跑 init（标签替换不丢用户内容）**

```bash
# 改一下用户原有内容
echo "# my updated rule" > /tmp/init-test-ws/CLAUDE.md  # 注意：这会丢标签，模拟首次
printf "/tmp/init-test-ws\n/tmp/init-out\nclaude\n" | node editor/init.mjs
# 第一次：appended
printf "/tmp/init-test-ws\n/tmp/init-out\nclaude\n" | node editor/init.mjs
# 第二次：updated（标签段替换，用户内容保留）
head -3 /tmp/init-test-ws/CLAUDE.md
# 应仍是 "# my updated rule"（用户内容未丢）
grep -c "loomgui-editor-begin" /tmp/init-test-ws/CLAUDE.md
# 应为 1（标签段只有一个，不重复）
```

- [ ] **Step 4: 清理临时目录 + commit**

```bash
rm -rf /tmp/init-test-ws /tmp/init-out /tmp/pack-test.mjs
git add editor/init.mjs
git commit -m "feat(editor): init.mjs 注入脚本 — 交互输工作区/输出/harness + 增量合并规则"
```

---

## Task 5: samples design-system 夹具（测试用，锁风格）

**Files:**
- Create: `samples/design-systems/loomgui/DESIGN.md`
- Create: `samples/design-systems/loomgui/tokens.css`
- Create: `samples/design-systems/loomgui/components.html`
- Create: `samples/ai-output/.gitkeep`

**Interfaces:**
- Consumes: v1-showcase 色板（bg `#1a1d2e` / surface `#252839` / border `#3a3f55` / fg `#e0e0e0` / muted `#9aa0b4` / accent `#5fb2c4` / ok `#6fa66c` / warn `#c2605a`）。
- Produces: 测试夹具（暗色 dashboard 风格，仅 `samples/` 用，不进 init 注入物）。

**背景**：B 方案不提供正式 design-system，但测试要锁风格免得 AI 乱跑。此夹具只在 `samples/`，DESIGN.md 纯风格（无围栏规则），components.html 必须是围栏正面教材（flex/gap，禁 grid/margin 间距/:focus/@media）。

- [ ] **Step 1: 写 DESIGN.md（纯风格）**

创建 `samples/design-systems/loomgui/DESIGN.md`：

```markdown
# LoomGUI Dashboard

> Category: Game UI
> 暗色游戏 dashboard 风格。用于 editor 工作流测试（锁风格，非正式交付物）。

## Visual Theme & Atmosphere
暗色、信息密集、状态色点缀。游戏 HUD/背包/设置面板基调。冷静的深蓝灰底 + 青色强调。

## Color Palette & Roles
- **Background:** `#1a1d2e`（深蓝灰底）
- **Surface:** `#252839`（卡片/面板）
- **Border:** `#3a3f55`（分隔线/边框）
- **Foreground:** `#e0e0e0`（主文本）
- **Muted:** `#9aa0b4`（次文本）
- **Accent:** `#5fb2c4`（青色强调，标题/激活态）
- **Success:** `#6fa66c` / **Warn:** `#c2605a` / **Dim:** `#6c7080`

## Typography Rules
- 字体：项目 ttf（LoomGUI 用包声明字体，预览 fallback sans-serif）。
- Scale (px): 14 · 16 · 18 · 22 · 28 · 36。
- 标题 font-weight 700，正文 400。

## Component Stylings
- **Button:** surface 底 + border，hover 变 accent 边框/字色，active scale(0.96)。
- **Card:** surface 底 + border + padding 16px + gap 12px。
- **Lamp/状态点:** 14×14 小方块，ok/warn/dim/acc 色。

## Layout Principles
- flex column 堆叠为主，row 用于按钮组/状态行。
- 间距用 `gap`，不用 margin。
- 设计稿 1080×1920（竖屏），参考分辨率缩放。

## Do's and Don'ts
- ✅ flex + gap 布局。
- ✅ 状态色点缀（accent/success/warn）。
- ❌ 不用 grid/absolute/float（围栏外）。
- ❌ 不用 margin 控子项间距（用 gap）。

## Agent Prompt Guide
生成暗色游戏 dashboard 面板：标题 + 卡片列表 + 状态点。守 LoomGUI 围栏（见 skill references/fence.md）。
```

- [ ] **Step 2: 写 tokens.css**

创建 `samples/design-systems/loomgui/tokens.css`：

```css
/* LoomGUI Dashboard 暗色 token（测试夹具，值取自 v1-showcase 色板）。
   AI 引用时 var(--bg) 等；也可直接写 hex（LoomGUI 不强制用 var）。 */
:root {
  --bg: #1a1d2e;
  --surface: #252839;
  --border: #3a3f55;
  --fg: #e0e0e0;
  --muted: #9aa0b4;
  --accent: #5fb2c4;
  --success: #6fa66c;
  --warn: #c2605a;
  --dim: #6c7080;

  --text-sm: 14px;
  --text-base: 16px;
  --text-lg: 18px;
  --text-xl: 22px;
  --text-2xl: 28px;
  --text-3xl: 36px;

  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 20px;
  --space-6: 24px;

  --radius-sm: 4px;
  --radius-md: 8px;
}
```

- [ ] **Step 3: 写 components.html（围栏正面教材）**

创建 `samples/design-systems/loomgui/components.html`。**关键**：用 flex/gap，禁 grid/margin 间距/:focus/@media（与 open-design 默认 components.html 反面）。

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>LoomGUI Dashboard — 围栏组件样例</title>
<style>
  /* 围栏正面教材：flex + gap，禁 grid/margin 间距/:focus/@media。
     本文件本身也是 LoomGUI 围栏合规的 HTML/CSS 样例。 */
  .root { width: 1080px; height: 1920px; background-color: #1a1d2e; display: flex; flex-direction: column; gap: 16px; padding: 24px; }
  .header { background-color: #252839; border: 1px solid #3a3f55; padding: 20px; display: flex; flex-direction: row; gap: 14px; align-items: center; }
  .title { color: #e0e0e0; font-size: 36px; font-weight: 700; }
  .card { background-color: #252839; border: 1px solid #3a3f55; padding: 16px; display: flex; flex-direction: column; gap: 12px; }
  .card-t { color: #e0e0e0; font-size: 22px; font-weight: 700; }
  .card-x { color: #9aa0b4; font-size: 14px; }
  .btn { background-color: #2d3148; border: 1px solid #3a3f55; padding: 10px 16px; color: #e0e0e0; font-size: 18px; }
  .btn:hover { background-color: #3a4258; color: #5fb2c4; border: 1px solid #5fb2c4; }
  .btn:active { background-color: #5fb2c4; color: #1a1d2e; }
  .lamps { display: flex; flex-direction: row; gap: 6px; }
  .lamp { width: 14px; height: 14px; background-color: #3a3f55; }
  .ok { background-color: #6fa66c; }
  .warn { background-color: #c2605a; }
  .acc { background-color: #5fb2c4; }
</style>
</head>
<body>
<div class="root">
  <div class="header">
    <div class="title">Dashboard</div>
  </div>
  <div class="card">
    <div class="card-t">状态</div>
    <div class="card-x">系统运行中</div>
    <div class="lamps">
      <div class="lamp ok"></div>
      <div class="lamp acc"></div>
      <div class="lamp warn"></div>
    </div>
    <div class="btn">操作</div>
  </div>
</div>
</body>
</html>
```

- [ ] **Step 4: 建 ai-output 占位 + 验证夹具打包合规**

```bash
mkdir -p samples/ai-output
touch samples/ai-output/.gitkeep
# 验证 components.html 能被 loomgui_pkg 打包（围栏合规）
cargo run -p loomgui_pkg -- samples/design-systems/loomgui/components.html /dev/null -o /tmp/fixture.pkg.bin -a fixture.atlas.png 2>&1 | tail -3
```
Expected: `wrote /tmp/fixture.pkg.bin (N bytes)`（零退出，证明夹具 HTML 围栏合规）。
若报围栏错 → components.html 有违规写法，修正后重跑。

- [ ] **Step 5: commit**

```bash
git add samples/design-systems/loomgui/ samples/ai-output/
git commit -m "feat(samples): design-system 测试夹具（暗色 dashboard）+ ai-output 占位"
```

---

## Task 6: .gitignore 追加（temp 隔离 + samples 生成物）

**Files:**
- Modify: `.gitignore`

**背景**：`temp/open-design/` 是只读调研参考未入库但有误提交风险；`samples/CLAUDE.md` + `samples/.claude/` 是 init 注入的生成物，不该提交（与 editor/ 模板重复）。

- [ ] **Step 1: 追加 gitignore 规则**

读 `.gitignore`，在末尾追加：

```
# open-design 源码（只读调研参考，不入库）
/temp/

# editor init 注入生成物（samples/ 自测注入）
samples/CLAUDE.md
samples/.claude/
```

- [ ] **Step 2: 验证 temp 不会被跟踪**

```bash
git check-ignore -v temp/open-design/package.json
# 应输出 .gitignore:N:/temp/ 命中
git status --short | grep -E "temp/|samples/CLAUDE|samples/.claude" || echo "无 temp/samples生成物 进入 git status（正确）"
```

- [ ] **Step 3: commit**

```bash
git add .gitignore
git commit -m "chore: gitignore 追加 /temp/ + samples init 生成物"
```

---

## Task 7: 文档同步验证

**Files:**
- Verify: `docs/design/fence.md`、`docs/roadmap/v1-scope.md` §2、`docs/roadmap/roadmap.md` §2（brainstorm 期已改，本 task 验证一致性）。

**背景**：brainstorm 期已同步三处文档，本 task 验证它们与 Task 1 的 fence_contract 测试一致——尤其 fence.md 标【推断·待测】的项，测试落地后实际行为若与 fence.md 不符，改 fence.md 标注。

- [ ] **Step 1: 跑 fence_contract 测试，记录任何与 fence.md 不符的实际行为**

Run: `cargo test -p loomgui_core --test fence_contract 2>&1`
Expected: 全绿。若 Task 1 Step 6 修正过断言，记录修正的实际行为。

- [ ] **Step 2: 对照 fence.md，把【推断·待测】转【实证】**

读 `docs/design/fence.md`，对 Task 1 测试覆盖的项（position:absolute / display:grid / float / @media / skew / 围栏外属性 return false），把标注从【推断·待测】改为【实证】。

具体：fence.md §2.4 表格的"标注"列，以下行改为【实证】（测试已锁定）：
- `position:absolute/fixed/sticky` → 【实证】
- `display:grid` → 【实证】（§2.1 表格里 display 行的"待测 grid 落 Flex"也改【实证】）
- `float` → 【实证】
- `@media` → 【实证】
- `transform: skew()/matrix()` → 【实证】
- §2.4 其余围栏外属性（align-content/cursor/clip-path/background-position/background-repeat/transform-origin/font-style/border-style）若 `fence_out_props_return_false` 测试覆盖 → 改【实证】。

若 Task 1 实际行为与 fence.md 描述不符（如某属性实际返回 true 而非 false），**改 fence.md 描述对齐实际**，并在 §0 记录纠正。

- [ ] **Step 3: 验证 v1-scope §2 + roadmap §2 引用一致**

```bash
grep -c "fence.md" docs/roadmap/v1-scope.md docs/roadmap/roadmap.md docs/design/fence.md
# v1-scope 和 roadmap 都应引用 fence.md
grep "filter\|border-image-slice\|:focus" docs/roadmap/v1-scope.md
# 应看到 v1.x 扩展已标注
```

- [ ] **Step 4: commit**

```bash
git add docs/design/fence.md
git commit -m "docs(fence): 【推断·待测】转【实证】（fence_contract 测试落地后对齐）"
```

---

## Task 8: 自测闭环（init 注入 samples → open-design → 生成 → 打包）

**Files:**
- 无新建文件。本 task 是端到端自测，验证 editor 工作流在 `samples/` 跑通。

**Interfaces:**
- Consumes: Task 1-7 全部产出。

**背景**：spec §2.3 自测工作流——`samples/` 自身当被注入的测试工作区。init 注入 → open-design import → AI 生成进 ai-output/ → pack.mjs 打包 → 对照 v1-showcase。本机是编码机，open-design 在本机跑（已装 stable 通道）；生成结果交家里机 Unity 验收（pkg.bin）。

- [ ] **Step 1: init 注入 samples/ 自身**

```bash
printf "samples\nsamples/ai-output\nclaude\n" | node editor/init.mjs
```
Expected:
- `[init] CLAUDE.md: created（samples/CLAUDE.md）`
- `[init] skill 注入: samples/.claude/skills/loomgui-editor`
- `[init] config.json: samples/.claude/skills/loomgui-editor/config.json`

验证：
```bash
ls samples/CLAUDE.md samples/.claude/skills/loomgui-editor/SKILL.md
grep "LOOMGUI_ROOT" samples/.claude/skills/loomgui-editor/tools/pack.mjs
# 占位符已替换为实际仓库根
git status --short | grep "samples/CLAUDE\|samples/.claude" || echo "生成物未进 git status（gitignore 生效）"
```

- [ ] **Step 2: open-design import samples/ + 选 design-system 夹具**

手动操作（open-design UI）：
1. 打开 open-design 桌面 app。
2. `od project import` 或 UI 导入 `F:\WorkSpace\projects\LoomGUI\samples`。
3. design-system picker 选 "LoomGUI Dashboard"（夹具）。
   - **若 picker 看不到夹具**（project 内 design-systems 不被读）：把 `samples/design-systems/loomgui/` 拷一份到 open-design 数据目录 `C:\Users\yuanwentao01\AppData\Roaming\Open Design\namespaces\release-stable-win\data\design-systems\loomgui\`，refresh 后再选。这是 spec §7 标的实测项，测出结果记录。
4. skill picker 应看到 `loomgui-editor` skill。

记录：picker 能否看到 design-system 夹具 + skill。

- [ ] **Step 3: AI 生成 + pack.mjs 打包**

在 open-design 里用自然语言生成一个暗色 dashboard 面板（如"生成一个游戏背包面板，标题+物品卡片列表+状态点"）。AI 应：
- 读 CLAUDE.md 围栏规则，生成 flex/gap 布局的 HTML+CSS。
- 跑 `node samples/.claude/skills/loomgui-editor/tools/pack.mjs <生成的html> <生成的css> -o samples/ai-output/test.pkg.bin`。

观察：
- 若 AI 生成违规 HTML（如 grid），pack.mjs 非零退出 + 报错，AI 应自纠重跑。
- 合规 → `samples/ai-output/test.pkg.bin` 产出。

手动验证产出：
```bash
ls -la samples/ai-output/test.pkg.bin
# 应存在且非空
```

- [ ] **Step 4: 对照 v1-showcase 验收**

生成的 UI 应与 `samples/v1-showcase/` 风格一致（暗色 dashboard，flex/gap 布局）。把生成的 HTML 在浏览器打开，目测：
- 暗色底 + 卡片 + 状态点。
- flex 堆叠 + gap 间距（非 margin）。
- 无 grid/absolute（预览里不该有脱离流元素）。

记录生成质量。若 AI 生成风格乱跑或违规多 → 检查 CLAUDE.md 围栏规则是否够清晰、skill SKILL.md 工作流引导是否够。

- [ ] **Step 5: 家里机 Unity 验收（pkg.bin）**

把 `samples/ai-output/test.pkg.bin` + atlas 拷到家里机 Unity StreamingAssets，PlayMode 加载渲染。
（两台机串行工作流：本机编码+打包，家里机 Unity 验收。若 pkg.bin 格式与现有 loom_showcase.pkg.bin 一致，Unity LoomStage 应能加载。）

记录 Unity 渲染是否与 open-design 预览一致（围栏内项应一致，围栏外项 LoomGUI 按 taffy 渲染）。

- [ ] **Step 6: 清理 + commit 自测记录**

```bash
# samples/ 的生成物（CLAUDE.md/.claude）已被 gitignore，不提交。
# ai-output 的测试 pkg.bin 可保留作回归对照，或清理。
git status
# 应只有 ai-output/test.pkg.bin（若没 gitignore）— 按需保留或清理
```

自测记录写进 session-summary skill（坑/经验）：
- picker 能否看到 project 内 design-system（Step 2 实测结果）。
- AI 生成合规率、常见违规（Step 3-4）。
- Unity 渲染一致性（Step 5）。

无代码 commit（自测 task）。若自测发现 bug 需改 editor/ 代码，单独 commit。

---

## Self-Review 结果

**1. Spec coverage**：
- spec §1.1 目标 → Task 1-8 全覆盖。
- spec §1.2 不做（YAGNI）→ 不需 task。
- spec §1.3 三处修订 → Task 7 验证文档同步。
- spec §2 架构 → Task 2-4（editor/）+ Task 5（samples 夹具）。
- spec §3.1 init.mjs → Task 4。
- spec §3.2 围栏规则 → Task 2 + Task 7。
- spec §3.3 skill → Task 3。
- spec §3.4 loomgui_pkg 复用 → Task 3 pack.mjs。
- spec §4 samples 合并 → Task 5 + Task 6。
- spec §5 环境整理 → Task 6。
- spec §6 验收 → Task 8。
- spec §7 未定项 → Task 8 Step 2 实测 design-system picker；opencode/codex skill 路径 Task 4 init.mjs 注释标"暂同 .claude/skills，实现期查"。
- 围栏维护机制（fence.md §4）→ Task 1（测试即真相源）+ Task 7（标注同步）。

**2. Placeholder scan**：无 TBD/TODO。Task 1 Step 6 的"修正断言"有明确原则（锁定实际行为）。Task 8 Step 2 的 picker 可见性是实测项（spec §7 已标），有 fallback（拷到 data/design-systems）。

**3. Type consistency**：`__LOOMGUI_ROOT__` 占位符在 Task 3 pack.mjs 定义、Task 4 init.mjs replaceAll、Task 4 Step 2 grep 验证——一致。`mergeRuleFile` 的标签 `<!-- loomgui-editor-begin/end -->` 在 Task 2 CLAUDE.md.tmpl + Task 4 init.mjs 一致。fence_contract 测试函数名在 Task 1 各 Step 一致。



---
