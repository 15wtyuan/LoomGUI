# 围栏清理（砍 l-container + 严 polyfill）设计

> **状态**：brainstorm 收敛稿，待 review → writing-plans。
> **日期**：2026-07-01
> **起因**：editor 工作流实测（backpack.html）暴露 `l-container` 在 open-design Chromium 预览塌成一坨——Chromium 不认 `l-container`（默认 inline），整页布局废。复盘发现 `l-container` 与 `div` 100% 同映射，是冗余假自定义元素，砍掉即可。

---

## 1. 问题

### 1.1 l-container 是冗余假自定义元素

- `FENCE_TAGS = ["div","span","img","button","l-container"]`（parse/dom.rs:29）。
- `"div" | "l-container" => NodeKind::Container`（scene/node.rs:297）——**与 div 100% 同映射，无任何独特语义/能力**。
- `l-` 前缀本该留给真·自定义元素（00-main-design.md:111：l-list/l-rich/l-loader 等有独特语义、v1.x 才来）。`l-container` 违背此原则。
- 全仓库无生产 HTML 用 `l-container`（v1-showcase 全用 div）；backpack.html 是首个用的，立刻塌。

### 1.2 l-container 破坏预览 + AI 可预测性

- **预览塌**：Chromium 不认 `l-container`（默认 `display:inline`），子项不排布、宽高塌陷 → 整页乱。`div` 至少 Chromium 认（默认 block），polyfill 能救；`l-container` 连 polyfill 都得额外覆盖。
- **AI 可预测性负收益**：AI 训练里没见过 `l-container`，见白名单有它反困惑（"该用 div 还是 l-container？"）。两标签同语义，纯增决策成本。

### 1.3 div 也需 polyfill 对齐（l-container 砍后唯一残留问题）

LoomGUI 契约：`<div>` 永远是 flex 容器，默认 `flex-direction: column`（00-main-design.md:109）。Chromium 的 div 默认 `display:block`（不是 flex）。不挂 polyfill，预览里 div 的 `gap`/`flex-grow`/`align-items` 全不生效，预览骗 AI。

v1-showcase 现有 polyfill（index.html head）：`div{display:flex;flex-direction:column}` + `*{box-sizing:border-box}`。但**靠设计师每次手抄**，backpack.html 没抄就塌。需固化进 skill，不靠手抄。

---

## 2. 目标

1. 砍 `l-container` 出 FENCE_TAGS + 所有围栏/设计文档。白名单只留 `div/span/img/button`（全 HTML 标准）。
2. 严 polyfill 固化进 skill：标准片段文件 + SKILL.md 强制 AI 生成时内联进 head。
3. 连带修正：fence_contract 测试、00-main-design.md 元素表、v1-scope.md、backpack 测试产物、samples 夹具 components.html。

## 3. 不做（YAGNI）

- 不砍 `l-` 前缀原则（l-list/l-rich 等 v1.x 真自定义元素仍用 l- 前缀，当前围栏砍它们不变）。
- 不改 pack.mjs（polyfill 在 head `<style>`，pack.mjs 吃外部 css 参数、忽略 head，天然不影响 pkg）。
- 不做 open-design iframe 自动注入 polyfill（不改 open-design 源码，注入不了）。
- 不补 `body{margin:0}` 之外的更多 polyfill（box-sizing + div flex column + body margin:0 够覆盖 LoomGUI 契约差异；margin 折叠/文本换行是算法级差异，polyfill 补不了，靠 preview-trust.md 不可信清单）。

---

## 4. 方案

### 4.1 砍 l-container

**核心代码**：
- `loomgui_core/src/parse/dom.rs:29`：`FENCE_TAGS` 移除 `"l-container"` → `["div","span","img","button"]`。
- `loomgui_core/src/scene/node.rs:297`：`"div" | "l-container" => NodeKind::Container` → `"div" => NodeKind::Container`。
- `dom.rs:65/106` 报错文案提"l-rich"保留（那是给想用富文本的提示，与 l-container 无关）。

**围栏契约测试** `loomgui_core/tests/fence_contract.rs`：
- `fence_tags_whitelist_accepted`：去掉 l-container（白名单只剩 4 个）。
- `fence_out_tags_rejected`：把 `l-container` 加进被拒列表（砍后它是围栏外标签，应报错）。

