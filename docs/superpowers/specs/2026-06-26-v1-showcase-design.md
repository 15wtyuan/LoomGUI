# v1 总测试界面（showcase）设计

> **目的**：v1 收尾的"门面"——fgui 式交互功能 showcase，整合 v1a-v1e 全部已实现功能（119 项）到一个界面，供家里机 PlayMode 逐区点验。
> **性质**：非自动化 assert；每卡展示功能 + 标"预期"小字，人眼对照。
> **依据**：v1-scope §1（能力）+ §2（围栏）；knowledge-reference §2 各层机制；现有 6 sample（atlas/cjk/merge/interact pkg + v1d3-transform/v1d4-tween Unity）；git log v1a-v1e。
> **前置**：v1e 已完成（实现+知识库），本 showcase 是 v1 ship 前的整合验收界面。

## 0. 决策（brainstorm 已定）

| # | 决策 | 选定 |
|---|---|---|
| 1 | 素材来源 | CC0/MIT（Game-icons.net 图标）；不上 fgui 官方素材 |
| 2 | 界面组织 | 单页长滚动 + 顶部固定 nav（点 nav → SetScrollPos 跳区） |
| 3 | 覆盖范围 | 完整 8 区大而全（~46 卡，覆盖 119 项所有值，一卡多值对照） |
| 4 | 视觉调性 | 暗色 dashboard（深底 + 青高亮 + 状态色 + 线条边框） |
| 5 | 交互反馈 | 灯阵 + tween（v1 无运行时改文本/style，见 §6） |
| 6 | background-image | **不补**（v1 围栏 §2 列了但 core 未解析，是 v1 gap，单独跟踪，不塞进 showcase） |

## 1. 定位

fgui 式交互功能 showcase。单 HTML → loomgui_pkg 打包 → Unity PlayMode 加载。家里机验收 = 逐区点 46 卡 + 人眼对照"预期"。整个 showcase 本身是一个大 ScrollPane（顺带自测滚动 + nav 跳转自测 SetScrollPos）。

## 2. v1 围栏对"好看"的约束（关键，定审美天花板）

**有**：`background-color`、`border`(color/width)、`opacity`、`transform`、`<img>`(流式)、`color/font-size/font-family/font-weight/font-style/text-align/line-height/letter-spacing`、flex 全套布局。
**砍（推 v1.x）**：`border-radius`、渐变、阴影、`filter`、`clip-path`、九宫格 `-l-slice`、`position:absolute/sticky/fixed`、`z-index`、`background-position`。
**gap（围栏 §2 列了但 core 未解析，grep 确认）**：`background-image(url)`、`background-size`（零解析）；`font-style`（mapping 无分支）。→ 图片只能走 `<img>` 流式元素，**不能当背景层**；font-style 设了不生效。其余文本属性 `font-family`/`font-weight`/`letter-spacing` grep 确认**已落地**（mapping.rs:321/325/345）。

**审美策略**：暗色 dashboard 风——靠**配色层次 + 图标 img + 线条边框 + 精致排版 + transform 动态**做现代感，**不靠 PNG 皮肤**（按钮/面板 = 纯 CSS 色块 + border）。契合 showcase "功能看板"性质。

> background-image gap 影响 AI 可预测性（围栏列了 AI 会写却不生效），应作为 v1 收尾独立 gap 修复项跟踪，但**不在本 showcase 范围内补**（避免 core 改动 + .dll 重编 + 两台机摩擦混入纯 sample 项目）。

## 3. 素材与调色

- **素材**：Game-icons.net（CC0）图标，作 `<img>` 内容元素（nav 图标、卡图标、状态图标）。无 PNG 皮肤。
- **调色板**（暗色 dashboard）：
  - 背景 `#1a1d2e` / 面板 `#252839` / 面板悬浮 `#2d3148`
  - 边框线条 `#3a3f55`（1px）
  - 青高亮（accent）`#4dd0e1`
  - 文字主 `#e0e0e0` / 次要 `#9aa0b4`
  - 通过绿 `#4caf50` / 警告红 `#f44336` / 禁用灰 `#6c7080`
- **按钮三态**（纯 CSS 伪类，interact sample 已验证路径）：normal=面板色；`:hover`=悬浮色 + 青边框；`:active`=青色填充；`:disabled`=灰 + opacity 0.5。

## 4. 布局骨架

