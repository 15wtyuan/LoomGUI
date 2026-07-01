## Task 4 Report: 严 polyfill 固化进 skill

**Date**: 2026-07-01
**Commit**: `a6a392b`

### Files created
- `editor/skill/loomgui-editor/references/preview-polyfill.html` — 标准 polyfill 片段（三行：div flex column, * border-box, body margin:0），供 SKILL.md 引用，AI 生成时抄进 head。

### Files modified
- `editor/skill/loomgui-editor/SKILL.md` — 工作流第 2 步：去 l-container；新增 polyfill 强制项（head 必须内联 preview-polyfill.html 的 `<style>`）。
- `editor/skill/loomgui-editor/references/preview-trust.md` — 可信段加 `overflow:scroll/auto`；不可信段加 `div` 默认 display 项（Chromium block vs LoomGUI flex column，需挂 polyfill）。
- `samples/design-systems/loomgui/components.html` — head `<style>` 开头加 polyfill 三行。

### Step 5 verification
```
cargo run -p loomgui_pkg -- samples/design-systems/loomgui/components.html %TEMP%\empty.css -o %TEMP%\fixture.pkg.bin -a %TEMP%\fixture.atlas.png
```
Output: `wrote ...\fixture.pkg.bin (6601 bytes) + atlas (0 bytes)` — 零退出，围栏合规。

### Concerns
- 无。polyfill 在 head `<style>`，pack.mjs 的 parse_html 忽略 head/body 包裹（dom.rs:34），不进 pkg，不影响打包。
