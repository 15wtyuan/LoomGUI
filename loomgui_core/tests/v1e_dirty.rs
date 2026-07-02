//! dirty 跟踪端到端：Stage 跨 tick 保持 hash 基线，静态帧产 Unchanged。
//!
//! v1.4-a T4：`Stage::load_inline` 已砍（D12）。本测用本地 helper `load_html_css`
//! 直接调 parse_html + build_scene 构 scene（同旧 load_inline 逻辑）。

use loomgui_core::parse::css::parse_css;
use loomgui_core::parse::dom::parse_html;
use loomgui_core::scene::node::build_scene;
use loomgui_core::stage::Stage;
use loomgui_core::style::cascade::resolve_styles;

fn font_path() -> (String, usize) {
    let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
    let n = p.len();
    (p, n)
}

/// v1.4-a T4 helper：HTML+CSS → scene（同旧 load_inline 逻辑）。
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

/// Stage 跨帧 dirty：首帧全 emit，第二帧静态 → Unchanged。
#[test]
fn stage_static_frame_produces_unchanged() {
    let (fp, _fplen) = font_path();
    let mut stage = Stage::new(&fp, (200.0, 100.0)).expect("stage");
    let html = r#"<div style="width:50px;height:50px;background-color:#ff0000"></div>"#;
    load_html_css(&mut stage, html, "");
    stage.advance_time(0.016);
    let f1 = stage.tick_and_render();
    // 首帧全 Mesh，无 Unchanged。
    assert!(f1.nodes.iter().all(|n| !matches!(n.payload, loomgui_core::render::node::NodePayload::Unchanged)),
        "首帧全 emit");
    // 第二帧静态 → 含 Unchanged。
    stage.advance_time(0.016);
    let f2 = stage.tick_and_render();
    assert!(f2.nodes.iter().any(|n| matches!(n.payload, loomgui_core::render::node::NodePayload::Unchanged)),
        "静态帧 → Unchanged");
}

/// reload 清 hash 基线：load 后首帧又全 emit。
#[test]
fn stage_reload_clears_dirty_baseline() {
    let (fp, _fplen) = font_path();
    let mut stage = Stage::new(&fp, (200.0, 100.0)).expect("stage");
    load_html_css(&mut stage, r#"<div style="width:50px;height:50px;background-color:#ff0000"></div>"#, "");
    stage.advance_time(0.016);
    let _ = stage.tick_and_render();
    stage.advance_time(0.016);
    let _ = stage.tick_and_render(); // 建立 hash 基线
    // reload（不同 HTML）→ 清基线 → 首帧全 emit。
    load_html_css(&mut stage, r#"<div style="width:60px;height:60px;background-color:#00ff00"></div>"#, "");
    stage.advance_time(0.016);
    let f3 = stage.tick_and_render();
    assert!(f3.nodes.iter().all(|n| !matches!(n.payload, loomgui_core::render::node::NodePayload::Unchanged)),
        "reload 后首帧全 emit（基线已清）");
}
