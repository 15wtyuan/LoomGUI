# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 这是什么

LoomGUI = 跨引擎游戏 UI 框架。**Rust 核心（引擎无关纯库）+ 引擎后端（Unity 首发）**，HTML/CSS 子集作 DSL，taffy flexbox 布局，自绘渲染。核心目的：**AI 驱动的界面拼装**——HTML 作 DSL，让 AI 既能编辑（文本）又能预测渲染结果（AI 对 HTML/CSS 有强先验）。每条 DSL 决策的首要判据都是"AI 读这段 HTML 能否正确预测渲染出的 UI"。

对标 FairyGUI（参考实现在 `temp/FairyGUI-unity/`，只读）。差异化：HTML/CSS DSL（vs fgui 的 `.fui` 二进制 AI 看不懂）、flexbox（vs 锚点 Relations）、一份 Rust 核心多后端、围栏验证器。

## 构建 / 测试命令

```bash
# 核心（引擎无关纯库，可单测）
cargo build -p loomgui_core
cargo test  -p loomgui_core

# 打包器 CLI（HTML+CSS+资源 → .pkg.bin + 图集；复用核心 parse 层）
cargo build -p loomgui_pkg
cargo test  -p loomgui_pkg

# FFI（C ABI；csbindgen 在 build.rs 里重新生成 C# 绑定）
cargo build -p loomgui_ffi_c

# 整个 workspace
cargo test
```

**Feature gate（`parse`）**：`scraper`+`cssparser` 是可选的，由 `parse` feature 控制（core/pkg/ffi 默认开）。运行时不带 HTML 解析器。不带 parse 编译全部：
```bash
cargo build --no-default-features --all-targets   # 按 crate，或 workspace 级
```
`snapshot` 集成测试需要 `parse`（`required-features`）。

**跑单个测试 / 围栏门**：
```bash
cargo test -p loomgui_core fence_contract   # ← 围栏契约门（见下）
cargo test -p loomgui_core --test snapshot -- <name>
```

**基准测试**：`cargo bench -p loomgui_core`（criterion，`frame_emit`）。

### Rust → Unity .dll 闭环（Windows 本机是唯一的编码机）

按记忆/工作流：本机负责 build `.dll` + commit + push；家里机只做 Unity PlayMode 验收。**任何** Rust 改动后必须重编 + commit `.dll`，否则家里机测不了。

```bash
cargo build -p loomgui_ffi_c --release
cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll
```
- **拷贝时 Unity 必须关着**（它锁 .dll）。
- **stale .dll 诊断**：PlayMode 全不渲 + Console 干净 → `md5sum target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`；不等 = stale（Rust 改了 blob/ABI，.dll 没换）。
- 入库的 `.dll` + csbindgen 生成的 `LoomGUIBindings.cs` 在 `loomgui_unity/Assets/Plugins/LoomGUI/`（`**/Plugins/**/*.dll` 和 bindings .cs 是 gitignore 白名单例外；其余 native 产物一律忽略）。
- Unity 6.5，URP。打开 `loomgui_unity/`，PlayMode 从 `StreamingAssets/` 加载 `.pkg.bin`。

## 架构（大局——权威契约读 `docs/design/main-design.md`）

Workspace 成员：`loomgui_core`、`loomgui_pkg`、`loomgui_ffi_c`（+ `loomgui_unity` Unity 工程、+ `editor/`、+ `samples/`）。

**分层、单向数据流、引擎对象不进核心：**
```
HTML/CSS DSL → 打包器（构建期；复用核心 parse/style）
  → Rust 核心：parse(scraper+cssparser+自写 ~100 行选择器匹配器)
    → style(cascade) → scene(Node 树，代际 NodeId)
    → text(ttf-parser 测量 → TextLayout) → layout(taffy flexbox solve)
    → render(Vec<RenderNode>) → stage.tick
  → FFI（csbindgen：Rust ↔ C ABI ↔ C# P/Invoke）
  → Unity 后端（GameObject+MeshRenderer 镜像渲染树；输入采集；资源加载）
```

**关键边界**（别跨越）：
- **Rust 核心**拥有：parse、style、layout、场景图、事件、动画、几何生成、批合、裁剪/顺序。产出 `Vec<RenderNode>` + 命中结果 + 事件。**不持任何引擎对象、不碰 GPU。** 非文本几何在核心生成；**文本 mesh 是例外**——核心只产 `TextLayout`，后端光栅化（动态字形 UV 只有引擎字体 API 才有）。
- **引擎后端**拥有：输入采集、渲染树→原生镜像、mesh 上传、DrawState 缓存+提交、资源加载代理。**不解析 DSL、不算布局、不生成几何。**
- 核心不知 GameObject/CanvasItem；后端不知 DSL/taffy/几何。

