# LoomGUI v0 骨架设计（spec）

- 日期：2026-06-18
- 状态：待审 → 批准后进 writing-plans
- 依据：`docs/design/00-main-design.md`（v1 实现真相源）、`docs/roadmap/v1-scope.md` §6（go/no-go 门第 3 条）
- 产出方式：superpowers brainstorming 流程

---

## 1. 目的

v0 是 v1-scope §6 go/no-go 门的第 3 条——**v1 大工期开工前的纯 Rust 探针**。两个验证目标：

1. **验证 Rust 核心能力**：parse/style/layout/text/render 各段在 Rust 里能跑通。其中文本测量（ttf-parser + unicode-linebreak）是最不确定、最该早验的一段。
2. **解耦 FFI/Unity 风险**：证明核心能在没有引擎、没有 FFI 的情况下独立跑通端到端——核心是纯库，引擎只是消费 `Vec<RenderNode>` 的后端。

v0 不碰引擎、不碰 FFI，纯 Rust。通了再投 v1 Unity 大工期。

---

## 2. 范围边界

**做**：`HTML/CSS fixture → parse → style → scene → text → layout → render → stage.tick → render_nodes JSON`，端到端。

**不做**（见 §7 defer 表）：event/anim、FFI、Unity 后端、打包器磁盘格式、纹理加载、NativeHost/virtualization/shape mask。

**范围决策记录**（brainstorming 钉定）：

- **打包器**：内存直通（parse→scene 直喂，跳过磁盘包），打包器磁盘格式 defer v1 第一阶段。理由：打包器是 v1 大头（~1.5 人月，G1），byte 格式非核心风险点；v0 验核心能力，不该被体力活拖住。
- **文本**：全做（测量 + SOA + Text 变体）。理由：文本是 Rust 侧最大未知，v0 不验它等于把最大风险留到 v1，违背解耦初衷；测量与 SOA 本就一体，无省事折中。
- **纹理**：占位 tex_id（src 哈希/固定 id，UV 占位全图）。理由：纹理加载是后端职责（§14），v1 在 Unity/Godot 侧做；v0 纯 Rust 不该验一个 v1 不会在 Rust 侧做的事。

---

## 3. 工程组织（crate 切分）

单 workspace，`loomgui_core` 一个 lib crate + `examples/` 跑端到端 + `tests/` 快照。

```
loomgui/
├── Cargo.toml                       # workspace
├── loomgui_core/
│   ├── src/
│   │   ├── parse/    # scraper HTML + cssparser CSS + 极简选择器匹配器
│   │   ├── style/    # cascade + CSS值→taffy映射 + 运行时状态查询
│   │   ├── layout/   # taffy 集成 + MeasureFunc
│   │   ├── scene/    # Node 树、Container/Text/Image/Button
│   │   ├── render/   # VertexBuffer/MeshFactory/FairyBatching/clip/RenderNode
│   │   ├── text/     # ttf-parser + linebreak → TextLayout SOA 三表
│   │   ├── asset/    # v0 极简：img src→占位 tex_id（无 load/图集）
│   │   └── stage.rs  # Stage::tick(input,dt) → Vec<RenderNode>
│   ├── examples/v0_snapshot.rs      # runner：HTML fixture → JSON（cargo run --example）
│   └── tests/snapshot.rs            # 快照测试（insta）
```

- v0 砍 `event/`、`anim/` 两模块（静态快照不接输入、不跑动画）。
- runner 用 `example`（既是测试入口也是手动验证入口），不独立 crate。

对照主文档 §16：workspace 框架照搬，v0 只实现 core 的子集 + runner + tests，`loomgui_pkg/ffi_c/unity/editor` 全 defer。

---

## 4. 端到端管线

