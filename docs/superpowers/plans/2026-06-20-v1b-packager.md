# v1b.1 打包器 + 二进制包 + 运行时加载器 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 LoomGUI 的加载路径从 inline HTML/CSS 串换成二进制包（`.pkg.bin`），运行时加载包渲染输出与 inline 路径逐节点等价（验收 #6）。

**Architecture:** 新增 `loomgui_pkg` CLI crate（HTML+CSS→`.pkg.bin`，复用 core parse）；core 新 `asset` 模块（`write_package`/`read_package`，包为 Rust-internal，C# 只透传 bytes）；`Stage::load_package`；`Scene::build` 共享建树重构；parse 抽成 core feature gate（runtime 可不带 parser）；FFI `loomgui_stage_load_package` + Unity 读 StreamingAssets。

**Tech Stack:** Rust 2021 / taffy 0.5.2（开 `serde` feature）/ serde + bincode 1.x / csbindgen / Unity 6.5 URP (C#)。

**Spec:** `docs/superpowers/specs/2026-06-20-v1b-packager-design.md`

## Global Constraints

- Workspace：`loomgui_core` + `loomgui_ffi_c` +（新）`loomgui_pkg`，`resolver = "2"`。
- 包 magic = `0x474B504C`（磁盘字节 `4C 50 4B 47` = "LPKG"，**不与 frame blob 的 "LOOM" 撞**）；`formatVersion = 1`；`flags` bit0=compressed（v1=0）。
- 全程小端（LE）。包是 **Rust-internal**：packager 写、core runtime 读，C# 不解析。
- StyleRecord = `ResolvedStyle` 的 **serde + bincode** 编码（u32 len 前缀 + blob）；taffy 0.5.2 `Style` 派生了 `PartialEq` + 条件 `Serialize/Deserialize`。
- **不引 clap**；CLI 用 `std::env::args`。
- parse feature：`scraper`/`cssparser` optional；`default = ["parse"]`；runtime .dll 仍带 parse 编（inline 路径在 PlayMode 迭代要）。
- FFI 改动后必**重编 + 关 Unity 换 `.dll`** 再 PlayMode 验（knowledge-reference 坑 10）。
- 实现语言：Rust（核心/FFI/打包器）+ C#（Unity）。提问/答复用中文（用户偏好）。

---

## File Structure

| 文件 | 责任 | 任务 |
|---|---|---|
| `loomgui_core/Cargo.toml` | 加 bincode、taffy serde feature、parse feature gate | T1, T3 |
| `loomgui_core/src/style/resolved.rs` | `ResolvedStyle`/`TextAlign` 加 serde+PartialEq 派生 | T1 |
| `loomgui_core/src/scene/node.rs` | 抽 `Scene::build`（常驻）；`build_scene` 改调它 + gate | T2, T3 |
| `loomgui_core/src/lib.rs` | gate `pub mod parse`；加 `pub mod asset` | T3, T4 |
| `loomgui_core/src/style/mod.rs` | gate `pub mod cascade` | T3 |
| `loomgui_core/src/stage.rs` | gate `load_inline`；加 `load_package` | T3, T5 |
| `loomgui_core/src/asset/mod.rs` | **新**：`write_package`/`read_package`/`PkgError` + 单测 | T4 |
| `Cargo.toml`（根） | members += `loomgui_pkg` | T6 |
| `loomgui_pkg/Cargo.toml` | **新** crate（dep core 开 parse） | T6 |
| `loomgui_pkg/src/lib.rs` | **新**：`pack(html, css, root_size) -> Vec<u8>` | T6 |
| `loomgui_pkg/src/main.rs` | **新**：极简 CLI（std::env::args） | T6 |
| `loomgui_pkg/tests/pack.rs` | **新**：pack → read_package round-trip | T6 |
| `loomgui_ffi_c/Cargo.toml` | parse feature 转发 core + `default-features = false` | T7 |
| `loomgui_ffi_c/src/lib.rs` | 加 `loomgui_stage_load_package`；gate `load_html`；bump version | T7 |
| `loomgui_unity/.../LoomStage.cs` | `_usePackage`/`_pkgFile` + Awake 分支 | T8 |
| `loomgui_unity/Assets/StreamingAssets/loom_default.pkg.bin` | **新** sample 包（packager 产出） | T8 |

---

## Task 1: serde/bincode 基础 + StyleRecord round-trip

**Files:**
- Modify: `loomgui_core/Cargo.toml`
- Modify: `loomgui_core/src/style/resolved.rs`
- Test: `loomgui_core/src/style/resolved.rs`（`#[cfg(test)]` 内新增）

**Interfaces:**
- Consumes: `taffy` serde feature（本任务开）、`serde` derive（core 已有）、`bincode`（本任务加）。
- Produces: `ResolvedStyle: Serialize + Deserialize + PartialEq`、`TextAlign: Serialize + Deserialize`。后续 T4 的 StyleRecord 编码依赖此。

- [ ] **Step 1: 加 bincode + taffy serde feature 到 core Cargo.toml**

把 `loomgui_core/Cargo.toml` 的 `[dependencies]` 改为（taffy 加 `features = ["serde"]`，末尾加 bincode）：

```toml
[dependencies]
scraper = "0.19"
cssparser = "0.34"
taffy = { version = "0.5", features = ["serde"] }
ttf-parser = "0.20"
unicode-linebreak = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"
```

> 本任务**不动** scraper/cssparser 的 optional 化（那是 T3）。

- [ ] **Step 2: 写失败的 round-trip 测**

在 `loomgui_core/src/style/resolved.rs` 的 `#[cfg(test)] mod tests` 内加（顶部加 `use serde::{Deserialize, Serialize};` 到文件 **非 test 区**，见 Step 3）：

```rust
    #[test]
    fn resolved_style_bincode_roundtrip_preserves_all_fields() {
        // 构造一个各字段都非默认的 ResolvedStyle（覆盖 taffy 字段 + 视觉字段）。
        let mut s = ResolvedStyle::default();
        s.taffy_style.flex_direction = taffy::FlexDirection::Row;
        s.taffy_style.padding = taffy::geometry::Rect::length(7.0);
        s.background_color = Some([0.1, 0.2, 0.3, 0.4]);
        s.border_color = Some([0.5, 0.6, 0.7, 0.8]);
        s.border_width = 3.0;
        s.opacity = 0.5;
        s.overflow_hidden = true;
        s.color = [1.0, 0.0, 0.0, 1.0];
        s.font_size = 48.0;
        s.font_family = Some("DejaVuSans".to_string());
        s.font_weight = 700;
        s.text_align = TextAlign::Center;
        s.line_height = 1.5;
        s.letter_spacing = 2.0;
        s.white_space_nowrap = true;
        s.order = 5;

        let bytes = bincode::serialize(&s).expect("serialize");
        let back: ResolvedStyle = bincode::deserialize(&bytes).expect("deserialize");

        assert_eq!(back, s, "全字段经 bincode round-trip 应相等");
    }
```

- [ ] **Step 3: 跑测确认失败**

Run: `cargo test -p loomgui_core style::resolved::tests::resolved_style_bincode_roundtrip_preserves_all_fields`
Expected: 编译失败（`ResolvedStyle` 未派生 `Serialize/Deserialize/PartialEq`）。

- [ ] **Step 4: 最小实现——加派生**

`loomgui_core/src/style/resolved.rs` 顶部加 `use serde::{Deserialize, Serialize};`（紧跟现有 `use taffy...` 之后）。把两个派生改为：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedStyle {
    // ...字段不变
}
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}
```

> `ResolvedStyle.taffy_style: taffy::style::Style` 已派生 `PartialEq` + 条件 `Serialize/Deserialize`（taffy serde feature 已在 Step 1 开），故父结构能派生。`Option<String>`/`[f32;4]`/原生均 serde 兼容。

- [ ] **Step 5: 跑测确认通过**

Run: `cargo test -p loomgui_core style::resolved`
Expected: PASS（含原有 `default_is_sane` + 新 round-trip）。

- [ ] **Step 6: 全核心测回归**

Run: `cargo test -p loomgui_core`
Expected: 全绿（加派生不应破坏现有测）。

- [ ] **Step 7: 提交**

```bash
git add loomgui_core/Cargo.toml loomgui_core/src/style/resolved.rs
git commit -m "feat(v1b.1 T1): ResolvedStyle serde+bincode round-trip（taffy serde feature）"
```

---

## Task 2: `Scene::build` 共享建树重构

抽出与 parse 无关的共享建树函数，让 T3 的 feature gate 能把 `build_scene`（依赖 `ElementTree`）与「建 Node+taffy 树」分离。`read_package`（T4）也将复用 `Scene::build`。

**Files:**
- Modify: `loomgui_core/src/scene/node.rs:71-197`（Scene 结构区 + build_scene/build_rec/build_text_child）
- Test: `loomgui_core/src/scene/node.rs`（`#[cfg(test)]` 内新增，**不走 parse**）

