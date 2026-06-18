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

use crate::scene::node::{NodeKind, Scene};
use crate::text::layout::{measure_text, Font};
use node::*;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 遍历 Scene → `Vec<RenderNode>`。
///
/// 顺序与 `scene.nodes` 同序（node_id == scene 索引），便于 batch DFS 对齐。
/// Text 节点调 `measure_text` 产 TextLayout；Container/Image 产 Mesh quad。
/// `font` 仅 Text 节点用（v0 单字体）。
pub fn build_render_nodes(scene: &Scene, font: &Font) -> Vec<RenderNode> {
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
                let layout = measure_text(
                    content,
                    s.font_size,
                    s.line_height,
                    s.letter_spacing,
                    s.text_align,
                    s.white_space_nowrap,
                    Some(rect.w),
                    font,
                );
                rn.payload = NodePayload::Text {
                    layout,
                    font_size: s.font_size,
                    color: s.color,
                    program: 1,
                };
            }
        }
    }
    batch::assign_sort_keys(scene, &mut nodes);
    nodes
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::*;
    use crate::style::resolved::TextAlign;

    /// 测试字体：Windows arial.ttf / Linux DejaVuSans.ttf，无则跳过。
    fn test_font() -> Option<Font> {
        for p in [
            "C:\\Windows\\Fonts\\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ] {
            if let Ok(f) = Font::from_path(p) {
                return Some(f);
            }
        }
        None
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
        let rns = build_render_nodes(&scene, &font);
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
        let rns = build_render_nodes(&scene, &font);
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

        let rns = build_render_nodes(&scene, &font);
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
        let rns = build_render_nodes(&scene, &font);
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
