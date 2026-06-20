# v1b.1 — 打包器 + 二进制包 + 运行时加载器 设计

- **日期**：2026-06-20
- **状态**：设计（待实现）
- **范围**：v1b 的第一个子项目（A）。B 真纹理 / C 图集 / D CJK 文本为各自后续 spec。
- **依据**：主设计 `docs/design/00-main-design.md` §5.5、§12、§14.3、§16；roadmap `docs/roadmap/v1-scope.md` §3-G1、验收 #6；`docs/roadmap/v1x-deferred.md` §6。
- **参考实现**：`temp/FairyGUI-unity`（包格式读取侧，§12.2 借鉴对象）。

---

## 1. 背景与目标

v1a 已让「inline HTML/CSS → Rust 核心 → Unity 渲染」端到端跑通（静态色块 + 文本 + rect mask + 500 节点压测，merged main @ 9889afa）。但运行时仍走 `Stage::load_inline(html, css)` 内联串，**没走二进制包**。

主设计 §12.1 的目标：编辑期 HTML+CSS+资源清单 → 发布产物 `.pkg.bin` 单一二进制 blob；**运行时只认二进制**，HTML 解析只在打包器。v1b 第一阶段（本 spec）落地这条——把加载路径从内联串换成二进制包。

**目标**：从 HTML 经打包器产出二进制包，运行时加载该包渲染，输出与 inline 路径**逐像素/逐节点等价**。即验收 #6「从 HTML 经打包器产出二进制包加载」达成。

---

## 2. 范围

**做**：
- `loomgui_pkg` crate（CLI）：HTML+CSS → `.pkg.bin`。
- `.pkg.bin` v1 二进制格式（扁平 + stringTable）。
- core 新 `asset` 模块：`write_package` / `read_package`。
- `Stage::load_package(bytes)`。
- parse feature 拆分（`scraper`/`cssparser` optional，gate 在 `'parse'` 后）。
- `Scene::build` 重构（共享建树，防逻辑重复）。
- FFI `loomgui_stage_load_package`。
- Unity `LoomStage` 读 StreamingAssets 包文件传 bytes。

**不做（各自后续 spec / defer）**：
- B 真纹理加载（散图 → Texture2D → TexId 注册）。v1b.1 包带 `<img src>` 引用，仍渲占位。
- C 图集打包（shelf/guillotine + TextureView）。
- D 文本 CJK/多字体（text_arena 升 runs/lines 三表）。
- 动态规则表（伪类重匹配）——消费方在 v1c（事件/状态），现在序列化一个无消费者。
- indexTable/Seek 块跳转、压缩、多包/跨包 URL `loom://`、分支/多分辨率（v1x-deferred §6 已记为 v1.x）。

---

## 3. 关键边界：`.pkg.bin` 是 Rust-internal

**包由 `loomgui_pkg`（Rust）写、由 core 运行时（Rust，在 .dll 内）读。C# 永不解析包**——Unity 只把文件读成 bytes 透传给 FFI。

这与 frame blob（渲染输出，Rust↔C# 跨语言契约，需 C# reader + 跨语言字节对齐）本质不同：
- 包格式契约只在 Rust 内部 → 序列化可直接投影 `ResolvedStyle`，无跨语言对齐风险。
- 无需 C# 侧 `FrameBlob` 式的 `Span<byte>` reader。
- Unity 侧职责仅：读文件 → `byte[]` → `fixed` 钉住 → 传 `load_package`。

---

## 4. 包内容模型

包 = **预编译的 `scene::Node` 树**（`build_scene` 的输出）序列化。

为什么是 Node 树而非 ElementTree：`ElementTree`/`StyleSheet` 在 `parse/`，要被 `'parse'` feature gate 掉（§11），运行时无此类型。`scene::Node` 在 `scene/`（常驻）。故包序列化 Node 树，运行时反序列化直进 scene，**不调 `build_scene`（从 ElementTree 那条）**。

每节点记录：
- `parentIndex`（i32，-1=根）。
- `kind`（u8：0=Container / 1=Button / 2=Image / 3=Text）。
- `ResolvedStyle`（taffy + 视觉字段全量）。
- payload 字符串索引：Text→content、Image→src（裸文本 div 在 v1a Phase 1 已是 Text 子节点，故 content 在 Text 节点上）。

**不**序列化：`NodeId`（运行时按 DFS 重分配 0..N）、`taffy_id`（运行时 `Scene::build` 重建 taffy 树）、`layout_rect`/`clip_rect`（运行时 solve 算）、dirty 标志（运行时态）。

**StringTable 内容**：text content、image src、`font_family`（可重复，去重有效）。类名/tag 不进——parse 期产物，`build_scene` 后 Node 不再持。

---