**Interfaces:**
- Consumes: `Node`/`NodeKind`/`Scene`/`NodeId`/`Rect`（同文件已有）、`ResolvedStyle`。
- Produces: `Scene::build(&[(Option<usize>, NodeKind, ResolvedStyle)]) -> Scene`（常驻，不依赖 parse）。`build_scene` 改为 gather 后调它。

- [ ] **Step 1: 写失败的 `Scene::build` 直测（不走 parse）**

在 `scene/node.rs` 的 `#[cfg(test)] mod tests` 内加：

```rust
    #[test]
    fn scene_build_constructs_tree_without_parse() {
        // 手搓 entries：root Container + 一个 Text 子（parent=Some(0)）。
        // 不走 parse_html/build_scene——证明 Scene::build 独立于 parse（read_package 依赖此）。
        let root_style = ResolvedStyle::default();
        let text_style = ResolvedStyle::default();
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = vec![
            (None, NodeKind::Container, root_style),
            (Some(0), NodeKind::Text { content: "hi".into() }, text_style),
        ];
        let scene = Scene::build(&entries);

        assert_eq!(scene.nodes.len(), 2);
        assert_eq!(scene.roots, vec![NodeId(0)], "根 = parent=None 的节点");
        let root = &scene.nodes[0];
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children, vec![NodeId(1)], "Text 子挂 root");
        assert!(root.clip_rect.is_none(), "overflow_hidden=false → 无 clip slot");
        assert!(!root.dirty_text, "Container dirty_text=false");
        let text = &scene.nodes[1];
        assert!(matches!(&text.kind, NodeKind::Text { content } if content == "hi"));
        assert_eq!(text.parent, Some(NodeId(0)));
        assert!(text.dirty_text, "Text 节点 dirty_text=true");

        // overflow_hidden → clip slot 派生
        let mut of = ResolvedStyle::default();
        of.overflow_hidden = true;
        let scene2 = Scene::build(&[(None, NodeKind::Container, of)]);
        assert!(scene2.nodes[0].clip_rect.is_some(), "overflow_hidden=true → clip slot");
    }
```

- [ ] **Step 2: 跑测确认失败**

Run: `cargo test -p loomgui_core scene::node::tests::scene_build_constructs_tree_without_parse`
Expected: 编译失败（`Scene::build` 不存在；`NodeKind` 等已 `use super::*` 可见）。

- [ ] **Step 3: 实现 `Scene::build`，重构 `build_scene` 调它**

在 `scene/node.rs`，给 `Scene` impl 加 `build`（紧跟 `pub struct Scene` 定义之后、`build_scene` 之前）：

```rust
impl Scene {
    /// 从扁平 entries（DFS 先序）建 Node 树。`NodeId = entries 下标`；
    /// `parent_idx` 指向 entries 下标，`None` = 根。
    /// clip_rect slot / dirty 标志按 style.overflow_hidden / kind 派生。
    /// parse 路径（build_scene）与包加载路径（read_package）共用——防建树逻辑分叉。
    pub fn build(entries: &[(Option<usize>, NodeKind, ResolvedStyle)]) -> Scene {
        let mut scene = Scene {
            roots: Vec::new(),
            nodes: Vec::new(),
        };
        for (i, (parent_idx, kind, style)) in entries.iter().enumerate() {
            scene.nodes.push(Node {
                id: NodeId(i),
                parent: parent_idx.map(NodeId),
                kind: kind.clone(),
                style: style.clone(),
                taffy_id: None,
                layout_rect: Rect::default(),
                clip_rect: if style.overflow_hidden {
                    Some(Rect::default())
                } else {
                    None
                },
                children: Vec::new(),
                dirty_mesh: true,
                dirty_text: matches!(kind, NodeKind::Text { .. }),
            });
        }
        // 接 children + roots（entries 先序 → 按 parent 出现序填，与旧 build_rec 一致）
        for i in 0..entries.len() {
            match entries[i].0 {
                Some(p) => scene.nodes[p].children.push(NodeId(i)),
                None => scene.roots.push(NodeId(i)),
            }
        }
        scene
    }
}
```

把 `build_scene` 改为 gather 后调 `Scene::build`，删掉 `build_rec` 里建 Node 的逻辑（搬到 gather），`build_text_child` 的「继承文本字段」逻辑内联进 gather：

```rust
/// 从 ElementTree + ResolvedStyle 构建 Node 树（gater 后调 `Scene::build`）。
pub fn build_scene(tree: &ElementTree, styles: &[ResolvedStyle]) -> Scene {
    let mut entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = Vec::new();
    for root in &tree.roots {
        gather_rec(tree, styles, *root, None, &mut entries);
    }
    Scene::build(&entries)
}

fn gather_rec(
    tree: &ElementTree,
    styles: &[ResolvedStyle],
    el_id: ElementId,
    parent_idx: Option<usize>,
    entries: &mut Vec<(Option<usize>, NodeKind, ResolvedStyle)>,
) -> usize {
    let el = &tree.nodes[el_id.0];
    let style = &styles[el_id.0];
    let kind = match el.tag.as_str() {
        "div" | "l-container" => NodeKind::Container,
        "button" => NodeKind::Button,
        "img" => NodeKind::Image {
            src: el.attrs.get("src").cloned().unwrap_or_default(),
        },
        "span" => NodeKind::Text {
            content: el.text.clone().unwrap_or_default(),
        },
        _ => unreachable!(
            "parse 层白名单已挡围栏外 tag，scene 不应见到 <{}>；这是 parse/scene 契约破坏",
            el.tag
        ),
    };
    let my_idx = entries.len();
    entries.push((parent_idx, kind.clone(), style.clone()));

    // §4.2：Container/Button 的裸文本 → Text 子节点。文本子像无 class 的 <span>：
    // taffy_style 取 DEFAULT（由测量定尺寸），视觉/字体字段继承父值。
    if matches!(kind, NodeKind::Container | NodeKind::Button) {
        if let Some(text) = &el.text {
            let mut ts = ResolvedStyle::default();
            ts.color = style.color;
            ts.font_size = style.font_size;
            ts.font_family = style.font_family.clone();
            ts.font_weight = style.font_weight;
            ts.line_height = style.line_height;
            ts.letter_spacing = style.letter_spacing;
            ts.text_align = style.text_align;
            ts.white_space_nowrap = style.white_space_nowrap;
            entries.push((Some(my_idx), NodeKind::Text { content: text.clone() }, ts));
        }
    }

    if !el.children.is_empty() {
        for c in &el.children {
            gather_rec(tree, styles, *c, Some(my_idx), entries);
        }
    }
    my_idx
}
```

