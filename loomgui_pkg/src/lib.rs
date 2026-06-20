//! 打包器库（spec §10）：HTML+CSS → .pkg.bin。复用 core parse/style/scene + asset::write_package。

/// 把 HTML+CSS 打成 .pkg.bin 字节（spec §10）。root_size 写进包 header。
///
/// v1b.3：AtlasSection 暂传空（图集打包留 T4——packer 调 image crate shelf 装图后
/// 填充 sprites 再 write_package）。本函数当前产「无图集」v2 包（atlas_count=0）。
pub fn pack(html: &str, css: &str, root_size: (f32, f32)) -> Result<Vec<u8>, String> {
    let tree = loomgui_core::parse::dom::parse_html(html).map_err(|e| format!("parse_html: {e}"))?;
    let sheet = loomgui_core::parse::css::parse_css(css).map_err(|e| format!("parse_css: {e}"))?;
    let styles = loomgui_core::style::cascade::resolve_styles(&tree, &sheet);
    let scene = loomgui_core::scene::build_scene(&tree, &styles);
    Ok(loomgui_core::asset::write_package(
        &scene,
        root_size,
        &loomgui_core::asset::AtlasSection::default(),
    ))
}
