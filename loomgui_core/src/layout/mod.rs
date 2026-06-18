//! Layout 层：taffy 集成。
//!
//! 消费 `Scene`（Node 树 + `ResolvedStyle`），建 taffy 树，注册叶子节点的
//! 测量上下文（Text/Image），solve 后把 taffy 的 `Layout.location`/`size`
//! 回写进 `Node.layout_rect`/`clip_rect`。对应主文档 §7。
//!
//! # taffy 0.5.2 API 适配（与 task-6-brief 的差异）
//!
//! brief 草稿用 `MeasureFunc::Boxed`（move 闭包进叶子），但 taffy 0.5.2 没有
//! `MeasureFunc` 枚举。实际 API 是 trait 对象模式：
//! - `TaffyTree<NodeContext>`：节点上下文是泛型，叶子节点用
//!   `new_leaf_with_context(style, ctx)` 存一个 owned `NodeContext`。
//! - 单个 `compute_layout_with_measure(root, avail, FnMut(...))` 闭包负责按
//!   `Option<&mut NodeContext>` 分派到 Text/Image 测量。
//!
//! **carry 项 1（Arc<Font>）因此自然消解**：brief 假设每个叶子闭包独立捕获
//! `font`，故要 'static → Arc。但 0.5.2 的测量是单个 `FnMut`（非 'static），
//! 生命周期与 `compute_layout_with_measure` 调用同界——闭包内借 `&font` 完全
//! 合法。每个叶子的 *文本参数*（content/font_size 等）已 owned 进
//! `NodeContext::Text`，font 不进 context 而走闭包借用。`solve` 签名保持收
//! `font: &Font`（与 brief 一致，不破下游 stage 契约）。
//!
//! **carry 项 2（measure_text 8 参数）**：原样保留，从 `NodeContext::Text`
//! 取参数 + 闭包借用 font 传入。
//!
//! **order 字段**：taffy 0.5.2 的 `Style` 无 `order`（确认见 `style/mod.rs`）。
//! v0 不做 flex order 排序，留 ledger（render 层按 DOM 顺序 / layout 输出的
//! `Layout.order` 渲染）。

use crate::scene::node::{NodeId, NodeKind, Rect, Scene};
use crate::style::resolved::TextAlign;
use crate::text::layout::{measure_text, Font};
use taffy::prelude::*;

/// 叶子节点的测量上下文。Container/Button 无上下文（用 None 叶子或 new_with_children）。
enum MeasureContext {
    /// Text 叶子：存全部测量参数（content owned）+ 字体度量字段。
    /// font *不* 进 context——font 在闭包借用层共享（见模块 doc）。
    Text {
        content: String,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        align: TextAlign,
        nowrap: bool,
    },
    /// Image 叶子：intrinsic 占位尺寸（声明 size 或 64x64）。
    Image { w: f32, h: f32 },
}

