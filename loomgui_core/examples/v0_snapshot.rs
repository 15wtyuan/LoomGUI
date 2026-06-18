//! v0 端到端 runner：HTML+CSS → Stage → render_nodes JSON。
//!
//! 跑法：`cargo run --example v0_snapshot [font_path]`
//! 产物：v0_snapshot.json（serde 序列化的 RenderNode 数组）。
//! 字体缺省路径按平台选 arial.ttf(Win)/DejaVuSans.ttf(Linux)。

use loomgui_core::stage::Stage;

fn main() {
    // img 是 void 元素：src 走属性（scene 层从 attrs 读，不读 text），
    // 不能写成 `<img>logo.png</img>`（HTML5 解析会把裸文本泄到父 div，触发 inline-mix 报错）。
    let html = r#"<div class="root"><div class="header">标题</div><button class="btn">确定</button><img class="logo" src="logo.png"></div>"#;
    let css = r#"
        .root { width: 300px; height: 200px; flex-direction: column; gap: 8px; background-color: #f0f0f0; }
        .header { font-size: 18px; color: #333333; height: 30px; }
        .btn { width: 100px; height: 40px; background-color: #0066cc; color: #ffffff; text-align: center; }
        .logo { width: 64px; height: 64px; }
    "#;
    let font = std::env::args().nth(1).unwrap_or_else(|| {
        if cfg!(windows) {
            "C:\\Windows\\Fonts\\arial.ttf".into()
        } else {
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into()
        }
    });
    let mut stage = Stage::new(&font, (300.0, 200.0)).expect("font load");
    stage.load_inline(html, css).expect("parse");
    let json = stage.render_json();
    println!("{}", json);
    std::fs::write("v0_snapshot.json", &json).ok();
}