```
root (column, 设计稿 1080×1920, 参考分辨率缩放)
├─ header (固定高 ~140px, 在 scroll 容器外 → 永远可见, 绕开无 sticky)
│   ├─ 标题 "LoomGUI v1 Showcase" + 副标题 version
│   └─ nav-bar: [§1元素][§2布局][§3视觉][§4交互][§5滚动][§6文本][§7动效][§8管线]
└─ scroll-pane (flex-grow, overflow-y:scroll, id="main-scroll")
    └─ content (column, gap:16px, padding:24px)
        ├─ §1 元素画廊  (id="sec-1", 高 H1)
        ├─ §2 布局练兵场 (id="sec-2", 高 H2)
        ├─ ... §3..§8
```

- header 在 scroll 外（flex 子项，固定高），永远可见——规避 v1 无 `position:sticky`。
- nav 按钮 click → C# `FindNodeById("main-scroll")` → `SetScrollPos(scroll_id, 0, target_y)`。
- `target_y` = 设计期按各区累积高度写死的数组（§8 风险：内容高度变需重算）。

## 5. 八区卡片清单（46 卡，覆盖 119 项所有值）

每卡：**卡名 — 覆盖围栏属性/对照值 — 预期 — 反馈方式**。反馈记号：[静]=静态展示；[伪]=CSS 伪类自动；[tween]=C# tween 触发；[灯]=灯阵点亮。

### §1 元素画廊（6 卡）
| 卡 | 覆盖 | 预期 | 反馈 |
|---|---|---|---|
| 1.1 色块矩阵 | `background-color` 调色板 8 色 | 8 色块正确呈现 | [静] |
| 1.2 img 整图 | `<img src>` 多张 | 按宽高呈现 | [静] |
| 1.3 img 尺寸 | `width/height` px·%·auto 对照 | 三种尺寸模式 | [静] |
| 1.4 span 文本 | `<span>`+裸文本 | 文本节点呈现 | [静] |
| 1.5 button 三态 | `<button>` + `:hover/:active/:disabled` | hover亮/active填充/disabled灰 | [伪] |
| 1.6 NativeHost | `<div id="model-slot">` + C# BindNativeHost | 外部 GO 跟随 slot | C# 绑定 |

### §2 布局练兵场（8 卡）— flex 全套
| 卡 | 覆盖 | 预期 |
|---|---|---|
| 2.1 flex-direction | row vs column | 横/纵堆叠 [静] |
| 2.2 flex-wrap | wrap vs nowrap | 换行/不换行 [静] |
| 2.3 justify-content | flex-start/center/flex-end/space-between/around/evenly（6 值并排） | 6 种主轴分布 [静] |
| 2.4 align-items + align-self | start/center/end/stretch + 单项 align-self | 交叉轴对齐 [静] |
| 2.5 gap vs margin | `gap` 子间距 vs `margin`（不折叠） | 间距对照 [静] |
| 2.6 flex grow/shrink/basis | grow 占余 / shrink 收缩 / basis 基线 | 三者行为 [静] |
| 2.7 尺寸单位 | `width/height` px·%·auto + `min/max` + `aspect-ratio` | 单位与约束 [静] |
| 2.8 order | `order` 打乱视觉序 | 顺序重排 [静] |

### §3 视觉样式板（5 卡）
| 卡 | 覆盖 | 预期 |
|---|---|---|
| 3.1 border | `border` width·color 四值 | 边框样式 [静] |
| 3.2 opacity 阶梯 | 1.0/0.7/0.4/0.2 | 透明度层级 [静] |
| 3.3 transform | translate/rotate/scale + scale∘rotate 剪切复合 | 变换对照 [静] |
| 3.4 文本样式全家 | font-size/family/weight/style/color/text-align/line-height/letter-spacing | 各样式生效 [静]† |
| 3.5 调色板 | background-color 全调色板 | 配色呈现 [静] |

> † grep 查证：`font-family`/`font-weight`/`letter-spacing` **已落地**（mapping.rs:321/325/345），正常演示；`font-style` **未落地**（mapping 无分支，围栏 §2 gap）→ 该值在卡内标灰 skip + 记 gap，不造假。

### §4 交互事件集（7 卡）
| 卡 | 覆盖 | 预期 | 反馈 |
|---|---|---|---|
| 4.1 click + dblclick | click + click_count(350ms) | 单击蓝灯/双击青灯 | [灯] |
| 4.2 hover/leave/active | `:hover/:active` + RollOver/Out | 三态色变 + 状态灯 | [伪]+[灯] |
| 4.3 disabled | SetNodeDisabled | 灰态不响应 click | [伪]+C# |
| 4.4 drag | `draggable="true"` + DragMove | 指示块跟随鼠标 | [tween] translate |
| 4.5 longpress | LongPress(1.5s+50px) | 长按亮灯 | [灯] |
| 4.6 focus + Tab + key | tabindex 链 + Tab/Shift+Tab + keydown | 焦点框移动 + 按键灯阵 | [伪]:focus+[灯] |
| 4.7 路由 + pointer-events | capture/bubble/StopPropagation + pointer-events:none 穿透 | 路由灯 + 穿透对照 | [灯] |