**围栏文档**（去掉 l-container）：
- `docs/design/fence.md` §1 元素白名单表。
- `editor/skill/loomgui-editor/references/fence.md`（同步副本，避免坑 83 漂移）。
- `editor/rules/{claude,opencode,codex}/*.tmpl` 元素白名单段。

**设计文档**：
- `docs/design/00-main-design.md:122` 元素表：`<div> / <l-container>` → `<div>`。
- `docs/roadmap/v1-scope.md` §2 元素行：去掉 l-container。

**测试产物**：
- `samples/backpack/backpack.html`：34 处 `<l-container>` → `<div>`。

### 4.2 严 polyfill 固化

**标准 polyfill 片段** `editor/skill/loomgui-editor/references/preview-polyfill.html`（新建）：
```html
<style>
  /* LoomGUI 预览对齐 polyfill（预览专用，打包器忽略 head 的 <style>，不影响 pkg.bin）。
     LoomGUI 契约：div 永远 flex column（00-main-design.md §109）；taffy 默认 border-box。
     Chromium 默认 div=block / content-box / body 有 8px margin → 预览失真，此 polyfill 对齐。 */
  div { display: flex; flex-direction: column; }
  * { box-sizing: border-box; }
  body { margin: 0; }
</style>
```

**SKILL.md 工作流强制**：生成 HTML 的 `<head>` 必须内联此 polyfill（从 references/preview-polyfill.html 抄）。明确：
- polyfill 只在 head `<style>`（预览用）。
- 设计师样式放外部 css 文件（跑 pack.mjs 传该 css 参数，打包用）。
- pack.mjs 吃外部 css、忽略 head `<style>`（parse_html 忽略 html/head/body 包裹，dom.rs:34），polyfill 不进 pkg。

**preview-trust.md 补两条**：
- `div` 预览需 polyfill 对齐（Chromium 默认 block，LoomGUI 默认 flex column）。
- `overflow:scroll/auto` 是**真交互滚动**（v1d.5 ScrollPane，knowledge-reference §2.23），预览可信。

**samples 夹具 components.html**：head 加 polyfill（夹具自身对齐，预览不塌）。

### 4.3 backpack 测试产物修正

`samples/backpack/backpack.html`：
- head 加 polyfill `<style>`。
- 34 处 `<l-container>` → `<div>`。
- （可选）验证：重跑 pack.mjs 打包零退出 + 浏览器打开预览正常。

REPORT.md 问题 2（.qty 角标 margin 写法错）+ 问题 3（内容超高）归生成方侧，不在本 spec 范围（REPORT 已自述）。

---

## 5. 影响评估

**向后不兼容**：已有用 `l-container` 的 HTML 会围栏报错。但全仓库仅 backpack.html（测试产物）用它，改掉即可。无生产 HTML 受影响。

**AI 可预测性净增益**：
- 砍 l-container：白名单全 HTML 标准，AI 不困惑。
- 严 polyfill：预览贴 LoomGUI 契约，AI 写的 CSS 如实呈现。唯一失效先验"div 默认 block flow"本就该失效（00-main-design.md:110 明说是 AI 须纠正的唯一 div 偏差）。摩擦点（水平排要显式 `display:flex`）靠 CLAUDE.md 一句话消，与浏览器里"要水平排写 flex"同等记忆负担。

**预览改善**：l-container 塌的坑彻底消失；div 靠 polyfill 对齐，预览不再骗 AI。

---

## 6. 验收

1. `cargo test -p loomgui_core --test fence_contract` 全绿（白名单测试 4 标签 + l-container 被拒）。
2. `node editor/init.test.mjs` 3/3（未受影响，回归）。
3. backpack.html 重跑 pack.mjs 零退出 + 浏览器预览正常（polyfill 生效，div 布局对）。
4. open-design import samples/ 看 backpack 预览不再塌（用户手动验）。
5. `diff -q docs/design/fence.md editor/skill/loomgui-editor/references/fence.md` 无差异（副本同步，避免坑 83）。

---

## 7. 下一步

review 通过 → writing-plans 拆任务：
1. 核心代码砍 l-container（dom.rs + node.rs）。
2. fence_contract 测试更新。
3. 围栏文档同步（fence.md + editor 副本 + rules 模板）。
4. 设计文档同步（00-main-design.md + v1-scope.md）。
5. polyfill 片段 + SKILL.md + preview-trust.md + components.html。
6. backpack.html 修正 + 验证。
