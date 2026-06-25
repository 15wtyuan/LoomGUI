# LoomGUI v1 实现范围

> v1 目标：在 **Unity（Win/Mac 桌面 + Mono backend）** 上跑通"可演示 UI"——按钮+文本+可滚动列表+自适应分辨率。证明架构成立。
> 设计依据见 `docs/design/00-main-design.md`（单一真相源）。本文只定 v1 干什么。
> IL2CPP/移动端（iOS/Android）推 v1.x。

---

## 1. v1 能力清单

### 渲染
- 贴图 quad + 纯文本 + 硬矩形裁剪（rect mask）
- FairyBatching 重排 + 绘制顺序
- Unity 后端：GameObject 镜像 + DrawState 缓存（MaterialManager）+ 提交

### 文本
- ttf-parser 测量 + unicode-linebreak 断行（**砍 rustybuzz 复杂 shaping + unicode-bidi**，亚洲/国内首发）
- 后端据 TextLayout（SOA 三表）生成 mesh
- 字体用包声明的同一 ttf（一致性根基）

### 事件
- 命中（按等效绘制顺序逆序）+ click/hover/leave + 基本拖拽
- 触摸捕获 + 拖拽/滚动仲裁（阈值 + 退让）
- **UI 输入消费 `is_pointer_on_ui`**

### 布局
- taffy flexbox（围栏 v1 子集）
- **参考分辨率缩放**（MatchWidthOrHeight）
- safe-area（异形屏）

### 滚动
- ScrollPane 基础：惯性 + 回弹 + 滚动条 + 鼠标滚轮（自维护可变 target tween，不走 GTween）

### 资源
- **打包器 `loomgui_pkg`**（HTML+CSS+资源→.pkg.bin+图集）——v0 后、v1 第一阶段落地（v0 内存直通 defer）
- 二进制包加载（formatVersion + 迁移器）
- 图集 TextureView（散图起步、图集紧随，**优先级高**）+ refcount

### FFI
- csbindgen 通路 + SOA+多 arena 渲染树同步

### 状态/样式
- `:hover/:active/:disabled`（运行时伪类 + 样式 dirty 重匹配）
- cascade 继承（打包期展开）+ 合并 + 出现顺序
- **砍 Controller/Gear/`[data-page]`/Transition**（v1.x）

---

## 2. v1 围栏冻结清单（动工前签字，防范围飘）

**元素**：`div`(Container) / `span`+裸文本(Text) / `img`(Image) / `button`(Button)。
砍：`l-rich`/`input`/`l-graph`/`l-loader`/`l-movie`/`l-list`/`l-slider`/`l-combobox`/`l-tree`/`l-native`（v1.x）。v1 可滚动列表用 `div`+ScrollPane 手搓 item，**不做 `<l-list>` 虚拟化**。

**CSS 布局**：`display:flex/none`、`flex-direction`、`flex-wrap`、`gap`、`row-gap`、`column-gap`、`justify-content`、`align-items`、`align-self`、`flex`(grow/shrink/basis)、`width/height/min/max`(px/%/auto)、`padding`、`margin`、`border-width`、`position:relative`、`aspect-ratio`、`order`。
砍：`position:absolute`、`align-content`（换行行分布不可配，围栏文档须告知）、`position:sticky/fixed`（v1.x）。
> `row-gap`/`column-gap` 是 `gap` 的 longhand，映射同 taffy 字段，支持它们以对齐 AI 先验（AI 常写 longhand）。

**CSS 视觉**：`background-color`、`background-image`(url)、`background-size`(cover/contain/100%)、`border`(color/width/solid)、`opacity`、`overflow:visible/hidden`、`color/font-size/font-family/font-weight/font-style`、`text-align`、`line-height`、`letter-spacing`、`white-space:nowrap`、`transform`(translate/rotate/scale；v1d.3)。
砍：`filter`、`clip-path`、`border-radius`、九宫格 `-l-slice`、`background-position`、`transform-origin`(自定义；固定 center)、`skew()`/`matrix()`(剪切由 scale∘rotate 复合支持)（v1.x）。

**交互/状态**：`pointer-events:auto/none`、`:hover/:active/:disabled`。
砍：`cursor`、`:focus`、Controller/Gear/`[data-page]`、`:nth-child`、属性选择器（v1.x）。

**选择器**：标签/类/id/后代/子代。

