//! 临时诊断：dump .pkg.bin 的每节点 id_attr/classes/kind/layout，验 popup id 是否进包。
use loomgui_core::asset::read_package;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: cargo run --example dump_pkg -- <pkg.bin>");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let (scene, root_size, atlas) = read_package(&bytes).unwrap_or_else(|e| panic!("read_package: {e:?}"));
    println!("root_size = {:?}", root_size);
    println!("atlas_count = {}", atlas.atlases.len());
    for n in &scene.nodes {
        println!(
            "node[{:?}] parent={:?} id={:?} classes={:?} layout={:?} tabindex={:?}",
            n.id, n.parent, n.id_attr, n.classes, n.layout_rect, n.tabindex
        );
    }
}
