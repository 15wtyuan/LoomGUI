# 围栏清理（砍 l-container + 严 polyfill）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 砍掉冗余假自定义元素 `l-container`（与 div 同映射），白名单只留 HTML 标准 4 标签；固化严 polyfill 进 skill 让 open-design 预览对齐 LoomGUI 契约。

**Architecture:** 两步：(1) 核心代码 FENCE_TAGS 移除 l-container + NodeKind 映射改 + 围栏契约测试更新（l-container 转被拒）；(2) 严 polyfill 片段固化进 skill（preview-polyfill.html + SKILL.md 强制 head 内联 + preview-trust.md 补）。连带围栏/设计文档同步 + backpack 测试产物修正。

**Tech Stack:** Rust（loomgui_core dom.rs/node.rs/fence_contract）、Markdown（fence.md/SKILL.md/preview-trust.md/rules 模板）、HTML（preview-polyfill.html/components.html/backpack.html）。

## Global Constraints

- **语言**：问答/总结中文，代码/commit 英文（用户只读中文）。
- **两台机串行**：本机唯一编码机（含 Rust build .dll + commit + push），家里机纯 Unity PlayMode 验收。改 Rust 后必重编 .dll 家里机才能测。
- **main 直推**：直接 push main，不建 feature 分支。
- **Ponytail full**：最小 diff，YAGNI，无未请求抽象，shortest working diff wins。
- **围栏单一真相源**：`loomgui_core/tests/fence_contract.rs` 是权威，`docs/design/fence.md` 是副本，不一致时测试赢。改 FENCE_TAGS 必须同步测试 + fence.md + editor 副本（坑 83：忘同步分发副本）。
- **polyfill 不影响打包**：polyfill 在 HTML head `<style>`，pack.mjs 吃外部 css 参数、parse_html 忽略 html/head/body 包裹（dom.rs:34），head `<style>` 不进 pkg。
- **严 polyfill 三行**：`div{display:flex;flex-direction:column}` + `*{box-sizing:border-box}` + `body{margin:0}`。
- **commit 规范**：每个 task 末尾 commit，消息英文，不加 Co-authored-by trailer。

---

## File Structure

```
loomgui_core/src/parse/dom.rs              ← 改：FENCE_TAGS 移除 l-container + 注释
loomgui_core/src/scene/node.rs             ← 改：映射 "div"|"l-container" → "div"
loomgui_core/tests/fence_contract.rs       ← 改：白名单去 l-container + 被拒加 l-container
loomgui_core/src/parse/dom.rs (内联测试)    ← 改：rejects_fence_out_element 注释 + fence_tags_all_accepted 去 l-container 断言
docs/design/fence.md                       ← 改：§1 元素白名单去 l-container
editor/skill/loomgui-editor/references/
  fence.md                                 ← 改：同步 docs 副本（去 l-container）
  preview-trust.md                         ← 改：补 div 需 polyfill + overflow:scroll 真滚动
  preview-polyfill.html                    ← 新建：严 polyfill 标准片段
editor/skill/loomgui-editor/SKILL.md       ← 改：工作流强制 head 内联 polyfill
editor/rules/{claude,opencode,codex}/*.tmpl ← 改：元素白名单去 l-container
docs/design/00-main-design.md              ← 改：元素表去 l-container
docs/roadmap/v1-scope.md                   ← 改：§2 元素行去 l-container
samples/design-systems/loomgui/components.html ← 改：head <style> 加 polyfill 三行
samples/backpack/backpack.html             ← 改：l-container→div + head 加 polyfill
```

---

## Task 1: 砍 l-container 核心代码 + 围栏契约测试（TDD）

**Files:**
- Modify: `loomgui_core/tests/fence_contract.rs`
- Modify: `loomgui_core/src/parse/dom.rs:29`（FENCE_TAGS）+ 内联测试（dom.rs:203-225）
- Modify: `loomgui_core/src/scene/node.rs:297`