### §5 滚动实验室（6 卡）— 本区嵌套滚动容器
| 卡 | 覆盖 | 预期 |
|---|---|---|
| 5.1 overflow 模式 | scroll/auto/hidden 三小容器并排 | 溢出行为对照 [交互] |
| 5.2 overflow-x/y | 嵌套水平滚（overflow-x:scroll overflow-y:hidden） | 单轴滚动 [交互] |
| 5.3 惯性+回弹+滚轮 | 主 scroll-pane 体验（惯性 0.967 + 回弹 cubicOut 0.3s + 滚轮） | 手感 [交互] |
| 5.4 滚动条 + grip | 合成 thumb + 拖 grip 百分比映射 | 拖条滚 [交互] |
| 5.5 嵌套+轴锁+仲裁 | 嵌套 scroll + draggable item（scroll-vs-drag 阈值赛跑 + 轴锁） | 仲裁正确 [交互] |
| 5.6 SetScrollPos | 编程跳转（= nav 机制复用） | 跳到目标 [交互] |

### §6 文本工坊（5 卡）
| 卡 | 覆盖 | 预期 |
|---|---|---|
| 6.1 CJK 逐字断行 | 中文长段（unicode-linebreak UAX#14） | 逐字可断 [静] |
| 6.2 ASCII 按词 | 英文长段（greedy fill） | 按词断 [静] |
| 6.3 中英混排 | 混排断行 | CJK+ASCII 混合 [静] |
| 6.4 nowrap + 超长词 | `white-space:nowrap` + 超长词逐字断（toMoveChars=1） | 不换行 + 逐字断 [静] |
| 6.5 行高字距 | `line-height`(倍数) + `letter-spacing` | 行距字距对照 [静]† |

### §7 动效舞台（5 卡）— v1d.4 GTween 子集
| 卡 | 覆盖 | 预期 | 反馈 |
|---|---|---|---|
| 7.1 6 tween prop | opacity/translate/scale/rotation/bg-color/text-color 各一 | 各 prop 动画 | [tween] |
| 7.2 10 缓动对照 | Linear/Quad×3/Cubic×3/Back×3 同 tween 并排 | 缓动曲线差异 | [tween] |
| 7.3 delay | delay 参数（<delay 期间跳过） | 延时启动 | [tween] |
| 7.4 complete 回调 | EVT_TWEEN_COMPLETE + tag/prop | 完成亮灯 | [tween]+[灯] |
| 7.5 kill/clear/replace-override | kill_tween / clear_anim / clear_anim_prop（回 CSS） | 停/清/覆盖 | C# API |

### §8 管线/分辨率（4 卡）
| 卡 | 覆盖 | 预期 |
|---|---|---|
| 8.1 FairyBatching 合并 | 复刻 merge sample（多图同 atlas → N→1 draw） | 合并正确 + Profiler draw call↓ [静] |
| 8.2 图集 atlas | 复刻 atlas sample（散图→atlas.png+UV 烤顶点） | atlas 图呈现 [静] |
| 8.3 dirty hash 静态帧 | 说明卡（v1e：静态帧全 Unchanged→C# 0 upload） | 说明 + Profiler GC Alloc≈0 [说明] |
| 8.4 参考分辨率 + safe-area | 改窗口大小看等比缩放 + safe-area letterbox | 缩放正确 [交互] |

## 6. 交互反馈硬约束（关键）

v1 **无运行时改文本/style API**（动态节点推 v1.x；grep 确认运行时仅 `set_node_disabled`/`set_scroll_pos`/tween/事件）。所以交互反馈**只能**：

- **CSS 伪类**（打包期展开）：`:hover`/`:active`/`:disabled`/`:focus` 视觉自动。
- **tween**（v1d.4）：opacity/translate/scale/rotation/bg-color/text-color。
- **set_disabled / set_scroll_pos**。

→ **计数不能用数字+1**（改不了节点文本）。用**灯阵**：C# 内存维护 int 计数，每触发一次 → `tween` 点亮下一盏灯（opacity 0→1）。每卡灯阵规模按该卡最大触发次数定（如 keydown 26 键 → 26 灯）。

