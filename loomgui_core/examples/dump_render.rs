//! 诊断：dump showcase 每节点 payload + style bg + layout_rect，定位"蓝底不显示"。
//! 首帧 prev_hashes 空 → 全 emit（无 Unchanged），看到每节点真实 payload。
//! 用 inline 路径（读 showcase HTML+CSS）验 parse 修复（绕过 pkg 缓存）。
use loomgui_core::render::node::NodePayload;
use loomgui_core::stage::Stage;

fn main() {
    let font = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
    let html_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../samples/v1-showcase/index.html"
    );
    let css_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../samples/v1-showcase/style.css"
    );
    let html = std::fs::read_to_string(html_path).expect("read html");
    let css = std::fs::read_to_string(css_path).expect("read css");
    let mut s = Stage::new(font, (1080.0, 1920.0)).expect("Stage::new");
    s.load_inline(&html, &css).expect("load_inline");
    let frame = s.tick_and_render();
    let scene = s.scene.as_ref().unwrap();
    println!(
        "frame_nodes={} scene_nodes={}",
        frame.nodes.len(),
        scene.nodes.len()
    );
    for rn in &frame.nodes {
        let nid = rn.node_id as usize;
        let n = match scene.get(loomgui_core::scene::node::NodeId(rn.node_id)) {
            Some(n) => n,
            None => {
                // scrollbar thumb sentinel 等
                println!("n{:>3} [sentinel/merged-anchor] {}", nid, payload_str(&rn.payload));
                continue;
            }
        };
        let id = n.id_attr.clone().unwrap_or_default();
        let classes = n.classes.join(",");
        let bg = n.style.background_color;
        let bg_s = match bg {
            Some(c) => format!(
                "bg({:.0},{:.0},{:.0},{:.0})",
                c[0] * 255.0,
                c[1] * 255.0,
                c[2] * 255.0,
                c[3]
            ),
            None => "no-bg".into(),
        };
        let r = n.layout_rect;
        println!(
            "n{:>3} id={:<16} cl={:<20} r=({:>5.0},{:>5.0},{:>4.0},{:>4.0}) {} {} wm({:.0},{:.0})",
            nid,
            id,
            classes,
            r.x,
            r.y,
            r.w,
            r.h,
            payload_str(&rn.payload),
            bg_s,
            rn.world_matrix[4],
            rn.world_matrix[5]
        );
    }
}

fn payload_str(p: &NodePayload) -> String {
    match p {
        NodePayload::Mesh { verts, colors, texture, .. } => {
            let c0 = colors.first().copied().unwrap_or([0.0; 4]);
            format!(
                "Mesh v{} tex{} c0({:.0},{:.0},{:.0},{:.0})",
                verts.len(),
                texture,
                c0[0] * 255.0,
                c0[1] * 255.0,
                c0[2] * 255.0,
                c0[3]
            )
        }
        NodePayload::Text { layout, .. } => format!("Text L{}", layout.lines.len()),
        NodePayload::Unchanged => "Unchanged".into(),
    }
}
