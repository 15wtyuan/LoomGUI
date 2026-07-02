use loomgui_core::asset::{read_package, PKG_MAGIC};
use loomgui_pkg::pack;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// 建临时 source_dir：写每个 html_name（无扩展名）对应 `<name>.html`，res 下写空 png 占位文件
/// （pack 不读像素，只记 path；空文件够测归一化进 manifest）。
/// res_files 形如 `["res/x.png", "res/icons/skin.png"]` —— 相对 source_dir。
fn make_tmp_dir_with_html(html_names: &[&str], res_files: &[&str]) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "loomgui_pkg_t3_{}_{}",
        std::process::id(),
        seq
    ));
    fs::create_dir_all(&dir).unwrap();
    for name in html_names {
        // 每个 HTML 一个 <div> 组件根（围栏内 tag），保证 parse_html 通过。
        let html = format!(r#"<div class="{}"><span>hi</span></div>"#, name);
        fs::write(dir.join(format!("{name}.html")), html).unwrap();
    }
    for rf in res_files {
        let p = dir.join(rf);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        // 空占位文件（pack 不解码像素，T3 砍了 image crate）
        fs::write(&p, b"").unwrap();
    }
    dir
}

#[test]
fn pack_multi_html_no_atlas() {
    // tmp 目录：a.html + b.html + res/x.png（不读 x.png 像素，只记 path）
    // pack 后：pkg_bytes 非空，atlas 字段已砍（PackedPackage 只剩 pkg_bytes + asset_manifest）
    // read_package(pkg_bytes).components 含 "a"、"b"
    // asset_manifest 含归一化 path（"x.png"）
    let dir = make_tmp_dir_with_html(&["a", "b"], &["res/x.png"]);
    // a.html 引用 res/x.png（img src），让 manifest 收到归一化 path
    fs::write(
        dir.join("a.html"),
        r#"<div class="a"><img src="res/x.png"></div>"#,
    )
    .unwrap();
    let packed = pack(
        &dir,
        "test",
        &["a.html".to_string(), "b.html".to_string()],
        "res",
    )
    .expect("pack ok");
    assert!(!packed.pkg_bytes.is_empty(), "pkg_bytes 非空");
    // atlas 字段已砍：PackedPackage 只有 pkg_bytes + asset_manifest
    assert!(packed.asset_manifest.contains(&"x.png".to_string()));
    assert_eq!(
        u32::from_le_bytes(packed.pkg_bytes[0..4].try_into().unwrap()),
        PKG_MAGIC
    );
    let pkg = read_package(&packed.pkg_bytes).expect("read ok");
    assert_eq!(pkg.components.len(), 2, "两 HTML → 两组件");
    assert!(pkg.components.contains_key("a"), "组件名 a（文件名去 .html）");
    assert!(pkg.components.contains_key("b"), "组件名 b");
    assert!(
        pkg.asset_manifest.contains(&"x.png".to_string()),
        "manifest 含归一化 path x.png（去 res 前缀）"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pack_path_normalization_in_manifest() {
    // HTML img src="res/icons/skin.png" → manifest 含 "icons/skin.png"
    // 断言归一化生效，不含 res/ 前缀
    let dir = make_tmp_dir_with_html(&["c"], &["res/icons/skin.png"]);
    fs::write(
        dir.join("c.html"),
        r#"<div class="c"><img src="res/icons/skin.png"></div>"#,
    )
    .unwrap();
    let packed = pack(
        &dir,
        "test",
        &["c.html".to_string()],
        "res",
    )
    .expect("pack ok");
    assert!(
        packed.asset_manifest.contains(&"icons/skin.png".to_string()),
        "归一化后 manifest 含 icons/skin.png，不含 res/ 前缀"
    );
    assert!(
        !packed.asset_manifest.iter().any(|p| p.contains("res/")),
        "manifest 不应含 res/ 前缀 path: {:?}",
        packed.asset_manifest
    );
    let pkg = read_package(&packed.pkg_bytes).expect("read ok");
    assert!(
        pkg.asset_manifest.contains(&"icons/skin.png".to_string()),
        "read_package 后 manifest 仍含归一化 path"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// v1.4-a：pkg 不再带 atlas（图集归 Unity，D8）。原 atlas section 测试已删（T3 砍 atlas）。
/// pack_produces_valid_package_roundtrips 旧签名 pack(html,css,root_size,res_dir) 已改 pack(source_dir,pkg_name,html_files,res_dir)。
#[test]
fn pack_single_html_roundtrips() {
    // 无图单 HTML：pack 产 v1.4-a 单组件 pkg，read_package round-trip。
    let dir = make_tmp_dir_with_html(&["scene"], &[]);
    let packed = pack(&dir, "test", &["scene.html".to_string()], "res").expect("pack ok");
    assert!(!packed.pkg_bytes.is_empty());
    let pkg = read_package(&packed.pkg_bytes).expect("read ok");
    assert_eq!(pkg.components.len(), 1);
    let comp = pkg.components.values().next().unwrap();
    assert!(!comp.nodes.is_empty());
    let has_text = comp.nodes.iter().any(|n| matches!(&n.kind,
        loomgui_core::scene::NodeKind::Text { content } if content == "hi"));
    assert!(has_text, "scene.html 的 span 文本 hi 应在节点树");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pack_missing_html_file_errors() {
    // html_files 指向不存在的文件 → Err（build-time fail）
    let dir = make_tmp_dir_with_html(&["a"], &[]);
    let r = pack(&dir, "test", &["nope.html".to_string()], "res");
    assert!(r.is_err(), "缺 HTML 文件应 Err");
    let _ = fs::remove_dir_all(&dir);
}
