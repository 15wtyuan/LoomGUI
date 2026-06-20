//! Stage 层：串起 parse → style → scene → layout → render 的端到端入口（§4-§6）。
//!
//! v0 内存直通：`load_inline` 吃 HTML+CSS 文本，`tick_and_render` 跑首帧
//! solve + build_render_nodes。`render_json` serde 序列化产 spec §5 JSON。
//! v0 无输入/动画/打包器，Stage 只是「装配 + 单帧」的薄壳。

use crate::layout::solve;
#[cfg(feature = "parse")]
use crate::parse::css::parse_css;
#[cfg(feature = "parse")]
use crate::parse::dom::parse_html;
use crate::render::build_render_nodes;
use crate::render::FrameData;
#[cfg(feature = "parse")]
use crate::scene::node::build_scene;
use crate::scene::node::Scene;
#[cfg(feature = "parse")]
use crate::style::cascade::resolve_styles;
use crate::text::layout::Font;
use std::sync::Arc;

pub struct Stage {
    pub scene: Option<Scene>,
    pub font: Arc<Font>,
    pub root_size: (f32, f32),
}

impl Stage {
    pub fn new(font_path: &str, root_size: (f32, f32)) -> Result<Self, String> {
        // Font::from_path 返回 Result<_, String>，直接 ? 传播（原 .map_err(|e| e)? 是 no-op）。
        let font = Font::from_path(font_path)?;
        Ok(Stage {
            scene: None,
            font: Arc::new(font),
            root_size,
        })
    }

    /// v0 内存直通：HTML+CSS 文本直接构 scene（不走打包器）。
    #[cfg(feature = "parse")]
    pub fn load_inline(&mut self, html: &str, css: &str) -> Result<(), String> {
        let tree = parse_html(html)?;
        let sheet = parse_css(css)?;
        let styles = resolve_styles(&tree, &sheet);
        self.scene = Some(build_scene(&tree, &styles));
        Ok(())
    }

    /// 从二进制包加载（spec §8）：read_package → self.scene + root_size（用包 header 的）。
    /// 与 `load_inline` 二选一设 scene；后续 tick_and_render 不变。不需 parse feature。
    pub fn load_package(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (scene, root_size) = crate::asset::read_package(bytes).map_err(|e| e.to_string())?;
        self.scene = Some(scene);
        self.root_size = root_size;
        Ok(())
    }

    /// 静态首帧：solve + render。v0 无输入/动画。返回 nodes + clip 表（§4.4）。
    pub fn tick_and_render(&mut self) -> FrameData {
        let scene = self.scene.as_mut().expect("load first");
        solve(scene, &self.font, self.root_size);
        build_render_nodes(scene, &self.font)
    }

    pub fn render_json(&mut self) -> String {
        let frame = self.tick_and_render();
        serde_json::to_string_pretty(&frame.nodes).unwrap()
    }
}

#[cfg(all(test, feature = "parse"))]
mod tests {
    use super::*;

    /// 黄金等价（最强门）：inline 渲染 == 包渲染。
    /// v0 fixture（div + 文本 + img + rect mask）经 pkg→load_package→render_json
    /// 必须 == inline load_inline→render_json。
    #[test]
    fn package_load_renders_identical_to_inline() {
        let font_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/DejaVuSans.ttf"
        );
        let html = r#"<div class="c"><span>hi</span><img src="logo.png"></div>"#;
        let css = ".c{width:200px;height:100px;overflow:hidden;background-color:#ff0000;}";

        // inline 路径
        let mut s_inline = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_inline.load_inline(html, css).unwrap();
        let inline_json = s_inline.render_json();

        // 序列化 inline 的 scene → 包
        let scene = s_inline.scene.as_ref().unwrap();
        let pkg = crate::asset::write_package(scene, (200.0, 100.0));

        // 包路径（新 Stage，同字体）
        let mut s_pkg = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_pkg.load_package(&pkg).unwrap();
        let pkg_json = s_pkg.render_json();

        assert_eq!(inline_json, pkg_json, "包路径渲染输出必须 == inline");
    }
}
