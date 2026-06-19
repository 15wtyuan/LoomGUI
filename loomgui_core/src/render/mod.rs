//! Render 层入口：遍历 solve 后的 Scene → `Vec<RenderNode>`（§8.7）。
//!
//! 顺序与 `scene.nodes` 索引一致（便于 node_id 对齐），payload 按 kind 决定：
//! - Container/Button → Mesh quad（背景色；无背景色时透明）
//! - Image → Mesh quad + 占位 tex_id = hash(src)
//! - Text → measure_text 产 TextLayout，装 Text payload
//!
//! 最后调 `batch::assign_sort_keys` 填 sort_key + mask_context。

pub mod batch;
pub mod mesh;
pub mod node;

use crate::scene::node::{NodeKind, Rect, Scene};
use crate::text::layout::{measure_text, Font};
use node::*;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use taffy::style::LengthPercentage;

/// clip 表条目：context_id（mask_context>0 的层级）→ 该层级的交集绝对 design rect。
///
/// 由 `batch::assign_sort_keys` 在 DFS 时产；`context_id` 与 RenderNode 的
/// `mask_context.0` 对齐（被该 clip 约束的节点引用同一 id）。§4.4 rect mask / §4.1 clip 表。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipEntry {
    pub context_id: u32,
    pub rect: Rect,
}

/// 一帧渲染数据：节点 + clip 表（FFI blob 同帧 emit）。
///
/// `clips` 只含 mask_context>0 的层级；context==0（无 clip）永不入表。
/// 由 `build_render_nodes` 产，`stage::tick_and_render` 透传，`blob::build_blob` 消费。
#[derive(Debug, Clone)]
pub struct FrameData {
    pub nodes: Vec<RenderNode>,
    pub clips: Vec<ClipEntry>,
}

/// 遍历 Scene → `FrameData`（nodes + clip 表）。
///
/// 顺序与 `scene.nodes` 同序（node_id == scene 索引），便于 batch DFS 对齐。
/// Text 节点调 `measure_text` 产 TextLayout；Container/Image 产 Mesh quad。
/// `font` 仅 Text 节点用（v0 单字体）。clip 表由 `batch::assign_sort_keys` 算
/// 祖先 clip 链交集后产出（§4.4）。
pub fn build_render_nodes(scene: &Scene, font: &Font) -> FrameData {
    // 预分配 Unchanged 占位，逐个按 scene 节点覆写。
    let mut nodes: Vec<RenderNode> = (0..scene.nodes.len())
        .map(|_| RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0, 1.0, 1.0, 1.0],
            transform: NodeTransform::default(),
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Unchanged,
        })
        .collect();

    for n in &scene.nodes {
        let rn = &mut nodes[n.id.0];
        rn.node_id = n.id.0 as u32;
        rn.parent_id = n.parent.map(|p| p.0 as u32);
        rn.alpha = n.style.opacity;
        rn.color_tint = n.style.color;
        rn.transform.x = n.layout_rect.x;
        rn.transform.y = n.layout_rect.y;
        rn.visible = true;
        let rect = &n.layout_rect;
        match &n.kind {
            NodeKind::Container | NodeKind::Button => {
                let color = n.style.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let (v, uvc, col, idx) = crate::render::mesh::quad(rect, color);
                rn.payload = NodePayload::Mesh {
                    verts: v,
                    uvs: uvc,
                    colors: col,
                    indices: idx,
                    texture: 0,
                    program: 0,
                };
            }
            NodeKind::Image { src } => {
                // 图片 quad：白色 tint（贴图本色），占位 tex_id = hash(src)。
                let (v, uvc, col, idx) = crate::render::mesh::quad(rect, [1.0, 1.0, 1.0, 1.0]);
                let tex = hash_str(src);
                rn.payload = NodePayload::Mesh {
                    verts: v,
                    uvs: uvc,
                    colors: col,
                    indices: idx,
                    texture: tex,
                    program: 0,
                };
            }
            NodeKind::Text { content } => {
                let s = &n.style;
                let mut layout = measure_text(
                    content,
                    s.font_size,
                    s.line_height,
                    s.letter_spacing,
                    s.text_align,
                    s.white_space_nowrap,
                    Some(rect.w),
                    font,
                );
                // §4.3：pen 必须 GO-local（相对节点 GO 原点 = layout_rect 原点）。
                // measure_text 产 glyph 坐标相对 content-box 原点（border+padding 内）。
                // 烤 content 偏移 = (border_left + padding_left, border_top + padding_top)
                // 进每个 glyph 的 (x, y)，让序列化的 pen_x/pen_y 直接可摆（Unity 不 re-base）。
                let off_x = resolve_lp(s.taffy_style.border.left)
                    + resolve_lp(s.taffy_style.padding.left);
                let off_y = resolve_lp(s.taffy_style.border.top)
                    + resolve_lp(s.taffy_style.padding.top);
                if off_x != 0.0 || off_y != 0.0 {
                    bake_content_offset(&mut layout, off_x, off_y);
                }
                rn.payload = NodePayload::Text {
                    layout,
                    font_size: s.font_size,
                    color: s.color,
                    program: 1,
                };
            }
        }
    }
    let clips = batch::assign_sort_keys(scene, &mut nodes);
    FrameData { nodes, clips }
}