> **删掉** 旧 `build_rec` 与 `build_text_child` 函数（逻辑已并入 `gather_rec` + `Scene::build`）。保留文件顶部 `use crate::parse::dom::{ElementId, ElementTree};`（gather_rec 还要用；T3 会随 build_scene 一起 gate）。

- [ ] **Step 4: 跑新测确认通过**

Run: `cargo test -p loomgui_core scene::node::tests::scene_build_constructs_tree_without_parse`
Expected: PASS。

- [ ] **Step 5: 全 scene 测回归（确保 build_scene 行为不变）**

Run: `cargo test -p loomgui_core scene::`
Expected: 全绿（原有 `builds_div_button_text_image` / `overflow_hidden_sets_clip_rect_slot` / `div_raw_text_becomes_text_child` / `text_child_inherits_parent_text_fields_resets_size` 等仍过——证明重构无行为变化）。

- [ ] **Step 6: 全核心测回归**

Run: `cargo test -p loomgui_core`
Expected: 全绿。

- [ ] **Step 7: 提交**

```bash
git add loomgui_core/src/scene/node.rs
git commit -m "refactor(v1b.1 T2): 抽 Scene::build 共享建树——build_scene 改 gather+build（无行为变化）"
```

---

## Task 3: parse feature gate（R1 最高风险）

把 `scraper`/`cssparser` 抽成 optional + `parse` feature；gate 掉 parse 模块、cascade、`build_scene`、`load_inline`。证明 runtime 不带 parser 能编（构建矩阵门）。

**Files:**
- Modify: `loomgui_core/Cargo.toml`
- Modify: `loomgui_core/src/lib.rs`
- Modify: `loomgui_core/src/style/mod.rs`
- Modify: `loomgui_core/src/scene/node.rs`（gate `build_scene`/`gather_rec` + parse-using 测）
- Modify: `loomgui_core/src/stage.rs`（gate `load_inline` + parse-using import）
- Modify: `loomgui_core/src/layout/mod.rs:190`（gate 用 parse 的测）

**Interfaces:**
- Consumes: T2 的 `Scene::build`（常驻，是 gate 能切干净的依赖）。
- Produces: core `parse` feature（`default = ["parse"]`）；`cargo build -p loomgui_core --no-default-features` 可编。

- [ ] **Step 1: Cargo.toml 加 parse feature，scraper/cssparser optional**

`loomgui_core/Cargo.toml` 改为：

```toml
[dependencies]
scraper = { version = "0.19", optional = true }
cssparser = { version = "0.34", optional = true }
taffy = { version = "0.5", features = ["serde"] }
ttf-parser = "0.20"
unicode-linebreak = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"

[features]
default = ["parse"]
parse = ["dep:scraper", "dep:cssparser"]
```

- [ ] **Step 2: gate `parse` 模块（lib.rs）**

`loomgui_core/src/lib.rs`：

```rust
pub mod layout;
#[cfg(feature = "parse")]
pub mod parse;
pub mod render;
pub mod scene;
pub mod stage;
pub mod style;
pub mod text;

pub use stage::Stage;
```

- [ ] **Step 3: gate `style::cascade`（mod.rs）**

`loomgui_core/src/style/mod.rs`（`resolved`/`mapping` 常驻——它们不依赖 parse/cssparser）：

```rust
pub mod resolved;
pub mod mapping;
#[cfg(feature = "parse")]
pub mod cascade;
```

- [ ] **Step 4: gate `build_scene` + parse-using 测（scene/node.rs）**

`scene/node.rs` 顶部 import：`use crate::parse::dom::{ElementId, ElementTree};` 改为条件 + 仅 build_scene 用，整段 gate。给 `build_scene` 和 `gather_rec` 加 `#[cfg(feature = "parse")]`：

```rust
#[cfg(feature = "parse")]
pub fn build_scene(tree: &ElementTree, styles: &[ResolvedStyle]) -> Scene { ... }

#[cfg(feature = "parse")]
fn gather_rec(...) -> usize { ... }
```

顶部 `use crate::parse::dom::{ElementId, ElementTree};` 也 gate：

```rust
#[cfg(feature = "parse")]
use crate::parse::dom::{ElementId, ElementTree};
```

`#[cfg(test)] mod tests` 内**所有**用 `parse_html`/`parse_css`/`resolve_styles`/`build_scene` 的测加条件——把整个 tests mod 的用 parse 的项 gate。最简：把 tests mod 改为 `#[cfg(all(test, feature = "parse"))] mod tests`（这些测都依赖 parse）。**保留** T2 加的 `scene_build_constructs_tree_without_parse` 测在不依赖 parse 的 mod——拆成两个 tests mod：

```rust
// 不依赖 parse 的测（Scene::build 直测）——常驻
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_build_constructs_tree_without_parse() { /* T2 的测 */ }
}

// 依赖 parse 的测（build_scene via parse）——gate
#[cfg(all(test, feature = "parse"))]
mod parse_tests {
    use super::*;
    use crate::parse::{css::parse_css, dom::parse_html};
    use crate::style::cascade::resolve_styles;
    // ...原 builds_div_button_text_image 等所有测搬到这里
}
```

- [ ] **Step 5: gate `Stage::load_inline`（stage.rs）**

`stage.rs` 顶部 `use crate::parse::css::parse_css;` / `use crate::parse::dom::parse_html;` / `use crate::style::cascade::resolve_styles;` / `use crate::scene::node::build_scene` 改为条件 import：

```rust
#[cfg(feature = "parse")]
use crate::parse::css::parse_css;
#[cfg(feature = "parse")]
use crate::parse::dom::parse_html;
#[cfg(feature = "parse")]
use crate::scene::node::build_scene;
#[cfg(feature = "parse")]
use crate::style::cascade::resolve_styles;
```

给 `load_inline` 加 `#[cfg(feature = "parse")]`：

```rust
    /// v0 内存直通：HTML+CSS 文本直接构 scene（不走打包器）。
    #[cfg(feature = "parse")]
    pub fn load_inline(&mut self, html: &str, css: &str) -> Result<(), String> {
        let tree = parse_html(html)?;
        let sheet = parse_css(css)?;
        let styles = resolve_styles(&tree, &sheet);
        self.scene = Some(build_scene(&tree, &styles));
        Ok(())
    }
```

- [ ] **Step 6: gate layout 里用 parse 的测**

`layout/mod.rs:190` 处 `use crate::parse::{css::parse_css, dom::parse_html};`（在 `#[cfg(test)]` 内）——给该 import 与用到它的测函数加 `#[cfg(feature = "parse")]`。这是核心里唯一在 layout 测中用 parse 的点（已 grep 确认）。若有其它 layout 测函数体调 parse_html/parse_css，同样加 cfg。

- [ ] **Step 7: 构建矩阵门——runtime 不带 parse 能编**

