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
