//! Render 层入口：遍历 solve 后的 Scene → `Vec<RenderNode>`（§8.7）。
//!
//! 顺序与 `scene.nodes` 索引一致（便于 node_id 对齐），payload 按 kind 决定：
//! - Container/Button → Mesh quad（背景色；无背景色时透明）
//! - Image → Mesh quad + tex_id（注册表查，未注册=0 哨兵→后端白占位）
//! - Text → measure_text 产 TextLayout，装 Text payload
//!
//! 最后调 `batch::assign_sort_keys` 填 sort_key + mask_context。

pub mod batch;
pub mod merge;
pub mod mesh;
pub mod node;

use crate::asset::texture::TextureRegistry;
use crate::scene::node::{NodeKind, Rect, Scene};
use crate::text::layout::{measure_text, Font};
use node::*;

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
pub fn build_render_nodes(scene: &Scene, font: &Font, textures: &TextureRegistry) -> FrameData {
    // 预分配 Unchanged 占位，逐个按 scene 节点覆写。
    let mut nodes: Vec<RenderNode> = (0..scene.nodes.len())
        .map(|_| RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0, 1.0, 1.0, 1.0],
            world_matrix: crate::transform::IDENTITY,
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Unchanged,
        })
        .collect();

    for n in &scene.nodes {
        let rn = &mut nodes[n.id.0];
        let anim = scene.anim.get(n.id);   // v1d.4：None 或全空 → None（退回 CSS）
        rn.node_id = n.id.0 as u32;
        rn.parent_id = n.parent.map(|p| p.0 as u32);
        rn.alpha = anim.and_then(|a| a.opacity).unwrap_or(n.style.opacity);
        rn.color_tint = anim.and_then(|a| a.text_color).unwrap_or(n.style.color);
        let wm = scene.world_transforms[n.id.0];
        rn.world_matrix = wm;
        rn.visible = true;
        // v1d.3 §3.5b：纯平移（identity/merge）→ 绝对顶点（layout_rect，供 merge）；
        // 非纯平移 → box 本地 (0,0,w,h)（供 matrix shader）。
        let rect = if crate::transform::is_pure_translation(&wm) {
            n.layout_rect
        } else {
            crate::scene::node::Rect { x: 0.0, y: 0.0, w: n.layout_rect.w, h: n.layout_rect.h }
        };
        let rect = &rect;
        match &n.kind {
            NodeKind::Container | NodeKind::Button => {
                let color = anim.and_then(|a| a.bg_color).unwrap_or(n.style.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]));
                let (v, uvc, col, idx) =
                    crate::render::mesh::quad(rect, color, [0.0, 0.0], [1.0, 1.0]);
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
                // tex_id + uv_region：注册表查（未注册=0 哨兵+全图 UV→后端白占位）。
                // 注册时按 atlas 子区烤 4 角 UV（TL/TR/BR/BL），让同一 atlas tex 切多 sprite。
                let (tex_id, uv_min, uv_max) = match textures.get(src) {
                    Some(m) => (m.tex_id, m.uv_min, m.uv_max),
                    None => (0u32, [0.0, 0.0], [1.0, 1.0]),
                };
                let (v, uvc, col, idx) =
                    crate::render::mesh::quad(rect, [1.0, 1.0, 1.0, 1.0], uv_min, uv_max);
                rn.payload = NodePayload::Mesh {
                    verts: v,
                    uvs: uvc,
                    colors: col,
                    indices: idx,
                    texture: tex_id,
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
                    color: anim.and_then(|a| a.text_color).unwrap_or(s.color),
                    program: 1,
                };
            }
        }
    }
    let clips = batch::assign_sort_keys(scene, &mut nodes);
    // §8.5/§8.8 v1b.4：先按 BatchingRoot AABB 保序重排（同 DrawState 不相交聚拢），
    // 再合并连续同 DrawState 的 program=0 Mesh → 1 draw call。clips 表由
    // assign_sort_keys 产（mask_context 在 reorder/merge 中透传，表内容不受影响）。
    batch::reorder_for_batching(scene, &mut nodes);
    let nodes = merge::merge_meshes(nodes);
    FrameData { nodes, clips }
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
    use crate::asset::texture::{TexMeta, TextureRegistry};
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
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(),
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
        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &TextureRegistry::default());
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
        // world_matrix 纯平移 → tx/ty = layout_rect x/y
        assert_eq!(rns[0].world_matrix[4], 1.0);
        assert_eq!(rns[0].world_matrix[5], 2.0);
    }

    #[test]
    fn build_image_uses_registered_tex_id() {
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default() };
        let mut a = Node::default();
        a.id = NodeId(0);
        a.kind = NodeKind::Image { src: "logo.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 };
        scene.nodes.push(a);

        let font = test_font().expect("need test font");
        let mut tex = TextureRegistry::default();
        // 非平凡 uv（atlas 子区）：TL=(0.25,0) BR=(0.5,1)。
        let tid = { tex.insert("logo.png", TexMeta {
            tex_id: 1, uv_min: [0.25, 0.0], uv_max: [0.5, 1.0], width: 200, height: 100,
        }); 1 };
        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &tex);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, uvs, .. } => {
                assert_eq!(*texture, tid, "注册后 Image.texture == 注册分配的 tex_id");
                assert_ne!(*texture, 0, "已注册 tex_id 不应为 0");
                // UV 按 region 烤：TL/BR 命中 uv_min/uv_max。
                assert_eq!(uvs[0], [0.25, 0.0], "TL == uv_min");
                assert_eq!(uvs[2], [0.5, 1.0], "BR == uv_max");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_image_unregistered_is_zero() {
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default() };
        let mut a = Node::default();
        a.id = NodeId(0);
        a.kind = NodeKind::Image { src: "logo.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 };
        scene.nodes.push(a);

        let font = test_font().expect("need test font");
        let tex = TextureRegistry::default(); // 未注册
        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &tex);
        let got = match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, .. } => *texture,
            _ => panic!("expected Mesh"),
        };
        assert_eq!(got, 0, "未注册 src → tex_id=0 哨兵");
    }

    #[test]
    fn build_image_unregistered_uv_is_full() {
        // 未注册 src → 哨兵 uv (0,0)-(1,1)（与 tex_id=0 白占位配合，UV 无关）。
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default() };
        let mut a = Node::default();
        a.id = NodeId(0);
        a.kind = NodeKind::Image { src: "logo.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 };
        scene.nodes.push(a);

        let font = test_font().expect("need test font");
        let tex = TextureRegistry::default(); // 未注册
        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &tex);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { uvs, .. } => {
                assert_eq!(uvs[0], [0.0, 0.0], "未注册 TL == (0,0)");
                assert_eq!(uvs[2], [1.0, 1.0], "未注册 BR == (1,1)");
            }
            _ => panic!("expected Mesh"),
        }
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
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(),
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

        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &TextureRegistry::default());
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
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(),
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

        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &TextureRegistry::default());
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
        // v1b.4 后：reorder+merge 接入。3 个同 DrawState Container 会合并成 1 个节点，
        // 故原「root > [a, b]」结构不再保 3 节点。改用嵌套 clip 链（root > mid > leaf，
        // 每层 clip_rect 开新 mask_context）→ 3 个不同 DrawState → 不合并 → 保 3 节点。
        // 验 sort_key 单调（batch 已测，这里走端到端确认 build 接通 assign_sort_keys）。
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(),
        };
        let mut root = container_node(0, None, Rect::default(), None);
        root.clip_rect = Some(Rect::default()); // 开 mask_context=1
        root.children = vec![NodeId(1)];
        scene.nodes.push(root);
        let mut mid = container_node(1, Some(0), Rect::default(), None);
        mid.clip_rect = Some(Rect::default()); // 开 mask_context=2
        mid.children = vec![NodeId(2)];
        scene.nodes.push(mid);
        let mut leaf = container_node(2, Some(1), Rect::default(), None);
        leaf.clip_rect = Some(Rect::default()); // 开 mask_context=3
        scene.nodes.push(leaf);

        let font = test_font().expect("need test font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &TextureRegistry::default());
        let rns = &frame.nodes;
        // 3 个不同 mask_context → 不合并 → 保 3 节点；sort_key 经 reorder 重赋后仍单调。
        assert_eq!(rns.len(), 3, "3 个不同 mask_context → 不合并");
        assert!(rns[0].sort_key < rns[1].sort_key);
        assert!(rns[1].sort_key < rns[2].sort_key);
    }

    /// §8.5/§8.8 v1b.4：端到端——build_render_nodes 已接 reorder + merge。
    /// root(Container, tex_id=0) > [img A, img B]（同 tex_id=1、同 mask_context、
    /// AABB 不相交）。reorder 让两 Image 相邻，merge 合两 Image 成 1 个 8-vert
    /// merged mesh；root 是 Container(tex_id=0) 不同 DrawState → 不合。
    /// 结果：FrameData 含恰好 1 个 8-vert Mesh payload（两 Image 合并）。
    #[test]
    fn build_merges_adjacent_same_drawstate_meshes() {
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default() };
        let mut root = container_node(
            0,
            None,
            Rect { x: 0.0, y: 0.0, w: 300.0, h: 50.0 },
            None,
        );
        root.children = vec![NodeId(1), NodeId(2)];
        scene.nodes.push(root);
        let mut a = Node::default();
        a.id = NodeId(1);
        a.parent = Some(NodeId(0));
        a.kind = NodeKind::Image { src: "a.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        scene.nodes.push(a);
        let mut b = Node::default();
        b.id = NodeId(2);
        b.parent = Some(NodeId(0));
        b.kind = NodeKind::Image { src: "a.png".into() };
        b.layout_rect = Rect { x: 100.0, y: 0.0, w: 10.0, h: 10.0 };
        scene.nodes.push(b);

        let font = test_font().expect("need test font");
        let mut tex = TextureRegistry::default();
        tex.insert(
            "a.png",
            TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 10, height: 10 },
        );

        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &tex);
        // root(Container, tex_id=0) + 1 merged(Image tex_id=1) = 2 节点（原 3）。
        let mesh_count = frame
            .nodes
            .iter()
            .filter(|n| matches!(&n.payload, NodePayload::Mesh { verts, .. } if verts.len() == 8))
            .count();
        assert_eq!(mesh_count, 1, "两同 atlas Image → 1 个 8-vert merged mesh");
        // merged node 的 world_matrix 应为 IDENTITY（merge_batch 把锚矩阵置 identity），
        // 顶点保持绝对 design 坐标。
        let merged = frame
            .nodes
            .iter()
            .find(|n| matches!(&n.payload, NodePayload::Mesh { verts, .. } if verts.len() == 8))
            .expect("merged node 存在");
        assert!(crate::transform::is_identity(&merged.world_matrix));
        assert!((merged.alpha - 1.0).abs() < 1e-6, "merged alpha=1 防 blob 二次烤");
    }

    /// v1d.4：build_render_nodes 读 anim.opacity/bg_color override（replace-override）。
    /// CSS opacity=1.0、bg=红；anim opacity=0.25、bg=蓝 → alpha=0.25、Mesh colors=蓝。
    #[test]
    fn build_reads_anim_opacity_and_bg_override() {
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(),
            anim: Default::default(),
            scroll: Default::default(),
        };
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.opacity = 1.0;
        scene.nodes.push(n);
        // anim override：opacity=0.25、bg=蓝
        scene.anim.ensure(1);
        scene.anim.0[0].opacity = Some(0.25);
        scene.anim.0[0].bg_color = Some([0.0, 0.0, 1.0, 1.0]);

        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let frame = build_render_nodes(&scene, &font, &TextureRegistry::default());
        assert!((frame.nodes[0].alpha - 0.25).abs() < 1e-5, "anim.opacity override → alpha=0.25");
        match &frame.nodes[0].payload {
            NodePayload::Mesh { colors, .. } => {
                assert_eq!(*colors.first().unwrap(), [0.0, 0.0, 1.0, 1.0], "anim.bg_color override → 蓝");
            }
            _ => panic!("expected Mesh"),
        }
    }
}