**关键架构不变量**（违反 = 隐 bug）：
- **`<div>` 永远是 flex 容器**（默认 `flex-direction: column`）。无浏览器 block/inline flow——只有 flex item 参与布局。行内混排（一元素内文本+元素+文本）是**编译期报错**，不降级。只有 `div`/`span`/`img`/`button` 标签；围栏外标签报错。
- **transform（x/y/scale/rotation）是渲染/命中层，不进 taffy**——改它只置 `transform_dirty`（刷新命中几何），绝不触发 `solve`。`style_size`/flex 进 taffy → `layout_dirty` → solve。位置/缩放动画走 transform 所以廉价。
- **所有布局帧末一致**：改属性只置 dirty；每帧一次 `solve`（vs fgui 立即推 DisplayObject）。
- **命中几何** = `layout_rect` 经累计（含父链）transform 变换后的 AABB。事件路由本身在**业务侧**（C# `LoomEventHandler`），非核心——核心只做命中 + hover/active diff + 伪类 rematch。
- **坐标系**：核心 = 左上原点、y 向下。核心代码无 `height-y` 翻转。y-flip 是**后端根 Stage 一次性变换**（Unity 根 GO scale (1,-1,1)；Godot flip = 单位矩阵，2D 本就 y 下）。
- **代际 NodeId**：`NodeId(pub u32)`（高 20bit index + 低 12bit gen），FFI/C# 透明不透明句柄；`remove_node` gen++ 让旧句柄自动失效。内部用 `SlotMap<DefaultKey, Node>` 桥接（slotmap 的 64-bit key 装不下 u32，见 `scene/node.rs`）。
- **单一动画时钟**：`TweenManager::update(dt)` 是唯一时钟。Controller/Gear/Transition 都不自驱——全往它提交/kill tween。ScrollPane 物理是**例外**（自维护 tween，绝不用 GTween——content 异步变化时 GTween 的固定 end 会跳变）。

**FFI 契约**（§13.3）：每帧 Rust 产出一个 SOA 公共头（渲染节点公共字段，当前 18 列）+ 按类型分区的扁平 arena（mesh_arena、text_arena）。C# 在 tick 内原子拷贝（拷贝非 pin），Rust 下帧 reset。`Unchanged` payload 变体 = 本帧该节点不 dirty，不进 arena，后端不动其镜像。C# 用 `Span<byte>` + `BinaryPrimitives` 读——**禁 `Marshal.PtrToStructure`**（IL2CPP struct 对齐坑），**禁跨 FFI 裸指针**。IL2CPP：回调必须 `static` + `[MonoPInvokeCallback]`。

## 围栏（Fence）——单一真相源

LoomGUI 只支持 HTML/CSS 的**明确子集**，称"围栏"。这是项目漂移高发区。

- **权威真相源 = `loomgui_core/tests/fence_contract.rs`**（可执行契约）。`docs/design/fence.md` 是人类可读副本；**不一致时测试赢**。`samples/CLAUDE.md` 带一份注入的围栏规则摘要给编辑器用。
- **改围栏 = 改 `fence_contract.rs` 测试 + `fence.md`**，不改 `main-design.md` §3（那节只写哲学，避免漂移）。
- **围栏门**：`cargo test -p loomgui_core fence_contract`——build .dll 前跑、改 `apply_decl`/`FENCE_TAGS`/选择器后跑。
- 两类围栏外行为（均**测试锁定**，别靠 grep 推断）：围栏外标签 + 行内混排 → **编译期报错**（parse 失败、打包器拒收）。围栏外 CSS 属性（如 `position:absolute`、`clip-path`、`cursor`）→ **静默忽略**（`apply_decl` 返 `false`）。
  - `position:relative` 教训："grep 无 match" ≠ "不支持"——可能是依赖默认值（taffy `Style::DEFAULT.position = Relative`）。声明支持前先核实依赖默认值 + 补测试。

## 在本仓库怎么干活

