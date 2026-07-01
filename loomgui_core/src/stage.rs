//! Stage 层：串起 parse → style → scene → layout → render 的端到端入口。
//!
//! 内存直通：`load_inline` 吃 HTML+CSS 文本，`tick_and_render` 跑首帧
//! solve + build_render_nodes。`render_json` serde 序列化产渲染 JSON。
//! 无输入/动画/打包器，Stage 只是「装配 + 单帧」的薄壳。

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
    pub textures: crate::asset::texture::TextureRegistry, // src→tex_id+维度
    /// 图集元数据（.pkg.bin AtlasSection.atlases）。FFI atlas_count/info 读。
    /// inline 路径恒空（inline 不走打包器，无图集）。
    pub atlases: Vec<crate::asset::AtlasInfo>,
    /// 单指针状态机（hover/active 状态 + 命中 diff + 产事件）。
    pub pointer_state: PointerState,
    /// set_input 缓存的本帧输入；tick_and_render 消费后 clear。
    pub pending_input: Vec<PointerEvent>,
    /// 本帧 tick 产出的事件序列（process 返回）；last_events/borrow_events 读。
    pub last_events: Vec<EventRecord>,
    /// set_key_input 缓存的本帧键盘输入；tick 消费后 clear。
    pub pending_keys: Vec<crate::input::KeyEvent>,
    /// set_wheel_input 缓存的本帧滚轮输入；tick 消费（apply_wheel_to_hit）后 clear。
    /// 累积式（extend，非 clear-then-set）——多组滚轮合并到一帧。
    pub pending_wheel: Vec<crate::scroll::WheelEvent>,
    /// 编程聚焦/清焦点请求（request_focus/blur tick 外调记，tick 最前消费）。
    /// 外层 Some=有请求；内层 Some(id)=聚焦某节点 / None=清焦点。
    pub pending_focus_request: Option<Option<NodeId>>,
    /// tween 引擎（每 tick update 写 scene.anim + 产 complete 事件）。
    pub tweens: crate::tween::TweenManager,
    /// advance_time stash 的本帧 dt（tick_and_render 消费，喂 tweens.update）。
    pub pending_dt: f32,
    /// 上帧每节点 dirty hash（NodeId 索引）。跨 tick 持续，供 build_render_nodes
    /// 比较决定 emit Unchanged。transient 不进 pkg（Stage 字段非 Scene 字段）。
    /// reload/节点数变 → clear → 下帧全 dirty（无基线）。
    pub prev_node_hashes: Vec<u64>,
}