## 7. 卡片视觉模板

```
┌─ 卡片 (面板 #252839, border 1px #3a3f55, padding 16) ──────┐
│ [图标] 卡标题 4.x              预期: <一句话说明> (次要色)  │
├──────────────────────────────────────────────────────────┤
│  ┌────┐  ┌────┐  ┌────┐  ← 多值并排对照（每值带小标签）     │
│  │值1 │  │值2 │  │值3 │                                    │
│  └────┘  └────┘  └────┘                                    │
│  [灯阵 ●●○○○○○○]  ← 仅交互卡，触发态指示                  │
└──────────────────────────────────────────────────────────┘
```

## 8. nav 跳转机制

- nav 8 按钮（header 内，scroll 外），各挂 click → C# `OnNavClick(sectionIndex)`。
- C# 持 `float[] section_y = { 0, H1, H1+H2, ... }`（设计期按各区累积高度写死）。
- `SetScrollPos(scroll_id, 0, section_y[i], animated:true)`。
- **风险**：分区内容高度变 → section_y 失准 → 需重算（demo 一次性，内容定后不变；ponytail 接受）。

## 9. 打包/落点

- **HTML 源**：`loomgui_pkg/samples/v1-showcase/page.html` + `style.css` + `assets/icons/`（Game-icons CC0 PNG）。
- **pkg 产出**：打包器 → `v1-showcase.pkg.bin` + atlas（图标进图集）。
- **Unity 入口**：`loomgui_unity/Assets/LoomGUI/Samples/v1-showcase/`：
  - 场景 `ShowcaseScene.unity`（LoomStage 挂载 + 加载 v1-showcase.pkg）。
  - `ShowcaseDriver.cs`（事件监听 + 灯阵 tween + nav SetScrollPos + NativeHost 绑定 + kill/clear 演示）。
- **version**：`v1e`（当前）；不改 FFI/blob/pkg 契约（纯 sample，零 core 改动）。

## 10. 验收标准（家里机 PlayMode）

1. 8 区 46 卡全部呈现，暗色 dashboard 调性正确。
2. 逐卡人眼对照"预期"：静态卡视觉对、交互卡触发反馈（灯/动效/状态）对。
3. nav 8 按钮跳转正确（SetScrollPos 到对应区）。
4. 主 scroll-pane 惯性/回弹/滚轮/滚动条手感正常。
5. §8.4 改窗口大小，等比缩放 + safe-area 正确。
6. 性能：~350 节点静态帧无卡顿（<500 性能线）。
7. 全程仅用围栏内属性（§2 清单）；未落地属性（font-family/weight/style/letter-spacing 等）若 §3.4/§6.5 验证失败则标灰 skip + 记 gap，不造假。

## 11. 不做（YAGNI）

- 不做 tab 切换（v1 无运行时改 display API；用单页滚动 + nav 替代）。
- 不做自动化 assert（人眼对照即可）。
- 不补 background-image/background-size（v1 gap，单独跟踪，不混入）。
- 不碰围栏外属性（border-radius/gradient/阴影/filter/九宫格/absolute/z-index）。
- 不做 PNG 皮肤（无 background-image；按钮/面板纯 CSS）。
- 不做多 pkg/多场景（单 pkg 单页够）。

## 12. 不变量 / 风险

**不变量**：
1. 仅用 v1 围栏内属性（§2 清单）；零 core/FFI/blob/pkg 契约改动。
2. 反馈不依赖运行时改文本（tween + 伪类 + 灯阵）。
3. 节点数 <500（性能线）；~46 卡 × ~7 节点 ≈ 350。
4. 纯 sample（HTML/CSS + C# 反馈脚本），本机改后家里机直接加载 pkg 验收。

**风险**：
1. **font-style 未落地**（grep 确认 mapping 无分支，围栏 §2 gap）→ §3.4 font-style 标灰 skip + 记 gap。font-family/weight/letter-spacing 已落地正常。background-image/background-size 同为 gap（§2）。
2. **nav target_y 写死** → 分区高度变需重算（ponytail 接受，demo 一次性）。
3. **background-image gap 限制审美** → dashboard 风弥补；若用户要 PNG 皮肤则需先补 gap（范围外）。
4. **46 卡节点数逼近性能线** → 实现期若 >500 则精简静态卡（合并对照值）。
5. **灯阵规模**（如 keydown 26 灯）→ 占空间；用紧凑横排灯 + scroll 内嵌。
