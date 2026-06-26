//! v1e dirty hash：逐节点算 u64 hash，供 Stage 跨帧比较决定 emit Unchanged 还是重传。
//! 字段集对照 blob 公共头列（blob.rs:23-27）+ payload 摘要。碰撞最坏 1 帧延迟，不破正确性。
//! ponytail: hash 碰撞最坏 1 帧视觉延迟；换全量 hash 若 profiling 显示遗漏。

use crate::render::node::{RenderNode, NodePayload};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 算节点 dirty hash。调用方保证 payload 为 Mesh/Text（非 Unchanged）。
/// Unchanged 防御性返回 0（不该被调）。
pub fn node_hash(rn: &RenderNode) -> u64 {
    let mut h = DefaultHasher::new();
    // 公共头列（blob.rs:23-27）。
    for &v in rn.world_matrix.iter() { v.to_le_bytes().hash(&mut h); } // Affine2=[f32;6]，含 scroll_pos
    rn.visible.hash(&mut h);
    rn.alpha.to_le_bytes().hash(&mut h);
    rn.grayed.hash(&mut h);
    for &v in rn.color_tint.iter() { v.to_le_bytes().hash(&mut h); }  // [f32;4]
    (match rn.blend { crate::render::node::BlendMode::Normal => 0u8 }).hash(&mut h);
    rn.mask_context.0.hash(&mut h);
    rn.sort_key.hash(&mut h);
    // payload 摘要。
    match &rn.payload {
        NodePayload::Unchanged => 0u64,      // 防御；调用方不应对 Unchanged 调
        NodePayload::Mesh { texture, verts, colors, .. } => {
            texture.hash(&mut h);
            verts.len().hash(&mut h);
            if let Some(c0) = colors.first() { for &v in c0.iter() { v.to_le_bytes().hash(&mut h); } }
            h.finish()
        }
        NodePayload::Text { layout, font_size, color, .. } => {
            font_size.to_le_bytes().hash(&mut h);
            for &v in color.iter() { v.to_le_bytes().hash(&mut h); }
            let glyph_count: usize = layout.lines.iter()
                .map(|l| l.runs.iter().map(|r| r.glyphs.len()).sum::<usize>())
                .sum();
            glyph_count.hash(&mut h);
            // 首 glyph codepoint（捕获内容变）。
            let first_cp: u32 = layout.lines.first()
                .and_then(|l| l.runs.first())
                .and_then(|r| r.glyphs.first())
                .map(|g| g.codepoint)
                .unwrap_or(0);
            first_cp.hash(&mut h);
            h.finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::node::{BlendMode, MaskContext, NodePayload, RenderNode};
    use crate::text::layout::{Glyph, GlyphRun, Line, TextLayout};
    use crate::transform::IDENTITY;

    fn mesh_rn(tex: u32, alpha: f32, color0: [f32;4]) -> RenderNode {
        RenderNode {
            node_id: 0, parent_id: None, visible: true, alpha,
            grayed: false, color_tint: [1.0;4],
            world_matrix: IDENTITY, blend: BlendMode::Normal,
            mask_context: MaskContext(0), sort_key: 0,
            payload: NodePayload::Mesh {
                verts: vec![[0.0,0.0];4], uvs: vec![[0.0,0.0];4],
                colors: vec![color0;4], indices: vec![0,1,2,0,2,3],
                texture: tex, program: 0,
            },
        }
    }

    #[test]
    fn identical_nodes_same_hash() {
        let a = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]);
        let b = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]);
        assert_eq!(node_hash(&a), node_hash(&b), "全等节点 hash 相等");
    }

    #[test]
    fn texture_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let b = mesh_rn(2, 1.0, [1.0;4]);
        assert_ne!(node_hash(&a), node_hash(&b), "texture 变 → hash 变");
    }

    #[test]
    fn color_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]);
        let b = mesh_rn(1, 1.0, [0.0,1.0,0.0,1.0]);
        assert_ne!(node_hash(&a), node_hash(&b), "colors[0] 变 → hash 变");
    }

    #[test]
    fn alpha_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let b = mesh_rn(1, 0.5, [1.0;4]);
        assert_ne!(node_hash(&a), node_hash(&b), "alpha 变 → hash 变");
    }

    #[test]
    fn world_matrix_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let mut b = mesh_rn(1, 1.0, [1.0;4]);
        b.world_matrix = [1.0, 0.0, 0.0, 1.0, 5.0, 0.0]; // tx=5（scroll 平移）
        assert_ne!(node_hash(&a), node_hash(&b), "world_matrix 变（含 scroll_pos）→ hash 变");
    }

    #[test]
    fn verts_len_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]); // 4 verts
        let mut b = mesh_rn(1, 1.0, [1.0;4]);
        if let NodePayload::Mesh { verts, .. } = &mut b.payload {
            verts.push([9.0, 9.0]); // 5 verts（尺寸变）
        }
        assert_ne!(node_hash(&a), node_hash(&b), "verts.len 变 → hash 变");
    }

    #[test]
    fn unchanged_returns_zero() {
        let mut rn = mesh_rn(1, 1.0, [1.0;4]);
        rn.payload = NodePayload::Unchanged;
        assert_eq!(node_hash(&rn), 0, "Unchanged 防御性返回 0");
    }

    // -----------------------------------------------------------------------
    // Text payload hash 变化测试（v1e-T1）
    // 直接构造 TextLayout（不走 measure_text），排除字体 IO 依赖。
    // -----------------------------------------------------------------------

    fn text_rn(font_size: f32, color: [f32; 4], codepoint: u32, glyph_count: usize) -> RenderNode {
        let glyphs: Vec<Glyph> = (0..glyph_count)
            .map(|i| Glyph {
                glyph_id: 1,
                codepoint: codepoint + i as u32,
                x: i as f32 * 10.0,
                y: 0.0,
                bearing_x: 0.0,
                bearing_y: 0.0,
            })
            .collect();
        let layout = TextLayout {
            text_width: 100.0,
            text_height: 20.0,
            lines: vec![Line {
                y: 0.0,
                height: 20.0,
                baseline: 16.0,
                width: 100.0,
                runs: vec![GlyphRun { font_size, glyphs }],
            }],
        };
        RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            world_matrix: IDENTITY,
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Text {
                layout,
                font_size,
                color,
                program: 1,
            },
        }
    }

    #[test]
    fn text_font_size_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(20.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        assert_ne!(node_hash(&a), node_hash(&b), "font_size 变 → hash 变");
    }

    #[test]
    fn text_codepoint_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 66, 3);
        assert_ne!(node_hash(&a), node_hash(&b), "首字 codepoint 变 → hash 变");
    }

    #[test]
    fn text_color_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [0.0, 1.0, 0.0, 1.0], 65, 3);
        assert_ne!(node_hash(&a), node_hash(&b), "color 变 → hash 变");
    }

    #[test]
    fn text_glyph_count_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 5);
        assert_ne!(node_hash(&a), node_hash(&b), "glyph_count 变 → hash 变");
    }

    #[test]
    fn text_identical_same_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        assert_eq!(node_hash(&a), node_hash(&b), "全等 Text hash 相等");
    }
}
