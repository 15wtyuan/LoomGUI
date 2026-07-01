# 预览可信清单

open-design Chromium iframe 预览 ≠ taffy 渲染。AI 须分清：

## 可信（Chrome ≈ LoomGUI）
flex 轴/方向、显式 `display:flex`、`gap` 间距、颜色、opacity、border、图片、px 尺寸、`background-image`/`background-size`（标准 CSS，Chrome 原生）、**`overflow:scroll/auto`（v1d.5 ScrollPane 真交互滚动，预览也是真滚动）**。

## 不可信（Chrome ≠ LoomGUI，别按预览调）
- **`div` 默认 display**：Chromium 默认 div=block，LoomGUI 默认 div=flex column。**必须挂 preview-polyfill.html 对齐**（head 内联），否则 gap/flex-grow/align-items 全不生效、预览塌。挂了 polyfill 后 div 行为可信。
- **margin 控间距**：Chrome（block flow）折叠 margin、LoomGUI（flex）求和不折叠。**子项间距用 `gap`**，别用 margin。
- **文本换行/像素级**：Chrome 文本引擎 vs LoomGUI（unicode-linebreak），换行点/塞文本宽度会偏。
- **`position:absolute`**：Chrome 脱离流、LoomGUI 不脱离（围栏外静默忽略）。预览会骗 AI。
- **`display:grid`**：Chrome 渲染 grid、LoomGUI 落 Flex。预览会骗 AI。
- **`@media` 响应式**：Chrome 响应、LoomGUI 用参考分辨率缩放不响应 @media。

## 口径
不可信项"信围栏规则，别信预览"。