**Interfaces:**
- Consumes: `parse_html(html) -> Result<ElementTree, String>`（parse/dom.rs:32）。
- Produces: FENCE_TAGS 白名单 `["div","span","img","button"]`，`l-container` 成围栏外标签（报错）。

**背景**：l-container 与 div 100% 同映射（node.rs:297），无独特语义，是冗余假自定义元素。砍掉后白名单全 HTML 标准，AI 不困惑 + 预览不塌（Chromium 不认 l-container）。

- [ ] **Step 1: 改 fence_contract 测试（先让它 fail）**

`loomgui_core/tests/fence_contract.rs` 当前 `fence_tags_whitelist_accepted` 测 5 标签含 l-container，`fence_out_tags_rejected` 测 6 标签不含 l-container。改成：白名单去 l-container，被拒加 l-container。

找到 `fence_tags_whitelist_accepted`（约 line 20-26），把 l-container 从白名单循环去掉：
```rust
#[test]
fn fence_tags_whitelist_accepted() {
    // FENCE_TAGS = div/span/img/button（砍 l-container，与 div 同映射冗余）。
    for tag in ["div", "span", "img", "button"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_ok(), "<{tag}> 应被围栏接受");
    }
}
```

找到 `fence_out_tags_rejected`（约 line 29-36），把 l-container 加进被拒列表：
```rust
#[test]
fn fence_out_tags_rejected() {
    // 围栏外标签一律报错，不降级。l-container 砍后是围栏外（用 div）。
    for tag in ["video", "input", "b", "section", "p", "ul", "l-container"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_err(), "<{tag}> 应被围栏拒绝");
    }
}
```

- [ ] **Step 2: 运行测试验证 fail**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: FAIL——`fence_out_tags_rejected` 的 `l-container` 断言 fail（当前 l-container 还在白名单，parse_html 不报错）。`fence_tags_whitelist_accepted` 应仍 pass（去 l-container 不影响其他 4 个）。

- [ ] **Step 3: 改核心代码——FENCE_TAGS + node.rs 映射 + dom.rs 内联测试**

`loomgui_core/src/parse/dom.rs:29`：
```rust
const FENCE_TAGS: &[&str] = &["div", "span", "img", "button"];
```
（去掉 `"l-container"`）

`loomgui_core/src/scene/node.rs:297`：
```rust
        "div" => NodeKind::Container,
```
（去掉 `| "l-container"`）

`loomgui_core/src/parse/dom.rs` 内联测试（约 line 203-225）：
- `rejects_fence_out_element` 注释 `// 围栏白名单：div/span/img/button/l-container` 改成 `// 围栏白名单：div/span/img/button`。
- `fence_tags_all_accepted`：删掉 l-container 相关断言。当前：
```rust
    fn fence_tags_all_accepted() {
        // 白名单内五种 tag 均通过（l-container 同 div）
        let html = r#"<div><span>x</span><img src="a.png"><button>ok</button></div>"#;
        let tree = parse_html(html).unwrap();
        assert_eq!(tree.roots.len(), 1);
        let lcontainer = parse_html(r#"<l-container></l-container>"#).unwrap();
        assert_eq!(lcontainer.nodes[lcontainer.roots[0].0].tag, "l-container");
    }
```
改成（删 l-container 断言 + 注释）：
```rust
    fn fence_tags_all_accepted() {
        // 白名单内四种 tag 均通过（l-container 砍，与 div 同映射冗余）
        let html = r#"<div><span>x</span><img src="a.png"><button>ok</button></div>"#;
        let tree = parse_html(html).unwrap();
        assert_eq!(tree.roots.len(), 1);
    }
```

- [ ] **Step 4: 运行测试验证 pass**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: 10 passed（含改后的 whitelist 4 标签 + l-container 被拒）。

再跑 dom.rs 内联测试：
Run: `cargo test -p loomgui_core --lib parse::dom`
Expected: pass（fence_tags_all_accepted + rejects_fence_out_element 都过）。

- [ ] **Step 5: commit**

```bash
git add loomgui_core/src/parse/dom.rs loomgui_core/src/scene/node.rs loomgui_core/tests/fence_contract.rs
git commit -m "feat(core): 砍 l-container 出围栏白名单（与 div 同映射冗余）"
```

