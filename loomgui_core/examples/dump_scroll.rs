//! 诊断：dump showcase pkg 所有 scroll 容器的 content/viewport/overlap。
//! 验 main-scroll overlap.y > 0（drag 能滚）还是 = 0（content 被 flex shrink）。
use loomgui_core::stage::Stage;

fn main() {
    let font = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
    let pkg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../loomgui_unity/Assets/StreamingAssets/loom_showcase.pkg.bin");
    let pkg = match std::fs::read(pkg_path) {
        Ok(b) => b,
        Err(e) => { eprintln!("read pkg {}: {}", pkg_path, e); return; }
    };
    let mut s = match Stage::new(font, (1080.0, 1920.0)) {
        Ok(s) => s,
        Err(e) => { eprintln!("Stage::new: {}", e); return; }
    };
    if let Err(e) = s.load_package("showcase", &pkg) {
        eprintln!("load_package: {}", e); return;
    }
    s.tick_and_render();
    let scene = s.scene.as_ref().unwrap();
    println!("n_nodes={}", scene.nodes.len());
    // scroll.0 是 HashMap<NodeId, ScrollPaneState>（T3）；按 NodeId 迭代。
    for (id, st) in scene.scroll.0.iter() {
        let n = scene.get(*id).expect("live node for scroll slot");
        let id_attr = n.id_attr.clone().unwrap_or_default();
        println!(
            "node id={:<24} ovf_x={:?} ovf_y={:?} content=({:>6.0},{:>6.0}) viewport=({:>6.0},{:>6.0}) overlap=({:>6.0},{:>6.0})",
            id_attr, n.style.overflow_x, n.style.overflow_y,
            st.content_size.0, st.content_size.1, st.viewport_size.0, st.viewport_size.1,
            st.overlap.0, st.overlap.1,
        );
    }
}