/// 就地 solve：建 taffy 树 → 注册测量上下文 → compute_layout → 回写 layout_rect/clip_rect。
///
/// `root_size` 是根节点固定尺寸（viewport / surface 尺寸）。`font` 借用到
/// `compute_layout_with_measure` 结束，闭包内解引用喂给 `measure_text`。
pub fn solve(scene: &mut Scene, font: &Font, root_size: (f32, f32)) {
    let mut taffy_tree: TaffyTree<MeasureContext> = TaffyTree::new();
    // scene NodeId → taffy NodeId 映射（按 NodeId.0 索引）。
    let mut taffy_ids: Vec<Option<taffy::NodeId>> = vec![None; scene.nodes.len()];

    fn build(
        scene: &Scene,
        tree: &mut TaffyTree<MeasureContext>,
        taffy_ids: &mut Vec<Option<taffy::NodeId>>,
        id: NodeId,
    ) -> taffy::NodeId {
        let node = &scene.nodes[id.0];
        let style = node.style.taffy_style.clone();
        // 叶子：Text/Image 装 MeasureContext。
        let ctx: Option<MeasureContext> = match &node.kind {
            NodeKind::Text { content } => {
                let s = &node.style;
                Some(MeasureContext::Text {
                    content: content.clone(),
                    font_size: s.font_size,
                    line_height: s.line_height,
                    letter_spacing: s.letter_spacing,
                    align: s.text_align,
                    nowrap: s.white_space_nowrap,
                })
            }
            NodeKind::Image { src: _ } => {
                // v0 占位：intrinsic 用声明的 size，无则 64x64。
                let s = &node.style.taffy_style;
                let w = if let Dimension::Length(v) = s.size.width { v } else { 64.0 };
                let h = if let Dimension::Length(v) = s.size.height { v } else { 64.0 };
                Some(MeasureContext::Image { w, h })
            }
            _ => None,
        };

        // 递归子节点（先建子，再建父以便 new_with_children）。
        let children_ids: Vec<taffy::NodeId> = node
            .children
            .iter()
            .map(|c| build(scene, tree, taffy_ids, *c))
            .collect();

        let tid = if let Some(mctx) = ctx {
            // 叶子：装测量上下文。children 应为空（Text/Image 是叶子）。
            tree.new_leaf_with_context(style, mctx).unwrap()
        } else {
            // 容器：用 children 建。
            tree.new_with_children(style, &children_ids).unwrap()
        };
        taffy_ids[id.0] = Some(tid);
        tid
    }

    let root_tid = build(scene, &mut taffy_tree, &mut taffy_ids, scene.roots[0]);

    // 设根 size：覆盖为调用方给的 root_size（viewport）。
    // Style.size 字段类型是 Size<Dimension>（不是 LengthPercentageAuto）。
    let root_style = taffy_tree.style(root_tid).unwrap().clone();
    taffy_tree
        .set_style(
            root_tid,
            Style {
                size: Size {
                    width: Dimension::Length(root_size.0),
                    height: Dimension::Length(root_size.1),
                },
                ..root_style
            },
        )
        .ok();

    // solve：单一 FnMut 闭包按 context 分派。
    // known.width: Option<f32> —— Some=约束宽，None=不限（→ measure_text max_width=None）。
    taffy_tree
        .compute_layout_with_measure(
            root_tid,
            Size::MAX_CONTENT,
            |known: Size<Option<f32>>,
             _avail: Size<AvailableSpace>,
             _nid: taffy::NodeId,
             node_ctx: Option<&mut MeasureContext>,
             _style: &Style|
             -> Size<f32> {
                match node_ctx {
                    None => Size::ZERO,
                    Some(MeasureContext::Image { w, h }) => Size { width: *w, height: *h },
                    Some(MeasureContext::Text { content, font_size, line_height, letter_spacing, align, nowrap }) => {
                        let layout = measure_text(
                            content,
                            *font_size,
                            *line_height,
                            *letter_spacing,
                            *align,
                            *nowrap,
                            known.width,
                            font,
                        );
                        Size { width: layout.text_width, height: layout.text_height }
                    }
                }
            },
        )
        .ok();

    // 回写 layout_rect + clip_rect（递归累加父 origin 得绝对坐标）。
    fn write_back(
        scene: &mut Scene,
        tree: &TaffyTree<MeasureContext>,
        taffy_ids: &[Option<taffy::NodeId>],
        id: NodeId,
        parent_origin: (f32, f32),
    ) {
        let tid = taffy_ids[id.0].unwrap();
        let layout = tree.layout(tid).unwrap();
        let x = parent_origin.0 + layout.location.x;
        let y = parent_origin.1 + layout.location.y;
        let (w, h) = (layout.size.width, layout.size.height);
        let node = &mut scene.nodes[id.0];
        node.layout_rect = Rect { x, y, w, h };
        // overflow:hidden 节点（build_scene 已建 Some 槽）：用自身 border 框填 clip。
        if node.clip_rect.is_some() {
            node.clip_rect = Some(Rect { x, y, w, h });
        }
        let kids = node.children.clone();
        for c in kids {
            write_back(scene, tree, taffy_ids, c, (x, y));
        }
    }
    write_back(scene, &taffy_tree, &taffy_ids, scene.roots[0], (0.0, 0.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{css::parse_css, dom::parse_html};
    use crate::scene::node::build_scene;
    use crate::style::cascade::resolve_styles;

    fn font() -> Option<Font> {
        for p in ["C:\\Windows\\Fonts\\arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"] {
            if let Ok(f) = Font::from_path(p) {
                return Some(f);
            }
        }
        None
    }

    #[test]
    fn column_stack_sizes_children() {
        // brief 原样保留 inline-style 版本作语义对照（v0 无 inline style → 用 class）。
        let _html = r#"<div class="root"><div style="height:50px"></div><div style="height:30px"></div></div>"#;
        let html2 = r#"<div class="root"><div class="a"></div><div class="b"></div></div>"#;
        // §4.1：div 默认 flex-direction: column（ResolvedStyle::default 落地）。
        // CSS 不写 flex-direction，子项也应垂直堆叠。
        let css = r#".root { width: 200px; height: 200px; } .a { height: 50px; } .b { height: 30px; }"#;
        let tree = parse_html(html2).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let mut scene = build_scene(&tree, &styles);
        solve(&mut scene, &font().expect("test needs a font"), (200.0, 200.0));
        let root = &scene.nodes[scene.roots[0].0];
        let a = &scene.nodes[root.children[0].0];
        let b = &scene.nodes[root.children[1].0];
        assert!((a.layout_rect.h - 50.0).abs() < 0.1);
        assert!((b.layout_rect.h - 30.0).abs() < 0.1);
        assert!((b.layout_rect.y - 50.0).abs() < 0.1); // 垂直堆叠
    }

    /// §4.1 回归：未显式写 flex-direction 的 div 默认垂直堆叠（column）。
    /// 防止有人把 ResolvedStyle::default 的 flex_direction 改回 Row。
    #[test]
    fn default_div_is_column() {
        let html = r#"<div class="root"><div class="a"></div><div class="b"></div></div>"#;
        let css = r#".root { width: 200px; height: 200px; } .a { width: 50px; height: 50px; } .b { width: 30px; height: 30px; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let mut scene = build_scene(&tree, &styles);
        solve(&mut scene, &font().expect("test needs a font"), (200.0, 200.0));
        let root = &scene.nodes[scene.roots[0].0];
        let a = &scene.nodes[root.children[0].0];
        let b = &scene.nodes[root.children[1].0];
        // 垂直堆叠：b.y ≈ a.h（a 在上，b 在下）。
        // 若默认回退到 row，b.y 会 ≈ 0（横排），此断言失败。
        assert!(
            (b.layout_rect.y - a.layout_rect.h).abs() < 0.1,
            "expected column stack (b.y ≈ a.h), got a.h={} b.y={}",
            a.layout_rect.h,
            b.layout_rect.y
        );
        // a、b 同 x（列内左对齐），x 都 ≈ 0。
        assert!(a.layout_rect.x.abs() < 0.1);
        assert!(b.layout_rect.x.abs() < 0.1);
    }
}