---

## Task 2: 围栏文档同步去 l-container（fence.md + editor 副本 + rules 模板）

**Files:**
- Modify: `docs/design/fence.md`（§1 元素白名单表）
- Modify: `editor/skill/loomgui-editor/references/fence.md`（同步副本）
- Modify: `editor/rules/claude/CLAUDE.md.tmpl` + `editor/rules/opencode/AGENTS.md.tmpl` + `editor/rules/codex/AGENTS.md.tmpl`（元素白名单段）

**Interfaces:**
- Consumes: Task 1 的 FENCE_TAGS 新白名单 `["div","span","img","button"]`。
- Produces: 围栏文档副本与权威一致（坑 83 防漂移）。

- [ ] **Step 1: 改 docs/design/fence.md §1 元素白名单表**

`docs/design/fence.md` §1 有元素白名单表（约 line 27-34），含 l-container 行。删掉 l-container 行：
```markdown
| 标签 | 映射 NodeKind | 出处 |
|---|---|---|
| `div` | Container | scene/node.rs:278 |
| `span` | Text（内容取 `el.text`） | scene/node.rs:283 |
| `img` | Image（src 取 `el.attrs["src"]`） | scene/node.rs:280 |
| `button` | Button | scene/node.rs:279 |
| `l-container` | Container（与 div 同） | scene/node.rs:278 |
```
改成（删 l-container 行）：
```markdown
| 标签 | 映射 NodeKind | 出处 |
|---|---|---|
| `div` | Container | scene/node.rs:278 |
| `span` | Text（内容取 `el.text`） | scene/node.rs:283 |
| `img` | Image（src 取 `el.attrs["src"]`） | scene/node.rs:280 |
| `button` | Button | scene/node.rs:279 |
```

§1 还有"白名单（`FENCE_TAGS`，parse/dom.rs:29）"后面的描述若提 l-container 也删。搜 `l-container` 全文，逐处删（§0 反例段不提 l-container，不用改）。

- [ ] **Step 2: 同步 editor 副本（坑 83 防漂移）**

```bash
cp docs/design/fence.md editor/skill/loomgui-editor/references/fence.md
diff -q docs/design/fence.md editor/skill/loomgui-editor/references/fence.md
```
Expected: `diff -q` 无输出（byte-identical）。

- [ ] **Step 3: 改三个 rules 模板的元素白名单段**

`editor/rules/claude/CLAUDE.md.tmpl` 元素白名单段当前：
```markdown
## 元素白名单
只用 `div` / `span`（+裸文本）/ `img` / `button` / `l-container`。其他标签（video/input/p/ul/...）会报错。
```
改成：
```markdown
## 元素白名单
只用 `div` / `span`（+裸文本）/ `img` / `button`。其他标签（video/input/p/ul/...）会报错。
```

opencode/codex 的 AGENTS.md.tmpl 内容与 claude/CLAUDE.md.tmpl 完全相同，同样改。改完三个文件用 diff 验证一致：
```bash
diff editor/rules/claude/CLAUDE.md.tmpl editor/rules/opencode/AGENTS.md.tmpl
diff editor/rules/claude/CLAUDE.md.tmpl editor/rules/codex/AGENTS.md.tmpl
```
Expected: 均无输出（三模板 byte-identical）。

- [ ] **Step 4: commit**

```bash
git add docs/design/fence.md editor/skill/loomgui-editor/references/fence.md editor/rules/
git commit -m "docs(fence): 围栏文档同步去 l-container（fence.md + editor 副本 + rules 模板）"
```

---

## Task 3: 设计文档同步去 l-container（00-main-design + v1-scope）

**Files:**
- Modify: `docs/design/00-main-design.md:122`（元素表）
- Modify: `docs/roadmap/v1-scope.md` §2（元素行）

**Interfaces:**
- Consumes: Task 1 新白名单。

- [ ] **Step 1: 改 00-main-design.md 元素表**