## 5. `.pkg.bin` v1 二进制格式（扁平 + stringTable，LE）

```
Header (28B 定长):
  magic         u32 = 0x4D4F4F4C   // 磁盘字节(LE) = 4C 4F 4F 4D = ASCII "LOOM"
  formatVersion u32 = 1
  flags         u32                // bit0=compressed（v1=0），余保留 0
  nodeCount     u32
  stringCount   u32
  rootSizeX     f32                // 设计稿宽（运行时 Stage root_size）
  rootSizeY     f32

StringTable:
  repeat stringCount:
    len   u16
    bytes [u8; len]  UTF-8          // 索引 = 顺序号（ReadS 风格）

NodeBlock (nodeCount 条，DFS 先序；parentIndex 指数组下标):
  per node:
    parentIndex i32                 // -1=根
    kind        u8                  // 0=Container 1=Button 2=Image 3=Text
    style       StyleRecord         // ResolvedStyle 二进制投影（定长，字段序定死）
    textIdx     u16                 // Text 的 content；非 Text = 0xFFFF(null)
    srcIdx      u16                 // Image 的 src；非 Image = 0xFFFF(null)
```

### StyleRecord
`ResolvedStyle`（`style/resolved.rs`）的逐字段二进制投影：
- `taffy_style: TaffyStyle` 的**全字段**（flex/padding/margin/size/min/max/gap/position 等 taffy 布局字段）。
- 视觉字段（精确集合，取自 `resolved.rs`）：`background_color: Option<[f32;4]>`、`border_color: Option<[f32;4]>`、`border_width: f32`、`opacity: f32`、`overflow_hidden: bool`、`color: [f32;4]`、`font_size: f32`、`font_family`（string 索引，0xFFFF=None）、`font_weight: u16`、`text_align`（枚举 u8）、`line_height: f32`、`letter_spacing: f32`、`white_space_nowrap: bool`、`order: i32`。
- `Option<[f32;4]>`（bg/border color）编码：1B has-flag + 16B rgba（None 时写 0）→ 定长 17B，StyleRecord 整体仍定长。
- 定长字段序在实现期 pin（契约附录随 impl 写进 `core::asset` 注释）。**exhaustive encode/decode**——对 `ResolvedStyle` 字段穷尽匹配，加字段时编译期强制更新 encode/decode，防静默遗漏。
- 字节序：全 LE。

> StyleRecord 是 Rust-internal（§3），故不需跨语言字节对齐表；round-trip 测试断言全字段存活即可。

---

## 6. 版本协商（运行时）

照 §12.2（fgui 缺这个，主设计要）：
- `magic != LOOM_MAGIC` → `Err(PkgError::BadMagic)`。
- `formatVersion < MIN(=1)` → `Err(TooOld)`；`> MAX(=1)` → `Err(TooNew)`。
- v1b.1 支持范围 [1,1]。将来加 indexTable/压缩时 bump 到 2 + 扩 MAX。

---

## 7. core `asset` 模块（write/read）

新模块 `loomgui_core/src/asset/mod.rs`（主设计 §16 列 `asset/`：包格式/TextureView/refcount/load，本 spec 只落包格式部分）：
- `pub fn write_package(scene: &Scene, root_size: (f32,f32)) -> Vec<u8>`——packager 用。**不需 `'parse'`**（只遍历已建好的 Node 树）。
- `pub fn read_package(bytes: &[u8]) -> Result<(Scene, (f32,f32)), PkgError>`——运行时用。**不需 `'parse'`**（从 bytes 建 Scene）。
- `pub enum PkgError { BadMagic, TooOld(u32), TooNew(u32), Truncated(&'static str), OobString(u16) }`（`read_package` 收 `&[u8]`，无 IO；文件 IO 错在打包器 CLI 层报）。

`write_package`：收集 stringTable（text/src/font_family 去重）→ 写 header → 写 stringTable → DFS 先序写 node 数组（parentIndex=数组下标）。

`read_package`：校验 magic+version → 读 stringTable → 读 node 数组 → 按 DFS 分配 NodeId、建 parent 链、填 kind/style/payload → 走 `Scene::build`（§9）建 taffy 树。

---

## 8. `Stage::load_package`

```rust
pub fn load_package(&mut self, bytes: &[u8]) -> Result<(), String> {
    let (scene, (w, h)) = asset::read_package(bytes).map_err(|e| e.to_string())?;
    self.scene = Some(scene);
    self.root_size = (w, h);          // 包 header 的设计稿尺寸覆盖
    Ok(())
}
```
`load_inline`（`#[cfg(feature="parse")]`，§11）与 `load_package` 二选一设 `self.scene`，后续 `tick_and_render` 不变。

---

## 9. `Scene::build` 重构（防逻辑重复）