**目标市场**：亚洲/国内首发（决定文本可砍 BiDi）。
**平台**：v1 仅 Win/Mac 桌面 + Mono backend。

---

## 2.1 v1 预览方案（临时，编辑器期换 WASM）

> 这是 v1 的临时预览手段，**不写进主文档**（主文档只写最终设计：编辑器用 WASM 跑核心做零偏差预览）。v1 还没有编辑器/WASM 渲染，用 Chrome 兜底。

**方案 = 围栏验证器 + Chrome MCP 预览 + JS polyfill**：
1. **围栏验证器**（必做）：检查 HTML/CSS 是否在围栏内，违规（display:grid/position:absolute/行内混排等）编译期报错。AI 的第一道反馈。
2. **Chrome MCP 预览**：AI 在 Chrome 打开 HTML 看效果。验证器通过后，除自定义属性外 Chrome 都能渲染（flex 布局/垂直堆叠/尺寸/普通文本/图片，偏差可控）。
3. **JS polyfill 脚本**（`loomgui-polyfill.js`，轻量 Web 资源）：把 LoomGUI 自定义属性翻译成 Chrome 能渲染的标准 CSS——
   - `border-image-slice`（九宫格，LoomGUI canonical 已是标准 CSS）→ Chrome 原生渲染（polyfill 几乎 no-op）
   - `[data-page]`（Controller 状态，v1.x 才用）→ JS 监听状态切换，动态改 `data-page` 属性（标准属性选择器，Chrome 原生匹配，零偏差）
   挂这个 polyfill 后，自定义属性也能在 Chrome 预览。

**围栏文档（给 AI 的 prompt）须写清**：
- 用 Chrome 预览时挂 `loomgui-polyfill.js`。
- 行内流/不支持属性别写（验证器挡）。
- Chrome 预览仅布局结构 + polyfill 后的自定义属性可信；文本换行细节/像素级以 LoomGUI 渲染为准（v1 容忍）。

**Chrome 预览可信清单（防 AI 被预览骗）**：polyfill 只管视觉/状态自定义属性，管不了 LoomGUI 与浏览器的**布局语义分歧**。AI 须分清——
- **可信**（Chrome ≈ LoomGUI）：flex 轴/方向、显式 `display:flex`、**`gap` 间距**、颜色、opacity、border、图片、px 尺寸。
- **不可信**（Chrome ≠ LoomGUI，别按预览调）：
  - **裸 div 混排**：Chrome 行内流 vs LoomGUI 堆叠——但验证器会挡，AI 拿不到预览，须改用 `<l-rich>`。
  - **margin 控间距**：Chrome（block flow）折叠 margin、LoomGUI（flex）求和不折叠。**子项间距用 `gap`**，别用 margin（gap 两边一致）。
  - **文本换行/像素级**：Chrome 文本引擎 vs LoomGUI（unicode-linebreak），换行点/塞文本宽度会偏。调宽塞文本时别全信 Chrome。
- **口径**：不可信项"信围栏规则，别信预览"。

**边界**：polyfill 搞定视觉表现（九宫格效果、状态样式），搞不定 LoomGUI 布局测量细节——但 v1 围栏内的自定义属性都是视觉/状态层（不涉布局尺寸），polyfill 够用。布局语义分歧（margin/换行）是围栏规则约束的，非 polyfill 职责。

**v2+ 替换**：编辑器（Claude Design 式 Web 应用）用 WASM 跑 LoomGUI 核心渲染，零偏差所见即所得，淘汰本临时方案。

---

## 3. v1 必做但主文档没显式列的 Unity 胶水任务

> 主文档定核心设计；这些是 Unity 后端"把核心接起来"的胶水，v1 必做。

