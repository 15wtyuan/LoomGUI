# LoomGUI 路线图（v1.x / v other / v2）

> **定稿 2026-06-29**。v1 架构验证完成（v1a-v1e + showcase 46 卡 + 家里机验收坑 58-73 修复），桌面 Mono 可演示。本文是 v1 之后的三板块路线决策。
> **依据**：v1-vs-mature-baseline 对标报告（fgui）+ design §0/§1（AI 驱动核心动机）+ v1x-deferred（机制草稿）。

---

## 0. 当前状态（TL;DR）

- **v1 = 架构走通 + 桌面可演示**（demo-grade，非上线）。
- **距上线三缺口**：① 移动平台 ② 编辑器/AI 工作流闭环 ③ 关键控件（列表/富文本/输入/状态机）。
- **差异化已立**（别丢）：AI 可预测性（HTML-as-DSL，AI 能编辑+预测渲染）+ flexbox（超 fgui Relations）+ Rust 跨引擎共享核心 + 围栏验证器（打包期挡违规，坑 57 实证）。

---

## 1. v1.x — 上线功能必备 + AI 可预测性

8 项（优先级待 brainstorm 排）：

| # | 项 | why | 机制草稿 |
|---|---|---|---|
| 1 | **虚拟化列表 `<l-list>`** | 背包/排行榜/邮件必备。v1 手搓 div+scroll 无 slot 复用 | v1x-deferred §1（slot 复用模型） |
| 2 | **Controller / Gear / Transition** | 标签页/弹窗/过场/状态切换必备 | v1x-deferred §4（状态机+联动+时间轴） |
| 3 | **富文本 `<l-rich>` + TextInput/IME** | 聊天/物品描述/登录/搜索必备 | — |
| 4 | **九宫格 `-l-slice` + 实性能 profiling** | UI 皮肤缩放不变形 + draw call/GC/内存实机达标 | — |
| 5 | **border-radius（圆角矩形 mesh）** | AI 必写 CSS，补 v1 gap（围栏外静默忽略违背可预测性） | — |
| 6 | **background-image** | AI 必写 `background-image:url`，补 v1 gap（坑 57）。按钮/面板 PNG 皮肤 | — |
| 7 | **soft clip（羽化）** | 配合虚拟化列表边缘渐隐体验 | v1x-deferred §2 |
| 8 | **ColorFilter（4x5 颜色矩阵）** | 色调统一 + disabled 灰化升级（替代 v1 简化 color_tint grayed） | — |

---

## 2. v other — 编辑器工作流（独立并行，不阻塞主线）

**壳 = open-design**（不自建；Apache-2.0；nexu-io；~18K star；agent-driven 生成器；本有 design-system 契约 + grep linter 现成机制；自托管 docker）。
- 不选 design.md（Google）：类别错配（token 字典 + 字典校验器，无编辑器/预览/渲染，契合 ≈10%）。可借鉴其 linter 架构。
- 不自建壳：复用 open-design 省 项目管理/对话/导出/部署 基建。

**LoomGUI 围栏层**（shell-agnostic，配置/插件级，不改 open-design 源码）：

1. **design-system**（open-design 契约 `design-systems/loomgui/`）：
   - `DESIGN.md` 写死围栏规则（flex only / 无 grid / 无 position:absolute / 无行内流 / 标签白名单 div-span-img-button / gap 标灰规则）——给 AI 的围栏 prompt。
   - `tokens.css`：围栏 design token（暗色 dashboard 调色 + 字号 + 间距）。
   - `components.html`：围栏组件库（Button/Card/List/ScrollPane/Text，纯围栏 CSS 实现）。
   - `components.manifest.json`。
2. **grep linter**（加 open-design `lint-artifact.ts` 扩展点）：挡 `display:grid`/`position:absolute`/`float`/围栏外标签（regex `<(?!div|span|img|button)`）——AI 生成时快速反馈自纠。
3. **skill**（独立于 open-design）：教 AI 用 LoomGUI 围栏 + 近似预览不可信项（v1-scope §2.1 可信清单：信 flex/gap/color/px，不信 margin 折叠/文本换行像素）。
4. **打包桥**：open-design 导出 HTML/CSS → `loomgui_pkg` → pkg.bin + atlas → Unity 加载。

