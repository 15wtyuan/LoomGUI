//! v1e dirty 跟踪端到端：Stage 跨 tick 保持 hash 基线，静态帧产 Unchanged。

use loomgui_core::stage::Stage;

fn font_path() -> (String, usize) {
    let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
    let n = p.len();
    (p, n)
}

/// Stage 跨帧 dirty：首帧全 emit，第二帧静态 → Unchanged。
#[test]
fn stage_static_frame_produces_unchanged() {
    let (fp, _fplen) = font_path();
    let mut stage = Stage::new(&fp, (200.0, 100.0)).expect("stage");
    let html = r#"<div style="width:50px;height:50px;background-color:#ff0000"></div>"#;
    stage.load_inline(html, "").expect("load");
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
    stage.load_inline(r#"<div style="width:50px;height:50px;background-color:#ff0000"></div>"#, "").expect("load");
    stage.advance_time(0.016);
    let _ = stage.tick_and_render();
    stage.advance_time(0.016);
    let _ = stage.tick_and_render(); // 建立 hash 基线
    // reload（不同 HTML）→ 清基线 → 首帧全 emit。
    stage.load_inline(r#"<div style="width:60px;height:60px;background-color:#00ff00"></div>"#, "").expect("reload");
    stage.advance_time(0.016);
    let f3 = stage.tick_and_render();
    assert!(f3.nodes.iter().all(|n| !matches!(n.payload, loomgui_core::render::node::NodePayload::Unchanged)),
        "reload 后首帧全 emit（基线已清）");
}
