//! 打包器库：系统目录多 HTML → .pkg.bin（无 atlas）。
//! v1.4-a：每个 HTML 独立 parse → resolve_styles → build_scene → 抽 TemplateNode；
//! img src / background-image url 归一化进 asset_manifest；CSS bake 进 style_blob。
//! 砍 image crate / shelf_pack / atlas.png（图集归 Unity，D8）。

use loomgui_core::asset::{PackageInput, TemplateNode, extract_component_css, normalize_path};
use loomgui_core::scene::NodeId;
use scraper::{Html, Selector as ScraperSelector};
use std::path::Path;

/// 打包产物：.pkg.bin bytes + asset_manifest（归一化 path 列表，供 Unity 校验 res 齐全）。
/// v1.4-a 砍 atlas_png/atlas_filename（图集归 Unity，D8）。
#[derive(Debug)]
pub struct PackedPackage {
    pub pkg_bytes: Vec<u8>,
    pub asset_manifest: Vec<String>,
}

/// 把单 scene 转 Vec<TemplateNode>（按 slotmap 插入序 = DFS 先序），同时把 img src +
/// background-image url 归一化收进 manifest（去重）。None（src 不在 res 下）→ warning 不入 manifest。
///
/// parent_idx = 父节点在 Vec 中的位置（None=组件根）。slotmap values() 对无删除的全新 map
/// 按槽位序迭代 = 插入序 = build_scene 的 DFS 先序，故 parent 总在 child 前出现，位置索引稳定。
fn scene_to_template(
    scene: &loomgui_core::scene::Scene,
    res_dir: &str,
    manifest: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<TemplateNode> {
    // NodeId → 在产物 Vec 中的位置（slotmap 插入序）。
    let pos_of: std::collections::HashMap<NodeId, usize> = scene
        .nodes
        .values()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();

    let mut nodes: Vec<TemplateNode> = Vec::with_capacity(scene.nodes.len());
    for n in scene.nodes.values() {
        // img src 归一化进 manifest（去重）。归一化后回写节点 src，让 write_package 的
        // StringTable 收归一化 path（非原 src）。
        let mut kind = n.kind.clone();
        if let loomgui_core::scene::NodeKind::Image { src } = &mut kind {
            if !src.is_empty() {
                match normalize_path(src, res_dir) {
                    Some(norm) => {
                        if seen.insert(norm.clone()) {
                            manifest.push(norm.clone());
                        }
                        *src = norm;
                    }
                    None => {
                        eprintln!("warn: img src `{src}` 不在 res 目录 `{res_dir}` 下，跳过 manifest");
                    }
                }
            }
        }
        // background-image url 同样归一化进 manifest（去重；与 img src 同 url 只入一次）。
        let mut style = n.style.clone();
        if let Some(url) = &style.background_image {
            if !url.is_empty() {
                match normalize_path(url, res_dir) {
                    Some(norm) => {
                        if seen.insert(norm.clone()) {
                            manifest.push(norm.clone());
                        }
                        style.background_image = Some(norm);
                    }
                    None => {
                        eprintln!("warn: background-image url `{url}` 不在 res 目录 `{res_dir}` 下，跳过 manifest");
                    }
                }
            }
        }
        nodes.push(TemplateNode {
            kind,
            style,
            parent_idx: n.parent.map(|p| pos_of[&p]),
            classes: n.classes.clone(),
            id_attr: n.id_attr.clone(),
            draggable: n.draggable,
            tabindex: n.tabindex,
        });
    }
    nodes
}

/// 从 HTML 串剥掉所有 `<style>...</style>` 和 `<link ...>` 元素（含内容），返回干净 HTML。
/// parse_html 的围栏白名单（div/span/img/button）拒绝 `<style>`/`<link>`，故打包器在
/// 调 parse_html 前先抽 CSS（extract_component_css）再剥这俩 tag。用 scraper 重构文档树
/// 后序列化回 HTML 字符串（与 parse_html 同后端，保证语义一致）。
fn strip_style_and_link(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut out = String::with_capacity(html.len());
    // 遍历 body 子树，跳过 style/link 节点，重新拼 HTML。
    let body_sel = ScraperSelector::parse("body").unwrap();
    if let Some(body) = document.select(&body_sel).next() {
        serialize_children(&body, &mut out);
    } else {
        // 无 body（scraper 对无 html/head 包裹的片段会合成）→ 退回原串（让 parse_html 报错）。
        return html.to_string();
    }
    out
}

/// 递归序列化元素的子节点（跳过 style/link），拼回 HTML 字符串。
fn serialize_children(el: &scraper::ElementRef, out: &mut String) {
    for child in el.children() {
        match child.value() {
            scraper::node::Node::Text(t) => {
                out.push_str(&t.text);
            }
            scraper::node::Node::Element(e) => {
                // 跳过 style/link（CSS 已由 extract_component_css 抽走）。
                if e.name() == "style" || e.name() == "link" {
                    continue;
                }
                if let Some(eref) = scraper::ElementRef::wrap(child) {
                    out.push('<');
                    out.push_str(e.name());
                    for (k, v) in e.attrs() {
                        out.push(' ');
                        out.push_str(k);
                        out.push_str("=\"");
                        out.push_str(v);
                        out.push('"');
                    }
                    out.push('>');
                    serialize_children(&eref, out);
                    out.push_str("</");
                    out.push_str(e.name());
                    out.push('>');
                }
            }
            _ => {}
        }
    }
}

/// 把系统目录下多个 HTML 打成 .pkg.bin（v1.4-a 多组件格式，无 atlas）。
///
/// - `source_dir`：包源目录（html + res 所在）。
/// - `pkg_name`：包名（当前未进 pkg.bin header，供 CLI 日志用；未来版本号/元数据可扩展）。
/// - `html_files`：要打包的 HTML 文件名列表（相对 sourceDir，含 .html 扩展名）。
/// - `res_dir`：资源目录名（默认 res，对应 spec D10；归一化 path 去此前缀）。
///
/// 每 HTML 独立：抽 CSS → 剥 style/link → parse_html → parse_css → resolve_styles →
/// build_scene → scene_to_template（归一化 src 进 manifest）→ 收 (组件名, nodes, dynamic_rules)。
/// 组件名 = 文件名去 .html。最后 write_package 产 pkg_bytes。
pub fn pack(
    source_dir: &Path,
    _pkg_name: &str,
    html_files: &[String],
    res_dir: &str,
) -> Result<PackedPackage, String> {
    // owned 生命周期：nodes/dynamic 需在 write_package 借用时存活，故先全部收集进 owned Vec。
    let mut owned: Vec<(String, Vec<TemplateNode>, loomgui_core::style::dynamic::DynamicRuleTable)> =
        Vec::with_capacity(html_files.len());
    let mut manifest: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for hf in html_files {
        let html_path = source_dir.join(hf);
        let html = std::fs::read_to_string(&html_path)
            .map_err(|e| format!("read {}: {e}", html_path.display()))?;
        // 1. 抽 CSS（<style> + <link>）—— parse_html 前调（围栏白名单挡 style/link）。
        let css = extract_component_css(&html, source_dir);
        // 2. 剥 style/link 后再 parse_html（否则围栏报错）。
        let stripped = strip_style_and_link(&html);
        let tree = loomgui_core::parse::dom::parse_html(&stripped)
            .map_err(|e| format!("parse_html {hf}: {e}"))?;
        let sheet = loomgui_core::parse::css::parse_css(&css)
            .map_err(|e| format!("parse_css {hf}: {e}"))?;
        let dynamic = loomgui_core::asset::extract_dynamic_rules(&sheet);
        let styles = loomgui_core::style::cascade::resolve_styles(&tree, &sheet);
        let scene = loomgui_core::scene::build_scene(&tree, &styles);
        let nodes = scene_to_template(&scene, res_dir, &mut manifest, &mut seen);
        let comp_name = hf
            .strip_suffix(".html")
            .unwrap_or(hf)
            .to_string();
        owned.push((comp_name, nodes, dynamic));
    }

    // 组 PackageInput（借用 owned）→ write_package。
    let comp_refs: Vec<(&str, &[TemplateNode], &loomgui_core::style::dynamic::DynamicRuleTable)> =
        owned
            .iter()
            .map(|(name, nodes, dyn_rules)| (name.as_str(), nodes.as_slice(), dyn_rules))
            .collect();
    let input = PackageInput {
        components: comp_refs,
        asset_manifest: &manifest,
    };
    let pkg_bytes = loomgui_core::asset::write_package(&input);

    Ok(PackedPackage {
        pkg_bytes,
        asset_manifest: manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_style_and_link_removes_style_and_link_elements() {
        let html = r#"<div class="a"><style>.a { color: red; }</style><span>hi</span><link rel="stylesheet" href="x.css"></div>"#;
        let stripped = strip_style_and_link(html);
        assert!(!stripped.contains("style"), "style 元素已剥: {stripped}");
        assert!(!stripped.contains("link"), "link 元素已剥: {stripped}");
        assert!(stripped.contains("hi"), "正文保留: {stripped}");
        assert!(stripped.contains("<div"), "div 保留: {stripped}");
        assert!(stripped.contains("<span"), "span 保留: {stripped}");
    }

    #[test]
    fn strip_style_and_link_preserves_img_src() {
        let html = r#"<div><img src="res/x.png"></div>"#;
        let stripped = strip_style_and_link(html);
        assert!(stripped.contains("res/x.png"), "img src 保留");
    }

    #[test]
    fn scene_to_template_normalizes_img_src_and_collects_manifest() {
        // 手搓 scene：root + img 子（src="res/icons/skin.png"）
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Image { src: "res/icons/skin.png".into() }, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let scene = Scene::build(&entries);
        let mut manifest = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let nodes = scene_to_template(&scene, "res", &mut manifest, &mut seen);
        assert_eq!(manifest, vec!["icons/skin.png".to_string()], "归一化 path 进 manifest");
        // 节点 src 也被归一化
        match &nodes[1].kind {
            NodeKind::Image { src } => assert_eq!(src, "icons/skin.png", "节点 src 归一化"),
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn scene_to_template_dedups_same_src_across_nodes() {
        // 两 img 同 src → manifest 只入一次
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Image { src: "res/a.png".into() }, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Image { src: "res/a.png".into() }, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let scene = Scene::build(&entries);
        let mut manifest = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let _ = scene_to_template(&scene, "res", &mut manifest, &mut seen);
        assert_eq!(manifest.len(), 1, "同 src 去重只入一次");
    }

    #[test]
    fn scene_to_template_skips_src_outside_res_with_warning() {
        // src 不在 res 下 → None → 不入 manifest（不 Err）
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Image { src: "other/foo.png".into() }, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let scene = Scene::build(&entries);
        let mut manifest = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let nodes = scene_to_template(&scene, "res", &mut manifest, &mut seen);
        assert!(manifest.is_empty(), "res 外 src 不入 manifest");
        // 节点 src 保持原样（未归一化）
        match &nodes[1].kind {
            NodeKind::Image { src } => assert_eq!(src, "other/foo.png", "未归一化的 src 保持原样"),
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn scene_to_template_parent_idx_maps_to_position() {
        // root(parent=None) + child(parent=root) → child parent_idx=Some(0)
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Text { content: "hi".into() }, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let scene = Scene::build(&entries);
        let mut manifest = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let nodes = scene_to_template(&scene, "res", &mut manifest, &mut seen);
        assert_eq!(nodes[0].parent_idx, None, "root parent=None");
        assert_eq!(nodes[1].parent_idx, Some(0), "child parent_idx=Some(0)");
    }
}
