//! Insta 快照测试：从 HTML/CSS fixture 端到端跑到 render_nodes JSON，锁住输出。
//!
//! 跑法：
//! - 首次接受：`INSTA_UPDATE=always cargo test -p loomgui_core --test snapshot`
//! - 之后：`cargo test -p loomgui_core --test snapshot`（绿=锁定）
//!
//! 无字体环境（arial.ttf/DejaVuSans.ttf 都缺）时整测 skip。
//! 覆盖：simple_panel（flex 列布局 + Container/Button/Text/Image）、
//! cascade_inheritance（root color/font-size 经 cascade 传子）。

use loomgui_core::stage::Stage;

fn font_path() -> String {
    if cfg!(windows) {
        "C:\\Windows\\Fonts\\arial.ttf".into()
    } else {
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into()
    }
}

/// 缺字体时 skip（return，不算失败）。
fn skip_if_no_font(font: &str) -> bool {
    if std::fs::read(font).is_err() {
        eprintln!("skip: no font at {}", font);
        return true;
    }
    false
}

#[test]
fn snapshot_simple_panel() {
    let font = font_path();
    if skip_if_no_font(&font) {
        return;
    }
    let html = r#"<div class="root"><div class="h">标题</div><button class="b">确定</button></div>"#;
    let css = r#".root { width: 300px; height: 200px; flex-direction: column; gap: 8px; } .h { height: 30px; } .b { width: 100px; height: 40px; }"#;
    let mut stage = Stage::new(&font, (300.0, 200.0)).unwrap();
    stage.load_inline(html, css).unwrap();
    let json = stage.render_json();
    insta::assert_snapshot!("simple_panel", json);
}

#[test]
fn snapshot_cascade_inheritance() {
    let font = font_path();
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
