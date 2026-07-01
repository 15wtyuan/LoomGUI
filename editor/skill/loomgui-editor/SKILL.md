---
name: loomgui-editor
description: |
  Generate LoomGUI fence-compliant UI (HTML+CSS) for game dashboards/panels.
  Uses flex-only layout, tag whitelist (div/span/img/button), no grid/absolute/margin-spacing.
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
   - 元素只用 `div`/`span`/`img`/`button`。
   - 布局用 flex + `gap`（子项间距用 gap 不用 margin）。
   - 禁 grid/absolute/float/@media/skew 等（详见 fence.md）。
   - 风格由设计师 prompt 决定（颜色/字号/字体自由，只要守围栏）。
   - **HTML `<head>` 必须内联预览 polyfill**（从 `references/preview-polyfill.html` 抄整段 `<style>`）。LoomGUI 契约 div 永远 flex column，Chromium 默认 block 会让预览塌。polyfill 只在 head（预览用），设计师样式放外部 css 文件（跑 pack.mjs 传该 css，打包用）。pack.mjs 吃外部 css、忽略 head `<style>`，polyfill 不进 pkg。

3. **生成完跑验证+打包**：
   ```bash
   node tools/pack.mjs <html路径> <css路径> -o <输出.pkg.bin> [-w 1080 -h 1920] [-a atlas.png]
   ```
   - **非零退出 = 围栏违规**（loomgui_pkg 报错）。读 stderr，自纠 HTML/CSS 后重跑。
   - **零退出 = 合规**，.pkg.bin + atlas.png 已产出到指定目录。

4. **报告**：向设计师报告产出路径（.pkg.bin + atlas.png），说明 Unity 加载方式（StreamingAssets/ 下，LoomStage 自动加载）。

## 注意

- **预览不可信项**：open-design 预览是 Chromium iframe，与 LoomGUI（taffy）有分歧。margin 折叠/文本换行/position:absolute/display:grid/@media 别按预览调。详见 references/preview-trust.md。
- **打包器即验证器**：pack.mjs 调的 loomgui_pkg 内含围栏验证（FENCE_TAGS + apply_decl），违规打包期报错。不需要单独的 lint 步骤。
