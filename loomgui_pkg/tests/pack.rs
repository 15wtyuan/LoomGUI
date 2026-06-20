use loomgui_core::asset::{read_package, PKG_MAGIC};
use loomgui_pkg::pack;

#[test]
fn pack_produces_valid_package_roundtrips() {
    let html = r#"<div class="c"><span>hi</span><img src="logo.png"></div>"#;
    let css = ".c{width:200px;height:100px;background-color:#ff0000;}";
    let bytes = pack(html, css, (200.0, 100.0)).expect("pack ok");

    // magic 头
    let m = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    assert_eq!(m, PKG_MAGIC);

    // round-trip：read_package 能读回，且结构对
    let (scene, rs, _atlas) = read_package(&bytes).expect("read ok");
    assert_eq!(rs, (200.0, 100.0));
    assert!(scene.roots.len() >= 1);
    // 至少有一个 Text(content="hi") 和一个 Image(src="logo.png")
    let has_text = scene.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Text { content } if content == "hi"));
    let has_img = scene.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Image { src } if src == "logo.png"));
    assert!(has_text && has_img);
}
