//! Render 层契约：RenderNode（§8.7）。
//!
//! `build_render_nodes` 遍历 solve 后的 `Scene`，为每个 `Node` 产一个 `RenderNode`；
//! payload 按节点 kind 决定（Container/Button → Mesh quad 背景色；Image → Mesh quad +
//! 占位 tex_id；Text → measure_text 产 TextLayout）。sort_key / mask_context 由
//! `batch::assign_sort_keys` 后处理（§8.5/§8.8）。Task 8 stage 层负责把
//! `Vec<RenderNode>` diff 成 draw list / JSON。

use serde::Serialize;

/// mask 上下文（rect clip 层级）。
///
/// `MaskContext(0)` = 无 clip；`>0` = clip 层级 id（v0 简化：用出现序作 id）。
/// 由 `batch::assign_sort_keys` 在 BatchingRoot（clip_rect 的 Container）上开新层级，
/// 子树继承。§8.5 / §8.8。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub struct MaskContext(pub u32);

/// 混合模式。v0 仅 Normal。
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum BlendMode {
    Normal,
}

/// 节点本地变换（相对父）。v0 仅平移 + 单位 scale（layout_rect 直填 x/y），
/// scale/rotation 留给后续动效。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct NodeTransform {
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
}

impl Default for NodeTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
        }
    }
}

/// 节点渲染载荷。
///
/// - `Unchanged`：脏标志未置（v0 build_render_nodes 不产出，留作 stage 层 diff 结果）。
/// - `Mesh`：quad 几何（背景色块 / 图片）。`texture`=0 表示纯色（无贴图），
///   非 0 为已注册 tex_id（Image 节点，注册表查得；未注册=0 哨兵→白占位）。`program`=0 = Image shader（v0 统一）。
/// - `Text`：measure_text 产 TextLayout + 颜色。`program`=1 = Text shader。
#[derive(Debug, Clone, Serialize)]
pub enum NodePayload {
    Unchanged,
    Mesh {
        verts: Vec<[f32; 2]>,
        uvs: Vec<[f32; 2]>,
        colors: Vec<[f32; 4]>,
        indices: Vec<u32>,
        texture: u32,
        program: u32,
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
/// - `node_id` / `parent_id`：与 scene.nodes 索引对齐（v0 build 直填 n.id.0）。
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
    pub transform: NodeTransform,
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
        // Task 8 契约：RenderNode 必须能 serde_json::to_string。
        let rn = RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            transform: NodeTransform::default(),
            blend: BlendMode::Normal,
            mask_context: MaskContext(2),
            sort_key: 5,
            payload: NodePayload::Mesh {
                verts: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: 7,
                program: 0,
            },
        };
        let s = serde_json::to_string(&rn).expect("RenderNode must serialize");
        assert!(s.contains("\"sort_key\":5"));
        assert!(s.contains("\"mask_context\":2"));
        assert!(s.contains("\"texture\":7"));
        assert!(s.contains("\"Mesh\""));
    }
}
