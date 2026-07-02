//! Insta 快照测试：从 HTML/CSS fixture 端到端跑到 render_nodes JSON，锁住输出。
//!
//! 跑法：
//! - 首次接受：`INSTA_UPDATE=always cargo test -p loomgui_core --test snapshot`
//! - 之后：`cargo test -p loomgui_core --test snapshot`（绿=锁定）
//!
//! 字体锁仓库内 `tests/fixtures/DejaVuSans.ttf`（开源，跨平台一致），
//! 不依赖系统字体（Linux CI 无 arial 会漂移）。DejaVu Sans 无 CJK glyph，
//! 故 fixture 用 ASCII 文本；CJK 渲染需 CJK 字体策略，另测。
//!
//! v1.4-a T4：`Stage::load_inline` 已砍（D12）。本集成测验证 parse→render 管线，
//! 用本地 helper `load_html_css` 直接调 parse_html + build_scene 构 scene（同旧 load_inline 逻辑）。
//! textures/atlases 已砍，Image 走未注册 fallback（tex_id=0，T6 改 payload 带 path）。

use loomgui_core::parse::css::parse_css;
use loomgui_core::parse::dom::parse_html;
use loomgui_core::scene::node::build_scene;
use loomgui_core::stage::Stage;
use loomgui_core::style::cascade::resolve_styles;

/// 测试字体：仓库内 DejaVuSans.ttf，跨平台一致。
fn test_font_path() -> String {
    format!(
        "{}/tests/fixtures/DejaVuSans.ttf",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// 缺字体时 skip（return，不算失败）。
fn skip_if_no_font(font: &str) -> bool {
    if std::fs::read(font).is_err() {
        eprintln!("skip: no font at {}", font);
        return true;
    }
    false
}

/// v1.4-a T4 helper：HTML+CSS → scene（同旧 load_inline 逻辑，parse 路径保留供集成测）。
fn load_html_css(stage: &mut Stage, html: &str, css: &str) {
    let tree = parse_html(html).unwrap();
    let sheet = parse_css(css).unwrap();
    let styles = resolve_styles(&tree, &sheet);
    stage.tweens.clear();
    if let Some(scene) = stage.scene.as_mut() {
        scene.scroll.clear();
    }
    stage.prev_node_hashes.clear();
    stage.scene = Some(build_scene(&tree, &styles));
}

#[test]
fn snapshot_simple_panel() {
    let font = test_font_path();
    if skip_if_no_font(&font) {
        return;
    }
    // fixture 用 ASCII（DejaVuSans 无 CJK glyph）
    let html = r#"<div class="root"><div class="h">Title</div><button class="b">OK</button></div>"#;
    let css = r#".root { width: 300px; height: 200px; flex-direction: column; gap: 8px; } .h { height: 30px; } .b { width: 100px; height: 40px; }"#;
    let mut stage = Stage::new(&font, (300.0, 200.0)).unwrap();
    load_html_css(&mut stage, html, css);
    let json = stage.render_json();
    insta::assert_snapshot!("simple_panel", json);
}

#[test]
fn snapshot_cascade_inheritance() {
    let font = test_font_path();
    if skip_if_no_font(&font) {
        return;
    }
    let html = r#"<div class="root"><span class="child">hi</span></div>"#;
    let css = r#".root { color: #ff0000; font-size: 20px; } .child { width: 50px; }"#;
    let mut stage = Stage::new(&font, (300.0, 200.0)).unwrap();
    load_html_css(&mut stage, html, css);
    let json = stage.render_json();
    insta::assert_snapshot!("cascade_inheritance", json);
}

/// `<img>` 渲染路径 snapshot（锁纹理路径输出）。
/// img 有显式 CSS 尺寸 → measure 用声明值；此测只锁几何 + payload 形状
/// （未注册 src 兜底 tex_id=0）。
#[cfg(feature = "parse")]
#[test]
fn snapshot_image_with_texture() {
    let font = test_font_path();
    if skip_if_no_font(&font) {
        return;
    }
    let html = r#"<div class="root"><img class="i" src="logo.png"></div>"#;
    let css = r#".root { width: 300px; height: 200px; } .i { width: 100px; height: 80px; }"#;
    let mut stage = Stage::new(&font, (300.0, 200.0)).unwrap();
    load_html_css(&mut stage, html, css);
    let json = stage.render_json();
    insta::assert_snapshot!("image_with_texture", json);
}