Run: `cargo build -p loomgui_core --no-default-features`
Expected: 编译成功（无 parse feature）。若失败，报错点即 gate 漏处——补 `#[cfg(feature="parse")]`。

- [ ] **Step 8: default（带 parse）全测回归**

Run: `cargo test -p loomgui_core`
Expected: 全绿（parse 测仍在 default 下跑）。

- [ ] **Step 9: 确认 runtime 不带 parser 也不带 scraper/cssparser 编译**

Run: `cargo build -p loomgui_core --no-default-features 2>&1 | grep -i "scraper\|cssparser"` 
Expected: 无输出（这俩 dep 在 no-default-features 下不参与编译）。

- [ ] **Step 10: 提交**

```bash
git add loomgui_core/Cargo.toml loomgui_core/src/lib.rs loomgui_core/src/style/mod.rs loomgui_core/src/scene/node.rs loomgui_core/src/stage.rs loomgui_core/src/layout/mod.rs
git commit -m "feat(v1b.1 T3): parse feature gate——scraper/cssparser optional，runtime 可不带 parser"
```

---

## Task 4: core `asset` 模块（write_package / read_package）

**Files:**
- Create: `loomgui_core/src/asset/mod.rs`
- Modify: `loomgui_core/src/lib.rs`（加 `pub mod asset;`，已在 T3 加过则跳过）
- Test: `loomgui_core/src/asset/mod.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: T1 `ResolvedStyle: Serialize+Deserialize`、T2 `Scene::build`、`Scene`/`Node`/`NodeKind`/`NodeId`。
- Produces:
  - `asset::write_package(scene: &Scene, root_size: (f32,f32)) -> Vec<u8>`
  - `asset::read_package(bytes: &[u8]) -> Result<(Scene, (f32,f32)), PkgError>`
  - `asset::PkgError`、`asset::PKG_MAGIC`、`asset::PKG_FORMAT_VERSION`

- [ ] **Step 1: 写失败的 round-trip 测**

`loomgui_core/src/asset/mod.rs` 先建空骨架（仅 `pub mod` 占位让 lib.rs 能挂），然后写测。实际先写完整测：

```rust
//! 包格式（§5）：.pkg.bin v1。Rust-internal（packager 写、runtime 读，C# 不解析）。
//! 扁平：Header(28B) + StringTable + NodeBlock。style 字段 = bincode(ResolvedStyle)。

use crate::scene::{NodeKind, Scene};
use crate::style::resolved::ResolvedStyle;

#[test]
fn write_read_roundtrip_preserves_scene() {
    // 手搓一个覆盖 4 种 kind + 嵌套的 Scene（不走 parse，靠 Scene::build）。
    let mut img_style = ResolvedStyle::default();
    img_style.background_color = Some([1.0, 0.0, 0.0, 1.0]);
    let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = vec![
        (None, NodeKind::Container, ResolvedStyle::default()),
        (Some(0), NodeKind::Text { content: "hi".into() }, ResolvedStyle::default()),
        (Some(0), NodeKind::Image { src: "logo.png".into() }, img_style.clone()),
        (None, NodeKind::Button, ResolvedStyle::default()),
    ];
    let scene = Scene::build(&entries);

    let bytes = write_package(&scene, (1080.0, 1920.0));
    let (scene2, rs) = read_package(&bytes).expect("read ok");

    assert_eq!(rs, (1080.0, 1920.0));
    assert_eq!(scene2.nodes.len(), scene.nodes.len());
    // 结构：parent / kind / children
    for (a, b) in scene.nodes.iter().zip(scene2.nodes.iter()) {
        assert_eq!(a.parent, b.parent);
        assert_eq!(a.children, b.children);
    }
    // kind + payload
    assert!(matches!(&scene2.nodes[1].kind, NodeKind::Text { content } if content == "hi"));
    assert!(matches!(&scene2.nodes[2].kind, NodeKind::Image { src } if src == "logo.png"));
    // style 经 bincode round-trip（background_color 非 None）
    assert_eq!(scene2.nodes[2].style, img_style);
}
```

- [ ] **Step 2: 跑测确认失败**

Run: `cargo test -p loomgui_core asset::tests::write_read_roundtrip_preserves_scene`
Expected: 编译失败（`write_package`/`read_package` 未定义）。先在 `lib.rs` 加 `pub mod asset;`（若 T3 未加）。

- [ ] **Step 3: 实现 write_package / read_package**

`loomgui_core/src/asset/mod.rs` 完整实现：

```rust
//! 包格式（spec §5）：.pkg.bin v1。Rust-internal（packager 写、runtime 读，C# 不解析）。
//! 布局：Header(28B) + StringTable + NodeBlock（DFS 先序）。style 字段 = bincode(ResolvedStyle)。

use crate::scene::{NodeKind, NodeId, Scene};
use crate::style::resolved::ResolvedStyle;

pub const PKG_MAGIC: u32 = 0x474B504C; // 磁盘字节(LE) "LPKG"（不与 frame blob "LOOM" 撞）
pub const PKG_FORMAT_VERSION: u32 = 1;
const MIN_VERSION: u32 = 1;
const MAX_VERSION: u32 = 1;
const NULL_IDX: u16 = 0xFFFF;

const KIND_CONTAINER: u8 = 0;
const KIND_BUTTON: u8 = 1;
const KIND_IMAGE: u8 = 2;
const KIND_TEXT: u8 = 3;

#[derive(Debug)]
pub enum PkgError {
    BadMagic,
    TooOld(u32),
    TooNew(u32),
    Truncated(&'static str),
    OobString(u16),
    Bincode(bincode::Error),
    BadKind(u8),
}

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgError::BadMagic => write!(f, "bad magic (not a loom package)"),
            PkgError::TooOld(v) => write!(f, "package formatVersion {v} too old (min {MIN_VERSION})"),
            PkgError::TooNew(v) => write!(f, "package formatVersion {v} too new (max {MAX_VERSION})"),
            PkgError::Truncated(ctx) => write!(f, "truncated package: {ctx}"),
            PkgError::OobString(i) => write!(f, "string index {i} out of range"),
            PkgError::Bincode(e) => write!(f, "style bincode: {e}"),
            PkgError::BadKind(k) => write!(f, "bad node kind tag {k}"),
        }
    }
}

impl std::error::Error for PkgError {}

impl From<bincode::Error> for PkgError {
    fn from(e: bincode::Error) -> Self { PkgError::Bincode(e) }
}

