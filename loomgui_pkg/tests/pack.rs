use loomgui_core::asset::{read_package, PKG_MAGIC};
use loomgui_pkg::pack;

#[test]
fn pack_produces_valid_package_roundtrips() {
    // 无图 scene：pack 产 v1.4-a 单组件 pkg（T1 桥接：scene→单组件 PackageInput）。
    let html = r#"<div class="c"><span>hi</span></div>"#;
    let css = ".c{width:200px;height:100px;background-color:#ff0000;}";
    let res_dir = std::path::Path::new(".");
    let p = pack(html, css, (200.0, 100.0), res_dir).expect("pack ok");
    assert_eq!(
        u32::from_le_bytes(p.pkg_bytes[0..4].try_into().unwrap()),
        PKG_MAGIC
    );
    // v1.4-a：read_package 返 Package（不再有 root_size/atlas tuple）。
    let pkg = read_package(&p.pkg_bytes).expect("read ok");
    assert!(p.atlas_png.is_empty(), "无图 → atlas_png 空");
    assert!(p.atlas_filename.is_empty());
    // 单组件包：T1 桥接用组件名 "scene"
    assert_eq!(pkg.components.len(), 1);
    let comp = pkg.components.values().next().unwrap();
    assert!(!comp.nodes.is_empty());
    let has_text = comp.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Text { content } if content == "hi"));
    assert!(has_text);
}

/// v1.4-a：pkg 不再带 atlas（图集归 Unity，D8）。本测验证的"pkg→atlas section"链路已断，
/// atlas 数据仅在 atlas_png 产物（T3 重写打包器时砍 atlas.png 改 path 归一化）。
/// **ignore**：Task 3 重写打包器后本测改为校验 asset_manifest（path 列表）。
#[test]
#[ignore = "v1.4-a: pkg 不带 atlas；Task 3 重写打包器后改测 asset_manifest"]
fn pack_with_images_builds_atlas_and_section() {
    // 占位：原验证 pkg 带 atlas section + atlas.png round-trip。
    // 新格式 atlas 不进 pkg；atlas.png 仍由 pack 产出（过渡期），但 pkg 内无 atlas 坐标。
}

#[test]
fn pack_missing_image_fails() {
    let html = r#"<div><img src="nope.png"></div>"#;
    let res_dir = std::path::Path::new(".");
    let r = pack(html, "", (10.0, 10.0), res_dir);
    assert!(r.is_err(), "缺图 build-time fail");
    let msg = r.unwrap_err();
    assert!(
        msg.contains("nope.png"),
        "错误消息应含 src 名: {msg}"
    );
}
