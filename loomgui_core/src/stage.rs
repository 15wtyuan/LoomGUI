//! Stage 层：串起 parse → style → scene → layout → render 的端到端入口（§4-§6）。
//!
//! v0 内存直通：`load_inline` 吃 HTML+CSS 文本，`tick_and_render` 跑首帧
//! solve + build_render_nodes。`render_json` serde 序列化产 spec §5 JSON。
//! v0 无输入/动画/打包器，Stage 只是「装配 + 单帧」的薄壳。

use crate::input::{EventRecord, PointerEvent, PointerState};
use crate::layout::solve;
#[cfg(feature = "parse")]
use crate::parse::css::parse_css;
#[cfg(feature = "parse")]
use crate::parse::dom::parse_html;
use crate::render::build_render_nodes;
use crate::render::FrameData;
#[cfg(feature = "parse")]
use crate::scene::node::build_scene;
use crate::scene::node::{NodeId, Scene};
use crate::style::dynamic::rematch_pseudo_classes;
#[cfg(feature = "parse")]
use crate::style::cascade::resolve_styles;
use crate::text::layout::Font;
use std::sync::Arc;

pub struct Stage {
    pub scene: Option<Scene>,
    pub font: Arc<Font>,
    pub root_size: (f32, f32),
    pub textures: crate::asset::texture::TextureRegistry, // v1b.2：src→tex_id+维度
    /// v1b.3：图集元数据（.pkg.bin v2 AtlasSection.atlases）。FFI T5 读（atlas_count/info）。
    /// inline 路径恒空（inline 不走打包器，无图集）。
    pub atlases: Vec<crate::asset::AtlasInfo>,
    /// v1c.1：单指针状态机（hover/active 状态 + 命中 diff + 产事件）。
    pub pointer_state: PointerState,
    /// v1c.1：set_input 缓存的本帧输入；tick_and_render 消费后 clear。
    pub pending_input: Vec<PointerEvent>,
    /// v1c.1：本帧 tick 产出的事件序列（process 返回）；last_events/borrow_events 读。
    pub last_events: Vec<EventRecord>,
}

impl Stage {
    pub fn new(font_path: &str, root_size: (f32, f32)) -> Result<Self, String> {
        // Font::from_path 返回 Result<_, String>，直接 ? 传播（原 .map_err(|e| e)? 是 no-op）。
        let font = Font::from_path(font_path)?;
        Ok(Stage {
            scene: None,
            font: Arc::new(font),
            root_size,
            textures: crate::asset::texture::TextureRegistry::default(),
            atlases: Vec::new(),
            pointer_state: PointerState::new(),
            pending_input: Vec::new(),
            last_events: Vec::new(),
        })
    }

    /// v0 内存直通：HTML+CSS 文本直接构 scene（不走打包器）。
    #[cfg(feature = "parse")]
    pub fn load_inline(&mut self, html: &str, css: &str) -> Result<(), String> {
        self.textures.clear();
        self.atlases.clear();
        let tree = parse_html(html)?;
        let sheet = parse_css(css)?;
        let styles = resolve_styles(&tree, &sheet);
        self.scene = Some(build_scene(&tree, &styles));
        Ok(())
    }

    /// 从二进制包加载（spec §8）：read_package → self.scene + root_size（用包 header 的）。
    /// 与 `load_inline` 二选一设 scene；后续 tick_and_render 不变。不需 parse feature。
    ///
    /// v1b.3：read_package 解出 AtlasSection → build_registry 建 TextureRegistry
    /// （atlas[0]→tex_id 1，sprite UV 来自 AtlasSprite），atlas 表存 self.atlases。
    pub fn load_package(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (scene, root_size, atlas_section) =
            crate::asset::read_package(bytes).map_err(|e| e.to_string())?;
        self.textures = crate::asset::build_registry(&atlas_section);
        self.atlases = atlas_section.atlases;
        self.scene = Some(scene);
        self.root_size = root_size;
        Ok(())
    }

    /// 缓存本帧指针输入（tick 前调；覆盖式——每帧全量替换 pending_input）。
    pub fn set_input(&mut self, events: &[PointerEvent]) {
        self.pending_input.clear();
        self.pending_input.extend_from_slice(events);
    }

    /// 业务设节点 disabled（伪类源 + active/click 抑制）。NodeId.0 越界静默跳过。
    pub fn set_node_disabled(&mut self, node_id: NodeId, disabled: bool) {
        if let Some(scene) = self.scene.as_mut() {
            if node_id.0 < scene.nodes.len() {
                scene.nodes[node_id.0].disabled = disabled;
            }
        }
    }

    /// UI 挡住时游戏不响应点击（§10.6）。v1c.3：委托 PointerState（任一活跃槽命中非根）。
    pub fn is_pointer_on_ui(&self) -> bool {
        match &self.scene {
            None => false,
            Some(scene) => self.pointer_state.is_pointer_on_ui(scene),
        }
    }

    /// 本帧产出的事件（tick 后读；FFI borrow_events 用）。
    pub fn last_events(&self) -> &[EventRecord] {
        &self.last_events
    }

