//! 诊断 v1d.4：对比 inline vs pkg 路径，dump popup world_matrix / anim.transform / merge 状态。
//! 验 pkg 路径 anim 是否进 world_matrix（Unity 用 pkg，example 之前只测 inline）。
use loomgui_core::stage::Stage;
use loomgui_core::transform::is_pure_translation;
use loomgui_core::tween::{Ease, TweenProp};

fn run(label: &str, mut s: Stage) {
    let popup = s.find_node_by_id("popup").expect("popup");
    println!("=== {label} === popup={popup:?} nodes={}", s.scene.as_ref().unwrap().nodes.len());
    s.tween(popup, TweenProp::Scale, [0.8, 0.8, 0.0, 0.0], [1.0, 1.0, 0.0, 0.0], Ease::BackOut, 0.0, 1.5, 2);
    for i in 0..3 {
        s.advance_time(0.15);
        let frame = s.tick_and_render();
        let scene = s.scene.as_ref().unwrap();
        let wm = scene.world_transforms.get(popup.0).cloned().unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let anim_tr = scene.anim.0.get(popup.0).and_then(|a| a.transform).is_some();
        let rn_found = frame.nodes.iter().any(|n| n.node_id == popup.0 as u32);
        println!(
            "{label} f{i} world=[{:.3},{:.3},{:.3},{:.3},{:.1},{:.1}] anim.transform={} rn_found={} pure={} (frame.nodes={})",
            wm[0], wm[1], wm[2], wm[3], wm[4], wm[5], anim_tr, rn_found, is_pure_translation(&wm), frame.nodes.len(),
        );
    }
}

fn main() {
    let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
    let html = r#"<div class="root"><div class="popup" id="popup">弹窗</div></div>"#;
    let css = ".root{width:1080px;height:1920px;} .popup{width:400px;height:240px;background-color:#2a6cc9;margin:400px auto 0;}";

    let mut s_inline = Stage::new(font_path, (1080.0, 1920.0)).unwrap();
    s_inline.load_inline(html, css).unwrap();
    run("inline", s_inline);

    let pkg_path = format!("{}/../loomgui_unity/Assets/StreamingAssets/loom_tween.pkg.bin", env!("CARGO_MANIFEST_DIR"));
    let pkg = std::fs::read(&pkg_path).expect("pkg.bin not found");
    let mut s_pkg = Stage::new(font_path, (1080.0, 1920.0)).unwrap();
    s_pkg.load_package(&pkg).unwrap();
    run("pkg   ", s_pkg);
}
