//! 诊断：dump showcase 所有 Text 节点，比对
//!   ① layout 阶段语义：measure_text(content, None)         —— intrinsic（不换行）
//!   ② render 阶段实际：measure_text(content, Some(rect.w))  —— 用 taffy 最终宽作 max_width 重测
//!   ③ 自指模拟：       measure_text(content, Some(text_width①))
//! flag：② 行数 ≠ ① 行数 → render 二次测量换行（回归现场）。
//! 用 CJK 字体（showcase 含中文标题），与 Unity 实际字体族接近。
use loomgui_core::stage::Stage;
use loomgui_core::scene::node::NodeKind;
use loomgui_core::text::layout::measure_text;

fn main() {
    let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/wqy-microhei.ttc");
    let pkg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../loomgui_unity/Assets/StreamingAssets/loom_showcase.pkg.bin");
    let pkg = match std::fs::read(pkg_path) {
        Ok(b) => b,
        Err(e) => { eprintln!("read pkg: {}", e); return; }
    };
    let mut s = match Stage::new(font_path, (1080.0, 1920.0)) {
        Ok(s) => s,
        Err(e) => { eprintln!("Stage::new({}): {}", font_path, e); return; }
    };
    if let Err(e) = s.load_package(&pkg) {
        eprintln!("load_package: {}", e); return;
    }
    s.tick_and_render();
    let scene = s.scene.as_ref().unwrap();
    let font = s.font.as_ref();
    println!("n_nodes={} font=wqy-microhei", scene.nodes.len());
    println!("{:<22} {:>8} {:>9} {:>8} {:>8} {:>8}  content", "id", "rect.w", "none.tw", "none.ln", "before", "after");
    // before = measure(rect.w).lines（用 rect.w 重测）；after = scene.text_layouts.lines（render 复用 layout 结果）。
    let mut flagged = 0;
    for n in &scene.nodes {
        let content = match &n.kind { NodeKind::Text { content } => content.clone(), _ => continue };
        let st = &n.style;
        let rect_w = n.layout_rect.w;
        let m_none = measure_text(&content, st.font_size, st.line_height, st.letter_spacing, st.text_align, st.white_space_nowrap, None, font);
        let before = measure_text(&content, st.font_size, st.line_height, st.letter_spacing, st.text_align, st.white_space_nowrap, Some(rect_w), font).lines.len();
        let after = scene.text_layouts.get(n.id.0).cloned().flatten().map(|l| l.lines.len()).unwrap_or(0);
        let id = n.id_attr.clone().unwrap_or_default();
        let flag = before != after;
        if flag { flagged += 1; }
        println!(
            "{:<22} {:>8.3} {:>9.3} {:>8} {:>8} {:>8}{}  {:?}",
            id, rect_w, m_none.text_width, m_none.lines.len(), before, after,
            if flag { "  <<< FIXED" } else { "" },
            content.chars().take(24).collect::<String>(),
        );
    }
    println!("\nflagged (修复前后 render 行数差异，应 = 短标题数): {}", flagged);
}