**预览妥协**（A 路径已知）：open-design Chromium iframe ≠ taffy（字体度量/flex 差异/margin 折叠）。skill 指导"信围栏规则别信预览不可信项"；真实靠 Unity 验（家里机）。**v2 WASM 跑核心**做零偏差预览替换近似。

**两层围栏验证**（互补）：open-design linter（grep，AI 生成时挡，快速粗筛）+ LoomGUI 打包器围栏验证器（parse FENCE_TAGS，打包时挡，最终把关）。

---

## 3. v2 — 平台 + 生态 + 特效

| # | 项 | why |
|---|---|---|
| 1 | **移动 + IL2CPP + WebGL** | 平台（原 v1.x 移出，移植工作重单独 v2）。上线游戏必备 |
| 2 | **多引擎（Godot）** | 验证跨引擎一致性（design G1）。Rust 核心共享核心的价值兑现 |
| 3 | **多语言 / 异步加载 / 热更新** | 上线运营必备 |
| 4 | **WASM 零偏差预览** | 替换 v other 近似（design §2.1 终极）。AI 闭环所见即所得 |
| 5 | **shape mask / alpha mask / paintingMode** | 异形遮罩/离屏 RT/特效隔离（v1x-deferred §2） |
| 6 | **BlurFilter / DropShadow / Glow** | 模糊/阴影/发光（PNG 皮肤能补，故推 v2） |
| 7 | **BlendMode 扩展（Add/Screen/...）** | v1 仅 Normal。特效混合 |
| 8 | **椭圆 / 多边形 / RadialFill mesh** | 几何扩展（v1 仅矩形 quad） |

---

## 4. 关键决策记录（why）

- **移动+IL2CPP 推 v2**（非 v1.x）：v1.x 聚焦功能必备，平台移植工作量重，单独 v2。
- **编辑器用 open-design 不自建**：复用其 design-system + grep linter 现成机制，省自建壳基建。design.md 类别错配（token 字典非编辑器）不选。
- **shape mask/filter 拆分**：border-radius/background-image/soft clip/ColorFilter 进 v1.x（AI 必写不可推 + 配合功能）；特效（blur/glow/异形 mask/blend 扩展）推 v2（PNG 皮肤/九宫格能补）。
- **border-radius + background-image 进 v1.x**：AI 写 CSS 必写，围栏外静默忽略违背 AI 可预测性（design §1.1）——实现优于 skill 提醒"别写"。
- **v other 并行**：编辑器工作流独立于 runtime，不阻塞 v1.x/v2。

---

## 5. 对标基线 + 成熟度

- **对标 FairyGUI**：10 年沉淀，跨引擎（Unity/Cocos/UE/Laya），可视化编辑器，30 示例，MIT，社区成熟。LoomGUI 精神继承 + 布局替换（flexbox 代 Relations）。
- **v1 成熟度**：架构完整（FFI/打包/Unity 后端/事件/滚动/动效/状态 全）+ 桌面可演示 + 性能 500 节点静态无卡顿。距上线 = v1.x（功能）+ v other（编辑器）+ v2（平台）。
- **LoomGUI 差异化**（对标 fgui 的竞争力所在）：AI 可预测性（HTML-DSL，fgui .fui 二进制 AI 不能编辑）+ flexbox（流式/响应式/内在尺寸，超 fgui 锚点）+ Rust 跨引擎共享核心（fgui 各引擎独立 SDK）+ 围栏验证器（AI 第一道反馈）。

---

## 6. 机制草稿 / 契约位置

- v1.x/v2 机制细节（虚拟化 slot / shape mask 两遍 DFS / Controller-Gear / 文本 fallback 链）：`v1x-deferred.md`（草稿，实现期定）。
- v1 围栏冻结子集：`v1-scope.md` §2。
- 设计契约（渲染树/分层/DSL 规范）：`docs/design/00-main-design.md`。
- v1 实现历史（v1a-v1e + showcase）：git log + `.claude/skills/knowledge-reference/`（坑 1-73 + 各层机制）。

---

## 7. 下一步（compact 后选）

1. **brainstorm v1.x 8 项优先级**（依赖关系 + 上线价值排序，定谁先做）。
2. **brainstorm v other 第一版拆解**（design-system / linter / skill / 打包桥 谁先）。