- **实现任何机制前，先对照 FairyGUI 源码**（`temp/FairyGUI-unity/`）。LoomGUI 的渲染/对象模型/批合/事件/动画/资源管线全面借鉴 fgui。先读对应 fgui 文件看它怎么做，再定设计。fgui 是 Built-in RP——URP/shader/材质 API 要适配。
- **设计文档 vs 踩坑**：`docs/design/main-design.md`（设计契约/当前实现真相）、`docs/design/fence.md`（围栏）、`docs/roadmap/roadmap.md`（范围+机制草稿）、`docs/pitfalls.md`（踩坑全库 + 依赖 API 适配，开工前读它查"具体怎么干 + 坑在哪"）、`docs/superpowers/specs|plans/`（历史 per-feature 记录）。
- **Rust edition 2021**，依赖钉版本：`taffy 0.5`、`ttf-parser 0.20`、`cssparser 0.34`、`scraper 0.19`、`slotmap 1.1`、`csbindgen 1`。snapshot 测试用 `insta`。
- `Cargo.lock` 入库（根级，尽管 `.gitignore` 有通用 `Cargo.lock` 行——它是被追踪的）。
- `editor/` 是 v-other 编辑器工作流（open-design 壳 + `loomgui-editor` skill + 围栏规则注入）。`samples/` 是 v1-showcase 基线 + 动态树 demo（dyn-mail/leaderboard）+ 编辑器测试夹具。`samples/CLAUDE.md` 是编辑器生成的，gitignored。
- 用户只读中文——问答/选项/总结用中文；代码/commit 照旧英文。

## 调试技巧

**dump_*.rs 诊断 example**（pkg.bin 路径，验 core 实际状态而非猜代码）：
- `dump_text` — 文本换行（验 known.width 来源、行数、pen 坐标）
- `dump_img` — 图片尺寸（css.w/h、rect、tex、闭包 `known.w`）
- `dump_scroll` — 滚动（overlap、scroll_pos、content_size）
- `dump_render` — 渲染节点（rect、bg、UV）
- `dump_sw` / `dump_bg` — 节点 base_style（验是否进 pkg）

**跨层特性 PlayMode 报错**（拖不动/晃动/错位）先 example 实测 core 状态（overlap/scroll_pos/content_size）再改，避免盲改物理掩盖 layout 根因。dump 边界/状态取证，别靠"浮点边界/epsilon"症状层猜测。

**改 parse-time 逻辑必重打 pkg**：`Node.base_style` 是打包期 `resolve_styles` 产物（不变）。改 cascade/mapping/parse 只重编 .dll 不够，须 `cargo run -p loomgui_pkg` 重打 pkg（html/css 未变也要）。纯 runtime（render/layout measure/scroll/anim）改 .dll 即可。

**C# 本机不编译（无 Unity）**：C# 代码本机写不编译，家里机才暴露编译错。csbindgen 不为 `#[repr(C)]` struct 生成 C# stub，须手补 C# 镜像文件；新增/改 FFI struct 须同步镜像（坑 35）。

## API 适配方法论

**plan/草稿的 API 常与 crate 实际不符**——遇编译错按 crate 实际源码（`~/.cargo/registry/src/<crate>-<ver>/src/`）调，**勿硬改依赖版本**。具体 crate 差异见 `docs/pitfalls.md` §3。

**FFI 边界 C-like enum 必须 `#[repr(uN)]`**（u8/u16/u32），否则判别 isize 跨平台不稳 + 撑大 struct（坑 34）。永远 `size_of::<T>()` 断言 ABI struct 尺寸，别信草稿。

**Rust FFI 返字符串一律 ptr+len**（不靠 NUL）；C# 侧禁用 NUL-scan 读法（坑 16）。

**移植 fgui 算法**：带数字后缀的变量名（`v2`/`pos2`/`d2`）不能望文生义——须读源码表达式确认是平方还是线性命名残留（fgui `v2` 是 `|v|·scale` 非 `v²`，坑 54）。算法移植按源码逐行 trace 验，勿按文字描述想当然。

## 坑索引

完整踩坑记录见 `docs/pitfalls.md`（坑 1-99+，按编号递增）。新踩坑继续编号递增，写法：症状/根因/解决/教训。

**易重踩的高频坑**：
- 坑 10 stale .dll、坑 66 改 parse-time 必重打 pkg、坑 41/43 跨 crate 签名变更漏改
- 坑 34 `#[repr(u8)]`、坑 35 csbindgen struct 手补镜像、坑 39 borrow_events out_len 是 count 非字节
- 坑 57 围栏外标签硬挡/属性静默死 CSS、坑 94 l-container 假自定义元素
- 坑 54 fgui v2 非 v²、坑 79 shader tex×vcol 非 CSS 合成
