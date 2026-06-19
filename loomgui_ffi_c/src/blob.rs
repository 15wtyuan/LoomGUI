//! 帧 blob 构建器：Vec<RenderNode> → 拍平 SOA blob（§4.1）。
//! mesh 顶点 re-base 到节点本地空间（v0 是父坐标系，减 transform.x/y）。

#[allow(unused_imports)] // BlendMode/MaskContext/NodeTransform 仅测试 helper 经 super::* 用。
use loomgui_core::render::node::{BlendMode, MaskContext, NodePayload, NodeTransform, RenderNode};

/// magic = "LOOM" little-endian。
const MAGIC: u32 = 0x4D4F4F4C;
const VERSION: u32 = 2;

/// 入口：RenderNode 切片 → blob 字节。
pub fn build_blob(nodes: &[RenderNode]) -> Vec<u8> {
    let n = nodes.len();
    // 列名 + 每元素字节数。v2 在 v1 的 11 列上 +text_off/text_len（§4.1）。
    let columns: &[(&str, usize)] = &[
        ("node_id", 4), ("parent_id", 4), ("visible", 1), ("alpha", 4),
        ("sort_key", 4), ("local_x", 4), ("local_y", 4), ("mask_context", 4),
        ("payload_kind", 1), ("mesh_off", 4), ("mesh_len", 4),
        ("text_off", 4), ("text_len", 4),   // v2 新增
    ];
    let num_col_offsets = columns.len();          // 13
    let header_len = 3 * 4                          // magic, version, node_count
        + num_col_offsets * 4                       // 列 offset（13）
        + 2 * 4                                     // mesh_arena off + len
        + 2 * 4                                     // text_arena off + len（v2 新增）
        + 2 * 4;                                    // clip_table off + len（v2 新增）

    // 先把 mesh arena + per-node 列值算出来（mesh arena 决定列值里的 mesh_off/len）。
    let mut mesh_arena: Vec<u8> = Vec::new();
    let mut col_node_id = Vec::<u8>::new();
    let mut col_parent_id = Vec::<u8>::new();
    let mut col_visible = Vec::<u8>::new();
    let mut col_alpha = Vec::<u8>::new();
    let mut col_sort_key = Vec::<u8>::new();
    let mut col_local_x = Vec::<u8>::new();
    let mut col_local_y = Vec::<u8>::new();
    let mut col_mask = Vec::<u8>::new();
    let mut col_kind = Vec::<u8>::new();
    let mut col_mesh_off = Vec::<u8>::new();
    let mut col_mesh_len = Vec::<u8>::new();
    let mut col_text_off = Vec::<u8>::new();   // v2 新增
    let mut col_text_len = Vec::<u8>::new();   // v2 新增

    for rn in nodes {
        col_node_id.extend_from_slice(&rn.node_id.to_le_bytes());
        col_parent_id.extend_from_slice(&rn.parent_id.map(|p| p as i32).unwrap_or(-1).to_le_bytes());
        col_visible.push(rn.visible as u8);
        col_alpha.extend_from_slice(&rn.alpha.to_le_bytes());
        col_sort_key.extend_from_slice(&rn.sort_key.to_le_bytes());
        col_local_x.extend_from_slice(&rn.transform.x.to_le_bytes());
        col_local_y.extend_from_slice(&rn.transform.y.to_le_bytes());
        col_mask.extend_from_slice(&rn.mask_context.0.to_le_bytes());

        // v2：text_off/text_len 每节点都写（T1 暂全 0，T3 给 Text 节点填实）。
        col_text_off.extend_from_slice(&0u32.to_le_bytes());
        col_text_len.extend_from_slice(&0u32.to_le_bytes());

        match &rn.payload {
            NodePayload::Mesh { verts, uvs, colors, indices, .. } => {
                col_kind.push(1);
                // re-base 顶点到本地：减 transform.x/y。
                let (tx, ty) = (rn.transform.x, rn.transform.y);
                let seg_off = mesh_arena.len() as u32;
                mesh_arena.extend_from_slice(&(verts.len() as u32).to_le_bytes());
                mesh_arena.extend_from_slice(&(indices.len() as u32).to_le_bytes());
                for v in verts {
                    mesh_arena.extend_from_slice(&(v[0] - tx).to_le_bytes());
                    mesh_arena.extend_from_slice(&(v[1] - ty).to_le_bytes());
                }
                for u in uvs {
                    mesh_arena.extend_from_slice(&u[0].to_le_bytes());
                    mesh_arena.extend_from_slice(&u[1].to_le_bytes());
                }
                for c in colors {
                    // §4.2b：shader 做 tex2D * v.color。v0 已把 background_color 烤进 mesh
                    // colors（背景色块=bg-color，图片=白）。color_tint(style.color)是前景/文本色，
                    // 不该乘背景——默认黑 color_tint 会把红背景涂黑。仅 × node opacity(alpha)。
                    let o0 = c[0];
                    let o1 = c[1];
                    let o2 = c[2];
                    let o3 = c[3] * rn.alpha;
                    mesh_arena.extend_from_slice(&o0.to_le_bytes());
                    mesh_arena.extend_from_slice(&o1.to_le_bytes());
                    mesh_arena.extend_from_slice(&o2.to_le_bytes());
                    mesh_arena.extend_from_slice(&o3.to_le_bytes());
                }
                for ix in indices {
                    mesh_arena.extend_from_slice(&(*ix as u32).to_le_bytes());
                }
                let seg_len = mesh_arena.len() as u32 - seg_off;
                col_mesh_off.extend_from_slice(&seg_off.to_le_bytes());
                col_mesh_len.extend_from_slice(&seg_len.to_le_bytes());
            }
            NodePayload::Text { .. } => {
                col_kind.push(2);  // T1 暂空 text_arena（T3 填）；C# 仍跳过。
                col_mesh_off.extend_from_slice(&0u32.to_le_bytes());
                col_mesh_len.extend_from_slice(&0u32.to_le_bytes());
            }
            NodePayload::Unchanged => {
                col_kind.push(0);
                col_mesh_off.extend_from_slice(&0u32.to_le_bytes());
                col_mesh_len.extend_from_slice(&0u32.to_le_bytes());
            }
        }
    }

    let col_bufs: Vec<(&str, &Vec<u8>)> = vec![
        ("node_id",&col_node_id),("parent_id",&col_parent_id),("visible",&col_visible),
        ("alpha",&col_alpha),("sort_key",&col_sort_key),("local_x",&col_local_x),
        ("local_y",&col_local_y),("mask_context",&col_mask),("payload_kind",&col_kind),
        ("mesh_off",&col_mesh_off),("mesh_len",&col_mesh_len),
        ("text_off",&col_text_off),("text_len",&col_text_len),
    ];

    // 算各列 offset。
    let mut off = header_len;
    let mut col_offsets: Vec<u32> = Vec::new();
    for (_name, buf) in &col_bufs {
        col_offsets.push(off as u32);
        off += buf.len();
    }
    // 三 arena header offset。text_arena 在 T1 暂空；clip 表 T1 仅 clip_count(u32)=0。
    let mesh_arena_off = off as u32;
    let mesh_arena_len = mesh_arena.len() as u32;
    let text_arena_off = mesh_arena_off + mesh_arena_len;
    let text_arena_len: u32 = 0;   // T1：text_arena 空。T3 填 Text 节点 layout。
    let clip_table_off = text_arena_off + text_arena_len;
    // T1：clip 表只含 clip_count(u32)=0，无 entries。T5 填嵌套交集 entries。
    let clip_count: u32 = 0;
    let clip_table_len: u32 = 4;

    // 拼装。
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    for o in &col_offsets { out.extend_from_slice(&o.to_le_bytes()); }
    out.extend_from_slice(&mesh_arena_off.to_le_bytes());
    out.extend_from_slice(&mesh_arena_len.to_le_bytes());
    out.extend_from_slice(&text_arena_off.to_le_bytes());
    out.extend_from_slice(&text_arena_len.to_le_bytes());
    out.extend_from_slice(&clip_table_off.to_le_bytes());
    out.extend_from_slice(&clip_table_len.to_le_bytes());
    for (_name, buf) in &col_bufs { out.extend_from_slice(buf); }
    out.extend_from_slice(&mesh_arena);
    // text_arena T1 空，跳过。
    // clip 表：仅 clip_count。
    out.extend_from_slice(&clip_count.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_node(id: u32, parent: Option<u32>, x: f32, y: f32, w: f32, h: f32) -> RenderNode {
        RenderNode {
            node_id: id,
            parent_id: parent,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            transform: NodeTransform { x, y, ..NodeTransform::default() },
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: id,
            payload: NodePayload::Mesh {
                // v0 父坐标系顶点：(x,y)(x+w,y)(x+w,y+h)(x,y+h)
                verts: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: 0,
                program: 0,
            },
        }
    }

    /// 同 mesh_node 但可指定 color_tint / alpha / vertex colors（用于 §4.2b tint×alpha 烘焙测试）。
    fn mesh_node_tinted(
        id: u32,
        tint: [f32; 4],
        alpha: f32,
        bg: [f32; 4],
    ) -> RenderNode {
        RenderNode {
            node_id: id,
            parent_id: None,
            visible: true,
            alpha,
            grayed: false,
            color_tint: tint,
            transform: NodeTransform::default(),
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: id,
            payload: NodePayload::Mesh {
                verts: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![bg; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: 0,
                program: 0,
            },
        }
    }

    #[test]
    fn build_blob_has_magic_and_count() {
        let blob = build_blob(&[mesh_node(0, None, 10.0, 20.0, 5.0, 5.0)]);
        assert_eq!(&blob[0..4], &MAGIC.to_le_bytes());
        let v = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        assert_eq!(v, VERSION);
        assert_eq!(v, 2, "v1a Phase 2：blob 版本应为 2");
        let n = u32::from_le_bytes(blob[8..12].try_into().unwrap());
        assert_eq!(n, 1);
    }

    /// §4.1 v2 header：13 col offset + mesh/text/clip 三 arena header。
    /// text_arena 在 T1 暂空（len=0），clip 表 T1 仅 4B clip_count=0。
    #[test]
    fn blob_v2_header_has_text_and_clip_arena_fields() {
        let blob = build_blob(&[mesh_node(0, None, 0.0, 0.0, 1.0, 1.0)]);

        // magic + version==2（已在 build_blob_has_magic_and_count 覆盖，此处再钉一次契约）。
        assert_eq!(u32::from_le_bytes(blob[0..4].try_into().unwrap()), MAGIC);
        assert_eq!(u32::from_le_bytes(blob[4..8].try_into().unwrap()), 2, "version=2");

        // 13 col offset @ [12 .. 12+13*4)。每 col_offset 非零且单调递增。
        let mut prev = 12 + 13 * 4; // header_len（col_offset 从这里起算）
        for i in 0..13usize {
            let o = 12 + i * 4;
            let off = u32::from_le_bytes(blob[o..o + 4].try_into().unwrap()) as usize;
            assert!(off >= prev, "col_offset[{}] 应 >= header_len({}), 实={}", i, prev, off);
            prev = off;
        }

        // mesh_arena header @ [64..72)：off/len（mesh 节点有内容，len>0）。
        let mesh_arena_off = u32::from_le_bytes(blob[64..68].try_into().unwrap()) as usize;
        let mesh_arena_len = u32::from_le_bytes(blob[68..72].try_into().unwrap()) as usize;
        assert!(mesh_arena_len > 0, "单 mesh 节点：mesh_arena_len 应 > 0");

        // text_arena header @ [72..80)：T1 暂空（len=0）。
        let text_arena_off = u32::from_le_bytes(blob[72..76].try_into().unwrap()) as usize;
        let text_arena_len = u32::from_le_bytes(blob[76..80].try_into().unwrap());
        assert_eq!(text_arena_len, 0, "T1: text_arena 暂空（T3 填）");
        assert_eq!(text_arena_off, mesh_arena_off + mesh_arena_len, "text_arena 紧跟 mesh_arena");

        // clip_table header @ [80..88)：T1 仅 4B clip_count=0（clip_table_len=4）。
        let clip_table_off = u32::from_le_bytes(blob[80..84].try_into().unwrap()) as usize;
        let clip_table_len = u32::from_le_bytes(blob[84..88].try_into().unwrap());
        assert_eq!(clip_table_len, 4, "T1: clip 表至少含 clip_count(u32)=0，故 len=4");
        assert_eq!(clip_table_off, text_arena_off + text_arena_len as usize, "clip_table 紧跟 text_arena");
        let clip_count = u32::from_le_bytes(blob[clip_table_off..clip_table_off + 4].try_into().unwrap());
        assert_eq!(clip_count, 0, "T1: clip_count=0（T5 填 entries）");
        assert_eq!(clip_table_off + clip_table_len as usize, blob.len(), "clip_table 应是 blob 末段");
    }

    /// TestView（C# FrameBlob 的 Rust 镜像）解析 v2 blob 时：13 列 + 三 arena 头读回正确，
    /// 且 T1 占位语义（text_off/text_len=0、text_arena_len=0、clip_count=0）成立。
    #[test]
    fn test_view_parses_v2_layout_and_t1_placeholders() {
        let blob = build_blob(&[mesh_node(0, None, 0.0, 0.0, 1.0, 1.0)]);
        let view = TestView::parse(&blob);
        assert_eq!(view.text_off(0), 0, "T1: text_off 占位 0");
        assert_eq!(view.text_len(0), 0, "T1: text_len 占位 0");
        assert_eq!(view.text_arena_len(), 0, "T1: text_arena 整段为空");
        assert_eq!(view.text_arena_off(), view.mesh_arena_off + u32::from_le_bytes(
            blob[68..72].try_into().unwrap()) as usize, "text_arena 紧跟 mesh_arena");
        assert_eq!(view.clip_count(), 0, "T1: clip_count=0");
    }

    #[test]
    fn mesh_verts_are_rebased_to_local() {
        // 顶点原本在 (10,20)..(15,25)；re-base 后应 (0,0)..(5,5)。
        let blob = build_blob(&[mesh_node(0, None, 10.0, 20.0, 5.0, 5.0)]);
        let view = TestView::parse(&blob);
        let verts = view.mesh_verts(0);
        assert_eq!(verts[0], [0.0, 0.0]);
        assert_eq!(verts[2], [5.0, 5.0]);
        // local_x/local_y 保留原 transform（10,20），供 GO localPosition。
        assert_eq!(view.local_x(0), 10.0);
        assert_eq!(view.local_y(0), 20.0);
    }

    #[test]
    fn parent_id_minus_one_for_none() {
        let blob = build_blob(&[mesh_node(0, None, 0.0, 0.0, 1.0, 1.0)]);
        let view = TestView::parse(&blob);
        assert_eq!(view.parent_id(0), -1);
    }

    /// §4.2b：mesh 顶点色 = background_color（**不乘 color_tint**——那是前景/文本色，
    /// 默认黑，乘了会把红背景涂黑），仅 alpha 分量 × node opacity。shader 做 tex2D*v.color。
    /// tint=[0.5,0.5,0.5,1.0]（应被忽略）alpha=0.5 bg=[1,0,0,1]
    /// → 首顶点色 = [1.0, 0.0, 0.0, 1.0×0.5] = [1.0, 0.0, 0.0, 0.5]（红，半透明）。
    #[test]
    fn mesh_colors_bake_alpha_not_tint() {
        let blob = build_blob(&[mesh_node_tinted(
            0,
            [0.5, 0.5, 0.5, 1.0],
            0.5,
            [1.0, 0.0, 0.0, 1.0],
        )]);
        let view = TestView::parse(&blob);
        let colors = view.mesh_colors(0);
        assert_eq!(colors.len(), 4);
        // 首顶点色 = background（rgb 不乘 color_tint），alpha × rn.opacity。
        assert_eq!(colors[0], [1.0, 0.0, 0.0, 0.5]);
        // alpha 列保留原 opacity 值。
        let alpha_o = view.col_off[3];
        let alpha = f32::from_le_bytes(view.buf[alpha_o..alpha_o + 4].try_into().unwrap());
        assert_eq!(alpha, 0.5);
    }

    // —— 测试用解析器（镜像 C# FrameBlob 逻辑，验 Rust 布局正确）——
    // col_off 索引：0=node_id 1=parent_id 2=visible 3=alpha 4=sort_key
    //              5=local_x 6=local_y 7=mask_context 8=payload_kind 9=mesh_off 10=mesh_len
    //              11=text_off 12=text_len   （v2 新增）
    struct TestView<'a> {
        buf: &'a [u8],
        col_off: [usize; 13],
        mesh_arena_off: usize,
        text_arena_off: usize,
        text_arena_len: u32,
        clip_table_off: usize,
        clip_table_len: u32,
    }
    impl<'a> TestView<'a> {
        fn parse(buf: &'a [u8]) -> Self {
            assert_eq!(&buf[0..4], &MAGIC.to_le_bytes());
            let mut col_off = [0usize; 13];
            let mut h = 12;
            for i in 0..13 {
                col_off[i] = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()) as usize;
                h += 4;
            }
            let mesh_arena_off = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()) as usize; h += 4;
            let _mesh_arena_len = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()); h += 4;
            let text_arena_off = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()) as usize; h += 4;
            let text_arena_len = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()); h += 4;
            let clip_table_off = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()) as usize; h += 4;
            let clip_table_len = u32::from_le_bytes(buf[h..h+4].try_into().unwrap());
            TestView { buf, col_off, mesh_arena_off, text_arena_off, text_arena_len, clip_table_off, clip_table_len }
        }
        fn parent_id(&self, i: usize) -> i32 {
            let o = self.col_off[1] + i * 4;
            i32::from_le_bytes(self.buf[o..o+4].try_into().unwrap())
        }
        fn local_x(&self, i: usize) -> f32 {
            let o = self.col_off[5] + i * 4;
            f32::from_le_bytes(self.buf[o..o+4].try_into().unwrap())
        }
        fn local_y(&self, i: usize) -> f32 {
            let o = self.col_off[6] + i * 4;
            f32::from_le_bytes(self.buf[o..o+4].try_into().unwrap())
        }
        /// 读节点 i 的 mesh 顶点（arena 段：vert_count, idx_count, verts[], uvs[], colors[], indices[]）。
        fn mesh_verts(&self, i: usize) -> Vec<[f32; 2]> {
            let (seg, vc) = self.mesh_seg(i);
            let mut p = seg + 8; // 跳 vert_count + idx_count，直接读 verts[]
            (0..vc).map(|_| {
                let vx = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                let vy = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                [vx, vy]
            }).collect()
        }
        /// 读节点 i 的 mesh 顶点色（§4.2b：已 baked color_tint×alpha）。
        fn mesh_colors(&self, i: usize) -> Vec<[f32; 4]> {
            let (seg, vc) = self.mesh_seg(i);
            let mut p = seg + 8;
            // verts + uvs 各 vc*2 f32。
            p += vc * 2 * 4 * 2;
            (0..vc).map(|_| {
                let r = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                let g = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                let b = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                let a = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                [r, g, b, a]
            }).collect()
        }
        /// 返回节点 i 的 mesh 段起始偏移 + vert_count。
        fn mesh_seg(&self, i: usize) -> (usize, usize) {
            let seg = self.mesh_arena_off + u32::from_le_bytes(
                self.buf[self.col_off[9] + i * 4..][0..4].try_into().unwrap()) as usize; // mesh_off
            let vc = u32::from_le_bytes(self.buf[seg..seg + 4].try_into().unwrap()) as usize;
            (seg, vc)
        }
        // v2 访问器（T3/T5 将扩展内容，T1 仅验布局占位）。
        fn text_off(&self, i: usize) -> u32 {
            u32::from_le_bytes(self.buf[self.col_off[11] + i * 4..][0..4].try_into().unwrap())
        }
        fn text_len(&self, i: usize) -> u32 {
            u32::from_le_bytes(self.buf[self.col_off[12] + i * 4..][0..4].try_into().unwrap())
        }
        fn text_arena_len(&self) -> u32 { self.text_arena_len }
        fn text_arena_off(&self) -> usize { self.text_arena_off }
        fn clip_count(&self) -> u32 {
            if self.clip_table_len >= 4 {
                u32::from_le_bytes(self.buf[self.clip_table_off..self.clip_table_off + 4].try_into().unwrap())
            } else {
                0
            }
        }
    }
}