/// 序列化 Scene → .pkg.bin bytes（spec §5）。
pub fn write_package(scene: &Scene, root_size: (f32, f32)) -> Vec<u8> {
    // 1. 收 stringTable（text content + image src），首次出现序建索引。
    let mut strings: Vec<String> = Vec::new();
    let mut idx_of: std::collections::HashMap<String, u16> = std::collections::HashMap::new();

    // 每节点：(parent_idx, kind_tag, style_blob, text_idx, src_idx)
    // scene.nodes 已是 DFS 先序、NodeId(i).0 == i（build_scene / Scene::build 不变量）。
    let mut nodes: Vec<(i32, u8, Vec<u8>, u16, u16)> = Vec::new();
    for n in &scene.nodes {
        let parent_idx = n.parent.map(|NodeId(p)| p as i32).unwrap_or(-1);
        let (kind_tag, text_idx, src_idx) = match &n.kind {
            NodeKind::Container => (KIND_CONTAINER, NULL_IDX, NULL_IDX),
            NodeKind::Button => (KIND_BUTTON, NULL_IDX, NULL_IDX),
            NodeKind::Image { src } => (KIND_IMAGE, NULL_IDX, intern(src, &mut strings, &mut idx_of)),
            NodeKind::Text { content } => (KIND_TEXT, intern(content, &mut strings, &mut idx_of), NULL_IDX),
        };
        let style_blob = bincode::serialize(&n.style).expect("ResolvedStyle serializable");
        nodes.push((parent_idx, kind_tag, style_blob, text_idx, src_idx));
    }

    let mut out: Vec<u8> = Vec::new();
    // Header (28B)
    out.extend_from_slice(&PKG_MAGIC.to_le_bytes());
    out.extend_from_slice(&PKG_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags（v1 uncompressed）
    out.extend_from_slice(&(scene.nodes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    out.extend_from_slice(&root_size.0.to_le_bytes());
    out.extend_from_slice(&root_size.1.to_le_bytes());
    // StringTable
    for s in &strings {
        let bytes = s.as_bytes();
        out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    // NodeBlock
    for (parent_idx, kind_tag, style_blob, text_idx, src_idx) in &nodes {
        out.extend_from_slice(&parent_idx.to_le_bytes());
        out.push(*kind_tag);
        out.extend_from_slice(&(style_blob.len() as u32).to_le_bytes());
        out.extend_from_slice(style_blob);
        out.extend_from_slice(&text_idx.to_le_bytes());
        out.extend_from_slice(&src_idx.to_le_bytes());
    }
    out
}

/// 反序列化 .pkg.bin → (Scene, root_size)（spec §5 + §6 版本协商）。
pub fn read_package(bytes: &[u8]) -> Result<(Scene, (f32, f32)), PkgError> {
    let mut r = Reader::new(bytes);
    // Header
    let magic = r.u32("magic")?;
    if magic != PKG_MAGIC { return Err(PkgError::BadMagic); }
    let version = r.u32("version")?;
    if version < MIN_VERSION { return Err(PkgError::TooOld(version)); }
    if version > MAX_VERSION { return Err(PkgError::TooNew(version)); }
    let _flags = r.u32("flags")?;
    let node_count = r.u32("node_count")? as usize;
    let string_count = r.u32("string_count")? as usize;
    let root_w = r.f32("root_w")?;
    let root_h = r.f32("root_h")?;
    // StringTable
    let mut strings: Vec<String> = Vec::with_capacity(string_count);
    for _ in 0..string_count {
        let len = r.u16("str_len")? as usize;
        let s = r.utf8(len, "str_bytes")?;
        strings.push(s);
    }
    // NodeBlock → entries
    let mut entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let pidx = r.i32("parent_idx")?;
        let kind_tag = r.u8("kind")?;
        let style_len = r.u32("style_len")? as usize;
        let style: ResolvedStyle = bincode::deserialize(r.take(style_len, "style_blob")?)?;
        let text_idx = r.u16("text_idx")?;
        let src_idx = r.u16("src_idx")?;
        let parent = if pidx < 0 { None } else { Some(pidx as usize) };
        let kind = match kind_tag {
            KIND_CONTAINER => NodeKind::Container,
            KIND_BUTTON => NodeKind::Button,
            KIND_IMAGE => NodeKind::Image { src: string_at(&strings, src_idx)? },
            KIND_TEXT => NodeKind::Text { content: string_at(&strings, text_idx)? },
            other => return Err(PkgError::BadKind(other)),
        };
        entries.push((parent, kind, style));
    }
    let scene = Scene::build(&entries);
    Ok((scene, (root_w, root_h)))
}

fn string_at(strings: &[String], idx: u16) -> Result<String, PkgError> {
    if idx == NULL_IDX { return Ok(String::new()); }
    strings.get(idx as usize).cloned().ok_or(PkgError::OobString(idx))
}

/// 把字符串 intern 进 stringTable（首次出现分配新索引，重复返回既有索引）。
fn intern(s: &str, strings: &mut Vec<String>, idx_of: &mut std::collections::HashMap<String, u16>) -> u16 {
    if let Some(&i) = idx_of.get(s) { return i; }
    let i = strings.len() as u16;
    strings.push(s.to_string());
    idx_of.insert(s.to_string(), i);
    i
}

/// 极简游标 reader：定长小端读取 + 截断保护。
struct Reader<'a> { buf: &'a [u8], pos: usize }
impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self { Reader { buf, pos: 0 } }
    fn need(&mut self, n: usize, ctx: &'static str) -> Result<&'a [u8], PkgError> {
        if self.pos + n > self.buf.len() { return Err(PkgError::Truncated(ctx)); }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self, ctx: &'static str) -> Result<u8, PkgError> { Ok(self.need(1, ctx)?[0]) }
    fn u16(&mut self, ctx: &'static str) -> Result<u16, PkgError> { Ok(u16::from_le_bytes(self.need(2, ctx)?.try_into().unwrap())) }
    fn u32(&mut self, ctx: &'static str) -> Result<u32, PkgError> { Ok(u32::from_le_bytes(self.need(4, ctx)?.try_into().unwrap())) }
    fn i32(&mut self, ctx: &'static str) -> Result<i32, PkgError> { Ok(i32::from_le_bytes(self.need(4, ctx)?.try_into().unwrap())) }
    fn f32(&mut self, ctx: &'static str) -> Result<f32, PkgError> { Ok(f32::from_le_bytes(self.need(4, ctx)?.try_into().unwrap())) }
    fn take(&mut self, n: usize, ctx: &'static str) -> Result<&'a [u8], PkgError> { self.need(n, ctx) }
    fn utf8(&mut self, n: usize, ctx: &'static str) -> Result<String, PkgError> {
        let s = self.need(n, ctx)?;
        std::str::from_utf8(s).map(String::from).map_err(|_| PkgError::Truncated(ctx))
    }
}
```

在 `lib.rs` 确保有 `pub mod asset;`（T3 已加则跳过）。

- [ ] **Step 4: 跑 round-trip 测通过**

Run: `cargo test -p loomgui_core asset::tests::write_read_roundtrip_preserves_scene`
Expected: PASS。

- [ ] **Step 5: 加版本协商 + stringTable 去重测**

```rust
    #[test]
    fn read_rejects_bad_magic() {
        let mut bad = vec![0u8; 28];
        // magic 改成 "LOOM"（frame blob 的）→ 应被拒
        bad[0..4].copy_from_slice(&0x4D4F4F4Cu32.to_le_bytes());
        assert!(matches!(read_package(&bad), Err(PkgError::BadMagic)));
    }

    #[test]
    fn read_rejects_unsupported_version() {
        // 借 round-trip 测的合法包，把 version 字段（offset 4）改成 2
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> =
            vec![(None, NodeKind::Container, ResolvedStyle::default())];
        let mut bytes = write_package(&Scene::build(&entries), (100.0, 100.0));
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes()); // version=2 → too new
        assert!(matches!(read_package(&bytes), Err(PkgError::TooNew(2))));
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes()); // version=0 → too old
        assert!(matches!(read_package(&bytes), Err(PkgError::TooOld(0))));
    }

    #[test]
    fn stringtable_dedups_repeated_strings() {
        // 两个 Text 同 content → stringTable 只一条，textIdx 相同。
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default()),
            (Some(0), NodeKind::Text { content: "dup".into() }, ResolvedStyle::default()),
            (Some(0), NodeKind::Text { content: "dup".into() }, ResolvedStyle::default()),
        ];
        let bytes = write_package(&Scene::build(&entries), (10.0, 10.0));
        // stringCount（offset 16）应为 1（"dup" 去重）
        let sc = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(sc, 1, "重复 content 应去重为 1 条");
        let (scene2, _) = read_package(&bytes).unwrap();
        assert!(matches!(&scene2.nodes[1].kind, NodeKind::Text { content } if content == "dup"));
        assert!(matches!(&scene2.nodes[2].kind, NodeKind::Text { content } if content == "dup"));
    }
```

Run: `cargo test -p loomgui_core asset::tests`
Expected: 全 PASS。

- [ ] **Step 6: 全核心测回归**

Run: `cargo test -p loomgui_core`
Expected: 全绿。

- [ ] **Step 7: 提交**

```bash
git add loomgui_core/src/asset/mod.rs loomgui_core/src/lib.rs
git commit -m "feat(v1b.1 T4): core asset 模块——write_package/read_package + 版本协商 + stringTable 去重"
```

---

## Task 5: `Stage::load_package` + 黄金等价测

**Files:**
- Modify: `loomgui_core/src/stage.rs`
- Test: `loomgui_core/src/stage.rs`（`#[cfg(test)]`，需 parse——放在 `#[cfg(all(test, feature="parse"))]` 或随 default feature）

**Interfaces:**
- Consumes: T4 `asset::read_package`/`write_package`、T2 `Scene`。
- Produces: `Stage::load_package(&mut self, bytes: &[u8]) -> Result<(), String>`。

- [ ] **Step 1: 写失败的黄金等价测**

在 `stage.rs` 的 tests mod 加（该测用 `load_inline` + `write_package`，需 parse；若 T3 把 stage tests 也 gate，此测随 `#[cfg(all(test, feature="parse"))]`）：

```rust
    /// 黄金等价（最强门）：inline 渲染 == 包渲染。
    /// v0 fixture（div + 文本 + img + rect mask）经 pkg→load_package→render_json
    /// 必须 == inline load_inline→render_json。
    #[test]
    fn package_load_renders_identical_to_inline() {
        let font_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/DejaVuSans.ttf"
        );
        let html = r#"<div class="c"><span>hi</span><img src="logo.png"></div>"#;
        let css = ".c{width:200px;height:100px;overflow:hidden;background-color:#ff0000;}";

        // inline 路径
        let mut s_inline = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_inline.load_inline(html, css).unwrap();
        let inline_json = s_inline.render_json();

        // 序列化 inline 的 scene → 包
        let scene = s_inline.scene.as_ref().unwrap();
        let pkg = crate::asset::write_package(scene, (200.0, 100.0));

        // 包路径（新 Stage，同字体）
        let mut s_pkg = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_pkg.load_package(&pkg).unwrap();
        let pkg_json = s_pkg.render_json();

        assert_eq!(inline_json, pkg_json, "包路径渲染输出必须 == inline");
    }
```

- [ ] **Step 2: 跑测确认失败**

Run: `cargo test -p loomgui_core stage::tests::package_load_renders_identical_to_inline`
Expected: 编译失败（`load_package` 未定义）。

- [ ] **Step 3: 实现 `Stage::load_package`**

在 `stage.rs` `impl Stage` 内（`load_inline` 之后）加（**不加** `#[cfg]`——常驻）：

```rust
    /// 从二进制包加载（spec §8）：read_package → self.scene + root_size（用包 header 的）。
    /// 与 `load_inline` 二选一设 scene；后续 tick_and_render 不变。不需 parse feature。
    pub fn load_package(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (scene, root_size) = crate::asset::read_package(bytes).map_err(|e| e.to_string())?;
        self.scene = Some(scene);
        self.root_size = root_size;
        Ok(())
    }
```

- [ ] **Step 4: 跑测通过**

Run: `cargo test -p loomgui_core stage::tests::package_load_renders_identical_to_inline`
Expected: PASS（包路径与 inline 渲染逐节点相等）。

- [ ] **Step 5: 全核心测 + 构建（含 no-default-features）回归**

Run: `cargo test -p loomgui_core && cargo build -p loomgui_core --no-default-features`
Expected: 全绿 + 无 parse 也编（load_package 不依赖 parse）。

- [ ] **Step 6: 提交**

```bash
git add loomgui_core/src/stage.rs
git commit -m "feat(v1b.1 T5): Stage::load_package + 黄金等价测（pkg 渲染 == inline 渲染）"
```

---

## Task 6: `loomgui_pkg` CLI crate

**Files:**
- Modify: `Cargo.toml`（根，members += loomgui_pkg）
- Create: `loomgui_pkg/Cargo.toml`
- Create: `loomgui_pkg/src/lib.rs`
- Create: `loomgui_pkg/src/main.rs`
- Create: `loomgui_pkg/tests/pack.rs`

**Interfaces:**
- Consumes: core 开 `'parse'`（`parse_html`/`parse_css`/`resolve_styles`/`build_scene`）+ T4 `asset::write_package`。
- Produces: `loomgui_pkg` 二进制（`loomgui_pkg <html> <css> [-o out] [-w w] [-h h]`）+ `loomgui_pkg::pack(html, css, root_size) -> Result<Vec<u8>, String>`。

- [ ] **Step 1: 写失败的 pack 测**

`loomgui_pkg/tests/pack.rs`：

```rust
use loomgui_core::asset::{read_package, PKG_MAGIC};
use loomgui_pkg::pack;

#[test]
fn pack_produces_valid_package_roundtrips() {
    let html = r#"<div class="c"><span>hi</span><img src="logo.png"></div>"#;
    let css = ".c{width:200px;height:100px;background-color:#ff0000;}";
    let bytes = pack(html, css, (200.0, 100.0)).expect("pack ok");

    // magic 头
    let m = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    assert_eq!(m, PKG_MAGIC);

    // round-trip：read_package 能读回，且结构对
    let (scene, rs) = read_package(&bytes).expect("read ok");
    assert_eq!(rs, (200.0, 100.0));
    assert!(scene.roots.len() >= 1);
    // 至少有一个 Text(content="hi") 和一个 Image(src="logo.png")
    let has_text = scene.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Text { content } if content == "hi"));
    let has_img = scene.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Image { src } if src == "logo.png"));
    assert!(has_text && has_img);
}
```

- [ ] **Step 2: 建 crate 骨架让测能编译**

根 `Cargo.toml`：

```toml
[workspace]
members = ["loomgui_core", "loomgui_ffi_c", "loomgui_pkg"]
resolver = "2"
```

`loomgui_pkg/Cargo.toml`：

```toml
[package]
name = "loomgui_pkg"
version = "0.1.0"
edition = "2021"

[dependencies]
loomgui_core = { path = "../loomgui_core", features = ["parse"] }
```

`loomgui_pkg/src/lib.rs`（先空，Step 3 填 pack）：

```rust
//! 打包器库（spec §10）：HTML+CSS → .pkg.bin。复用 core parse/style/scene + asset::write_package。

/// 把 HTML+CSS 打成 .pkg.bin 字节（spec §10）。root_size 写进包 header。
pub fn pack(html: &str, css: &str, root_size: (f32, f32)) -> Result<Vec<u8>, String> {
    let tree = loomgui_core::parse::dom::parse_html(html).map_err(|e| format!("parse_html: {e}"))?;
    let sheet = loomgui_core::parse::css::parse_css(css).map_err(|e| format!("parse_css: {e}"))?;
    let styles = loomgui_core::style::cascade::resolve_styles(&tree, &sheet);
    let scene = loomgui_core::scene::build_scene(&tree, &styles);
    Ok(loomgui_core::asset::write_package(&scene, root_size))
}
```

- [ ] **Step 3: 跑测确认失败再实现（pack 已在 Step 2 写）**

Run: `cargo test -p loomgui_pkg`
Expected: PASS（pack 在 Step 2 已实现；若失败按报错修）。

- [ ] **Step 4: 写 CLI main.rs**

`loomgui_pkg/src/main.rs`：

```rust
//! 极简 CLI（不引 clap）：loomgui_pkg <html> <css> [-o out.pkg.bin] [-w W] [-h H]。
//! 默认 out = <html 去 .html>.pkg.bin，默认 root_size = 1080×1920。

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} <html> <css> [-o out.pkg.bin] [-w designW] [-h designH]", args.first().map(String::as_str).unwrap_or("loomgui_pkg"));
        return ExitCode::from(2);
    }
    let html_path = &args[1];
    let css_path = &args[2];
    let mut out_path = html_path.rsplit_once('.').map(|(stem, _)| format!("{stem}.pkg.bin")).unwrap_or_else(|| format!("{html_path}.pkg.bin"));
    let mut w = 1080.0f32;
    let mut h = 1920.0f32;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => { out_path = args.get(i + 1).cloned().unwrap_or(out_path); i += 2; }
            "-w" => { w = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(w); i += 2; }
            "-h" => { h = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(h); i += 2; }
            other => { eprintln!("unknown arg: {other}"); return ExitCode::from(2); }
        }
    }

    let html = match fs::read_to_string(html_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("read {html_path}: {e}"); return ExitCode::FAILURE; }
    };
    let css = match fs::read_to_string(css_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("read {css_path}: {e}"); return ExitCode::FAILURE; }
    };

    match loomgui_pkg::pack(&html, &css, (w, h)) {
        Ok(bytes) => match fs::write(&out_path, &bytes) {
            Ok(_) => { eprintln!("wrote {out_path} ({} bytes)", bytes.len()); ExitCode::SUCCESS }
            Err(e) => { eprintln!("write {out_path}: {e}"); ExitCode::FAILURE }
        },
        Err(e) => { eprintln!("pack: {e}"); ExitCode::FAILURE }
    }
}
```

> `other` 臂兜底所有未知 flag（报错退出），故无 `_` 臂。`-o/-w/-h` 各消耗 2 个 arg（flag + 值）。

- [ ] **Step 5: 编译 CLI（冒烟）**

Run: `cargo build -p loomgui_pkg`
Expected: 编译成功。

- [ ] **Step 6: CLI 端到端冒烟（临时文件 round-trip）**

Run:
```bash
printf '<div class="c">hi</div>' > /tmp/t.html && printf '.c{width:50px;height:50px;}' > /tmp/t.css && cargo run -q -p loomgui_pkg -- /tmp/t.html /tmp/t.css -o /tmp/t.pkg.bin && ls -l /tmp/t.pkg.bin
```
Expected: 产出 `/tmp/t.pkg.bin` 非空文件，stderr 打 "wrote ... (N bytes)"。

- [ ] **Step 7: 全 workspace 测 + 三构建设置回归**

Run: `cargo test --workspace && cargo build -p loomgui_ffi_c --no-default-features 2>&1 | tail -5`
Expected: workspace 全绿；ffi_c no-default-features 暂仍可能因 load_html 未 gate 失败——**T7 修**。此处只确认 `cargo test --workspace` 绿 + `cargo build -p loomgui_pkg` 绿。

- [ ] **Step 8: 提交**

```bash
git add Cargo.toml loomgui_pkg/
git commit -m "feat(v1b.1 T6): loomgui_pkg CLI crate——HTML+CSS→.pkg.bin（pack 库 + std::env::args CLI）"
```

---

## Task 7: FFI `loomgui_stage_load_package` + load_html gate

**Files:**
- Modify: `loomgui_ffi_c/Cargo.toml`
- Modify: `loomgui_ffi_c/src/lib.rs`
- Modify: `loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs`（csbindgen build.rs 自动重生成，确认即可）

**Interfaces:**
- Consumes: T5 `Stage::load_package`。
- Produces: `loomgui_stage_load_package(h, bytes*, len) -> i32`（C# `Native.loomgui_stage_load_package`，csbindgen 生成）。

- [ ] **Step 1: 写失败的 abi 测**

`loomgui_ffi_c/src/lib.rs` 的 `#[cfg(test)] mod abi_tests` 内加（用 `Scene::build` 手搓 scene，不依赖 parse）：

```rust
    #[test]
    fn load_package_builds_blob_from_package() {
        use loomgui_core::asset::write_package;
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let (fp, fplen) = font_path();
        // 手搓 scene（不走 parse），打成包
        let entries = vec![
            (None, NodeKind::Container, ResolvedStyle::default()),
            (Some(0), NodeKind::Text { content: "hi".into() }, ResolvedStyle::default()),
        ];
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0));

        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        assert!(!h.is_null());
        let r = loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len());
        assert_eq!(r, 0, "load_package ok");
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_frame(h, &mut len);
        assert!(!ptr.is_null() && len > 12, "tick 后应有 blob");
        loomgui_stage_free(h);
    }
```

- [ ] **Step 2: 跑测确认失败**

Run: `cargo test -p loomgui_ffi_c abi_tests::load_package_builds_blob_from_package`
Expected: 编译失败（`loomgui_stage_load_package` 未定义）。

- [ ] **Step 3: 加 FFI 函数 + gate load_html + bump version**

`loomgui_ffi_c/Cargo.toml`：

```toml
[package]
name = "loomgui_ffi_c"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[features]
default = ["parse"]
parse = ["loomgui_core/parse"]

[dependencies]
loomgui_core = { path = "../loomgui_core", default-features = false }

[build-dependencies]
csbindgen = "1"
```

`loomgui_ffi_c/src/lib.rs`：

(a) `loomgui_version` 的字符串 `"v1a"` 改 `"v1b"`；对应测 `version_returns_c_string_v1a` 重命名为 `version_returns_c_string_v1b` 并把断言 `"v1a"` 改 `"v1b"`。

(b) 给 `loomgui_stage_load_html` 加 `#[cfg(feature = "parse")]`：

```rust
#[cfg(feature = "parse")]
#[no_mangle]
pub extern "C" fn loomgui_stage_load_html(
    h: *mut StageHandle,
    html: *const u8,
    html_len: usize,
    css: *const u8,
    css_len: usize,
) -> i32 { ... }
```

（`full_ffi_roundtrip_builds_blob` 测用 load_html → 加 `#[cfg(feature = "parse")]`。）

(c) 加新函数（紧跟 `load_html` 之后，**不加 cfg**——常驻）：

```rust
/// 装载二进制包（spec §12/§13）。bytes = .pkg.bin（指针+len）。0=ok，-1=err。
/// null 句柄/空指针返回 -1。包是 Rust-internal，C# 只透传 bytes（不解析）。
#[no_mangle]
pub extern "C" fn loomgui_stage_load_package(
    h: *mut StageHandle,
    bytes: *const u8,
    len: usize,
) -> i32 {
    if h.is_null() || bytes.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let bytes = unsafe { std::slice::from_raw_parts(bytes, len) };
    match sh.stage.load_package(bytes) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}
```

- [ ] **Step 4: 跑 abi 测通过**

Run: `cargo test -p loomgui_ffi_c`
Expected: 全绿（含新 load_package 测 + 原有 load_html/version 测）。

- [ ] **Step 5: 构建矩阵门——ffi_c no-default-features（仅 load_package）能编**

Run: `cargo build -p loomgui_ffi_c --no-default-features`
Expected: 编译成功（load_html 被 cfg 掉、core 无 parse）。若失败，补 cfg。

- [ ] **Step 6: 重生成 C# 绑定（build.rs 自动）**

Run: `cargo build -p loomgui_ffi_c`（default，带 parse——dev .dll）
然后确认 `loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs` 含 `loomgui_stage_load_package` 声明：

Run: `grep -n 'load_package' loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs`
Expected: 命中一行 `[DllImport] public static extern ... loomgui_stage_load_package(...)`。

- [ ] **Step 7: 重编 .dll（坑 10：FFI 改了必重编）**

Run: `cargo build -p loomgui_ffi_c --release`
Expected: 产出 `target/release/loomgui_ffi_c.dll`（Windows）。

> **关 Unity 才能换 .dll**（它锁文件）——换 .dll 步骤在 T8 与 Unity 验证一起做。

- [ ] **Step 8: 提交**

```bash
git add loomgui_ffi_c/Cargo.toml loomgui_ffi_c/src/lib.rs loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs
git commit -m "feat(v1b.1 T7): FFI loomgui_stage_load_package + load_html gate + version bump v1b"
```

---

## Task 8: Unity LoomStage 接线 + sample 包

**Files:**
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`
- Create: `loomgui_unity/Assets/StreamingAssets/loom_default.pkg.bin`（packager 产出）
- Swap: `loomgui_unity/Assets/Plugins/loomgui_ffi_c.dll`（T7 重编的）

**Interfaces:**
- Consumes: T7 `Native.loomgui_stage_load_package`、T6 打包器、T7 重编 .dll。
- Produces: Unity 可 `_usePackage=true` 从 StreamingAssets 加载包渲染。

> 本任务含 .dll 换 + PlayMode 验，需**关 Unity**操作 .dll，PlayMode 验**押用户批次**（无 headless Unity CLI）。

- [ ] **Step 1: LoomStage 加字段 + Awake 分支**

`LoomStage.cs`：在 `[SerializeField] Font _font;` 之后加：

```csharp
        // §13 v1b.1：从二进制包加载（true）vs inline _html/_css（false，默认保现有行为）。
        // true 时从 StreamingAssets/_pkgFile 读 .pkg.bin → loomgui_stage_load_package。
        [SerializeField] bool _usePackage;
        [SerializeField] string _pkgFile = "loom_default.pkg.bin";
```

在 `Awake()`，把现有 `if (!LoadHtml()) { ... }` 块替换为分支（load_html 调用挪进 else）：

```csharp
            bool loaded;
            if (_usePackage)
            {
                loaded = LoadPackage();
            }
            else
            {
                loaded = LoadHtml();
            }
            if (!loaded)
            {
                Debug.LogError("[LoomStage] load 失败");
                FreeStage();
                return;
            }
```

在 `LoadHtml()` 方法之后加 `LoadPackage()`：

```csharp
        /// <summary>
        /// §13 v1b.1：从 StreamingAssets/_pkgFile 读 .pkg.bin → loomgui_stage_load_package。
        /// 包是 Rust-internal，C# 只读文件透传 bytes（不解析）。editor/desktop 用 File.ReadAllBytes。
        /// </summary>
        bool LoadPackage()
        {
            if (_stage == null) return false;
            string pkgPath = System.IO.Path.Combine(Application.streamingAssetsPath, _pkgFile);
            if (!System.IO.File.Exists(pkgPath))
            {
                Debug.LogError($"[LoomStage] 包文件不存在：{pkgPath}");
                return false;
            }
            byte[] pkg = System.IO.File.ReadAllBytes(pkgPath);
            fixed (byte* pp = pkg)
            {
                int r = Native.loomgui_stage_load_package(_stage, pp, (nuint)pkg.Length);
                return r == 0;
            }
        }
```

> `fixed (byte* pp = pkg)` 要求 `LoomStage` 是 `unsafe`（已是：`public sealed unsafe class LoomStage`）。`Native.loomgui_stage_load_package` 由 T7 csbindgen 生成。

- [ ] **Step 2: 生成 sample 包到 StreamingAssets**

用当前默认场景（v1a 红块）的 html/css 打包。Run（在仓库根）：

```bash
mkdir -p loomgui_unity/Assets/StreamingAssets
printf '<div class="b"></div>' > /tmp/v1b_default.html
printf '.b{width:200px;height:100px;background-color:#ff0000;}' > /tmp/v1b_default.css
cargo run -q -p loomgui_pkg -- /tmp/v1b_default.html /tmp/v1b_default.css -o loomgui_unity/Assets/StreamingAssets/loom_default.pkg.bin -w 1080 -h 1920
```

Expected: `loomgui_unity/Assets/StreamingAssets/loom_default.pkg.bin` 产出（非空）。Unity 会自动生成 `.meta`。

- [ ] **Step 3: 关 Unity，换 .dll（坑 10）**

请用户关闭 Unity（.dll 被锁）。然后：

```bash
cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/loomgui_ffi_c.dll
```

Expected: 新 .dll（含 load_package）就位。

- [ ] **Step 4: PlayMode 验（押用户批次）**

用户开 Unity → 场景里 LoomStage 勾 `_usePackage=true`（_pkgFile 默认 `loom_default.pkg.bin`）→ Enter PlayMode。
Expected: Game 视图渲染 200×100 红块（与 inline 路径 `_usePackage=false` 视觉一致）；Console 无红字；Hierarchy 无累积 loom_node。
再切 `_usePackage=false` 验 inline 路径仍工作。

- [ ] **Step 5: 提交**

```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs loomgui_unity/Assets/StreamingAssets/loom_default.pkg.bin loomgui_unity/Assets/StreamingAssets/loom_default.pkg.bin.meta loomgui_unity/Assets/Plugins/loomgui_ffi_c.dll
git commit -m "feat(v1b.1 T8): Unity LoomStage 包加载接线 + sample pkg + 换 .dll（load_package）"
```

> `.meta`/`.dll` 文件视 Unity 实际生成情况 add；缺 .meta 不阻塞（Unity 开了会补）。

---

## 验收（全任务后）

- [ ] `cargo test --workspace` 全绿（core round-trip + 黄金等价 + 版本协商 + stringTable + ffi abi + pkg pack）。
- [ ] 构建矩阵：`cargo build -p loomgui_core --no-default-features`、`cargo build -p loomgui_ffi_c --no-default-features`、`cargo build -p loomgui_pkg`（带 parse）三者皆编。
- [ ] Unity PlayMode：`_usePackage=true` 渲染 == `_usePackage=false`（inline），命中验收 #6。
- [ ] knowledge-reference session-summary：记 v1b.1 新机制（§2 asset 层）+ 新坑（如有）+ ledger（v1b.1 ✅）。

## 风险复盘（对照 spec §16）

- **R1 feature gate**：T3 + T7 的构建矩阵门兜底（`--no-default-features` 编）。
- **R2 Scene::build 重构**：T2 直测 + 全 scene 测回归兜底。
- **R3 StyleRecord 字段遗漏**：T1 serde 派生穷尽（加字段编译期覆盖）+ round-trip 全字段断言。
