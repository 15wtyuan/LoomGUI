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
    /// v1d.2：set_key_input 缓存的本帧键盘输入；tick 消费后 clear。
    pub pending_keys: Vec<crate::input::KeyEvent>,
    /// v1d.2：编程聚焦/清焦点请求（request_focus/blur tick 外调记，tick 最前消费）。
    /// 外层 Some=有请求；内层 Some(id)=聚焦某节点 / None=清焦点。
    pub pending_focus_request: Option<Option<NodeId>>,
    /// v1d.4：tween 引擎（每 tick update 写 scene.anim + 产 complete 事件）。
    pub tweens: crate::tween::TweenManager,
    /// v1d.4：advance_time stash 的本帧 dt（tick_and_render 消费，喂 tweens.update）。
    pub pending_dt: f32,
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
            pending_keys: Vec::new(),
            pending_focus_request: None,
            tweens: crate::tween::TweenManager::new(),
            pending_dt: 0.0,
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
        self.tweens.clear();   // v1d.4：旧 tween 指向失效 node_id，随 scene 重建清空
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
        self.tweens.clear();   // v1d.4：旧 tween 指向失效 node_id，随 scene 重建清空
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

    /// 按 CSS id 属性查节点（首个匹配）。无 scene / 无匹配 → None。
    /// 供 FFI find_node_by_id：业务用 id 定位节点（注册 listener / 设 disabled）。
    pub fn find_node_by_id(&self, id: &str) -> Option<NodeId> {
        self.scene.as_ref().and_then(|s| s.find_by_id_attr(id))
    }

    /// UI 挡住时游戏不响应点击（§10.6）。v1c.3：委托 PointerState（任一活跃槽命中非根）。
    pub fn is_pointer_on_ui(&self) -> bool {
        match &self.scene {
            None => false,
            Some(scene) => self.pointer_state.is_pointer_on_ui(scene),
        }
    }

    /// 加 touch monitor（C# CaptureTouch 后经 FFI 调）。
    pub fn add_touch_monitor(&mut self, touch_id: i32, node: NodeId) {
        self.pointer_state.add_touch_monitor(touch_id, node);
    }
    /// 移除 touch monitor（C# 主动释放经 FFI 调）。
    pub fn remove_touch_monitor(&mut self, node: NodeId) {
        self.pointer_state.remove_touch_monitor(node);
    }

    /// v1c.4：累积时间（C# 传 Time.unscaledDeltaTime；双击窗口用）。
    pub fn advance_time(&mut self, dt: f32) {
        self.pointer_state.time_s += dt;
        self.pending_dt = dt;   // v1d.4：stash 给 tick_and_render 喂 tweens.update
    }

    /// v1c.4：外部取消待 click（照 fgui CancelClick）。FFI cancel_click 转发。
    pub fn cancel_click(&mut self, touch_id: i32) {
        self.pointer_state.cancel_click(touch_id);
    }

    /// v1d.2：缓存本帧键盘输入（tick 前调；覆盖式）。
    pub fn set_key_input(&mut self, keys: &[crate::input::KeyEvent]) {
        self.pending_keys.clear();
        self.pending_keys.extend_from_slice(keys);
    }

    /// v1d.2：编程聚焦（照 fgui RequestFocus）。强制聚焦任意非 disabled 节点
    /// （含 tabindex=None/-1——request_focus 是编程 API，不查 tabindex）。
    /// disabled 拒 / 越界跳过。记 pending_focus_request，下 tick 最前消费（不直接写 last_events）。
    pub fn request_focus(&mut self, node_id: NodeId) {
        if let Some(scene) = self.scene.as_ref() {
            if node_id.0 >= scene.nodes.len() {
                return;
            }
            if scene.nodes[node_id.0].disabled {
                return; // §3.5 disabled 拒
            }
        } else {
            return;
        }
        self.pending_focus_request = Some(Some(node_id));
    }

    /// v1d.2：编程清焦点。记 pending_focus_request = Some(None)，下 tick 消费。
    pub fn blur(&mut self) {
        self.pending_focus_request = Some(None);
    }

    /// v1d.4：注册 tween。start/end 取前 value_size 个分量（prop 决定 size）。
    /// duration<=0 → update 首帧即结束并产 complete。无 scene / 越界 node → update 跳过（不报错）。
    #[allow(clippy::too_many_arguments)]   // 参数与 C# FFI 签名 1:1 对齐（同 text/layout.rs 惯例）
    pub fn tween(
        &mut self, node: NodeId, prop: crate::tween::TweenProp,
        start: [f32; 4], end: [f32; 4],
        ease: crate::tween::Ease, delay: f32, duration: f32, tag: u32,
    ) {
        self.tweens.tween(node, prop, start, end, ease, delay, duration, tag);
    }

    /// v1d.4：停该节点该 prop 的 tween（override 保留末值）。
    pub fn kill_tween(&mut self, node: NodeId, prop: crate::tween::TweenProp) {
        self.tweens.kill(node, prop);
    }

    /// v1d.4：清该节点所有动画 override（回 CSS）。
    pub fn clear_anim(&mut self, node: NodeId) {
        if let Some(scene) = self.scene.as_mut() {
            scene.anim.clear_node(node);
        }
    }

    /// v1d.4：清该节点某 prop 对应通道（回 CSS）。
    pub fn clear_anim_prop(&mut self, node: NodeId, prop: crate::tween::TweenProp) {
        if let Some(scene) = self.scene.as_mut() {
            scene.anim.clear_prop(node, prop);
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
        let mut out: Vec<EventRecord> = Vec::new();
        // v1d.4：tween 推进（写 scene.anim + 产 complete 事件进 out）。须在 solve/compute_world_transforms 前。
        let dt = self.pending_dt;
        self.pending_dt = 0.0;
        self.tweens.update(dt, scene, &mut out);
        // v1d.2：消费 pending_focus_request（编程聚焦/清焦点，tick 外 request_focus/blur 记）
        // 最前消费——下 tick 才生效，避免 R3（tick 覆写 last_events 丢请求事件）。
        if let Some(req) = self.pending_focus_request.take() {
            crate::input::focus_node(scene, req, &mut out);
        }
        // 1. solve（先解 layout_rect，hit_test 要用）
        solve(scene, &self.font, self.root_size, &self.textures);
        // v1d.3：solve 后算 world matrix（hit/render 用；命中须用本帧刚 solve 布局）
        crate::scene::transform::compute_world_transforms(scene);
        // 借用冲突解（brief Step 3 注）：process 借 &mut scene + &input——scene 与 pending_input
        // 都是 self 字段，同时借 self 冲突。先 take 出 input（离开 self 借用），process 返回后 drop。
        let input = std::mem::take(&mut self.pending_input);
        let mut ptr_out = self.pointer_state.process(scene, &input);
        out.append(&mut ptr_out);
        // v1d.2：键盘事件（keydown/up + Tab 导航 + FocusIn/Out）
        let keys = std::mem::take(&mut self.pending_keys);
        crate::input::process_keys(scene, &keys, &mut out);
        self.last_events = out;
        // 4. 伪类重匹配（按新 hover/active/focused 改 Node.style——视觉变本帧 render 吃到）
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
        s.scene = Some(crate::scene::node::Scene { roots: vec![], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default() });
        s.set_input(&[crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        s.tick_and_render();
        assert!(!s.is_pointer_on_ui(), "空 scene → false");
    }

    /// v1d.4：tween 经 Stage 公共 API 注册 → advance_time stash dt → tick update 写 anim + 产 complete。
    /// 注：.b 是 CSS class 不是 id 属性，find_node_by_id("b") 返 None。div.b 在 build 序为 NodeId(0)。
    #[test]
    fn stage_tween_advances_opacity_and_emits_complete() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="b"></div>"#;
        let css = ".b{width:100px;height:50px;}";
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(html, css).unwrap();
        // div.b 是 node 0（仅一个元素）；opacity 0→1，1s Linear，tag=99
        s.tween(crate::scene::node::NodeId(0), crate::tween::TweenProp::Opacity,
                [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                crate::tween::Ease::Linear, 0.0, 1.0, 99);
        s.advance_time(0.5);
        s.tick_and_render();
        let op = s.scene.as_ref().unwrap().anim.0[0].opacity;
        assert!((op.unwrap() - 0.5).abs() < 1e-4, "半程 opacity=0.5");
        assert!(s.last_events().iter().all(|e| e.event_type != crate::input::EVT_TWEEN_COMPLETE), "未结束");
        s.advance_time(0.5);
        s.tick_and_render();
        assert!(s.last_events().iter().any(|e| e.event_type == crate::input::EVT_TWEEN_COMPLETE
            && e.touch_id == 99), "结束 → complete(tag=99)");
    }

    /// v1d.4：零回归门——直接 tick_and_render()（不 advance_time）→ pending_dt=0。
    /// 用 delay=1.0 注册 tween：elapsed(0) < delay(1) → update 跳过 apply，opacity 保持 None。
    /// 验证 tween 集成对「不 advance_time」的现有 stage 调用模式无副作用。
    #[test]
    fn stage_tick_without_advance_time_is_zero_regression() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(r#"<div class="b"></div>"#, ".b{width:100px;height:50px;}").unwrap();
        // delay=1.0：dt=0 时 elapsed=0 < delay → 不 apply（若用 delay=0，update 会写 start 值）
        s.tween(crate::scene::node::NodeId(0), crate::tween::TweenProp::Opacity,
                [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                crate::tween::Ease::Linear, 1.0, 1.0, 0);
        s.tick_and_render();   // 无 advance_time → dt=0 → elapsed < delay → 不推进
        assert!(s.scene.as_ref().unwrap().anim.0[0].opacity.is_none(), "dt=0 不写 override");
    }
}
