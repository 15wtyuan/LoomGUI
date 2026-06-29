//! 打包器库：HTML+CSS+散图 → .pkg.bin + atlas.png。
//! 复用 core parse/style/scene + asset::write_package；新加 image crate 解码/编码 PNG + shelf 打包。

use loomgui_core::asset::{AtlasInfo, AtlasSection, AtlasSprite};
use std::io::Cursor;
use std::path::Path;

/// 打包产物：.pkg.bin bytes + atlas.png bytes + atlas 相对文件名（写进 .pkg.bin header）。
#[derive(Debug)]
pub struct PackedPackage {
    pub pkg_bytes: Vec<u8>,
    pub atlas_png: Vec<u8>,
    pub atlas_filename: String,
}

/// 一个 sprite 在 shelf 打包后的位置（atlas 像素坐标，y-down）。
#[derive(Debug)]
struct PlacedSprite {
    src: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// 把 HTML+CSS+res_dir 下散图打成 .pkg.bin + atlas.png。
/// res_dir = 解析 `<img src>` 的基准目录（CLI 传 html_path.parent()）。
/// 无图 → 空 atlas（atlas_count=0，pkg.bin 仍可读，runtime 跳过 atlas 加载）。
/// 缺图 → Err（build-time fail）。
fn pack_inner(
    html: &str,
    css: &str,
    root_size: (f32, f32),
    res_dir: &Path,
    atlas_name: &str,
) -> Result<PackedPackage, String> {
    let tree = loomgui_core::parse::dom::parse_html(html).map_err(|e| format!("parse_html: {e}"))?;
    let sheet = loomgui_core::parse::css::parse_css(css).map_err(|e| format!("parse_css: {e}"))?;
    let dynamic_rules = loomgui_core::asset::extract_dynamic_rules(&sheet);
    let styles = loomgui_core::style::cascade::resolve_styles(&tree, &sheet);
    let scene = loomgui_core::scene::build_scene(&tree, &styles);

    // 1. 收集 Image src（DFS 先序去重）—— scene.nodes 已 DFS 先序。
    let mut srcs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for n in &scene.nodes {
        if let loomgui_core::scene::NodeKind::Image { src } = &n.kind {
            if seen.insert(src.as_str()) {
                srcs.push(src.clone());
            }
        }
    }
    // 1b. 收集 CSS background-image url（同 srcs/seen 去重，img+bg 同 url 只入一次）。
    for n in &scene.nodes {
        if let Some(url) = &n.style.background_image {
            if seen.insert(url.as_str()) {
                srcs.push(url.clone());
            }
        }
    }

    // 2. 无图 → 空 atlas。
    if srcs.is_empty() {
        let pkg = loomgui_core::asset::write_package(&scene, root_size, &AtlasSection::default(), &dynamic_rules);
        return Ok(PackedPackage {
            pkg_bytes: pkg,
            atlas_png: Vec::new(),
            atlas_filename: String::new(),
        });
    }

    // 3. 解码每张 PNG（image crate）→ (src, w, h, RgbaImage)。
    let mut decoded: Vec<(String, u32, u32, image::RgbaImage)> = Vec::with_capacity(srcs.len());
    for src in &srcs {
        let path = res_dir.join(src);
        let img = image::open(&path).map_err(|e| format!("image not found: {src} ({e})"))?;
        let rgba = img.to_rgba8();
        let w = rgba.width();
        let h = rgba.height();
        decoded.push((src.clone(), w, h, rgba));
    }

    // 4. shelf 打包。
    let mut dims: Vec<(String, u32, u32)> = decoded
        .iter()
        .map(|(s, w, h, _)| (s.clone(), *w, *h))
        .collect();
    let (atlas_w, atlas_h, placed) = shelf_pack(&mut dims);

    // 5. blit 进 atlas buffer（RGBA8）。
    let mut buf = vec![0u8; (atlas_w * atlas_h * 4) as usize];
    for (src, _w, _h, rgba) in &decoded {
        let p = placed
            .iter()
            .find(|p| &p.src == src)
            .ok_or_else(|| format!("internal: placement missing for {src}"))?;
        for row in 0..p.h {
            for col in 0..p.w {
                let px = rgba.get_pixel(col, row).0;
                let di = (((p.y + row) * atlas_w + (p.x + col)) * 4) as usize;
                buf[di..di + 4].copy_from_slice(&px);
            }
        }
    }

    // 6. 编码 atlas.png 到内存。
    //    image 0.25 无 save_buffer_to_memory；用 RgbaImage::write_to(&mut Cursor<Vec<u8>>, Png)。
    let atlas_img = image::RgbaImage::from_raw(atlas_w, atlas_h, buf)
        .ok_or_else(|| String::from("atlas buffer size mismatch"))?;
    let mut png_bytes: Vec<u8> = Vec::new();
    atlas_img
        .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("encode atlas png: {e}"))?;

    // 7. AtlasSection + write_package。
    let atlas_section = AtlasSection {
        atlases: vec![AtlasInfo {
            filename: atlas_name.into(),
            width: atlas_w,
            height: atlas_h,
        }],
        sprites: placed
            .iter()
            .map(|p| AtlasSprite {
                src: p.src.clone(),
                x: p.x,
                y: p.y,
                w: p.w,
                h: p.h,
            })
            .collect(),
    };
    let pkg = loomgui_core::asset::write_package(&scene, root_size, &atlas_section, &dynamic_rules);

    Ok(PackedPackage {
        pkg_bytes: pkg,
        atlas_png: png_bytes,
        atlas_filename: atlas_name.into(),
    })
}

