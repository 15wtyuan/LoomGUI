//! 诊断：dump 所有 Image 节点的 CSS size / layout_rect / 纹理注册情况。
//! 定位 ① width:50% 压扁（高没等比）② width:auto 不渲染（rect 0? 未注册?）。
//!
//! v1.4-a T4：load_package 不再建 scene（进资源池）。本 example 暂只验 load_package 成功 +
//! 包内 Image 节点 src（从 packages 字典读，非 scene）。T5 instantiate 后恢复 scene 转储。
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
    // v1.4-a T4：load_package(name, bytes) 进资源池不建 scene。
    if let Err(e) = s.load_package("showcase", &pkg) { eprintln!("load_package: {}", e); return; }
    // v1.4-a T4：scene 未建（load_package 不建 scene）；从 packages 字典读组件模板节点。
    // T5 instantiate 后改回 scene 转储。
    println!("{:<22} {:<18} {:<16} {:<16}", "id", "src", "css.w", "css.h");
    for pkg in s.packages.values() {
        for comp in pkg.components.values() {
            for n in &comp.nodes {
                let src = match &n.kind { NodeKind::Image { src } => src.clone(), _ => continue };
                let st = &n.style.taffy_style;
                let css_w = format!("{:?}", st.size.width);
                let css_h = format!("{:?}", st.size.height);
                let id = n.id_attr.clone().unwrap_or_default();
                println!("{:<22} {:<18} {:<16} {:<16}", id, src, css_w, css_h);
            }
        }
    }
}
