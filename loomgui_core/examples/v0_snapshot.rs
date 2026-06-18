//! v0 端到端 runner：HTML+CSS → Stage → render_nodes JSON。
//!
//! 跑法：`cargo run --example v0_snapshot [font_path]`
//! 产物：v0_snapshot.json（serde 序列化的 RenderNode 数组）。
//!
//! 字体策略：默认字体锁仓库内 `tests/fixtures/DejaVuSans.ttf`（开源，跨平台一致），
//! 不再依赖系统 arial.ttf / DejaVuSans（Linux CI 无 arial 会漂移）。
//! 可选 `font_path` 命令行参数仍允许调用者覆盖（便于本地用其他字体试）。
//! fixture 用 ASCII 文本（DejaVuSans 无 CJK，CJK 验证留 v1）。

use loomgui_core::stage::Stage;

/// 默认测试字体：仓库内 DejaVuSans.ttf，跨平台一致。
fn default_font_path() -> String {
    format!(
        "{}/tests/fixtures/DejaVuSans.ttf",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn main() {
    // img 是 void 元素：src 走属性（scene 层从 attrs 读，不读 text），
    // 不能写成 `<img>logo.png</img>`（HTML5 解析会把裸文本泄到父 div，触发 inline-mix 报错）。
    // fixture 文本用 ASCII（DejaVuSans 无 CJK，CJK 渲染验证留 v1）。
    let html = r#"<div class="root"><div class="header">Title</div><button class="btn">OK</button><img class="logo" src="logo.png"></div>"#;
    let css = r#"
        .root { width: 300px; height: 200px; flex-direction: column; gap: 8px; background-color: #f0f0f0; }
        .header { font-size: 18px; color: #333333; height: 30px; }
        .btn { width: 100px; height: 40px; background-color: #0066cc; color: #ffffff; text-align: center; }
        .logo { width: 64px; height: 64px; }
    "#;
    let font = std::env::args().nth(1).unwrap_or_else(default_font_path);
    let mut stage = Stage::new(&font, (300.0, 200.0)).expect("font load");
    stage.load_inline(html, css).expect("parse");
    let json = stage.render_json();
    println!("{}", json);
    std::fs::write("v0_snapshot.json", &json).ok();
}