现 `build_scene(tree: &ElementTree, styles: &[ResolvedStyle]) -> Scene` 同时建 Node 树 + taffy 树。若 `read_package` 另写一份建树，两份会分叉。

抽出共享建树：`Scene::build(nodes: &[(parent: Option<NodeId>, kind: NodeKind, style: ResolvedStyle)])`——建 Node 树 + taffy 树 + taffy_id 映射。payload（Image 的 src / Text 的 content）已在 `NodeKind` enum 内，无需另传。
- `build_scene`（从 ElementTree，`'parse'` gate 后）= 收集 (结构,style,payload) → `Scene::build`。
- `read_package`（从 bytes）= 收集 (结构,style,payload) → `Scene::build`。

`NodeKind`（`scene::Node`）当前含 `Image{src}` / `Text{content}` —— payload 字符串在 NodeKind 里。重构时保持 NodeKind 不变（仍是 enum 带 src/content），序列化从 NodeKind 取/填。

> 这一步是 R2 风险点（重构回归）。黄金等价测（§15）兜底。

---

## 10. `loomgui_pkg` CLI crate

- 新 workspace member（根 `Cargo.toml` `members += "loomgui_pkg"`）。
- `Cargo.toml` dep：`loomgui_core = { path = "..", features = ["parse"] }`。**不引 clap**，用 `std::env::args` 极简 CLI（对齐核心轻依赖）：
  ```
  loomgui_pkg <html> <css> [-o out.pkg.bin] [-w designW] [-h designH]
  ```
  默认 `-o` = `<html 去扩展>.pkg.bin`、`-w/-h` 默认 1080×1920。
- 管线：`read html/css 文件 → parse_html → parse_css → resolve_styles → build_scene → asset::write_package(&scene, root_size) → 写文件`。
- **不加载字体、不 solve、不 render**：`build_scene` 只建结构+样式，measure 在运行时 solve 期（packager 无需 ttf）。
- `examples/` 或 `tests/` 放一个 round-trip 例（见 §15）。

---

## 11. parse feature 拆分

根 `loomgui_core/Cargo.toml`：`scraper`/`cssparser` 改 optional，加：
```toml
[features]
default = ["parse"]
parse = ["dep:scraper", "dep:cssparser"]
```
taffy/ttf-parser/unicode-linebreak/serde 等仍无条件（运行时要）。

**gate 在 `'parse'` 后**：
- `core::parse`（HTML/CSS 解析 + `ElementTree`/`StyleSheet` 类型）。
- `style::resolve_styles`（cascade，引用 `ElementTree`）。
- `build_scene`（从 `ElementTree` 那条）。
- `Stage::load_inline`（调 parse_html/parse_css）。

**常驻（运行时要，不 gate）**：
- `ResolvedStyle` 类型（`style/resolved.rs`，`render` 引用）。
- `Scene::build`（§9 重构出的共享建树）。
- `scene` / `layout` / `render` / `text` / `stage` / `asset`。

**`loomgui_ffi_c`**：`load_package` 永远在；`load_html` 在 `#[cfg(feature="parse")]` 后。**v1b.1 的 .dll 仍带 `'parse'` 编**（PlayMode inline 迭代要），gate 的价值是架构正确 + 将来能出无 parser 的精简 build。

**gate 有效性的证明（构建矩阵门，§15）**：
- `cargo build -p loomgui_ffi_c`（默认带 parse）✅。
- `cargo build -p loomgui_ffi_c --no-default-features`（无 parse，仅 `load_package`）✅。
- `cargo build -p loomgui_pkg`（带 parse）✅。

> R1 风险点：`style/cascade` 引用 `ElementTree`、`build_scene` 引用 `ElementTree`、`render` 引用 `ResolvedStyle`——拆分要切干净。`Scene::build` 重构（§9）正是为了让「建树」不依赖 `ElementTree`，从而运行时（无 parse）也能建树。

---

## 12. FFI

`loomgui_ffi_c` 新增（签名镜像 `loomgui_stage_load_html`）：
```c
int32_t loomgui_stage_load_package(StageHandle* stage, const uint8_t* bytes, uintptr_t len);
```
- `0` = ok，非 0 = err（`load_package` 的 `Err` 字符串走 `Debug.LogError` 或 last-error 通道，与 `load_html` 一致）。
- csbindgen 重新生成 → `Native.loomgui_stage_load_package`。
- `load_html` 在 `#[cfg(feature="parse")]` 后（v1b.1 .dll 仍生成它）。

---

## 13. Unity `LoomStage` 接线

