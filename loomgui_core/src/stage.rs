//! Stage 层：串起 parse → style → scene → layout → render 的端到端入口（§4-§6）。
//!
//! v0 内存直通：`load_inline` 吃 HTML+CSS 文本，`tick_and_render` 跑首帧
//! solve + build_render_nodes。`render_json` serde 序列化产 spec §5 JSON。
//! v0 无输入/动画/打包器，Stage 只是「装配 + 单帧」的薄壳。

use crate::layout::solve;
use crate::parse::css::parse_css;
use crate::parse::dom::parse_html;
use crate::render::build_render_nodes;
use crate::render::node::RenderNode;
use crate::scene::node::{build_scene, Scene};
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
    pub fn load_inline(&mut self, html: &str, css: &str) -> Result<(), String> {
        let tree = parse_html(html)?;
        let sheet = parse_css(css)?;
        let styles = resolve_styles(&tree, &sheet);
        self.scene = Some(build_scene(&tree, &styles));
        Ok(())
    }

    /// 静态首帧：solve + render。v0 无输入/动画。
    pub fn tick_and_render(&mut self) -> Vec<RenderNode> {
        let scene = self.scene.as_mut().expect("load first");
        solve(scene, &self.font, self.root_size);
        build_render_nodes(scene, &self.font)
    }

    pub fn render_json(&mut self) -> String {
        let nodes = self.tick_and_render();
        serde_json::to_string_pretty(&nodes).unwrap()
    }
}