/// atlas 名固定 "loom.atlas.png"（默认 sample 行为）。
pub fn pack(html: &str, css: &str, root_size: (f32, f32), res_dir: &Path) -> Result<PackedPackage, String> {
    pack_inner(html, css, root_size, res_dir, "loom.atlas.png")
}

/// 指定 atlas 文件名（多 sample 共存 StreamingAssets 时用独立名避免互相覆盖）。
pub fn pack_named(html: &str, css: &str, root_size: (f32, f32), res_dir: &Path, atlas_name: &str) -> Result<PackedPackage, String> {
    pack_inner(html, css, root_size, res_dir, atlas_name)
}

/// shelf 打包：按高降序、atlas_w=max(512,最宽)、逐行摆、超宽换行。NPOT。无旋转/trim。
fn shelf_pack(sprites: &mut Vec<(String, u32, u32)>) -> (u32, u32, Vec<PlacedSprite>) {
    // sprites 元组 = (src, w, h)。
    const DEFAULT_ATLAS_W: u32 = 512;
    sprites.sort_by(|a, b| b.2.cmp(&a.2)); // 按高 h 降序
    let atlas_w = sprites
        .iter()
        .map(|(_, w, _)| *w)
        .max()
        .unwrap_or(0)
        .max(DEFAULT_ATLAS_W);
    let mut placed = Vec::with_capacity(sprites.len());
    let mut x = 0u32;
    let mut y = 0u32;
    let mut shelf_h = 0u32;
    for (src, w, h) in sprites.iter() {
        if x + w > atlas_w {
            y += shelf_h;
            x = 0;
            shelf_h = 0;
        }
        placed.push(PlacedSprite {
            src: src.clone(),
            x,
            y,
            w: *w,
            h: *h,
        });
        x += w;
        shelf_h = shelf_h.max(*h);
    }
    (atlas_w, y + shelf_h, placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_pack_no_overlap_within_bounds() {
        let mut s = vec![
            ("a".into(), 100, 50),
            ("b".into(), 200, 80),
            ("c".into(), 300, 30),
        ];
        let (aw, ah, placed) = shelf_pack(&mut s);
        // 不出界
        for p in &placed {
            assert!(p.x + p.w <= aw && p.y + p.h <= ah, "{p:?} out of bounds");
        }
        // 两两不重叠
        for i in 0..placed.len() {
            for j in (i + 1)..placed.len() {
                let (a, b) = (&placed[i], &placed[j]);
                let overlap = a.x < b.x + b.w
                    && b.x < a.x + a.w
                    && a.y < b.y + b.h
                    && b.y < a.y + a.h;
                assert!(!overlap, "sprites {a:?} {b:?} overlap");
            }
        }
        // atlas_w 至少 = 最宽 sprite（300）
        assert!(aw >= 300);
    }

    #[test]
    fn shelf_pack_wraps_when_exceeding_atlas_w() {
        // 两个 600×10 sprite：atlas_w=max(512,600)=600；第二个塞不下换行。
        let mut s = vec![("a".into(), 600, 10), ("b".into(), 600, 10)];
        let (aw, ah, placed) = shelf_pack(&mut s);
        assert_eq!(aw, 600);
        assert_eq!(ah, 20, "两行各 10px 高");
        assert_eq!(placed[0].y, 0);
        assert_eq!(placed[1].y, 10, "第二个换行到 y=10");
    }

    #[test]
    fn shelf_pack_empty() {
        let mut s: Vec<(String, u32, u32)> = vec![];
        let (aw, ah, placed) = shelf_pack(&mut s);
        assert_eq!(aw, 512, "空 → 默认宽");
        assert_eq!(ah, 0);
        assert!(placed.is_empty());
    }

    use std::fs;
    use std::path::PathBuf;

    /// 建临时 res_dir，写一张 300×4 红 PNG（宽度确保两图不能并排于 512 宽 atlas，便于测去重），返回 (res_dir, png_filename)。
    fn write_tmp_png() -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("loomgui_pkg_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let name = "red.png".to_string();
        let img = image::RgbaImage::from_fn(300, 4, |_, _| image::Rgba([255, 0, 0, 255]));
        img.save(dir.join(&name)).unwrap();
        (dir, name)
    }

    #[test]
    fn pack_collects_background_image_url_into_atlas() {
        // 纯 background-image（无 img）→ atlas 含该 src
        let (dir, name) = write_tmp_png();
        let html = format!(
            r#"<div style="background-image:url({})"></div>"#, name
        );
        let css = "";
        let packed = pack_inner(&html, css, (100.0, 100.0), &dir, "test.atlas.png").unwrap();
        assert!(!packed.atlas_png.is_empty(), "纯 bg-image → atlas 非空");
        assert_eq!(packed.atlas_filename, "test.atlas.png");
        // 清理
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_dedupes_img_and_bg_same_url() {
        // img + background-image 同 url → atlas 只入一次（去重）
        let (dir, name) = write_tmp_png();
        let html = format!(
            r#"<div style="background-image:url({})"><img src="{}"></div>"#, name, name
        );
        let css = "";
        let packed = pack_inner(&html, css, (100.0, 100.0), &dir, "test.atlas.png").unwrap();
        // atlas 非空（含该图）；去重后只有一张 4×4
        assert!(!packed.atlas_png.is_empty());
        // 验证 atlas 尺寸 = 单张 4×4（shelf_pack：atlas_w=max(512,4)=512, h=4）
        let atlas_img = image::load_from_memory(&packed.atlas_png).unwrap();
        assert_eq!(atlas_img.height(), 4, "单图去重 → atlas 高=4");
        let _ = fs::remove_dir_all(&dir);
    }
}