    /// 每帧管线（§4.5 + 首帧修正）：
    /// ①solve（算 layout_rect——hit_test 必须用已解矩形，故 solve 在 process 前）
    /// ②process（hit+状态 diff+产事件，存 last_events，更新各槽 last_hit + hovered/active）
    /// ③rematch_pseudo_classes（按新 hover/active 状态改 Node.style——本帧渲染吃到视觉变）
    /// ④build_render_nodes
    ///
    /// **与 spec §4.5 的差异**：spec 原「process→rematch→solve」在首帧 hit_test 读全零
    /// layout_rect（未 solve）→ 无命中 → 1 tick 出不来事件。本实现把 solve 前移到 process
    /// 前——hit_test 用本帧刚解的矩形，首帧即能产 RollOver。代价：rematch 改的 *layout*
    /// 属性（如 `.btn:hover { width:200px }`）延到下帧 solve 才吃（spec §15 已认此为延迟
    /// 项）；rematch 的纯视觉变（background-color 等）本帧 render 立即吃到（render 读 style
    /// 不读 layout_rect.size）。v1c.1 视觉伪类为主，layout 伪类为边角，此权衡 OK。
    pub fn tick_and_render(&mut self) -> FrameData {
        let scene = self.scene.as_mut().expect("load first");
        // 1. solve（先解 layout_rect，hit_test 要用）
        solve(scene, &self.font, self.root_size, &self.textures);
        // 借用冲突解（brief Step 3 注）：process 借 &mut scene + &input——scene 与 pending_input
        // 都是 self 字段，同时借 self 冲突。先 take 出 input（离开 self 借用），process 返回后 drop。
        let input = std::mem::take(&mut self.pending_input);
        self.last_events = self.pointer_state.process(scene, &input);
        // 3. cur_hit 已在 process 内更新各槽 last_hit；is_pointer_on_ui 读各槽
        // 4. 伪类重匹配（按新 hover/active 改 Node.style——视觉变本帧 render 吃到）
        rematch_pseudo_classes(scene);
        // 5. 渲染
        build_render_nodes(scene, &self.font, &self.textures)
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
        s_inline.textures.insert("logo.png", crate::asset::texture::TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 64, height: 32 }); // v1b.2：强化真实 tex_id + 真实尺寸路径
        let inline_json = s_inline.render_json();

        // 序列化 inline 的 scene → 包（v2：空 AtlasSection——此测手工 insert 真 TexMeta，
        // 不走 build_registry 路径，故传空 atlas section；包路径 load_package 时 build_registry
        // 建空 registry，再手工 insert 覆盖为真实 tex_id，与 inline 路径对齐）
        let scene = s_inline.scene.as_ref().unwrap();
        let pkg = crate::asset::write_package(scene, (200.0, 100.0), &crate::asset::AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());

        // 包路径（新 Stage，同字体，同纹理注册）
        let mut s_pkg = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_pkg.load_package(&pkg).unwrap();
        s_pkg.textures.insert("logo.png", crate::asset::texture::TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 64, height: 32 });
        let pkg_json = s_pkg.render_json();

        assert_eq!(inline_json, pkg_json, "包路径渲染输出必须 == inline（含真实 tex_id + 尺寸）");
    }

    #[cfg(feature = "parse")]
    #[test]
    fn set_input_hover_emits_rollover_and_rematch() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        // 按钮 + :hover 规则
        let html = r#"<div class="root"><button class="btn">OK</button></div>"#;
        let css = r#".btn { width: 100px; height: 50px; background-color: #cccccc; } .btn:hover { background-color: #0000ff; }"#;
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(html, css).unwrap();
        // inline 路径 dynamic_rules 空——伪类不生效；打成包验伪类：
        let scene = s.scene.as_ref().unwrap().clone();
        let sheet = crate::parse::css::parse_css(css).unwrap();
        let dynamic = crate::asset::extract_dynamic_rules(&sheet);
        let pkg = crate::asset::write_package(&scene, (200.0, 100.0), &crate::asset::AtlasSection::default(), &dynamic);
        let mut s2 = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s2.load_package(&pkg).unwrap();
        // 输入：Move 到按钮 (50,25)（按钮在 (0,0,100,50)）
        s2.set_input(&[crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        s2.tick_and_render();
        let events = s2.last_events();
        assert!(events.iter().any(|e| e.event_type == crate::input::EVT_ROLL_OVER), "Move 到按钮 → RollOver");
        assert!(s2.is_pointer_on_ui(), "命中按钮 → is_pointer_on_ui=true");
        // hover 后 rematch：btn style.background_color 应变蓝（dynamic 规则 .btn:hover）
        let btn = &s2.scene.as_ref().unwrap().nodes[1];
        assert_eq!(btn.style.background_color, Some([0.0, 0.0, 1.0, 1.0]), ":hover 伪类重匹配 → 蓝");
    }

    #[cfg(feature = "parse")]
    #[test]
    fn set_node_disabled_inhibits_click() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="root"><button class="btn">OK</button></div>"#;
        let css = r#".btn { width: 100px; height: 50px; }"#;
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(html, css).unwrap();
        let scene = s.scene.as_ref().unwrap().clone();
        let pkg = crate::asset::write_package(&scene, (200.0, 100.0), &crate::asset::AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());
        let mut s2 = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s2.load_package(&pkg).unwrap();
        s2.set_node_disabled(crate::scene::node::NodeId(1), true);
        // Down + Up 在按钮上——disabled 不产 Click
        s2.set_input(&[
            crate::input::PointerEvent { kind: crate::input::PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 },
            crate::input::PointerEvent { kind: crate::input::PointerKind::Up, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        s2.tick_and_render();
        let events = s2.last_events();
        assert!(!events.iter().any(|e| e.event_type == crate::input::EVT_CLICK), "disabled → 不产 Click");
    }

    #[test]
    fn is_pointer_on_ui_false_when_miss() {
        // 空 scene / 命中根外 → false。手搓 Stage（不走 parse）
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        // 手搓空 scene
        s.scene = Some(crate::scene::node::Scene { roots: vec![], nodes: vec![], dynamic_rules: Default::default() });
        s.set_input(&[crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        s.tick_and_render();
        assert!(!s.is_pointer_on_ui(), "空 scene → false");
    }
}
