//! T10 验证：读 loom_showcase.pkg.bin 回读，打印组件名 + manifest path/尺寸。
//! 用法：cargo run -p loomgui_core --example verify_showcase_pkg -- <path-to-pkg.bin>
use loomgui_core::asset::read_package;
use std::env;

fn main() {
    let path = env::args().nth(1).expect("usage: verify_showcase_pkg <pkg.bin>");
    let bytes = std::fs::read(&path).expect("read pkg.bin");
    let pkg = read_package(&bytes).expect("read_package");
    println!("package name: {:?}", pkg.name);
    println!("component count: {}", pkg.components.len());
    let mut names: Vec<&String> = pkg.components.keys().collect();
    names.sort();
    for n in &names {
        let c = &pkg.components[*n];
        println!("  - {:<20} nodes={:<4} dyn_rules={}", c.name, c.nodes.len(), c.dynamic_rules.rules.len());
    }
    println!("asset_manifest ({} paths):", pkg.asset_manifest.len());
    for e in &pkg.asset_manifest {
        println!("  - {:<24} {}x{}", e.path, e.w, e.h);
    }
    // 校验根节点 parent_idx=None
    let mut bad = 0;
    for n in &names {
        let c = &pkg.components[*n];
        if c.nodes.first().map(|n| n.parent_idx).flatten().is_some() {
            println!("ERROR: component {} root has parent_idx != None", n);
            bad += 1;
        }
    }
    if bad == 0 {
        println!("OK: all component roots have parent_idx=None");
    }
}