impl Stage {
    pub fn new(font_path: &str, root_size: (f32, f32)) -> Result<Self, String> {
        // Font::from_path 返回 Result<_, String>，直接 ? 传播。
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
            pending_wheel: Vec::new(),
            pending_focus_request: None,
            tweens: crate::tween::TweenManager::new(),
            pending_dt: 0.0,
            prev_node_hashes: Vec::new(),
        })
    }

    /// 内存直通：HTML+CSS 文本直接构 scene（不走打包器）。
    #[cfg(feature = "parse")]
    pub fn load_inline(&mut self, html: &str, css: &str) -> Result<(), String> {
        self.textures.clear();
        self.atlases.clear();
        let tree = parse_html(html)?;
        let sheet = parse_css(css)?;
        let styles = resolve_styles(&tree, &sheet);
        self.tweens.clear();   // 旧 tween 指向失效 node_id，随 scene 重建清空
        if let Some(scene) = self.scene.as_mut() { scene.scroll.clear(); }   // 旧 scroll 槽随 scene 重建清空（防悬空 NodeId）
        self.prev_node_hashes.clear();   // 旧 hash 基线随 scene 重建失效（防 NodeId 错位）
        self.scene = Some(build_scene(&tree, &styles));
        Ok(())
    }

    /// 从二进制包加载：read_package → self.scene + root_size（用包 header 的）。
    /// 与 `load_inline` 二选一设 scene；后续 tick_and_render 不变。不需 parse feature。
    ///
    /// read_package 解出 AtlasSection → build_registry 建 TextureRegistry
    /// （atlas[0]→tex_id 1，sprite UV 来自 AtlasSprite），atlas 表存 self.atlases。
    pub fn load_package(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (scene, root_size, atlas_section) =
            crate::asset::read_package(bytes).map_err(|e| e.to_string())?;
        self.textures = crate::asset::build_registry(&atlas_section);
        self.atlases = atlas_section.atlases;
        self.tweens.clear();   // 旧 tween 指向失效 node_id，随 scene 重建清空
        if let Some(s) = self.scene.as_mut() { s.scroll.clear(); }   // 旧 scroll 槽随 scene 重建清空（防悬空 NodeId）
        self.prev_node_hashes.clear();   // 旧 hash 基线随 scene 重建失效（防 NodeId 错位）
        self.scene = Some(scene);
        self.root_size = root_size;
        Ok(())
    }

    /// 缓存本帧指针输入（tick 前调；覆盖式——每帧全量替换 pending_input）。
    pub fn set_input(&mut self, events: &[PointerEvent]) {
        self.pending_input.clear();
        self.pending_input.extend_from_slice(events);
    }

    /// 业务设节点 disabled（伪类源 + active/click 抑制）。悬空 NodeId 静默跳过。
    pub fn set_node_disabled(&mut self, node_id: NodeId, disabled: bool) {
        if let Some(scene) = self.scene.as_mut() {
            if let Some(n) = scene.get_mut(node_id) {
                n.disabled = disabled;
            }
        }
    }

    /// 按 CSS id 属性查节点（首个匹配）。无 scene / 无匹配 → None。
    /// 供 FFI find_node_by_id：业务用 id 定位节点（注册 listener / 设 disabled）。
    pub fn find_node_by_id(&self, id: &str) -> Option<NodeId> {
        self.scene.as_ref().and_then(|s| s.find_by_id_attr(id))
    }

    /// UI 挡住时游戏不响应点击。委托 PointerState（任一活跃槽命中非根）。
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

    /// 累积时间（C# 传 Time.unscaledDeltaTime；双击窗口用）。
    pub fn advance_time(&mut self, dt: f32) {
        self.pointer_state.time_s += dt;
        self.pending_dt = dt;   // stash 给 tick_and_render 喂 tweens.update
    }

    /// 外部取消待 click（照 fgui CancelClick）。FFI cancel_click 转发。
    pub fn cancel_click(&mut self, touch_id: i32) {
        self.pointer_state.cancel_click(touch_id);
    }

    /// 缓存本帧键盘输入（tick 前调；覆盖式）。
    pub fn set_key_input(&mut self, keys: &[crate::input::KeyEvent]) {
        self.pending_keys.clear();
        self.pending_keys.extend_from_slice(keys);
    }

    /// 缓存本帧滚轮输入（tick 前调；**累积式** extend——多组滚轮合并）。
    /// wire 进 tick 消费（apply_wheel_to_hit）。
    pub fn set_wheel_input(&mut self, events: &[crate::scroll::WheelEvent]) {
        self.pending_wheel.extend_from_slice(events);
    }

    /// 编程滚动到指定位置。非 scroll 容器 / 越界 node → no-op（不 panic）。
    /// animated=false 直接 snap+clamp；true 启 cubic-out tween（调 set_pos）。
    pub fn set_scroll_pos(&mut self, node: NodeId, x: f32, y: f32, animated: bool) {
        if let Some(scene) = self.scene.as_mut() {
            if scene.get(node).is_some() {
                if let Some(s) = scene.scroll.get_mut(node) {
                    s.set_pos((x, y), animated);
                }
            }
        }
    }

    /// 编程聚焦（照 fgui RequestFocus）。强制聚焦任意非 disabled 节点
    /// （含 tabindex=None/-1——request_focus 是编程 API，不查 tabindex）。
    /// disabled 拒 / 越界跳过。记 pending_focus_request，下 tick 最前消费（不直接写 last_events）。
    pub fn request_focus(&mut self, node_id: NodeId) {
        if let Some(scene) = self.scene.as_ref() {
            match scene.get(node_id) {
                None => return,
                Some(n) if n.disabled => return, // disabled 拒
                _ => {}
            }
        } else {
            return;
        }
        self.pending_focus_request = Some(Some(node_id));
    }

    /// 编程清焦点。记 pending_focus_request = Some(None)，下 tick 消费。
    pub fn blur(&mut self) {
        self.pending_focus_request = Some(None);
    }

    /// 注册 tween。start/end 取前 value_size 个分量（prop 决定 size）。
    /// duration<=0 → update 首帧即结束并产 complete。无 scene / 越界 node → update 跳过（不报错）。
    #[allow(clippy::too_many_arguments)]   // 参数与 C# FFI 签名 1:1 对齐（同 text/layout.rs 惯例）
    pub fn tween(
        &mut self, node: NodeId, prop: crate::tween::TweenProp,
        start: [f32; 4], end: [f32; 4],
        ease: crate::tween::Ease, delay: f32, duration: f32, tag: u32,
    ) {
        self.tweens.tween(node, prop, start, end, ease, delay, duration, tag);
    }

    /// 停该节点该 prop 的 tween（override 保留末值）。
    pub fn kill_tween(&mut self, node: NodeId, prop: crate::tween::TweenProp) {
        self.tweens.kill(node, prop);
    }

    /// 清该节点所有动画 override（回 CSS）。
    pub fn clear_anim(&mut self, node: NodeId) {
        if let Some(scene) = self.scene.as_mut() {
            scene.anim.clear_node(node);
        }
    }

    /// 清该节点某 prop 对应通道（回 CSS）。
    pub fn clear_anim_prop(&mut self, node: NodeId, prop: crate::tween::TweenProp) {
        if let Some(scene) = self.scene.as_mut() {
            scene.anim.clear_prop(node, prop);
        }
    }

    /// 删节点（递归删子 + 联动清 anim/scroll/tween + slotmap remove）。
    /// 旧 NodeId 此后失效（gen++）。无 scene / 已删节点 → no-op。
    /// spec §5.3：删节点联动清持久附属 map，防悬空 NodeId 残留。
    pub fn remove_node(&mut self, node: NodeId) {
        if let Some(scene) = self.scene.as_mut() {
            crate::scene::dynamic::remove_node(scene, &mut self.tweens, node);
        }
    }

    // ---- T6 动态建树 API（转调 scene::dynamic） ----

    /// 建根节点：create_node + roots.push(id)。返回新 NodeId。
    pub fn create_root(&mut self, kind: &str, css: &str) -> Result<NodeId, String> {
        let scene = self.scene.as_mut().ok_or("no scene")?;
        crate::scene::dynamic::create_root(scene, kind, css)
    }

    /// 建节点（不挂父）：kind_from_tag + apply_css 填 base_style + slotmap insert。
    /// 返回新 NodeId，需配合 append_child/insert_before 挂到树。
    pub fn create_node(&mut self, kind: &str, css: &str) -> Result<NodeId, String> {
        let scene = self.scene.as_mut().ok_or("no scene")?;
        crate::scene::dynamic::create_node(scene, kind, css)
    }

    /// 挂子到 parent 末尾。child 必须当前无父。
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), String> {
        crate::scene::dynamic::append_child(self.scene.as_mut().ok_or("no scene")?, parent, child)
    }

    /// 在 parent.children 中 ref_id 之前插 child。ref_id=INVALID → 末尾追加。
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        ref_id: NodeId,
    ) -> Result<(), String> {
        crate::scene::dynamic::insert_before(
            self.scene.as_mut().ok_or("no scene")?,
            parent,
            child,
            ref_id,
        )
    }

    /// 摘子（不删节点）：从 parent.children 移除 + child.parent=None。节点仍 live 可重挂。
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), String> {
        crate::scene::dynamic::remove_child(self.scene.as_mut().ok_or("no scene")?, parent, child)
    }

    /// 改 Text 节点 content + 标 dirty_text。
    pub fn set_text(&mut self, node: NodeId, text: &str) -> Result<(), String> {
        crate::scene::dynamic::set_text(self.scene.as_mut().ok_or("no scene")?, node, text)
    }

    /// 改 Image 节点 src + 标 dirty_mesh。
    pub fn set_src(&mut self, node: NodeId, src: &str) -> Result<(), String> {
        crate::scene::dynamic::set_src(self.scene.as_mut().ok_or("no scene")?, node, src)
    }

    /// 改 base_style（apply_css）+ 标 dirty_mesh。下帧 rematch 从 base 重算 style。
    pub fn set_style(&mut self, node: NodeId, css: &str) -> Result<(), String> {
        crate::scene::dynamic::set_style(self.scene.as_mut().ok_or("no scene")?, node, css)
    }

    /// 测试 helper：建空 scene 的 Stage（不依赖 parse feature）。
    /// 供 T6 动态建树 API 测试用——用 create_root/create_node 返回的 NodeId，不硬编码值。
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 200.0)).unwrap();
        s.scene = Some(crate::scene::node::Scene {
            roots: vec![],
            nodes: slotmap::SlotMap::with_key(),
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(),
            anim: Default::default(),
            scroll: Default::default(),
            text_layouts: Vec::new(),
        });
        s
    }

    /// 本帧产出的事件（tick 后读；FFI borrow_events 用）。
    pub fn last_events(&self) -> &[EventRecord] {
        &self.last_events
    }

    /// 每帧管线：
    /// ①tween ②focus_request ③solve ④refresh_content_sizes
    /// ⑤process（仲裁+拖拽跟手写 scroll_pos；hit_test 读上帧 world_transforms，1 帧延迟
    ///   已认） ⑥scroll update（消费 pending_wheel + inertia/bounce advance）
    /// ⑦process_keys ⑧compute_world_transforms（process/scroll 后：读 scroll_pos 同帧
    ///   进 world matrix，零拖拽延迟） ⑨rematch_pseudo_classes ⑩build_render_nodes
    ///
    /// **compute_world_transforms 时机**：process/scroll 之后、render 之前，每帧 1 次
    /// （不再"末尾 + 首帧 guard"）。scroll_pos 同帧进 world matrix（spec §9.3）。
    /// **1 帧延迟语义**：hit_test 用上帧 world_transforms。首帧 world_transforms 为空，
    /// hit_test bounds guard 拦截（越界返 None → 未命中，零回归安全）。仲裁在 Down 未滚动前
    /// 不影响；clip 门控用 viewport 固定主导，不依赖每帧变换精度。
    pub fn tick_and_render(&mut self) -> FrameData {
        let scene = self.scene.as_mut().expect("load first");
        let mut out: Vec<EventRecord> = Vec::new();
        // tween 推进（写 scene.anim + 产 complete 事件进 out）。须在 solve/compute_world_transforms 前。
        let dt = self.pending_dt;
        self.pending_dt = 0.0;
        self.tweens.update(dt, scene, &mut out);
        // 消费 pending_focus_request（编程聚焦/清焦点，tick 外 request_focus/blur 记）。
        // 最前消费——下 tick 才生效，避免 tick 覆写 last_events 丢请求事件。
        if let Some(req) = self.pending_focus_request.take() {
            crate::input::focus_node(scene, req, &mut out);
        }
        // 1. solve（先解 layout_rect，hit_test 要用）
        solve(scene, &self.font, self.root_size, &self.textures);
        // 2. content_size 填充（solve 后 content_size/viewport/overlap）
        crate::scroll::refresh_content_sizes(scene);
        // 3. process（仲裁 + 拖拽跟手写 scroll_pos）
        // hit_test 读上帧 world_transforms（1 帧延迟，已认）——首帧 world_transforms 为空，
        // hit_test bounds guard 拦截（越界返 None → 未命中，零回归安全）。
        // 借用冲突解：process 借 &mut scene + &input——scene 与 pending_input 都是 self 字段，
        // 同时借 self 冲突。先 take 出 input（离开 self 借用），process 返回后 drop。
        let input = std::mem::take(&mut self.pending_input);
        let mut ptr_out = self.pointer_state.process(scene, &input);
        out.append(&mut ptr_out);
        // 4. scroll.update（消费 pending_wheel + 惯性/回弹 advance）
        let wheels = std::mem::take(&mut self.pending_wheel);
        for w in &wheels {
            crate::scroll::apply_wheel_to_hit(scene, *w);
        }
        crate::scroll::advance_all(dt, scene);
        // 5. 键盘事件（keydown/up + Tab 导航 + FocusIn/Out）
        let keys = std::mem::take(&mut self.pending_keys);
        crate::input::process_keys(scene, &keys, &mut out);
        // 6. compute_world_transforms（读 scroll_pos，offset 同帧生效）
        crate::scene::transform::compute_world_transforms(scene);
        self.last_events = out;
        // 7. 伪类重匹配（按新 hover/active/focused 改 Node.style——视觉变本帧 render 吃到）
        rematch_pseudo_classes(scene);
        // 8. 渲染（+ 合成 scrollbar）。传上帧 hash 基线，未变节点 emit Unchanged；
        //    返回新 hash 存 self.prev_node_hashes 供下帧比。
        let (frame, new_hashes) = build_render_nodes(scene, &self.font, &self.textures, &self.prev_node_hashes);
        self.prev_node_hashes = new_hashes;
        frame
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
    /// fixture（div + 文本 + img + rect mask）经 pkg→load_package→render_json
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
        s_inline.textures.insert("logo.png", crate::asset::texture::TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 64, height: 32 }); // 强化真实 tex_id + 真实尺寸路径
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
        // 预热 tick：compute_world_transforms 在 process/scroll 后跑，hit_test 读上帧 world_transforms
        // （1 帧延迟语义，T4）。首帧 world_transforms 空 → 首帧 hit_test 全 None，故输入前先 warmup。
        s2.tick_and_render();
        // 输入：Move 到按钮 (50,25)（按钮在 (0,0,100,50)）
        s2.set_input(&[crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        s2.tick_and_render();
        let events = s2.last_events();
        assert!(events.iter().any(|e| e.event_type == crate::input::EVT_ROLL_OVER), "Move 到按钮 → RollOver");
        assert!(s2.is_pointer_on_ui(), "命中按钮 → is_pointer_on_ui=true");
        // hover 后 rematch：btn style.background_color 应变蓝（dynamic 规则 .btn:hover）
        let scene = s2.scene.as_ref().unwrap();
        let btn_id = scene.get(scene.roots[0]).unwrap().children[0];
        let btn = scene.get(btn_id).unwrap();
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
        // btn = root 的首个子（root=Container, btn=Button, btn 的 Text 子）
        let btn_id = {
            let sc = s2.scene.as_ref().unwrap();
            sc.get(sc.roots[0]).unwrap().children[0]
        };
        s2.set_node_disabled(btn_id, true);
        // warmup tick：compute_world_transforms 在 process/scroll 后跑，hit_test 读上帧
        // world_transforms（1 帧延迟语义，T4）。首帧 world_transforms 空 → 首帧 hit_test 全 None，
        // 故输入前先 warmup，否则 Down 落不到 btn → 测因"未命中"通过而非"disabled 抑制"（T4 Minor-1）。
        s2.tick_and_render();
        // 命中前置断言：Move 到按钮 (50,25)（按钮在 (0,0,100,50)）→ is_pointer_on_ui=true
        // 证明按钮被几何命中，disabled 才有抑制对象（否则"无 Click"是因未命中，非 disabled 抑制）。
        s2.set_input(&[
            crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        s2.tick_and_render();
        assert!(s2.is_pointer_on_ui(), "Move 到按钮 → 命中 UI（命中前置：证明按钮被几何命中）");
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
        // 手搓空 scene（SlotMap::with_key()）
        s.scene = Some(crate::scene::node::Scene {
            roots: vec![],
            nodes: slotmap::SlotMap::with_key(),
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        });
        s.set_input(&[crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        s.tick_and_render();
        assert!(!s.is_pointer_on_ui(), "空 scene → false");
    }

    /// load 时 scroll 表清空（防 reload 后旧容器 NodeId 悬空，同 tween clear）。
    /// 塞 scroll_pos 后 reload → scroll 表为空（get 返 None）；重新 ensure 后归零。
    #[cfg(feature = "parse")]
    #[test]
    fn load_clears_scroll_state() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="c"></div>"#;
        let css = ".c{width:200px;height:100px;overflow:scroll;}";
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(html, css).unwrap();
        let root_id = s.scene.as_ref().unwrap().roots[0];
        // 手动塞 scroll_pos，模拟上一会话残留
        s.scene.as_mut().unwrap().scroll.ensure(root_id).scroll_pos = (50.0, 50.0);
        // reload → scroll 表应被清
        s.load_inline(html, css).unwrap();
        assert!(s.scene.as_ref().unwrap().scroll.get(root_id).is_none(),
            "reload 后 scroll 表清空，旧 NodeId 槽不存在");
    }

    /// tween 经 Stage 公共 API 注册 → advance_time stash dt → tick update 写 anim + 产 complete。
    /// 注：.b 是 CSS class 不是 id 属性，find_node_by_id("b") 返 None。div.b 是唯一根节点。
    #[test]
    fn stage_tween_advances_opacity_and_emits_complete() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="b"></div>"#;
        let css = ".b{width:100px;height:50px;}";
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(html, css).unwrap();
        let rid = s.scene.as_ref().unwrap().roots[0];
        // opacity 0→1，1s Linear，tag=99
        s.tween(rid, crate::tween::TweenProp::Opacity,
                [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                crate::tween::Ease::Linear, 0.0, 1.0, 99);
        s.advance_time(0.5);
        s.tick_and_render();
        let op = s.scene.as_ref().unwrap().anim.0.get(&rid).and_then(|a| a.opacity);
        assert!((op.unwrap() - 0.5).abs() < 1e-4, "半程 opacity=0.5");
        assert!(s.last_events().iter().all(|e| e.event_type != crate::input::EVT_TWEEN_COMPLETE), "未结束");
        s.advance_time(0.5);
        s.tick_and_render();
        assert!(s.last_events().iter().any(|e| e.event_type == crate::input::EVT_TWEEN_COMPLETE
            && e.touch_id == 99), "结束 → complete(tag=99)");
    }

    /// 直接 tick_and_render()（不 advance_time）→ pending_dt=0。
    /// 用 delay=1.0 注册 tween：elapsed(0) < delay(1) → update 跳过 apply，opacity 保持 None。
    /// 验证 tween 集成对「不 advance_time」的现有 stage 调用模式无副作用。
    #[test]
    fn stage_tick_without_advance_time_is_zero_regression() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(r#"<div class="b"></div>"#, ".b{width:100px;height:50px;}").unwrap();
        let rid = s.scene.as_ref().unwrap().roots[0];
        // delay=1.0：dt=0 时 elapsed=0 < delay → 不 apply（若用 delay=0，update 会写 start 值）
        s.tween(rid, crate::tween::TweenProp::Opacity,
                [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                crate::tween::Ease::Linear, 1.0, 1.0, 0);
        s.tick_and_render();   // 无 advance_time → dt=0 → elapsed < delay → 不推进
        assert!(s.scene.as_ref().unwrap().anim.0.get(&rid).is_none(), "dt=0 不写 override（HashMap 无条目）");
    }

    /// Critical-1 回归：tween 写 scene.anim（用 id.index()）→ render 读 anim.opacity
    /// （AnimTable::get 现用 node.index()）→ frame.nodes[该节点].alpha 吃到 override。
    /// 堵「tween 写入正确但 render 读取失败」盲区（旧 bug：AnimTable::get 用 node.0 as usize，
    /// 打包 NodeId.0=4097 越界 → anim override 在渲染层丢失 → alpha 退回 CSS 默认 1.0）。
    #[test]
    fn tween_anim_override_visible_in_render_output() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_inline(r#"<div class="b"></div>"#, ".b{width:100px;height:50px;}").unwrap();
        let rid = s.scene.as_ref().unwrap().roots[0];
        // tween opacity 0→0.5，delay=0、duration=1.0、Linear。
        s.tween(rid, crate::tween::TweenProp::Opacity,
                [0.0, 0.0, 0.0, 0.0], [0.5, 0.0, 0.0, 0.0],
                crate::tween::Ease::Linear, 0.0, 1.0, 0);
        // 推进整段 duration → tt=1.0 → Linear 插值末值 0.5。
        s.advance_time(1.0);
        let frame = s.tick_and_render();
        // 唯一根节点 → frame.nodes[pos=0]。断言 render 输出吃到 anim override（alpha=0.5），
        // 不是只断言 anim 表内值——确保读写对称贯穿到渲染层。
        assert!((frame.nodes[0].alpha - 0.5).abs() < 1e-5,
                "tween anim.opacity override 应在 render 输出可见：alpha={}（期望 0.5）",
                frame.nodes[0].alpha);
    }

    /// 拖拽滚动容器 → 同 tick world_transforms 已含 scroll_pos（零延迟）。
    /// process 写 scroll_pos（drag_follow）→ compute_world_transforms 在 process 后读 scroll_pos
    /// → world matrix 含 T(-scroll_pos) offset。
    #[cfg(feature = "parse")]
    #[test]
    fn drag_follow_visible_same_frame_in_world_transforms() {
        use crate::transform::Affine2Ext;
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="scroll"><div class="content"></div></div>"#;
        let css = r#".scroll{width:200px;height:200px;overflow:scroll;} .content{width:50px;height:400px;flex-shrink:0;}"#;
        let mut s = Stage::new(font_path, (200.0, 200.0)).unwrap();
        s.load_inline(html, css).unwrap();
        // 首 tick 建立 layout + content_size/overlap
        s.tick_and_render();
        // feed 拖拽输入（mouse touch_id=-1，dy=20 > SCROLL_THRESHOLD_MOUSE=8）
        s.advance_time(0.016);
        s.set_input(&[
            crate::input::PointerEvent { kind: crate::input::PointerKind::Down, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 },
            crate::input::PointerEvent { kind: crate::input::PointerKind::Move, x: 25.0, y: 45.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        s.tick_and_render();
        // 子节点 world.apply 反映 scroll_pos（非 0）
        let scene = s.scene.as_ref().unwrap();
        // content 子 = root 的首个子；drag 下拖 → scroll_pos.y 减（越界打折负值）→ world y 反映（≠0）
        let content_id = scene.get(scene.roots[0]).unwrap().children[0];
        let (_x, y) = scene.world_transforms[content_id.index()].apply_point(0.0, 0.0);
        assert!(y != 0.0, "拖拽同帧进 world matrix：y={}", y);
    }

    /// T4：compute_world_transforms 在 render 前每帧跑（不再"末尾+首帧 guard"）。
    /// tick 后 world_transforms 应非空——证明 compute 在 render 前执行过。
    #[cfg(feature = "parse")]
    #[test]
    fn tick_computes_world_transforms_before_render() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 200.0)).unwrap();
        s.load_inline(r#"<div class="c"></div>"#, ".c{width:100px;height:50px;}").unwrap();
        s.tick_and_render();
        // tick 后 world_transforms 应非空（compute 在 render 前跑过）
        assert!(!s.scene.as_ref().unwrap().world_transforms.is_empty(),
            "compute_world_transforms 在 render 前跑过");
    }

    /// T4：hit_test 在 world_transforms 空/未对齐时不 panic（bounds guard 拦截）。
    /// 结构变更帧新增节点本帧 world_transforms 未算 → 未命中（1 帧延迟语义），不越界 panic。
    #[test]
    fn hit_test_bounds_guard_no_panic_on_empty_worlds() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 200.0)).unwrap();
        // 手搓 scene：1 个 touchable 节点（root，覆盖点 50,50）但 world_transforms 空。
        // hit_subtree 走到 bounds guard（id.index() >= world_transforms.len()）→ 返 None，不 panic。
        use crate::scene::node::{Node, NodeKind, Rect, Scene};
        use crate::style::resolved::ResolvedStyle;
        let mut root = Node::default();
        root.kind = NodeKind::Container;
        root.style = ResolvedStyle::default();
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        root.touchable = true;
        let scene = Scene::from_nodes(vec![root], vec![]);
        s.scene = Some(scene);
        // world_transforms 空（未 compute）→ hit_test bounds guard 应拦截，不 panic，返 None
        let hit = crate::hit::hit_test(s.scene.as_ref().unwrap(), (50.0, 50.0));
        assert_eq!(hit, None, "world_transforms 空 → bounds guard 返 None（未命中，1 帧延迟语义）");
    }

    /// T5：remove_node 后 tick_and_render 不 panic（容量化并行数组防越界）。
    /// 删中间节点产生 slotmap 间隙 → 高 idx live 节点 id.index() > nodes.len()。
    /// 若 world_transforms/taffy_ids/text_layouts 按存活数(len)分配 → 越界 panic。
    /// T5 改按 capacity+1 分配 → 间隙安全。此测验证整条管线（solve+compute+render）不崩。
    #[cfg(feature = "parse")]
    #[test]
    fn remove_node_then_tick_does_not_panic_on_slot_gap() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        // 4 节点：root + 3 子（a, b, c），删 b（中间）→ a/c 仍 live，c 在高 idx。
        let html = r#"<div class="root"><div class="a"></div><div class="b"></div><div class="c"></div></div>"#;
        let css = ".root{width:200px;height:200px;} .a,.b,.c{width:50px;height:50px;}";
        let mut s = Stage::new(font_path, (200.0, 200.0)).unwrap();
        s.load_inline(html, css).unwrap();
        s.tick_and_render();   // 首帧：建 world_transforms 基线
        // 取 b 的 NodeId（root 的第 2 个 div 子——注意 root 的 Text 子不在这里，3 个 div 子直接挂 root）
        let scene = s.scene.as_ref().unwrap();
        let root_id = scene.roots[0];
        let div_kids: Vec<_> = scene.get(root_id).unwrap().children.iter()
            .filter(|&&c| matches!(scene.get(c).unwrap().kind, crate::scene::node::NodeKind::Container))
            .copied().collect();
        assert_eq!(div_kids.len(), 3, "3 个 div 子");
        let b_id = div_kids[1];
        // 删 b（中间子）→ slotmap 间隙
        s.remove_node(b_id);
        // tick + render：solve/compute_world_transforms/build_render_nodes 全跑，不应越界 panic
        s.tick_and_render();
        // b 已删（旧 NodeId 失效），a/c 仍 live
        let scene = s.scene.as_ref().unwrap();
        assert!(scene.get(b_id).is_none(), "b 删除后旧 NodeId 失效");
        assert!(scene.get(div_kids[0]).is_some(), "a 仍 live");
        assert!(scene.get(div_kids[2]).is_some(), "c 仍 live（高 idx，间隙后仍可索引）");
        // 再 tick 一帧确认稳定（world_transforms 已按新容量重算）
        s.tick_and_render();
    }
}

/// T6 动态建树 API 测试（不依赖 parse feature——runtime API 可用性门）。
/// 用 Stage::new_for_test() 建空 scene，用 create_root/create_node 返回的 NodeId，不硬编码值。
#[cfg(test)]
mod dynamic_tests {
    use super::*;
    use crate::scene::node::NodeKind;

    #[test]
    fn create_node_and_append_builds_tree() {
        let mut s = Stage::new_for_test();
        let root = s.create_root("div", "width:100px;height:100px").unwrap();
        let child = s.create_node("div", "width:50px;height:50px").unwrap();
        s.append_child(root, child).unwrap();
        let sc = s.scene.as_ref().unwrap();
        assert_eq!(sc.roots, vec![root]);
        assert_eq!(sc.get(root).unwrap().children, vec![child]);
        assert_eq!(sc.get(child).unwrap().parent, Some(root));
        // CSS 应用生效：base_style width 100px
        use taffy::style::Dimension;
        assert!(matches!(
            sc.get(root).unwrap().base_style.taffy_style.size.width,
            Dimension::Length(100.0)
        ));
    }

    #[test]
    fn set_text_changes_content_and_marks_dirty() {
        let mut s = Stage::new_for_test();
        let t = s.create_node("span", "").unwrap();
        // create_node 时 Text 节点 dirty_text=true，先清掉验 set_text 重标
        s.scene.as_mut().unwrap().get_mut(t).unwrap().dirty_text = false;
        s.set_text(t, "hello").unwrap();
        let sc = s.scene.as_ref().unwrap();
        assert!(sc.get(t).unwrap().dirty_text);
        match &sc.get(t).unwrap().kind {
            NodeKind::Text { content } => assert_eq!(content, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn set_style_changes_base_style() {
        let mut s = Stage::new_for_test();
        let n = s.create_node("div", "").unwrap();
        s.set_style(n, "background-color:#ff0000").unwrap();
        let bg = s.scene.as_ref().unwrap().get(n).unwrap().base_style.background_color;
        assert_eq!(bg, Some([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn remove_child_detaches_but_keeps_node() {
        let mut s = Stage::new_for_test();
        let root = s.create_root("div", "").unwrap();
        let child = s.create_node("div", "").unwrap();
        s.append_child(root, child).unwrap();
        s.remove_child(root, child).unwrap();
        let sc = s.scene.as_ref().unwrap();
        assert!(sc.get(root).unwrap().children.is_empty());
        assert!(
            sc.get(child).unwrap().parent.is_none(),
            "child 变孤立但仍存活"
        );
        assert!(sc.get(child).is_some());
    }

    /// 动态建树后 tick_and_render 正确渲染（layout solve 每帧从零建 taffy 树，自动跟进结构变更）。
    /// 核心不变量：动态建的树经完整管线（solve+compute+render）不 panic，frame 产出。
    /// 注：merge_meshes 会把同 DrawState 的 Mesh 节点合并 → frame.nodes.len() 可小于节点数，
    /// 故只断言 frame 非空 + 至少一个 Mesh 含几何（证明渲染吃到动态建的树）。
    #[test]
    fn dynamic_tree_tick_and_render_does_not_panic() {
        let mut s = Stage::new_for_test();
        let root = s.create_root("div", "width:200px;height:200px").unwrap();
        let child = s.create_node("div", "width:100px;height:100px;background-color:#00ff00").unwrap();
        s.append_child(root, child).unwrap();
        // 完整管线跑一遍：solve 建 taffy 树 + compute_world_transforms + render
        let frame = s.tick_and_render();
        // frame 非空 + 至少一个 Mesh 含顶点（root/child 合并后仍应有几何）
        assert!(!frame.nodes.is_empty(), "动态建的树应渲染出节点");
        let has_mesh = frame.nodes.iter().any(|rn| {
            matches!(&rn.payload, crate::render::node::NodePayload::Mesh { verts, .. } if !verts.is_empty())
        });
        assert!(has_mesh, "应有含几何的 Mesh 节点（动态树渲染产出）");
        // 再 tick 一帧（dirty 标志清后稳定，仍不 panic）
        s.tick_and_render();
    }

    /// set_text 后 tick_and_render 重算文本（dirty_text → render 重测）。
    #[test]
    fn set_text_then_tick_renders() {
        let mut s = Stage::new_for_test();
        let t = s.create_node("span", "width:100px;height:20px").unwrap();
        s.set_text(t, "hi").unwrap();
        let frame = s.tick_and_render();
        // span 节点应进 frame
        assert!(frame.nodes.len() >= 1);
    }

    /// create_node 拒绝未知 tag。
    #[test]
    fn create_node_rejects_unknown_tag() {
        let mut s = Stage::new_for_test();
        assert!(s.create_node("ul", "").is_err());
    }

    /// insert_before 中间插入经 Stage API。
    #[test]
    fn stage_insert_before_middle() {
        let mut s = Stage::new_for_test();
        let root = s.create_root("div", "").unwrap();
        let a = s.create_node("div", "").unwrap();
        let b = s.create_node("div", "").unwrap();
        let c = s.create_node("div", "").unwrap();
        s.append_child(root, a).unwrap();
        s.append_child(root, b).unwrap();
        s.insert_before(root, c, a).unwrap();
        let sc = s.scene.as_ref().unwrap();
        assert_eq!(sc.get(root).unwrap().children, vec![c, a, b]);
    }
}