`docs/design/00-main-design.md:122` 当前：
```markdown
| `<div>` / `<l-container>` | Container | 通用 flex 容器，可裁剪/遮罩，可挂 ScrollPane |
```
改成：
```markdown
| `<div>` | Container | 通用 flex 容器，可裁剪/遮罩，可挂 ScrollPane |
```

搜 00-main-design.md 全文 `l-container`，若有其他提及（如示例代码）也删/改。`l-` 前缀原则那句（:111"自定义元素 kebab-case：`<l-list>`/`<l-loader>` 等用 `l-` 前缀"）**保留**——l-list/l-rich 等 v1.x 真自定义元素仍用 l- 前缀，只砍 l-container。

- [ ] **Step 2: 改 v1-scope.md §2 元素行**

`docs/roadmap/v1-scope.md` §2 元素行当前（约 line 55）：
```markdown
**元素**：`div`(Container) / `span`+裸文本(Text) / `img`(Image) / `button`(Button) / `l-container`(Container，与 div 同)。
```
改成：
```markdown
**元素**：`div`(Container) / `span`+裸文本(Text) / `img`(Image) / `button`(Button)。
```

§2 上方"纠正"段（约 line 53）若提 l-container 也删：
```markdown
> **纠正**（fence.md 核实）：`position:relative` 靠 taffy 默认 Relative 生效（非显式映射，写不写一致）；`font-style` 无 handler 静默忽略（原 §2 误列支持）；`l-container` 与 div 同映射（原 §2 漏列）。
```
改成（删 l-container 那句，因已砍）：
```markdown
> **纠正**（fence.md 核实）：`position:relative` 靠 taffy 默认 Relative 生效（非显式映射，写不写一致）；`font-style` 无 handler 静默忽略（原 §2 误列支持）。
```

- [ ] **Step 3: commit**

```bash
git add docs/design/00-main-design.md docs/roadmap/v1-scope.md
git commit -m "docs(design): 00-main-design + v1-scope 去 l-container"
```

---

## Task 4: 严 polyfill 固化进 skill

**Files:**
- Create: `editor/skill/loomgui-editor/references/preview-polyfill.html`
- Modify: `editor/skill/loomgui-editor/SKILL.md`（工作流强制 head 内联 polyfill）
- Modify: `editor/skill/loomgui-editor/references/preview-trust.md`（补两条）
- Modify: `samples/design-systems/loomgui/components.html`（head `<style>` 加 polyfill 三行）

**Interfaces:**
- Consumes: 无（polyfill 是预览对齐层，独立）。
- Produces: `preview-polyfill.html` 标准片段（SKILL.md 引用，AI 生成时抄进 head）。

**背景**：LoomGUI 契约 div 永远 flex column（00-main-design.md:109），Chromium 默认 div=block/content-box/body 8px margin。严 polyfill 三行对齐。polyfill 在 head `<style>`，pack.mjs 吃外部 css、忽略 head，不影响 pkg。

- [ ] **Step 1: 建 preview-polyfill.html 标准片段**

创建 `editor/skill/loomgui-editor/references/preview-polyfill.html`：
```html
<style>
  /* LoomGUI 预览对齐 polyfill（预览专用，打包器忽略 head 的 <style>，不影响 pkg.bin）。
     LoomGUI 契约：div 永远 flex column（00-main-design.md §109）；taffy 默认 border-box。
     Chromium 默认 div=block / content-box / body 有 8px margin → 预览失真，此 polyfill 对齐。
     生成 HTML 时把这段 <style> 内联进 <head>。设计师样式放外部 css 文件（跑 pack.mjs 传该 css）。 */
  div { display: flex; flex-direction: column; }
  * { box-sizing: border-box; }
  body { margin: 0; }
</style>
```

- [ ] **Step 2: 改 SKILL.md 工作流强制 head 内联 polyfill**