- 加字段：`[SerializeField] bool _usePackage`（**默认 false**，保现有 inline 行为）、`[SerializeField] string _pkgFile`（相对 `Application.streamingAssetsPath`）。
- Awake：`_usePackage` → 读 `Path.Combine(streamingAssetsPath, _pkgFile)` → `File.ReadAllBytes` → `fixed` 钉 → `loomgui_stage_load_package`；否则现有 inline `_html/_css` 路径。
- 保留 inline 路径（dev/PlayMode 迭代）。
- sample `.pkg.bin`：用当前默认场景（`<div class="b">` + `.b{...}` 红块 css）由打包器产出，放 `Assets/StreamingAssets/loom_default.pkg.bin`；PlayMode 验时勾 `_usePackage=true`。
- Domain reload 无改动（`stage_new` + `load_package` 在 Awake，生命周期同 `load_html`）；`loomgui_shutdown` 不变。

> `.dll` 重编换（坑 10）：FFI 加 `load_package` 后必重编+关 Unity 换 .dll 再 PlayMode 验。

---

## 14. 错误处理

- `read_package`：截断/越界/坏 magic/坏 version → `Err(PkgError)` 带上下文。全 `Result`，**无 unwrap/panic 跨 FFI**。
- `Stage::load_package` 透传 `Err` → FFI 返非 0 → Unity `Debug.LogError`。
- 打包器：文件缺失/IO → stderr + 非零退出。

---

## 15. 测试与验收

**Rust 单测（core::asset）**：
- **round-trip**：`write_package → read_package` 产出的 Scene 结构相等（节点数/kind/parent 链/ResolvedStyle 全字段/payload）。覆盖 4 种 kind（Container/Button/Image/Text）+ 嵌套。
- **黄金等价（最强门）**：v0 fixture（div + 文本 + img + rect mask）经 `pkg → load_package → tick_and_render → render_json` 字符串相等 `inline load_inline → tick_and_render → render_json`。证明包路径渲染输出 == inline。
- **版本协商**：坏 magic reject、version=0 reject、version=2（too new）reject、合法 v1 接受。
- **stringTable 去重**：同 `font_family` 多节点 → 同索引；0xFFFF=null 语义。
- **StyleRecord 完整性**：round-trip 断言每个 ResolvedStyle 字段存活（含 Option 的 None/Some 两支）。

**构建矩阵**（R1 门）：上节三条 `cargo build` 皆编。

**Unity PlayMode（批次，押用户）**：sample `.pkg.bin` → `_usePackage=true` → 同 v1a 红块/文本渲染；inline 路径（`_usePackage=false`）仍工作。

**命中验收 #6**：从 HTML 经打包器产出二进制包加载，达成。

---

## 16. 风险

- **R1（最高）feature gate 在 no-parse 配置编译断裂**：`style/cascade` 引 `ElementTree`、`build_scene` 引 `ElementTree`、`render` 引 `ResolvedStyle`。缓解：`Scene::build` 重构先行让建树脱离 ElementTree；构建矩阵门从首 task 起；TDD。
- **R2 `Scene::build` 重构回归**：抽共享建树漏字段 → 包路径偏离 inline。缓解：黄金等价测兜底。
- **R3 StyleRecord 字段遗漏**：少序列化一个 ResolvedStyle 字段 → 静默丢样式。缓解：exhaustive encode/decode（加字段编译期强制更新）+ round-trip 全字段断言。

---

## 17. 与主设计 / roadmap 对齐

- **不改 `docs/design/00-main-design.md`**：§12.2 的 indexTable/压缩/分支愿景已由 `v1x-deferred §6` 记为 v1.x；§12.1「打包器 v1 第一阶段落地」+ roadmap G1 已记 v1b。本 v1 扁平格式是其首批实现，字节布局留本 spec（Rust-internal，非跨语言契约，同 frame blob 处理）。
- 若实现期发现主设计 §12 与本 spec 矛盾（如 §12.2 暗示 v1 即有 indexTable），届时加一行订正注。
- `knowledge-reference`：实现后用 session-summary 记新机制（§2 asset 层）+ 新坑（如有）+ ledger（v1b.1 ✅）。

---

## 18. 非目标 / defer 清单

| 项 | 去向 |
|---|---|
| 真纹理加载（散图→Texture2D→TexId） | v1b.2（B） |
| 图集打包（shelf/guillotine + TextureView） | v1b.3（C） |
| 文本 CJK/多字体（text_arena 升三表） | v1b.4（D） |
| 动态规则表（伪类重匹配） | v1c（事件/状态，消费方在那） |
| indexTable/Seek 块跳转、压缩 | formatVersion=2（v1x-deferred §6） |
| 多包/跨包 URL `loom://pkgName#resId` | v1.x |
| 分支（多语言）/ highResolution（1x/2x/3x） | v1.x（v1x-deferred §6） |
| 集中式迁移器链 / nextPos forward-compat | 多版本累积后（v1x-deferred §6） |
