use loomgui_core::asset::{read_package, AssetEntry, PKG_MAGIC};
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

/// 写一个最小合法 PNG header（w×h IHDR）到 path。用于 D17 测 read_png_size 取真实尺寸。
/// 只写 magic + IHDR chunk（read_png_size 只读前 24 字节，不需 IDAT/IEND）。
/// PNG = magic(8) + IHDR chunk: length(4 BE)=13 + "IHDR" + data(13) + CRC(4, 写 0 不校验)。
/// IHDR data: w(4 BE) + h(4 BE) + bit_depth(1)=8 + color_type(1)=2(RGB) + compression(1)=0
///            + filter(1)=0 + interlace(1)=0。
fn write_minimal_png(path: &std::path::Path, w: u32, h: u32) {
    use std::io::Write;
    let mut buf: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr_data: Vec<u8> = [
        &w.to_be_bytes()[..],
        &h.to_be_bytes()[..],
        &[8u8, 2u8, 0u8, 0u8, 0u8],
    ]
    .concat();
    buf.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
    buf.extend_from_slice(b"IHDR");
    buf.extend_from_slice(&ihdr_data);
    buf.extend_from_slice(&[0u8; 4]); // CRC（read_png_size 不校验）
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&buf).unwrap();
}

#[test]
fn pack_multi_html_no_atlas() {
    // tmp 目录：a.html + b.html + res/x.png（不读 x.png 像素，只记 path）
    // pack 后：pkg_bytes 非空，atlas 字段已砍（PackedPackage 只剩 pkg_bytes + asset_manifest）
    // read_package(pkg_bytes).components 含 "a"、"b"
    // asset_manifest 含归一化 path（"x.png"）。D17：空文件 → w/h=0（非 PNG）。
    let dir = make_tmp_dir_with_html(&["a", "b"], &["res/x.png"]);
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
    assert!(
        packed.asset_manifest.iter().any(|e| e.path == "x.png"),
        "manifest 含归一化 path x.png"
    );
    assert_eq!(
        u32::from_le_bytes(packed.pkg_bytes[0..4].try_into().unwrap()),
        PKG_MAGIC
    );
    let pkg = read_package(&packed.pkg_bytes).expect("read ok");
    assert_eq!(pkg.components.len(), 2, "两 HTML → 两组件");
    assert!(pkg.components.contains_key("a"), "组件名 a（文件名去 .html）");
    assert!(pkg.components.contains_key("b"), "组件名 b");
    assert!(
        pkg.asset_manifest.iter().any(|e| e.path == "x.png"),
        "manifest 含归一化 path x.png（去 res 前缀）"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pack_path_normalization_in_manifest() {
    // HTML img src="res/icons/skin.png" → manifest 含 "icons/skin.png"
    let dir = make_tmp_dir_with_html(&["c"], &["res/icons/skin.png"]);
    fs::write(
        dir.join("c.html"),
        r#"<div class="c"><img src="res/icons/skin.png"></div>"#,
    )
    .unwrap();
    let packed = pack(&dir, "test", &["c.html".to_string()], "res").expect("pack ok");
    assert!(
        packed.asset_manifest.iter().any(|e| e.path == "icons/skin.png"),
        "归一化后 manifest 含 icons/skin.png，不含 res 前缀"
    );
    assert!(
        !packed.asset_manifest.iter().any(|e| e.path.contains("res/")),
        "manifest 不应含 res/ 前缀 path: {:?}",
        packed.asset_manifest
    );
    let pkg = read_package(&packed.pkg_bytes).expect("read ok");
    assert!(
        pkg.asset_manifest.iter().any(|e| e.path == "icons/skin.png"),
        "read_package 后 manifest 仍含归一化 path"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// v1.4-a：pkg 不再带 atlas（图集归 Unity，D8）。原 atlas section 测试已删（T3 砍 atlas）。
#[test]
fn pack_single_html_roundtrips() {
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
    let dir = make_tmp_dir_with_html(&["a"], &[]);
    let r = pack(&dir, "test", &["nope.html".to_string()], "res");
    assert!(r.is_err(), "缺 HTML 文件应 Err");
    let _ = fs::remove_dir_all(&dir);
}

// ── D17：打包期 PNG IHDR 图尺寸 ──────────────────────────────

/// D17：打包器读 PNG IHDR → manifest AssetEntry.w/h = 真实像素尺寸。
/// 40×20 PNG → AssetEntry { path, w:40, h:20 }（非 0/0 兜底）。
#[test]
fn pack_reads_png_ihdr_dimensions_into_manifest() {
    let dir = make_tmp_dir_with_html(&["c"], &["res/wide.png"]);
    write_minimal_png(&dir.join("res/wide.png"), 40, 20);
    fs::write(
        dir.join("c.html"),
        r#"<div class="c"><img src="res/wide.png"></div>"#,
    )
    .unwrap();
    let packed = pack(&dir, "test", &["c.html".to_string()], "res").expect("pack ok");
    let entry = packed
        .asset_manifest
        .iter()
        .find(|e| e.path == "wide.png")
        .expect("manifest 含 wide.png");
    assert_eq!(entry.w, 40, "PNG IHDR width=40 读进 manifest");
    assert_eq!(entry.h, 20, "PNG IHDR height=20 读进 manifest");
    // roundtrip 保留
    let pkg = read_package(&packed.pkg_bytes).expect("read ok");
    let entry2 = pkg
        .asset_manifest
        .iter()
        .find(|e| e.path == "wide.png")
        .expect("read 后 manifest 含 wide.png");
    assert_eq!((entry2.w, entry2.h), (40, 20), "roundtrip 保留 40×20");
    let _ = fs::remove_dir_all(&dir);
}

/// D17：非 PNG 文件（空占位）→ w/h=0（核心 measure fallback 64×64）。
#[test]
fn pack_non_png_returns_zero_dims() {
    let dir = make_tmp_dir_with_html(&["c"], &["res/notpng.png"]);
    // make_tmp_dir_with_html 已写空文件（非 PNG magic）→ read_png_size 返 (0,0)
    fs::write(
        dir.join("c.html"),
        r#"<div class="c"><img src="res/notpng.png"></div>"#,
    )
    .unwrap();
    let packed = pack(&dir, "test", &["c.html".to_string()], "res").expect("pack ok");
    let entry = packed
        .asset_manifest
        .iter()
        .find(|e| e.path == "notpng.png")
        .expect("manifest 含 notpng.png");
    assert_eq!((entry.w, entry.h), (0, 0), "非 PNG → 0/0（fallback 64×64）");
    let _ = fs::remove_dir_all(&dir);
}

/// D17：PNG 缺失文件 → w/h=0（不 panic，warning 跳过）。
#[test]
fn pack_missing_png_file_returns_zero_dims() {
    let dir = make_tmp_dir_with_html(&["c"], &[]); // 不建 res/missing.png
    fs::write(
        dir.join("c.html"),
        r#"<div class="c"><img src="res/missing.png"></div>"#,
    )
    .unwrap();
    let packed = pack(&dir, "test", &["c.html".to_string()], "res").expect("pack ok");
    let entry = packed
        .asset_manifest
        .iter()
        .find(|e| e.path == "missing.png")
        .expect("manifest 含 missing.png（归一化仍收，读 PNG 失败 w/h=0）");
    assert_eq!((entry.w, entry.h), (0, 0), "PNG 缺失 → 0/0（不 panic）");
    let _ = fs::remove_dir_all(&dir);
}

/// D17：用已有 fixture a.png(4×4) + b.png(2×2) 验 read_png_size 读真实尺寸。
#[test]
fn pack_reads_fixture_png_dimensions() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dir = make_tmp_dir_with_html(&["c"], &[]);
    // 复制 fixture 到 tmp res/（pack 按 res_dir/path 找文件）
    fs::create_dir_all(dir.join("res")).unwrap();
    fs::copy(fixtures.join("a.png"), dir.join("res/a.png")).unwrap();
    fs::copy(fixtures.join("b.png"), dir.join("res/b.png")).unwrap();
    fs::write(
        dir.join("c.html"),
        r#"<div class="c"><img src="res/a.png"><img src="res/b.png"></div>"#,
    )
    .unwrap();
    let packed = pack(&dir, "test", &["c.html".to_string()], "res").expect("pack ok");
    let a = packed.asset_manifest.iter().find(|e| e.path == "a.png").expect("a.png in manifest");
    let b = packed.asset_manifest.iter().find(|e| e.path == "b.png").expect("b.png in manifest");
    assert_eq!((a.w, a.h), (4, 4), "fixture a.png = 4×4");
    assert_eq!((b.w, b.h), (2, 2), "fixture b.png = 2×2");
    let _ = fs::remove_dir_all(&dir);
}