| # | 步 | 模块 | 主文档 | v0 说明 |
|---|---|---|---|---|
| ① | parse | parse | §5.1 | scraper HTML + cssparser CSS + 自写 ~100 行极简选择器匹配器 |
| ② | style | style | §5.2-5.3 | cascade（继承/合并/顺序）+ 打包期继承展开 + CSS值→taffy映射 |
| ③ | scene | scene | §6 | 构建 Node 树（Container/Text/Image/Button，围栏 v1 子集） |
| ④ | text | text | §9 | ttf-parser 度量 + unicode-linebreak 断行 → TextLayout SOA 三表 |
| ⑤ | layout | layout | §7 | taffy flexbox + MeasureFunc（文本/图片 intrinsic 尺寸） |
| ⑥ | render | render | §8 | 几何 MeshFactory + FairyBatching sort_key + 绘制序 + rect clip 意图 |
| ⑦ | asset | asset | §12（简化） | 内存直通：③→scene 直喂；img src→占位 tex_id，无 load/图集 |
| ⑧ | stage | stage | §2.6 | `tick(空输入, dt=0)` 首帧直出 → `Vec<RenderNode>` |

⑧ 之后 serde 序列化为 JSON 快照。

---

## 5. 输出形态（render_nodes JSON）

`RenderNode` 树经 serde 序列化（公共头 + payload enum，v1 契约 §8.7）。变体：`Mesh` / `Text` / `Unchanged`。

- Mesh：quad 几何 + 颜色（bg-color/opacity/tint）+ 占位 `tex_id` + UV 占位全图 + blend + sort_key。
- Text：TextLayout SOA 三表（glyphs/runs/lines）+ 颜色 + sort_key。
- 快照用 `insta` 锁定，变更走 `cargo insta review`。

---

## 6. 验收标准（"v0 跑通"）

1. 一个能演示的 fixture：`div` flex 布局 + 文本 + `img` 占位 + `rect mask` 的面板。
2. `cargo run --example v0_snapshot` 产 JSON；`cargo test` 快照全绿。
3. 每个验证点有对应 fixture 覆盖：
   - 解析（HTML/CSS → DOM/CSSOM）
   - cascade（继承/合并/顺序/specificity）
   - flex 布局（方向/gap/对齐/grow-shrink/百分比）
   - 文本 SOA（测量/断行/align/baseline）
   - 几何（padding/border/mesh 顶点）
   - 批合 sort_key + 绘制序
   - rect clip 意图

---

## 7. defer → 落地表

| defer 项 | 落地阶段 | 依据 |
|---|---|---|
| 打包器磁盘格式（.pkg.bin + 图集 + 迁移器） | v1 第一阶段（G1，紧接 v0） | v1-scope §3 G1 / §5 |
| 纹理加载（PNG 解码 / GPU / TexId 注册） | v1 第二阶段（G7，后端职责） | v1-scope §3 G7 |
| FFI（csbindgen + SOA arena + C ABI） | v1 第二阶段（G11） | v1-scope §3 G11 / §14 |
| event（命中/输入/拖拽/滚动仲裁） | v1 第二阶段（G4） | v1-scope §3 G4 |
| anim（GTween/Timers）+ ScrollPane | v1 | §11 / §12.7 / G14 |
| Unity 后端镜像（DrawState/GO 池） | v1 第二阶段（G9-G14） | v1-scope §3 |
| NativeHost / virtualization / shape mask | v1.x | roadmap/v1x-deferred.md |

每条有归属，不悬空。

---

## 8. 工期估算

~2-3 周（文本全做扩展了原 1-2 周）。分阶段：

- parse + style：~4 天
- layout（taffy + MeasureFunc）：~3 天
- text（最深，ttf-parser + linebreak + SOA）：~5-7 天
- render + 快照测试：~3-4 天

---

## 9. 风险与未决

- **文本测量深度**：v0 文本全做是最大工期变量。若 taffy MeasureFunc 反复调用 + 文本测量撞性能墙，登记"测量缓存"（v1x-deferred §5），v0 naive 重算先过。
- **line-height / kinsoku 公式**：主文档只留原则（§9.1），v0 实现期对照 Chrome 调，不预设公式。
- **JSON 快照 schema 稳定性**：v0 期间 schema 会随实现微调；快照锁定让变更显式，跨版本比较能力 v0 不做（够用即可）。

---

## 10. 文档同步（本 spec 引入的 gap 回写）

写本 spec 时同步回写 `docs/roadmap/v1-scope.md`（消除 v0 定义与 spec 不一致）：

- §6 第 3 条：v0 定义从「含打包→加载」改为「内存直通（parse→scene 直喂）」。
- §1 资源 / §3 G1：打包器补「v0 后、v1 第一阶段落地」阶段归属。

回写随本 spec 同 commit。