`editor/skill/loomgui-editor/SKILL.md` 工作流第 2 步（生成 HTML+CSS）当前：
```markdown
2. **按设计师 prompt 生成 HTML+CSS**：
   - 元素只用 `div`/`span`/`img`/`button`/`l-container`。
   - 布局用 flex + `gap`（子项间距用 gap 不用 margin）。
   - 禁 grid/absolute/float/@media/skew 等（详见 fence.md）。
   - 风格由设计师 prompt 决定（颜色/字号/字体自由，只要守围栏）。
```
改成（去 l-container + 加 polyfill 强制）：
```markdown
2. **按设计师 prompt 生成 HTML+CSS**：
   - 元素只用 `div`/`span`/`img`/`button`。
   - 布局用 flex + `gap`（子项间距用 gap 不用 margin）。
   - 禁 grid/absolute/float/@media/skew 等（详见 fence.md）。
   - 风格由设计师 prompt 决定（颜色/字号/字体自由，只要守围栏）。
   - **HTML `<head>` 必须内联预览 polyfill**（从 `references/preview-polyfill.html` 抄整段 `<style>`）。LoomGUI 契约 div 永远 flex column，Chromium 默认 block 会让预览塌。polyfill 只在 head（预览用），设计师样式放外部 css 文件（跑 pack.mjs 传该 css，打包用）。pack.mjs 吃外部 css、忽略 head `<style>`，polyfill 不进 pkg。
```

- [ ] **Step 3: 改 preview-trust.md 补两条**

`editor/skill/loomgui-editor/references/preview-trust.md` 当前"可信"段 + "不可信"段。在"可信"段补 overflow:scroll 真滚动，在"不可信"段补 div 需 polyfill。

找到"可信"段，加一条：
```markdown
## 可信（Chrome ≈ LoomGUI）
flex 轴/方向、显式 `display:flex`、`gap` 间距、颜色、opacity、border、图片、px 尺寸、`background-image`/`background-size`（标准 CSS，Chrome 原生）、**`overflow:scroll/auto`（v1d.5 ScrollPane 真交互滚动，预览也是真滚动）**。
```

找到"不可信"段，加一条 div polyfill 说明：
```markdown
## 不可信（Chrome ≠ LoomGUI，别按预览调）
- **`div` 默认 display**：Chromium 默认 div=block，LoomGUI 默认 div=flex column。**必须挂 preview-polyfill.html 对齐**（head 内联），否则 gap/flex-grow/align-items 全不生效、预览塌。挂了 polyfill 后 div 行为可信。
- **margin 控间距**：...（保留原有）
```

- [ ] **Step 4: 改 components.html head 加 polyfill**

`samples/design-systems/loomgui/components.html` head 已有 `<style>`（围栏正面教材注释 + 组件 CSS）。在 `<style>` 开头（`/* 围栏正面教材 */` 注释前）加 polyfill 三行：
```html
<style>
  /* LoomGUI 预览对齐 polyfill（div 永远 flex column + border-box + body 贴边） */
  div { display: flex; flex-direction: column; }
  * { box-sizing: border-box; }
  body { margin: 0; }
  /* 围栏正面教材：flex + gap，禁 grid/margin 间距/:focus/@media。
     本文件本身也是 LoomGUI 围栏合规的 HTML/CSS 样例。 */
  .root { ... }
  ...
```

- [ ] **Step 5: 验证 components.html 仍围栏合规**

```bash
echo '' > /tmp/empty.css
cargo run -p loomgui_pkg -- samples/design-systems/loomgui/components.html /tmp/empty.css -o /tmp/fixture.pkg.bin -a fixture.atlas.png 2>&1 | tail -1
```
Expected: `wrote /tmp/fixture.pkg.bin (N bytes)`，零退出（加 polyfill 不影响围栏合规，polyfill 在 head `<style>` 被 parse_html 忽略）。
```bash
rm -f /tmp/empty.css /tmp/fixture.pkg.bin samples/design-systems/loomgui/fixture.atlas.png
```

- [ ] **Step 6: commit**

```bash
git add editor/skill/loomgui-editor/references/preview-polyfill.html editor/skill/loomgui-editor/SKILL.md editor/skill/loomgui-editor/references/preview-trust.md samples/design-systems/loomgui/components.html
git commit -m "feat(editor): 严 polyfill 固化（preview-polyfill.html + SKILL 强制 + preview-trust 补）"
```

