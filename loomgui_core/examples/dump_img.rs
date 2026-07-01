//! 诊断：dump 所有 Image 节点的 CSS size / layout_rect / 纹理注册情况。
//! 定位 ① width:50% 压扁（高没等比）② width:auto 不渲染（rect 0? 未注册?）。
use loomgui_core::scene::node::NodeKind;
use loomgui_core::stage::Stage;

fn main() {
    let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/wqy-microhei.ttc");
    let pkg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../loomgui_unity/Assets/StreamingAssets/loom_showcase.pkg.bin");
    let pkg = match std::fs::read(pkg_path) {
        Ok(b) => b,
        Err(e) => { eprintln!("read pkg: {}", e); return; }
    };
    let mut s = match Stage::new(font_path, (1080.0, 1920.0)) {
        Ok(s) => s,
        Err(e) => { eprintln!("Stage::new: {}", e); return; }
    };
    if let Err(e) = s.load_package(&pkg) { eprintln!("load_package: {}", e); return; }
    s.tick_and_render();
    let scene = s.scene.as_ref().unwrap();
    println!("{:<22} {:<18} {:<16} {:<16} {:>8} {:>8} {:>10}", "id", "src", "css.w", "css.h", "rect.w", "rect.h", "tex(iw,ih)");
    for n in scene.nodes.values() {
        let src = match &n.kind { NodeKind::Image { src } => src.clone(), _ => continue };
        let st = &n.style.taffy_style;
        let css_w = format!("{:?}", st.size.width);
        let css_h = format!("{:?}", st.size.height);
        let tex = s.textures.get(&src);
        let tex_s = tex.map(|m| format!("({},{})", m.width, m.height)).unwrap_or_else(|| "UNREGISTERED".into());
        let id = n.id_attr.clone().unwrap_or_default();
        println!(
            "{:<22} {:<18} {:<16} {:<16} {:>8.1} {:>8.1} {:>10}",
            id, src, css_w, css_h, n.layout_rect.w, n.layout_rect.h, tex_s,
        );
    }
}
