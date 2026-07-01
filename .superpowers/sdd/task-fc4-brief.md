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