---

## Task 5: backpack 测试产物修正 + 验证

**Files:**
- Modify: `samples/backpack/backpack.html`（l-container→div + head 加 polyfill）

**Interfaces:**
- Consumes: Task 4 的 polyfill 片段。

**背景**：backpack.html 是 editor 工作流实测产物（REPORT.md），用了 l-container（现砍）+ 没挂 polyfill（预览塌）。修正后作 regression 样例。REPORT 问题 2/3 归生成方侧，不在本 task。

- [ ] **Step 1: backpack.html 全量 l-container→div**

`samples/backpack/backpack.html` 含 62 处 l-container（开+闭）。用 sed 全量替换：
```bash
sed -i 's/<l-container/<div/g; s/<\/l-container/<\/div/g' samples/backpack/backpack.html
grep -c "l-container" samples/backpack/backpack.html
```
Expected: `grep -c` 输出 `0`（无残留）。

- [ ] **Step 2: backpack.html head 加 polyfill**

`samples/backpack/backpack.html` head 当前（约 line 4-7）：
```html
<head>
<meta charset="utf-8">
<title>背包</title>
<link rel="stylesheet" href="backpack.css">
</head>
```
在 `<link>` 后、`</head>` 前加 polyfill `<style>`：
```html
<head>
<meta charset="utf-8">
<title>背包</title>
<link rel="stylesheet" href="backpack.css">
<style>
  /* LoomGUI 预览对齐 polyfill（div 永远 flex column + border-box + body 贴边） */
  div { display: flex; flex-direction: column; }
  * { box-sizing: border-box; }
  body { margin: 0; }
</style>
</head>
```

- [ ] **Step 3: 验证 backpack 打包零退出**

```bash
node samples/.claude/skills/loomgui-editor/tools/pack.mjs samples/backpack/backpack.html samples/backpack/backpack.css -o /tmp/backpack.pkg.bin -a backpack.atlas.png 2>&1 | tail -1
```
Expected: `wrote /tmp/backpack.pkg.bin (N bytes) + atlas ...`，零退出（l-container 砍后 backpack 全 div，围栏合规）。
若报围栏错 → backpack.html 还有 l-container 残留或别的围栏外标签，修后重跑。
```bash
rm -f /tmp/backpack.pkg.bin samples/backpack/backpack.atlas.png
```

- [ ] **Step 4: （可选）浏览器预览 backpack 正常**

在浏览器打开 `samples/backpack/backpack.html`，目测：暗色背包 UI 布局正常（div flex column 排布、gap 生效、不塌）。若仍塌 → 查 backpack.css 是否有围栏外写法（grid/absolute）。

- [ ] **Step 5: commit**

```bash
git add samples/backpack/backpack.html
git commit -m "fix(samples): backpack.html l-container→div + head 加 polyfill（l-container 砍后 regression）"
```

---

## Self-Review 结果

**1. Spec coverage**：
- spec §4.1 砍 l-container → Task 1（核心+测试）+ Task 2（围栏文档）+ Task 3（设计文档）+ Task 5（backpack）。
- spec §4.2 严 polyfill 固化 → Task 4（polyfill 片段 + SKILL + preview-trust + components）。
- spec §4.3 backpack 修正 → Task 5。
- spec §6 验收 → Task 1 Step 4（fence_contract）+ Task 5 Step 3（backpack 打包）+ Task 2 Step 2（diff 副本）；open-design 预览验收待用户手动。

**2. Placeholder scan**：无 TBD/TODO。Task 5 Step 4 标"可选"（浏览器预览），有明确操作。

**3. Type consistency**：FENCE_TAGS 新值 `["div","span","img","button"]` 在 Task 1 定义、Task 2/3 文档同步引用——一致。polyfill 三行在 Task 4 定义、Task 5 backpack 引用——一致。`l-container` 跨 Task 1/2/3/5 都是被砍对象，无歧义。

