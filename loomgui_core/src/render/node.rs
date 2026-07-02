//! Render 层契约：RenderNode。
//!
//! `build_render_nodes` 遍历 solve 后的 `Scene`，为每个 `Node` 产一个 `RenderNode`；
//! payload 按节点 kind 决定（Container/Button → Mesh quad 背景色；Image → Mesh quad +
//! image_path；Text → measure_text 产 TextLayout）。sort_key / mask_context 由
//! `batch::assign_sort_keys` 后处理。stage 层负责把 `Vec<RenderNode>` diff 成 draw list / JSON。
//!
//! v1.4-a T6：核心不知图集。Mesh payload 带 `image_path: Option<String>`（Image 节点 /
//! bg-image 容器填 path，纯色容器 None）。render 不再查 textures/atlas——path 推给 Unity，
//! Unity 按 path 查 Sprite Atlas 拿 Sprite（含 UV+Texture）。UV 始终全图 (0,0)-(1,1)
//! （Unity Sprite 自带真实 UV；核心无子区概念）。

use serde::Serialize;

/// mask 上下文（rect clip 层级）。
///
/// `MaskContext(0)` = 无 clip；`>0` = clip 层级 id（用出现序作 id）。
/// 由 `batch::assign_sort_keys` 在 BatchingRoot（clip_rect 的 Container）上开新层级，
/// 子树继承。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub struct MaskContext(pub u32);

/// 混合模式。仅 Normal。
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum BlendMode {
    Normal,
}

/// 节点渲染载荷。
///
/// - `Unchanged`：脏标志未置（build_render_nodes 不产出，留作 stage 层 diff 结果）。
/// - `Mesh`：quad 几何（背景色块 / 图片）。`image_path`=None 表示纯色（无贴图），
///   `Some(path)` 为 Image 节点 / bg-image 容器的归一化图片 path（核心不知图集，path 推给
///   Unity 查 Sprite）。UV 始终 (0,0)-(1,1)（Unity Sprite 自带真实 UV；核心无子区）。
///   `program`=0 = 纯色/无图 Image shader，2=Container+bg-image 合成，3=filter 无 bg-image，
///   4=filter+bg-image。
/// - `Text`：measure_text 产 TextLayout + 颜色。`program`=1 = Text shader。
#[derive(Debug, Clone, Serialize)]
pub enum NodePayload {
    Unchanged,
    Mesh {
        verts: Vec<[f32; 2]>,
        uvs: Vec<[f32; 2]>,
        colors: Vec<[f32; 4]>,
        indices: Vec<u32>,
        image_path: Option<String>,  // v1.4-a T6：None=纯色，Some=图片 path（核心不知图集）
        program: u32,
        color_matrix: [f32; 20],   // v1.3 ColorFilter 矩阵；program≠3/4 全零
    },
    Text {
        layout: crate::text::layout::TextLayout,
        font_size: f32,
        color: [f32; 4],
        program: u32,
    },
}

/// 渲染节点（draw list 的最小单元）。
///
/// 字段映射 Node → 渲染语义：
/// - `node_id` / `parent_id`：与 scene.nodes 索引对齐（build 直填 n.id.0）。
/// - `alpha` ← `style.opacity`；`color_tint` ← `style.color`。
/// - `transform.x/y` ← `layout_rect.x/y`（父坐标系）。
/// - `mask_context` / `sort_key`：batch::assign_sort_keys 后填。
#[derive(Debug, Clone, Serialize)]
pub struct RenderNode {
    pub node_id: u32,
    pub parent_id: Option<u32>,
    pub visible: bool,
    pub alpha: f32,
    pub grayed: bool,
    pub color_tint: [f32; 4],
    pub world_matrix: crate::transform::Affine2,
    pub blend: BlendMode,
    pub mask_context: MaskContext,
    pub sort_key: u32,
    pub payload: NodePayload,
}

#[cfg(test)]
mod serde_smoke_tests {
    use super::*;
    #[test]
    fn render_node_serializes_to_json() {
        // 契约：RenderNode 必须能 serde_json::to_string。
        let rn = RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            world_matrix: crate::transform::IDENTITY,
            blend: BlendMode::Normal,
            mask_context: MaskContext(2),
            sort_key: 5,
            payload: NodePayload::Mesh {
                verts: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                image_path: Some("icons/skin.png".into()),
                program: 0,
                color_matrix: [0.0; 20],
            },
        };
        let s = serde_json::to_string(&rn).expect("RenderNode must serialize");
        assert!(s.contains("\"sort_key\":5"));
        assert!(s.contains("\"mask_context\":2"));
        assert!(s.contains("\"image_path\""));
        assert!(s.contains("icons/skin.png"));
        assert!(s.contains("\"Mesh\""));
    }
}
