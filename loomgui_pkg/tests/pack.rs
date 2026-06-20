use loomgui_core::asset::{read_package, PKG_MAGIC};
use loomgui_pkg::pack;

#[test]
fn pack_produces_valid_package_roundtrips() {
    // 无图 scene：pack 仍产 v2 pkg（空 atlas），不需 res_dir 有图。
    let html = r#"<div class="c"><span>hi</span></div>"#;
    let css = ".c{width:200px;height:100px;background-color:#ff0000;}";
    let res_dir = std::path::Path::new(".");
    let p = pack(html, css, (200.0, 100.0), res_dir).expect("pack ok");
    assert_eq!(
        u32::from_le_bytes(p.pkg_bytes[0..4].try_into().unwrap()),
        PKG_MAGIC
    );
    let (scene, rs, atlas) = read_package(&p.pkg_bytes).expect("read ok");
    assert_eq!(rs, (200.0, 100.0));
    assert!(atlas.atlases.is_empty(), "无图 → 空 atlas");
    assert!(p.atlas_png.is_empty(), "无图 → atlas_png 空");
    assert!(p.atlas_filename.is_empty());
    assert!(scene.roots.len() >= 1);
    let has_text = scene.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Text { content } if content == "hi"));
    assert!(has_text);
}

#[test]
fn pack_with_images_builds_atlas_and_section() {
    let html = r#"<div><img src="a.png"><img src="b.png"></div>"#;
    let css = "";
    let res_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let p = pack(html, css, (100.0, 100.0), &res_dir).expect("pack ok");
    assert!(!p.atlas_png.is_empty(), "有图 → atlas 非空");
    assert_eq!(p.atlas_filename, "loom.atlas.png");

    // pkg.bin round-trip：atlas 表有 1 atlas / 2 sprites。
    let (_scene, _rs, atlas) = read_package(&p.pkg_bytes).unwrap();
    assert_eq!(atlas.atlases.len(), 1);
    assert_eq!(atlas.atlases[0].filename, "loom.atlas.png");
    assert_eq!(atlas.sprites.len(), 2);

    // PNG round-trip：decode atlas，验 a.png 的 region 左上像素 == 红。
    let atlas_img = image::load_from_memory(&p.atlas_png).unwrap().to_rgba8();
    let a = atlas
        .sprites
        .iter()
        .find(|s| s.src == "a.png")
        .expect("a.png sprite 在 atlas 中");
    assert_eq!((a.w, a.h), (4, 4));
    let px = atlas_img.get_pixel(a.x, a.y).0;
    assert_eq!(px, [255, 0, 0, 255], "a.png region 左上像素应红");

    let b = atlas
        .sprites
        .iter()
        .find(|s| s.src == "b.png")
        .expect("b.png sprite 在 atlas 中");
    assert_eq!((b.w, b.h), (2, 2));
    let px_b = atlas_img.get_pixel(b.x, b.y).0;
    assert_eq!(px_b, [0, 255, 0, 255], "b.png region 左上像素应绿");
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
