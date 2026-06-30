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

**排期已定（2026-06-29，编号统一 2026-06-30）**：v1.x 单一编号，一功能一号，按完成序递增；补丁不占号（归原号标注）。首要判据 = AI 可预测性 → v1.1-v1.3 先填"静默忽略"视觉 gap + 绘制质量（低风险快赢），v1.4-v1.7 上线控件按草稿成熟度排序。本机唯一编码机，串行推进。

| v1.x | 项（合并后） | 旧批序 | why | 草稿 | 量 | 状态 |
|---|---|---|---|---|---|---|
| v1.1 | **background-image**（+坑79 共存视觉补丁）| 1-1 | AI 必写 `background-image:url`；**围栏内已声明却零解析**（坑 57 / mapping.rs 仅 background-color），静默忽略 = 契约违背。坑79 补丁：图透明区显 bg-color（CSS 合成）| 低·机制清晰（复用 Image quad） | 小 | ✅ |
| v1.2 | **border-radius（圆角 mesh）** | 1-2 | AI 必写 CSS；围栏外静默丢弃（mapping.rs:443 装饰忽略），无反馈违背可预测性 | 低·mesh 方案待定 | 中 | ✅ |
| **v1.3** | **ColorFilter + 九宫格 slice + profiling** | 2-3·4 | ColorFilter：色调统一 + disabled 灰化升级（替代 v1 简化 color_tint grayed）。九宫格：UI 皮肤缩放不变形（配合 bg-image 按钮/面板皮肤）。profiling：draw call/GC/内存实机达标 | 低·DrawState 扩展 + polyfill | 中 | 待开 |
| **v1.4** | **虚拟化列表 `<l-list>` + soft clip** | 3-5·6 | 列表：背包/排行榜/邮件必备，v1 手搓 div+scroll 无 slot 复用。soft clip：配合列表边缘渐隐体验 | 高·v1x-deferred §1（slot 复用+防花屏不变量）+ §2 | 大 | 待开 |
| **v1.5** | **Controller / Gear / Transition** | 4-7 | 标签页/弹窗/过场/状态切换必备 | **高·v1x-deferred §4（三件套+gear_locked 同步同栈帧守卫）** | 大 | 待开 |
| **v1.6** | **富文本 `<l-rich>`（inline layout）** | 5-8 | 聊天/物品描述必备。多样式/图文混排，复用 v1 文本测量 | 中·v1x-deferred §5（cluster/font_id 字段） | 大 | 待开 |
| **v1.7** | **TextInput / IME（光标/选区/composing）** | 5-9 | 登录/搜索必备。IME 最重，可能需 rustybuzz shaping | 中·IME 草稿最缺 | 最大 | 待开 |

> **合并记录（2026-06-30）**：原 9 项按功能内聚合并为 7 号——v1.3 合 ColorFilter+九宫格+profiling（绘制质量层）；v1.4 合列表+soft clip（soft clip 为列表服务）；富文本/IME 拆开不合并（IME 最大工作量，合一号 spec 失控）。补丁（如坑79）不占号。

---

## 2. v other — 编辑器工作流（独立并行，不阻塞主线）

> **2026-06-30 修订**（brainstorm + open-design 源码调研 + 实测 `od project import` 后定稿，详见 `docs/superpowers/specs/2026-06-30-editor-workflow-design.md`）：
> - 围栏规则**不进 DESIGN.md**，放工作区 `CLAUDE.md`（按 harness，Claude Code 自动读 cwd）。DESIGN.md 只写风格。
> - **砍 grep linter 层**：`lint-artifact.ts` 是 open-design 硬编码函数非扩展点，改不动；围栏把关靠 `loomgui_pkg` 打包验证 + skill 引导 AI 自检。
> - **不提供正式 design-system**：每个 UI 风格由设计师 prompt 驱动，design-system 仅作测试夹具存 `samples/`。
> - skill **封装 loomgui_pkg**（验证器+打包器进 skill，不向设计师暴露）。
> - 围栏权威清单 = `docs/design/fence.md`（单一真相源 `loomgui_core/tests/fence_contract.rs`）。

**壳 = open-design 桌面 app**（不自建；Apache-2.0；nexu-io；~18K star；agent-driven 生成器；插件/扩展架构，不改源码在上面工作；实装 stable 通道 Win 桌面 app 验证）。
- 不选 design.md（Google）：类别错配（token 字典 + 字典校验器，无编辑器/预览/渲染，契合 ≈10%）。
- 不自建壳：复用 open-design 省 项目管理/对话/导出/部署 基建。

**机制**（调研确认）：`od project import <baseDir>` 导入指定目录为工作区 → daemon 把 project cwd = baseDir → open-design spawn harness（Claude Code 等）在该 cwd → harness 自动读 cwd 的 `CLAUDE.md` + `.claude/skills/`。

**LoomGUI editor 层**（shell-agnostic，模板源 `editor/`，init 脚本注入设计师工作区）：

1. **init 脚本**（`editor/init.mjs`，Node 单文件）：交互输工作区路径/输出路径/harness → 拷围栏规则 + skill 进目标工作区。CLAUDE.md 增量合并（标签包裹，不覆盖用户已有）。
2. **围栏规则**（`editor/rules/<harness>/CLAUDE.md.tmpl`）：围栏权威清单见 `docs/design/fence.md`。AI 守围栏生成 HTML+CSS。
3. **skill**（`editor/skill/loomgui-editor/`，封装 loomgui_pkg 不暴露）：教 AI 围栏生成 + 生成完跑 `pack.mjs` 验证+打包（违规非零退出 AI 自纠，合规产出 pkg.bin）。
4. **打包桥**：`loomgui_pkg` CLI（已存在，验证+打包合一）。

**预览妥协**：open-design Chromium iframe ≠ taffy（字体度量/flex 差异/margin 折叠/position:absolute 脱离流分歧）。skill 教"信围栏规则别信预览不可信项"（fence.md §6）；真实靠 Unity 验（家里机）。**v2 WASM 跑核心**做零偏差预览替换近似。

**围栏验证**（单一真相源）：`loomgui_core/tests/fence_contract.rs` 可执行围栏契约（支持项断言生效 + 围栏外项断言静默忽略）。`cargo test -p loomgui_core fence_contract` 是防漂移门。打包器围栏验证器（FENCE_TAGS）是打包期最终把关。

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

## 7. 下一步

1. **v1.x 编号统一**（§1，2026-06-30）：7 号方案，v1.1/v1.2 已完成，下一项 v1.3。
2. **v1.1 background-image + v1.2 border-radius** 已完成（含坑79 共存视觉补丁）；家里机 PlayMode 验收债待补，不阻塞 v1.3。
3. **brainstorm v1.3**（ColorFilter + 九宫格 slice + profiling）：设计 → plan → SDD。
4. （并行可选）brainstorm v other 第一版拆解（design-system / linter / skill / 打包桥 谁先）—— 独立 workstream，不阻塞 v1.x。
