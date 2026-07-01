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
    if let Err(e) = s.load_package(&pkg) {
        eprintln!("load_package: {}", e); return;
    }
    s.tick_and_render();
    let scene = s.scene.as_ref().unwrap();
    println!("n_nodes={}", scene.nodes.len());
    for (i, st) in scene.scroll.0.iter().enumerate() {
        if let Some(st) = st {
            // scroll.0 按 NodeId::index() 索引（slotmap idx，从 1 起）；用 idx 定位节点。
            let n = scene
                .nodes
                .values()
                .find(|n| n.id.index() == i)
                .expect("live node for scroll slot");
            let id = n.id_attr.clone().unwrap_or_default();
            println!(
                "node{:>3} id={:<24} ovf_x={:?} ovf_y={:?} content=({:>6.0},{:>6.0}) viewport=({:>6.0},{:>6.0}) overlap=({:>6.0},{:>6.0})",
                i, id, n.style.overflow_x, n.style.overflow_y,
                st.content_size.0, st.content_size.1, st.viewport_size.0, st.viewport_size.1,
                st.overlap.0, st.overlap.1,
            );
        }
    }
}