/// src → 占位 tex_id：DefaultHasher 低 16 位。
///
/// v0 无贴图集；tex_id 仅用于区分不同 src（Task 8 stage 层 / 后端按 tex_id 断批合占位）。
/// 16 位碰撞概率对 v0 单页面图片数足够。
fn hash_str(s: &str) -> u32 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    (h.finish() & 0xFFFF) as u32
}

/// 把 taffy `LengthPercentage` 解析为 px。
///
/// - `Length(v)` → v。
/// - `Percent(_)` → 0.0。**已知缺口**（记 ledger）：渲染阶段无父 content-box 宽度上下文，
///   无法解析百分比的 padding/border。v0 `style::mapping::parse_four` 对 padding/border
///   只产 `Length`（裸数字/px），故实际不会命中 Percent 分支；若未来 CSS 允许百分比
///   padding/border，需在 layout 阶段把解析结果写回 ResolvedStyle（v1b 补）。
fn resolve_lp(lp: LengthPercentage) -> f32 {
    match lp {
        LengthPercentage::Length(v) => v,
        LengthPercentage::Percent(_) => 0.0,
    }
}

/// 烤 content 偏移进 TextLayout 每个 glyph 的 (x, y)（pen = GO-local）。
/// layout 是刚由 measure_text 产的 owned 值，直接 mutate。
fn bake_content_offset(layout: &mut crate::text::layout::TextLayout, off_x: f32, off_y: f32) {
    for line in &mut layout.lines {
        for run in &mut line.runs {
            for g in &mut run.glyphs {
                g.x += off_x;
                g.y += off_y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::*;
    use crate::style::resolved::TextAlign;

    /// 测试字体：仓库内 DejaVuSans.ttf（跨平台一致），缺则跳过。
    fn test_font() -> Option<Font> {
        let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
        Font::from_path(&p).ok()
    }

    /// 构造一个带 layout_rect 的 Container Node。
    fn container_node(id: usize, parent: Option<usize>, rect: Rect, bg: Option<[f32; 4]>) -> Node {
        let mut n = Node::default();
        n.id = NodeId(id);
        n.parent = parent.map(NodeId);
        n.kind = NodeKind::Container;
        n.layout_rect = rect;
        n.style.background_color = bg;
        n
    }

    #[test]
    fn build_container_produces_mesh_quad() {
        // root 红底 10x10 → Mesh payload，4 verts / 6 indices，背景色烤进 colors。
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
        };
        scene.nodes.push(container_node(
            0,
            None,
            Rect {
                x: 1.0,
                y: 2.0,
                w: 10.0,
                h: 10.0,
            },
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        let font = test_font().expect("need test font for build_render_nodes");
        let frame = build_render_nodes(&scene, &font);
        let rns = &frame.nodes;
        assert_eq!(rns.len(), 1);
        match &rns[0].payload {
            NodePayload::Mesh {
                verts,
                indices,
                colors,
                texture,
                program,
                ..
            } => {
                assert_eq!(verts.len(), 4);
                assert_eq!(indices.len(), 6);
                assert_eq!(*texture, 0, "Container 无贴图");
                assert_eq!(*program, 0);
                for c in colors {
                    assert_eq!(*c, [1.0, 0.0, 0.0, 1.0]);
                }
            }
            _ => panic!("expected Mesh payload"),
        }
        // transform 直填 layout_rect
        assert_eq!(rns[0].transform.x, 1.0);
        assert_eq!(rns[0].transform.y, 2.0);
    }

    #[test]
    fn build_image_hashes_tex_id() {
        // 同 src → 同 tex_id；不同 src → 不同 tex_id。
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
        };
        let mut a = Node::default();
        a.id = NodeId(0);
        a.kind = NodeKind::Image {
            src: "logo.png".into(),
        };
        a.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 5.0,
            h: 5.0,
        };
        scene.nodes.push(a);

        let mut b = Node::default();
        b.id = NodeId(1);
        b.kind = NodeKind::Image {
            src: "other.png".into(),
        };
        b.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 5.0,
            h: 5.0,
        };
        scene.nodes.push(b);
        // 让 roots 指两个独立根（避免 batch DFS 跨连）。
        scene.roots = vec![NodeId(0), NodeId(1)];

        let font = test_font().expect("need test font");
        let frame = build_render_nodes(&scene, &font);
        let rns = &frame.nodes;
        let tex_a = match &rns[0].payload {
            NodePayload::Mesh { texture, .. } => *texture,
            _ => panic!("expected Mesh"),
        };
        let tex_b = match &rns[1].payload {
            NodePayload::Mesh { texture, .. } => *texture,
            _ => panic!("expected Mesh"),
        };
        assert_ne!(tex_a, tex_b, "不同 src 应得不同 tex_id");
        assert_eq!(tex_a, hash_str("logo.png"));
    }

    #[test]
    fn build_text_produces_text_layout() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
        };
        let mut n = Node::default();
        n.id = NodeId(0);
        n.kind = NodeKind::Text {
            content: "Hello".into(),
        };
        n.style.font_size = 16.0;
        n.style.text_align = TextAlign::Left;
        n.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
        };
        scene.nodes.push(n);

        let frame = build_render_nodes(&scene, &font);
        let rns = &frame.nodes;
        match &rns[0].payload {
            NodePayload::Text {
                layout,
                font_size,
                program,
                ..
            } => {
                assert_eq!(*font_size, 16.0);
                assert_eq!(*program, 1);
                assert!(!layout.lines.is_empty());
            }
            _ => panic!("expected Text payload"),
        }
    }

    /// §4.3：pen 必须 GO-local——measure_text 产 content-box 相对坐标，
    /// build_render_nodes 烤 (border_left+padding_left, border_top+padding_top) 偏移。
    /// 设 padding=4px、border=2px → content 偏移 (6, 6)，每 glyph 的 (x,y) 应 +6。
    #[test]
    fn build_text_bakes_content_offset_into_glyph_pen() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
        };
        let mut n = Node::default();
        n.id = NodeId(0);
        n.kind = NodeKind::Text {
            content: "AB".into(),
        };
        n.style.font_size = 16.0;
        // padding/border 四向 4px/2px → content 偏移 left=2+4=6, top=2+4=6。
        n.style.taffy_style.padding = taffy::geometry::Rect {
            left: taffy::style::LengthPercentage::Length(4.0),
            right: taffy::style::LengthPercentage::Length(4.0),
            top: taffy::style::LengthPercentage::Length(4.0),
            bottom: taffy::style::LengthPercentage::Length(4.0),
        };
        n.style.taffy_style.border = taffy::geometry::Rect {
            left: taffy::style::LengthPercentage::Length(2.0),
            right: taffy::style::LengthPercentage::Length(2.0),
            top: taffy::style::LengthPercentage::Length(2.0),
            bottom: taffy::style::LengthPercentage::Length(2.0),
        };
        n.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
        };
        scene.nodes.push(n);

        let frame = build_render_nodes(&scene, &font);
        let rns = &frame.nodes;
        match &rns[0].payload {
            NodePayload::Text { layout, .. } => {
                let g = &layout.lines[0].runs[0].glyphs;
                assert_eq!(g.len(), 2, "AB = 2 glyphs");
                // 每 glyph 的 y 原 = line.y(0)，+content offset(6) = 6.0。
                assert_eq!(g[0].y, 6.0, "pen_y 烤 content 偏移 (border+padding top=6)");
                assert_eq!(g[1].y, 6.0);
                // 首 glyph x 原 = 0（Left align），+6 = 6.0；次 glyph x 应 > 6（advance）。
                assert_eq!(g[0].x, 6.0, "首 glyph pen_x = align 偏移0 + content 偏移6");
                assert!(g[1].x > 6.0, "次 glyph pen_x > 6（含 advance + 偏移）");
                // codepoint 也顺带验（T3 Step 1）。
                assert_eq!(g[0].codepoint, b'A' as u32);
                assert_eq!(g[1].codepoint, b'B' as u32);
            }
            _ => panic!("expected Text payload"),
        }
    }

    #[test]
    fn build_assigns_monotonic_keys() {
        // root > [a, b]：sort_key 0 < 1 < 2（batch 已测，这里走端到端确认 build 接通）。
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
        };
        let mut root = container_node(0, None, Rect::default(), None);
        root.children = vec![NodeId(1), NodeId(2)];
        scene.nodes.push(root);
        scene.nodes.push(container_node(1, Some(0), Rect::default(), None));
        scene.nodes.push(container_node(2, Some(0), Rect::default(), None));

        let font = test_font().expect("need test font");
        let frame = build_render_nodes(&scene, &font);
        let rns = &frame.nodes;
        assert!(rns[0].sort_key < rns[1].sort_key);
        assert!(rns[1].sort_key < rns[2].sort_key);
    }

    #[test]
    fn hash_str_is_deterministic_and_low16() {
        // 同输入同输出；输出在 16 位范围。
        let a = hash_str("foo.png");
        let b = hash_str("foo.png");
        assert_eq!(a, b);
        assert!(a < (1u32 << 16));
        assert_ne!(a, hash_str("bar.png"));
    }
}