| # | 任务 | 说明 |
|---|---|---|
| G1 | 打包器 `loomgui_pkg` | v0 后、v1 第一阶段落地（v0 内存直通 defer） |
| G2 | Stage MonoBehaviour 驱动 | 唯一 tick 入口、Unity 生命周期挂钩 |
| G3 | 根 Stage 挂 Unity（Camera/GameObject + 根 (1,-1,1) y-flip） | 屏幕空间 Stage 挂载点 |
| G4 | 输入采集→扁平事件→FFI 注入 | Unity 新/旧输入系统，含 IME character |
| G5 | IME/字符输入（v1 最小：PC 键盘字符级，不做 composing） | TextInput 在围栏外，但文本交互基础 |
| G6 | 字体资源进 Unity + 注册给核心（同一 ttf） | 后端 RequestCharactersInTexture 光栅化 |
| G7 | 纹理加载（磁盘→Unity→上传 GPU→注册 TexId） | Addressables/YooAsset/直接 File，v1 选一个 |
| G8 | 坐标翻转（根 Stage 一次性） | §8.1，不在 mesh/输入/命中分别翻 |
| G9 | GameObject 镜像池 diff（NodeId→GO，含 slot 复用、Mask 独立对象、Unchanged） | ~600-1000 行 C# |
| G10 | DrawState 缓存（MaterialManager，key 含 mask_context）+ Image/Text shader | v1 至少 Image+Text shader + grayed keyword |
| G11 | csbindgen 生成代码纳入 Unity + native lib 构建脚本 | Rust 编译→拷 Plugins/→生成 .cs |
| G12 | 参考分辨率 Unity 侧落地 | screenSize 变化触发重 solve |
| G13 | Domain reload / Play mode 重置保护 | `[RuntimeInitializeOnLoadMethod(SubsystemRegistration)]` |
| G14 | 滚动条 Unity 侧渲染 | §12.7 |

---

## 4. v1 验收标准

能做出并演示：
1. 一个含 **按钮 + 文本 + 图片** 的面板
2. **可滚动容器**（惯性+回弹+滚动条）
3. 按钮的 **hover/active 视觉反馈**（:hover/:active 伪类）
4. **自适应分辨率**（设计稿 1080×1920 在不同窗口等比缩放）
5. **UI 挡住时游戏不响应点击**（is_pointer_on_ui）
6. 从 **HTML 经打包器产出二进制包** 加载

性能基线：500 节点静态 UI 每帧无卡顿（v1 中段 stress 测试，早暴露 FFI/批合问题）。
**冷帧/换页帧**：冷启动首帧 + controller 全量换页帧（500 节点全 dirty）也每帧无卡顿——单帧 FFI 拷贝 + arena 解析 ≤ 2ms（§14.3 ArrayPool 租用，零 GC）。静态帧是便宜的那个，冷帧/换页帧才是 FFI 压力点。

---

## 5. v1 估时（参考）

~7-10 人月（1 Rust + 1 Unity 并行约 4-5 个月日历）。大头：
- Unity 后端镜像同步层（G9-G14）：~2-2.5 人月（最被低估）
- 文本（测量 + 后端 mesh，砍 BiDi 后）：~1 人月
- FFI 通路（csbindgen + SOA+arena + 构建脚本）：~1.5 人月
- 打包器 + 图集：~1.5 人月
- 核心（解析/样式/布局/渲染状态/事件/滚动）：~2 人月

---

## 6. v1 开工前置（go/no-go 门）

1. **v1 围栏冻结清单签字**（§2）。
2. **打包器 + 15 项胶水任务**写进任务拆解（§3），每项估时。
3. **v0 纯 Rust 骨架**先跑通（**内存直通**：HTML→parse→scene→taffy→render_nodes JSON 快照，打包器磁盘格式 defer v1 第一阶段），~2-3 周验证 Rust 能力（含文本测量全链）+ 解耦 FFI/Unity 风险。骨架通再投 Unity 后端大工期。v0 详见 [`docs/superpowers/specs/2026-06-18-v0-skeleton-design.md`](../superpowers/specs/2026-06-18-v0-skeleton-design.md)。

---

## 7. v1 明确不做（推 v1.x+）

富文本、九宫格/平铺/填充、软裁剪/形状遮罩(paintingMode)、动画(GTween 全套)+Transition+Controller+Gear、列表虚拟化、滚动分页/吸附/下拉刷新、动态节点 API 完整化、自定义控件扩展、IME 完整链路+软键盘、字体 fallback 链、NativeHost、rustybuzz 复杂 shaping+BiDi、IL2CPP+移动端、grid、CSS transition。

**v1 性能优化暂不做（实现期撞墙再加）**：文本测量缓存（`(text_hash,font,size,constraint)→(w,h)`）——v1 节点少（500）、naive 重算够用，待 taffy 反复调 measure 撞性能墙时再上 cache。

> 完整缺口登记见各轮 review 文档（`docs/review/`）。
