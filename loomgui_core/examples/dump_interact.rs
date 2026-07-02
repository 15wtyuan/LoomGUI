//! 诊断：button :active{transform:scale(0.96)} 按下后 Text 子字消失。
//! 打包伪类 → load → tick → Down → tick×2 → dump btn + Text payload + world_matrix。
use loomgui_core::asset::{extract_dynamic_rules, write_package, PackageInput, TemplateNode};
use loomgui_core::input::{PointerEvent, PointerKind};
use loomgui_core::parse::css::parse_css;
use loomgui_core::render::node::NodePayload;
use loomgui_core::scene::node::{build_scene, NodeId};
use loomgui_core::stage::Stage;
use loomgui_core::style::cascade::resolve_styles;

fn dump(stage: &Stage, label: &str, focus: u32) {
    let scene = stage.scene.as_ref().unwrap();
    let focus_nid = loomgui_core::scene::node::NodeId(focus);
    // 取最近 tick 的 frame：重跑 build_render_nodes 不现实；改读 scene 节点状态 + prev_hashes
    println!("=== {} ===", label);
    for (i, n) in scene.nodes.values().enumerate() {
        if n.id == focus_nid || n.parent == Some(focus_nid) {
            let wi = n.id.index();
            println!(
                "  n{} kind={:?} active={} hovered={} rect=({:.0},{:.0},{:.0},{:.0}) bg={:?} wm={:?}",
                i,
                n.kind,
                n.active,
                n.hovered,
                n.layout_rect.x,
                n.layout_rect.y,
                n.layout_rect.w,
                n.layout_rect.h,
                n.style.background_color,
                scene.world_transforms.get(wi).copied().unwrap_or_default()
            );
        }
    }
}

fn dump_frame(stage: &mut Stage, label: &str, focus: u32) {
    let frame = stage.tick_and_render();
    println!("=== {} (frame payload) ===", label);
    let scene = stage.scene.as_ref().unwrap();
    let focus_nid = loomgui_core::scene::node::NodeId(focus);
    for rn in &frame.nodes {
        let nid = rn.node_id as usize;
        let is_focus_or_child = if rn.node_id == focus {
            true
        } else {
            scene
                .get(loomgui_core::scene::node::NodeId(rn.node_id))
                .map(|n| n.parent == Some(focus_nid))
                .unwrap_or(false)
        };
        if is_focus_or_child {
            let pk = match &rn.payload {
                NodePayload::Mesh { verts, colors, .. } => {
                    let c0 = colors.first().copied().unwrap_or([0.0; 4]);
                    format!("Mesh v{} c0({:.0},{:.0},{:.0},{:.0})", verts.len(), c0[0] * 255.0, c1(c0), c2(c0), c3(c0))
                }
                NodePayload::Text { layout, .. } => format!("Text L{}", layout.lines.len()),
                NodePayload::Unchanged => "Unchanged".into(),
            };
            println!("  n{} wm=({:.2},{:.2},{:.2},{:.2},{:.0},{:.0}) {}", nid,
                rn.world_matrix[0], rn.world_matrix[1], rn.world_matrix[2], rn.world_matrix[3],
                rn.world_matrix[4], rn.world_matrix[5], pk);
        }
    }
    let _ = scene;
}

fn c1(c: [f32; 4]) -> f32 { c[1] * 255.0 }
fn c2(c: [f32; 4]) -> f32 { c[2] * 255.0 }
fn c3(c: [f32; 4]) -> f32 { c[3] }

fn main() {
    let font = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
    let html = r#"<div class="root"><div class="btn" id="b1"><span>OK</span></div></div>"#;
    let css = r#".btn{width:100px;height:50px;background-color:#cccccc;} .btn:active{transform:scale(0.96);background-color:#5fb2c4;}"#;
    let tree = loomgui_core::parse::dom::parse_html(html).unwrap();
    let sheet = parse_css(css).unwrap();
    let styles = resolve_styles(&tree, &sheet);
    let scene = build_scene(&tree, &styles);
    let dynamic = extract_dynamic_rules(&sheet);
    // v1.4-a：write_package 接 PackageInput（多组件）。本 example 单 scene → 单组件桥接。
    let pos_of: std::collections::HashMap<NodeId, usize> = scene
        .nodes
        .values()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let nodes: Vec<TemplateNode> = scene
        .nodes
        .values()
        .map(|n| TemplateNode {
            kind: n.kind.clone(),
            style: n.style.clone(),
            parent_idx: n.parent.map(|p| pos_of[&p]),
            classes: n.classes.clone(),
            id_attr: n.id_attr.clone(),
            draggable: n.draggable,
            tabindex: n.tabindex,
        })
        .collect();
    let input = PackageInput {
        components: vec![("scene", nodes.as_slice(), &dynamic)],
        asset_manifest: &[],
    };
    let pkg = write_package(&input);

    let mut s = Stage::new(font, (200.0, 100.0)).unwrap();
    s.load_package("showcase", &pkg).unwrap();
    let btn = s.find_node_by_id("b1").expect("b1").0 as u32;
    let r = s.scene.as_ref().unwrap().get(loomgui_core::scene::node::NodeId(btn)).expect("live node").layout_rect;
    println!("btn={} rect=({:.0},{:.0},{:.0},{:.0})", btn, r.x, r.y, r.w, r.h);

    dump_frame(&mut s, "frame1 (首帧)", btn);
    dump(&s, "scene after frame1", btn);

    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    s.set_input(&[PointerEvent { kind: PointerKind::Down, x: cx, y: cy, button: 0, pad: [0, 0], touch_id: -1 }]);
    dump_frame(&mut s, "frame2 (Down 本帧)", btn);

    s.set_input(&[PointerEvent { kind: PointerKind::Down, x: cx, y: cy, button: 0, pad: [0, 0], touch_id: -1 }]);
    dump_frame(&mut s, "frame3 (Down 次帧 transform 进 world)", btn);
    dump(&s, "scene after frame3", btn);

    // 松手（Up）→ active 解除 → transform 回 identity → Text 切回 pure
    s.set_input(&[PointerEvent { kind: PointerKind::Up, x: cx, y: cy, button: 0, pad: [0, 0], touch_id: -1 }]);
    dump_frame(&mut s, "frame4 (Up 当帧)", btn);
    s.set_input(&[]);
    dump_frame(&mut s, "frame5 (Up 次帧 transform 解除→pure)", btn);
    dump(&s, "scene after frame5", btn);
}
