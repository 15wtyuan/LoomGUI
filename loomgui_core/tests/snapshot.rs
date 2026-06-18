//! Insta 快照测试：从 HTML/CSS fixture 端到端跑到 render_nodes JSON，锁住输出。
//!
//! 跑法：
//! - 首次接受：`INSTA_UPDATE=always cargo test -p loomgui_core --test snapshot`
//! - 之后：`cargo test -p loomgui_core --test snapshot`（绿=锁定）
//!
//! 字体策略：测试字体锁仓库内 `tests/fixtures/DejaVuSans.ttf`（开源，跨平台一致），
//! 不再依赖系统 arial.ttf / DejaVuSans（Linux CI 无 arial 会漂移）。
//! DejaVu Sans 无 CJK glyph，故 fixture 用 ASCII 文本；CJK 渲染验证留 v1
//! （需 CJK 字体策略，见 spec §9）。
//! 覆盖：simple_panel（flex 列布局 + Container/Button/Text）、
//! cascade_inheritance（root color/font-size 经 cascade 传子）。

use loomgui_core::stage::Stage;

/// 测试字体：仓库内 DejaVuSans.ttf，跨平台一致。
/// 用 `env!("CARGO_MANIFEST_DIR")` 拼绝对路径，不依赖系统字体安装。
fn test_font_path() -> String {
    format!(
        "{}/tests/fixtures/DejaVuSans.ttf",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// 缺字体时 skip（return，不算失败）。
/// 防御性保留：仓库内字体正常情况下必存在，实际不会 skip。
fn skip_if_no_font(font: &str) -> bool {
    if std::fs::read(font).is_err() {
        eprintln!("skip: no font at {}", font);
        return true;
    }
    false
}

#[test]
fn snapshot_simple_panel() {
    let font = test_font_path();
    if skip_if_no_font(&font) {
        return;
    }
    // fixture 用 ASCII（DejaVuSans 无 CJK，CJK 验证留 v1）
    let html = r#"<div class="root"><div class="h">Title</div><button class="b">OK</button></div>"#;
    let css = r#".root { width: 300px; height: 200px; flex-direction: column; gap: 8px; } .h { height: 30px; } .b { width: 100px; height: 40px; }"#;
    let mut stage = Stage::new(&font, (300.0, 200.0)).unwrap();
    stage.load_inline(html, css).unwrap();
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
    stage.load_inline(html, css).unwrap();
    let json = stage.render_json();
    insta::assert_snapshot!("cascade_inheritance", json);
}
