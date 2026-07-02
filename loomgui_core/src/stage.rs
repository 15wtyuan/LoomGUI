//! Stage 层：串起 parse → style → scene → layout → render 的端到端入口。
//!
//! v1.4-a 资源池模型：`load_package(name, bytes)` 进 `packages` 字典不建 scene（D3）；
//! scene 由 `create_root`/`create_node` 建（v1.3+ 动态树 API）。`tick_and_render` 跑
//! solve + build_render_nodes。`render_json` serde 序列化产渲染 JSON。

use crate::input::{EventRecord, PointerEvent, PointerState};
use crate::layout::solve;
use crate::render::build_render_nodes;
use crate::render::FrameData;
use crate::scene::node::{NodeId, Scene};
use crate::style::dynamic::rematch_pseudo_classes;
use crate::text::layout::Font;
use std::sync::Arc;

pub struct Stage {
    pub scene: Option<Scene>,
    pub font: Arc<Font>,
    pub root_size: (f32, f32),
    /// 资源池：pkg_name → Package（多包共存）。load_package 填，instantiate 读。
    /// v1.4-a：load_package 不建 scene，只填本字典（spec §4.1 D3）。
    pub packages: std::collections::HashMap<String, crate::asset::Package>,
    /// D17 图尺寸表：归一化 path → (w, h) 像素（打包期 PNG IHDR 静态数据）。
    /// `load_package` 时从所有包的 `asset_manifest` 合并填入（多包共存，path 全局唯一）。
    /// `solve`/`build_render_nodes` 查此表算 Image intrinsic 尺寸（measure 三档）+ 九宫格 UV。
    /// path 缺失或 w/h=0 → fallback 64×64（核心不知图集，但知图尺寸）。
    pub image_sizes: std::collections::HashMap<String, (u32, u32)>,
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
            packages: std::collections::HashMap::new(),
            image_sizes: std::collections::HashMap::new(),
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

    /// 加载包进资源池（不碰 scene）。重复 load 同名包 = 替换。多包共存。
    ///
    /// v1.4-a（spec §4.2 D3）：`load_package(name, bytes)` 解析 pkg.bin → Package，
    /// 存进 `self.packages[name]`。**不建 scene**——加载与实例化解耦（fgui/Unity prefab 模型）。
    /// scene 由 `create_root`/`create_node` 建；组件实例化由 `instantiate`（Task 5）做。
    /// `root_size` 归 Stage（不从包来，D9）；图集归 Unity（核心不知图集，D8）。
    ///
    /// **D17（图尺寸）**：同步把本包 `asset_manifest` 的 `path → (w,h)` 合并进 `self.image_sizes`
    /// （多包共存，path 全局唯一）。`solve`/`build_render_nodes` 查此表算 Image intrinsic +
    /// 九宫格 UV。重复 load 同名包 → 旧包的 path 条目被新包覆盖（path 全局唯一，语义安全）。
    pub fn load_package(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let mut pkg = crate::asset::read_package(bytes).map_err(|e| e.to_string())?;
        pkg.name = name.to_string(); // read_package 填空串，这里覆盖为真实包名
        // D17：把本包 manifest 的 path → (w,h) 合并进全局尺寸表。
        // 重复 load 同名包前，先清旧包的 path 条目（避免旧 path 残留——虽然 path 全局唯一，
        // 但若旧包有 path 而新包没有，旧条目会悬空）。简单实现：直接 extend 覆盖（同 path 后写赢）。
        for entry in &pkg.asset_manifest {
            self.image_sizes.insert(entry.path.clone(), (entry.w, entry.h));
        }
        self.packages.insert(name.to_string(), pkg);
        Ok(())
    }

    /// D17：查图尺寸（path → (w, h) 像素）。供 layout/render 用。
    /// path 缺失或 w/h=0 → None（调用方 fallback 64×64）。
    pub fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        self.image_sizes.get(path).copied().filter(|(w, h)| *w != 0 && *h != 0)
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

    /// scene 不存在则建空骨架（首次 create_root/create_node 调用时初始化）。
    /// spec §4.2：scene 初始由 create_root 建（load_package 不建 scene）。
    /// 多次调用幂等（已存在 scene → no-op）。`pub(crate)` 供集成测试直接初始化场景
    /// （如黄金等价测试需 instantiate 后把孤立根 push 进 scene.roots，不套额外 stage_root）。
    pub(crate) fn ensure_scene(&mut self) {
        if self.scene.is_none() {
            self.scene = Some(crate::scene::node::Scene {
                roots: vec![],
                nodes: slotmap::SlotMap::with_key(),
                dynamic_rules: Default::default(),
                focused_node: None,
                world_transforms: Vec::new(),
                anim: Default::default(),
                scroll: Default::default(),
                text_layouts: Vec::new(),
            });
            self.prev_node_hashes.clear(); // 新 scene → 无基线，下帧全 dirty
        }
    }

    /// 建根节点：create_node + roots.push(id)。返回新 NodeId。
    /// scene 不存在则首次调用建空骨架（spec：scene 初始由 create_root 建）。
    pub fn create_root(&mut self, kind: &str, css: &str) -> Result<NodeId, String> {
        self.ensure_scene();
        let scene = self.scene.as_mut().unwrap();
        crate::scene::dynamic::create_root(scene, kind, css)
    }

    /// 建节点（不挂父）：kind_from_tag + apply_css 填 base_style + slotmap insert。
    /// 返回新 NodeId，需配合 append_child/insert_before 挂到树。
    /// scene 不存在则首次调用建空骨架。
    pub fn create_node(&mut self, kind: &str, css: &str) -> Result<NodeId, String> {
        self.ensure_scene();
        let scene = self.scene.as_mut().unwrap();
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

    /// 从包克隆一个组件进当前 scene，返回组件根 NodeId（孤立，parent=None，调用方 append_child 挂载）。
    ///
    /// v1.4-a T5（spec §4.2/§4.4）：
    /// 1. 查 `packages[pkg].components[component]`，clone 出 ComponentTemplate（避开 packages/scene 双借）。
    /// 2. 遍历 template.nodes，按 parent_idx 序建 live Node（父先建于子），复用 v1.3+ 节点构造
    ///    （`create_node_from_template`：kind + baked style → base_style/style 初始 + clip_rect +
    ///    dirty_text + slotmap insert + id 回填），再填 classes/id_attr/draggable/tabindex。
    ///    按 parent_idx 串子树（append_child 语义：parent.children.push + child.parent=Some(parent)）。
    ///    根（parent_idx=None）不串父，记录返回。
    /// 3. 伪类规则合并去重：遍历 template.dynamic_rules，相同选择器（ParsedSelector.eq）不重复加进
    ///    scene.dynamic_rules。规则按 class 匹配，多实例共享；hit_test 返具体 NodeId → 各实例独立 :hover。
    /// 4. scene 必须已存在（create_root 建过），否则 Err。
    ///
    /// 多实例独立：同组件多次 instantiate → 各自独立子树（NodeId 不同）+ 各自独立事件/伪类命中。
    /// id_attr 多实例约定限制：find_node_by_id 返首个匹配（不做核心 id 去重，YAGNI）。
    pub fn instantiate(&mut self, pkg: &str, component: &str) -> Result<NodeId, String> {
        let scene = self.scene.as_mut().ok_or("no scene (create_root first)")?;
        // clone 出 template 避开 packages + scene 双借（packages 在 self 上，scene 也在 self 上）。
        let template = self
            .packages
            .get(pkg)
            .and_then(|p| p.components.get(component))
            .cloned()
            .ok_or_else(|| format!("component `{component}` not in pkg `{pkg}`"))?;

        // 遍历 template.nodes 建树（父先建于子——parent_idx < i 由打包器/读保证）。
        // id_map[模板 idx] = live NodeId（slotmap 分配）。
        let mut id_map: Vec<Option<NodeId>> = vec![None; template.nodes.len()];
        let mut root_id: Option<NodeId> = None;
        for (i, tn) in template.nodes.iter().enumerate() {
            let node_id = crate::scene::dynamic::create_node_from_template(
                scene,
                tn.kind.clone(),
                tn.style.clone(),
            );
            // 填 classes/id_attr/draggable/tabindex（create_node_from_template 不填这些，同 create_node）
            let n = scene.get_mut(node_id).unwrap();
            n.classes = tn.classes.clone();
            n.id_attr = tn.id_attr.clone();
            n.draggable = tn.draggable;
            n.tabindex = tn.tabindex;
            id_map[i] = Some(node_id);
            // 按 parent_idx 串子树（根 parent_idx=None 不串）
            if let Some(pidx) = tn.parent_idx {
                let parent = id_map[pidx].expect("parent built before child (parent_idx < i)");
                scene.get_mut(parent).unwrap().children.push(node_id);
                scene.get_mut(node_id).unwrap().parent = Some(parent);
            } else {
                // 组件根（parent_idx=None）——记录返回（多根取最后一个，spec 约定单根组件）
                root_id = Some(node_id);
            }
        }
        let root = root_id.ok_or("component has no root node (parent_idx=None missing)")?;

        // 伪类规则合并去重：相同选择器（ParsedSelector PartialEq）不重复加。
        // 规则按 class 匹配，多实例共享同一规则条目；hit_test 返具体 NodeId → 各实例独立命中。
        for rule in &template.dynamic_rules.rules {
            let dup = scene
                .dynamic_rules
                .rules
                .iter()
                .any(|r| r.selector == rule.selector);
            if !dup {
                scene.dynamic_rules.rules.push(rule.clone());
            }
        }
        Ok(root)
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
        // D17：核心知图尺寸（打包期 PNG IHDR 静态，存 Stage.image_sizes）。solve 查尺寸表算
        // Image intrinsic（三档：CSS > 真实像素 > 64×64）。不知图集（运行时纹理/UV 归 Unity）。
        solve(scene, &self.font, self.root_size, &self.image_sizes);
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
        // D17：build_render_nodes 查 Stage.image_sizes 算九宫格 UV（slice_px / src_px）。
        // Image payload 带 path，UV 全图 (0,0)-(1,1)（无 atlas 子区），Unity 查 Sprite 拿真实 UV（T8）。
        let (frame, new_hashes) = build_render_nodes(scene, &self.font, &self.prev_node_hashes, &self.image_sizes);
        self.prev_node_hashes = new_hashes;
        frame
    }

    pub fn render_json(&mut self) -> String {
        let frame = self.tick_and_render();
        serde_json::to_string_pretty(&frame.nodes).unwrap()
    }

    /// 测试专用：HTML+CSS 文本直接构 scene（v1.4-a 砍了 load_inline，此 helper 保留 parse 路径
    /// 供 stage/render 集成测试用——这些测验证 parse→render 管线，不走 package/instantiate）。
    /// 语义同旧 load_inline：parse_html → resolve_styles → build_scene → self.scene。
    /// v1.4-a T4：textures/atlases 已砍（图集归 Unity），故不涉及纹理注册。
    #[cfg(all(test, feature = "parse"))]
    pub fn load_inline_for_test(&mut self, html: &str, css: &str) -> Result<(), String> {
        let tree = crate::parse::dom::parse_html(html)?;
        let sheet = crate::parse::css::parse_css(css)?;
        let styles = crate::style::cascade::resolve_styles(&tree, &sheet);
        self.tweens.clear();
        if let Some(scene) = self.scene.as_mut() {
            scene.scroll.clear();
        }
        self.prev_node_hashes.clear();
        self.scene = Some(crate::scene::node::build_scene(&tree, &styles));
        Ok(())
    }
}

#[cfg(all(test, feature = "parse"))]
mod tests {
    use super::*;
    use crate::asset::{PackageInput, TemplateNode};
    use crate::parse::dom::{ElementId, ElementTree};
    use crate::scene::NodeKind;
    use crate::style::resolved::ResolvedStyle;

    /// 测试辅助：从 inline HTML+CSS 抽出 ComponentTemplate 数据（nodes + dynamic_rules），
    /// 模仿 loomgui_pkg 打包器的提取逻辑（gather_rec 同构：tag→kind、resolve_styles 烘焙 style、
    /// classes/id/draggable/tabindex 从 ElementData 取）。供黄金等价测试把 inline 场景序列化成包。
    ///
    /// 约定：整棵 inline 树打包成一个名为 "scene" 的组件（nodes[0]=根，parent_idx=None）。
    fn gather_template_nodes(
        tree: &ElementTree,
        styles: &[ResolvedStyle],
        el_id: ElementId,
        parent_idx: Option<usize>,
        out: &mut Vec<TemplateNode>,
    ) {
        let el = &tree.nodes[el_id.0];
        let style = &styles[el_id.0];
        let mut kind = crate::scene::dynamic::kind_from_tag(&el.tag)
            .unwrap_or_else(|_| unreachable!("parse 层白名单已挡围栏外 tag"));
        match &mut kind {
            NodeKind::Image { src } => {
                *src = el.attrs.get("src").cloned().unwrap_or_default();
            }
            NodeKind::Text { content } => {
                *content = el.text.clone().unwrap_or_default();
            }
            _ => {}
        }
        let draggable = el.attrs.get("draggable").map(|v| v == "true").unwrap_or(false);
        let tabindex = el.attrs.get("tabindex").and_then(|v| v.parse::<i32>().ok());
        let my_idx = out.len();
        out.push(TemplateNode {
            kind: kind.clone(),
            style: style.clone(),
            parent_idx,
            classes: el.classes.clone(),
            id_attr: el.id.clone(),
            draggable,
            tabindex,
        });
        // Container/Button 的裸文本 → Text 子（同 gather_rec，继承字体/颜色字段）
        if matches!(kind, NodeKind::Container | NodeKind::Button) {
            if let Some(text) = &el.text {
                let mut ts = ResolvedStyle::default();
                ts.color = style.color;
                ts.font_size = style.font_size;
                ts.font_family = style.font_family.clone();
                ts.font_weight = style.font_weight;
                ts.line_height = style.line_height;
                ts.letter_spacing = style.letter_spacing;
                ts.text_align = style.text_align;
                ts.white_space_nowrap = style.white_space_nowrap;
                out.push(TemplateNode {
                    kind: NodeKind::Text { content: text.clone() },
                    style: ts,
                    parent_idx: Some(my_idx),
                    classes: Vec::new(),
                    id_attr: None,
                    draggable: false,
                    tabindex: None,
                });
            }
        }
        for c in &el.children {
            gather_template_nodes(tree, styles, *c, Some(my_idx), out);
        }
    }

    /// 测试辅助：把 inline HTML+CSS 打成一个名为 "scene" 的单组件包 bytes。
    /// 返回 (pkg_bytes, asset_manifest)。dynamic_rules 从 CSS 抽（含 :hover 等伪类的规则）。
    fn pkg_bytes_from_inline(html: &str, css: &str) -> (Vec<u8>, Vec<String>) {
        let tree = crate::parse::dom::parse_html(html).unwrap();
        let sheet = crate::parse::css::parse_css(css).unwrap();
        let styles = crate::style::cascade::resolve_styles(&tree, &sheet);
        let dynamic = crate::asset::extract_dynamic_rules(&sheet);
        let mut nodes: Vec<TemplateNode> = Vec::new();
        // 单根树（inline 测试都是单根）；多根场景测试不在此 helper 范围
        for root in &tree.roots {
            gather_template_nodes(&tree, &styles, *root, None, &mut nodes);
        }
        // asset_manifest：扫所有 Image 节点的 src（已归一化路径——测试用 src 直接作 path）。
        // D17：图尺寸测试 helper 无 PNG 文件 → w/h=0（核心 measure fallback 64×64）。
        // 真实尺寸由 loomgui_pkg 打包器读 PNG IHDR 填（见 pkg 测试）。
        let manifest: Vec<crate::asset::AssetEntry> = nodes
            .iter()
            .filter_map(|tn| match &tn.kind {
                NodeKind::Image { src } if !src.is_empty() => Some(crate::asset::AssetEntry {
                    path: src.clone(),
                    w: 0,
                    h: 0,
                }),
                _ => None,
            })
            .collect();
        let manifest_paths: Vec<String> = manifest.iter().map(|e| e.path.clone()).collect();
        let input = PackageInput {
            components: vec![("scene", nodes.as_slice(), &dynamic)],
            asset_manifest: &manifest,
        };
        (crate::asset::write_package(&input), manifest_paths)
    }

    /// 黄金等价（最强门）：inline 渲染 == 包渲染。
    ///
    /// v1.4-a T5 改写（原 T4 暂 ignore）：load_package 进资源池不建 scene，包路径走
    /// `load_package → instantiate("scene") → append_child → render`，与 inline 路径
    /// （load_inline_for_test → render）渲染输出逐字等价对比。证明 instantiate 克隆子树 +
    /// 挂载后几何/样式与 inline 同构（零回归）。
    #[test]
    fn package_load_renders_identical_to_inline() {
        let font_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/DejaVuSans.ttf"
        );
        let html = r#"<div class="c"><span>hi</span><img src="logo.png"></div>"#;
        let css = ".c{width:200px;height:100px;overflow:hidden;background-color:#ff0000;}";

        // inline 路径（test-only helper，保留 parse→scene 管线验证）
        let mut s_inline = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_inline.load_inline_for_test(html, css).unwrap();
        let inline_json = s_inline.render_json();

        // 包路径：load_package → instantiate("scene") → 挂为 scene 根 → render。
        // inline 路径把 .c div 作 scene 根；包路径 instantiate 返回孤立根，直接 push 进
        // scene.roots（同 create_root 语义），不套额外 stage_root——保证两路径节点树同构。
        let (pkg_bytes, _) = pkg_bytes_from_inline(html, css);
        let mut s_pkg = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s_pkg.load_package("bag", &pkg_bytes).unwrap();
        // ensure_scene（首次建空骨架）+ instantiate 返回孤立根 → push 进 scene.roots 作场景根
        s_pkg.ensure_scene();
        let comp_root = s_pkg.instantiate("bag", "scene").unwrap();
        s_pkg.scene.as_mut().unwrap().roots.push(comp_root);
        let pkg_json = s_pkg.render_json();

        assert_eq!(inline_json, pkg_json, "包路径渲染输出必须 == inline（instantiate 克隆子树等价）");
    }

    /// v1.4-a T5 改写（原 T4 暂 ignore）：load_package → instantiate → :hover 重匹配验证。
    /// 按钮 + :hover 规则打成包，instantiate 后 Move 到按钮 → RollOver + 伪类重匹配变蓝。
    #[cfg(feature = "parse")]
    #[test]
    fn set_input_hover_emits_rollover_and_rematch() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="root"><button class="btn">OK</button></div>"#;
        let css = r#".btn { width: 100px; height: 50px; background-color: #cccccc; } .btn:hover { background-color: #0000ff; }"#;
        let (pkg_bytes, _) = pkg_bytes_from_inline(html, css);

        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_package("bag", &pkg_bytes).unwrap();
        // 包路径 instantiate 返回孤立根（div.root）→ push 进 scene.roots 作场景根（同 inline 语义）
        s.ensure_scene();
        let comp_root = s.instantiate("bag", "scene").unwrap();
        s.scene.as_mut().unwrap().roots.push(comp_root);
        // comp_root = div.root；btn = root 的首个 button 子（gather_rec 把 <button>OK</button> 建成 Button + auto Text 子）
        // warmup tick：compute_world_transforms 在 process/scroll 后跑，hit_test 读上帧
        // world_transforms（1 帧延迟语义）。首帧 world_transforms 空 → 首帧 hit_test 全 None。
        s.tick_and_render();
        // btn = comp_root 的首个 button 子（gather_rec 把 <button>OK</button> 建成 Button + auto Text 子）
        let btn_id = {
            let sc = s.scene.as_ref().unwrap();
            *sc.get(comp_root).unwrap().children.iter().find(|&&c| {
                matches!(sc.get(c).unwrap().kind, NodeKind::Button)
            }).unwrap()
        };
        // Move 到按钮 (50,25)（按钮在 (0,0,100,50)）
        s.set_input(&[crate::input::PointerEvent {
            kind: crate::input::PointerKind::Move,
            x: 50.0,
            y: 25.0,
            button: 0,
            pad: [0, 0],
            touch_id: -1,
        }]);
        s.tick_and_render();
        let events = s.last_events();
        assert!(
            events.iter().any(|e| e.event_type == crate::input::EVT_ROLL_OVER),
            "Move 到按钮 → RollOver"
        );
        assert!(s.is_pointer_on_ui(), "命中按钮 → is_pointer_on_ui=true");
        // hover 后 rematch：btn style.background_color 应变蓝（dynamic 规则 .btn:hover）
        let scene = s.scene.as_ref().unwrap();
        let btn = scene.get(btn_id).unwrap();
        assert_eq!(
            btn.style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            ":hover 伪类重匹配 → 蓝"
        );
    }

    /// v1.4-a T5 改写（原 T4 暂 ignore）：load_package → instantiate → disabled 抑制 click。
    /// 按钮打成包，instantiate 后 set_node_disabled(true) → Down+Up 不产 Click。
    #[cfg(feature = "parse")]
    #[test]
    fn set_node_disabled_inhibits_click() {
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let html = r#"<div class="root"><button class="btn">OK</button></div>"#;
        let css = r#".btn { width: 100px; height: 50px; }"#;
        let (pkg_bytes, _) = pkg_bytes_from_inline(html, css);

        let mut s = Stage::new(font_path, (200.0, 100.0)).unwrap();
        s.load_package("bag", &pkg_bytes).unwrap();
        s.ensure_scene();
        let comp_root = s.instantiate("bag", "scene").unwrap();
        s.scene.as_mut().unwrap().roots.push(comp_root);
        // btn = comp_root 的首个 Button 子
        let btn_id = {
            let sc = s.scene.as_ref().unwrap();
            *sc.get(comp_root).unwrap().children.iter().find(|&&c| {
                matches!(sc.get(c).unwrap().kind, NodeKind::Button)
            }).unwrap()
        };
        s.set_node_disabled(btn_id, true);
        // warmup tick（同 hover 测：hit_test 1 帧延迟，首帧 world_transforms 空）
        s.tick_and_render();
        // 命中前置：Move 到按钮 → is_pointer_on_ui=true（证明按钮被几何命中，disabled 才有抑制对象）
        s.set_input(&[crate::input::PointerEvent {
            kind: crate::input::PointerKind::Move,
            x: 50.0,
            y: 25.0,
            button: 0,
            pad: [0, 0],
            touch_id: -1,
        }]);
        s.tick_and_render();
        assert!(
            s.is_pointer_on_ui(),
            "Move 到按钮 → 命中 UI（命中前置：证明按钮被几何命中）"
        );
        // Down + Up 在按钮上——disabled 不产 Click
        s.set_input(&[
            crate::input::PointerEvent {
                kind: crate::input::PointerKind::Down,
                x: 50.0,
                y: 25.0,
                button: 0,
                pad: [0, 0],
                touch_id: -1,
            },
            crate::input::PointerEvent {
                kind: crate::input::PointerKind::Up,
                x: 50.0,
                y: 25.0,
                button: 0,
                pad: [0, 0],
                touch_id: -1,
            },
        ]);
        s.tick_and_render();
        let events = s.last_events();
        assert!(
            !events.iter().any(|e| e.event_type == crate::input::EVT_CLICK),
            "disabled → 不产 Click"
        );
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
        s.load_inline_for_test(html, css).unwrap();
        let root_id = s.scene.as_ref().unwrap().roots[0];
        // 手动塞 scroll_pos，模拟上一会话残留
        s.scene.as_mut().unwrap().scroll.ensure(root_id).scroll_pos = (50.0, 50.0);
        // reload → scroll 表应被清
        s.load_inline_for_test(html, css).unwrap();
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
        s.load_inline_for_test(html, css).unwrap();
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
        s.load_inline_for_test(r#"<div class="b"></div>"#, ".b{width:100px;height:50px;}").unwrap();
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
        s.load_inline_for_test(r#"<div class="b"></div>"#, ".b{width:100px;height:50px;}").unwrap();
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
        s.load_inline_for_test(html, css).unwrap();
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
        s.load_inline_for_test(r#"<div class="c"></div>"#, ".c{width:100px;height:50px;}").unwrap();
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
        s.load_inline_for_test(html, css).unwrap();
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

/// T4 资源池测试：load_package 进 packages 字典不建 scene + 多包共存 + 同名替换。
/// 不依赖 parse feature——用内存 pkg（write_package）。
#[cfg(test)]
mod load_package_tests {
    use super::*;
    use crate::asset::{PackageInput, TemplateNode};
    use crate::scene::NodeKind;
    use crate::style::resolved::ResolvedStyle;

    /// 辅助：内存建单组件 pkg（组件名 comp_name，单 Container 根）。
    /// 走 write_package → bytes，供 load_package 消费。
    fn make_test_pkg(_comp_name: &str) -> Vec<u8> {
        let nodes = [TemplateNode {
            kind: NodeKind::Container,
            style: ResolvedStyle::default(),
            parent_idx: None, // 组件根
            classes: vec![],
            id_attr: None,
            draggable: false,
            tabindex: None,
        }];
        let rules = crate::style::dynamic::DynamicRuleTable::default();
        let input = PackageInput {
            components: vec![(_comp_name, &nodes, &rules)],
            asset_manifest: &[],
        };
        crate::asset::write_package(&input)
    }

    #[test]
    fn load_package_into_pool_without_scene() {
        let mut s = Stage::new_for_test(); // scene = Some(空骨架)
        let pkg_bytes = make_test_pkg("comp1");
        s.load_package("bag", &pkg_bytes).unwrap();
        assert!(s.packages.contains_key("bag"), "进资源池");
        assert!(s.scene.is_some(), "scene 不变（load 不建/不清 scene）");
        // scene 仍是空骨架（无 roots）——load_package 没碰 scene
        assert!(
            s.scene.as_ref().unwrap().roots.is_empty(),
            "scene roots 仍空（load 不建 scene）"
        );
    }

    #[test]
    fn load_package_multi_pkg_coexist() {
        let mut s = Stage::new_for_test();
        s.load_package("bag", &make_test_pkg("c1")).unwrap();
        s.load_package("mail", &make_test_pkg("c2")).unwrap();
        assert_eq!(s.packages.len(), 2, "多包共存");
        assert!(s.packages.contains_key("bag"));
        assert!(s.packages.contains_key("mail"));
    }

    #[test]
    fn load_package_replace_same_name() {
        let mut s = Stage::new_for_test();
        s.load_package("bag", &make_test_pkg("c1")).unwrap();
        assert_eq!(s.packages.len(), 1);
        s.load_package("bag", &make_test_pkg("c2")).unwrap();
        assert_eq!(s.packages.len(), 1, "同名替换（不堆积）");
        // 替换后包内组件应是 c2（验证是替换不是 no-op）
        assert!(
            s.packages["bag"].components.contains_key("c2"),
            "替换后是新包（含 c2）"
        );
    }

    /// load_package 不碰 scene 的不变量：load 前 scene 有内容，load 后 scene 不变。
    /// 验证 load_package 不清/不重建 scene（与旧 load_package 建 scene 语义对立）。
    #[test]
    fn load_package_does_not_touch_scene() {
        let mut s = Stage::new_for_test();
        // 先建 scene 内容（create_root 建根）
        let root = s.create_root("div", "width:100px;height:100px").unwrap();
        let scene_root_count_before = s.scene.as_ref().unwrap().roots.len();
        assert_eq!(scene_root_count_before, 1);
        // load_package 进资源池
        s.load_package("bag", &make_test_pkg("c1")).unwrap();
        // scene 完全不变（roots 不变、节点数不变）
        let scene = s.scene.as_ref().unwrap();
        assert_eq!(scene.roots.len(), 1, "scene roots 不变");
        assert_eq!(scene.roots[0], root, "scene root NodeId 不变");
        assert_eq!(scene.nodes.len(), 1, "scene 节点数不变");
    }
}

/// T5 instantiate 测试：从包克隆组件子树进 scene + 伪类规则合并去重 + 多实例独立。
/// 不依赖 parse feature——用内存 PackageInput（write_package）。
#[cfg(test)]
mod instantiate_tests {
    use super::*;
    use crate::asset::{PackageInput, TemplateNode};
    use crate::scene::NodeKind;
    use crate::style::resolved::ResolvedStyle;

    /// 辅助：建带子树的 pkg（comp1 = root(Container) + child(Container)）。
    fn make_test_pkg_with_subtree() -> Vec<u8> {
        let mut root_style = ResolvedStyle::default();
        // 给 root 显式尺寸，便于后续断言可扩展（此处仅验结构）
        crate::scene::dynamic::apply_css(&mut root_style, "width:100px;height:100px");
        let nodes = [
            TemplateNode {
                kind: NodeKind::Container,
                style: root_style,
                parent_idx: None,
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
            TemplateNode {
                kind: NodeKind::Container,
                style: ResolvedStyle::default(),
                parent_idx: Some(0),
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
        ];
        let rules = crate::style::dynamic::DynamicRuleTable::default();
        let input = PackageInput {
            components: vec![("comp1", &nodes, &rules)],
            asset_manifest: &[],
        };
        crate::asset::write_package(&input)
    }

    #[test]
    fn instantiate_clones_subtree_returns_orphan_root() {
        let mut s = Stage::new_for_test();
        s.create_root("div", "width:100px;height:100px").unwrap();
        s.load_package("bag", &make_test_pkg_with_subtree()).unwrap();
        let root = s.instantiate("bag", "comp1").unwrap();
        let scene = s.scene.as_ref().unwrap();
        // 组件根 parent = None（孤立）
        assert!(scene.get(root).unwrap().parent.is_none(), "孤立根");
        // comp1 含 root + child → 子树串好（root.children 含 child）
        assert_eq!(scene.get(root).unwrap().children.len(), 1, "root 有 1 子");
        let child = scene.get(root).unwrap().children[0];
        assert_eq!(scene.get(child).unwrap().parent, Some(root), "child.parent=root");
        // scene 节点数 = create_root 的 1 + 组件的 2 = 3
        assert_eq!(scene.nodes.len(), 3, "scene 多了组件的 2 节点");
    }

    #[test]
    fn instantiate_multi_instance_independent() {
        let mut s = Stage::new_for_test();
        s.create_root("div", "").unwrap();
        s.load_package("bag", &make_test_pkg_with_subtree()).unwrap();
        let i1 = s.instantiate("bag", "comp1").unwrap();
        let i2 = s.instantiate("bag", "comp1").unwrap();
        assert_ne!(i1, i2, "两实例不同 NodeId");
        // 两实例都孤立，各自独立子树
        let scene = s.scene.as_ref().unwrap();
        assert!(scene.get(i1).unwrap().parent.is_none(), "i1 孤立");
        assert!(scene.get(i2).unwrap().parent.is_none(), "i2 孤立");
        // 各自的 child 不同（独立子树，不串）
        let c1 = scene.get(i1).unwrap().children[0];
        let c2 = scene.get(i2).unwrap().children[0];
        assert_ne!(c1, c2, "两实例的 child 不同");
        assert_eq!(scene.get(c1).unwrap().parent, Some(i1), "c1.parent=i1");
        assert_eq!(scene.get(c2).unwrap().parent, Some(i2), "c2.parent=i2");
    }

    #[test]
    fn instantiate_missing_pkg_or_comp_errors() {
        let mut s = Stage::new_for_test();
        s.create_root("div", "").unwrap();
        // 用 load_package_tests 的 make_test_pkg（单组件 c1）——这里内联一个最小 pkg
        let nodes = [TemplateNode {
            kind: NodeKind::Container,
            style: ResolvedStyle::default(),
            parent_idx: None,
            classes: vec![],
            id_attr: None,
            draggable: false,
            tabindex: None,
        }];
        let rules = crate::style::dynamic::DynamicRuleTable::default();
        let input = PackageInput {
            components: vec![("c1", &nodes, &rules)],
            asset_manifest: &[],
        };
        s.load_package("bag", &crate::asset::write_package(&input)).unwrap();
        assert!(s.instantiate("nope", "c1").is_err(), "包不存在");
        assert!(s.instantiate("bag", "nope").is_err(), "组件不存在");
    }

    #[test]
    fn instantiate_without_scene_errors() {
        // scene 必须已存在（create_root 建过），否则 Err
        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (200.0, 200.0)).unwrap();
        // 不调 create_root，scene = None
        s.load_package("bag", &make_test_pkg_with_subtree()).unwrap();
        assert!(s.instantiate("bag", "comp1").is_err(), "无 scene → Err");
    }

    /// pkg 的 comp1 带 :hover 规则；instantiate 两次 → scene.dynamic_rules 只多一份（去重）。
    /// parse-gated：需 parse_selector 构真实选择器（runtime 无字符串解析输入）。
    #[cfg(feature = "parse")]
    #[test]
    fn instantiate_merges_dynamic_rules_dedup() {
        use crate::parse::css::Declaration;
        use crate::parse::selector::parse_selector;
        use crate::style::dynamic::{DynamicRule, DynamicRuleTable};
        let rules = DynamicRuleTable {
            rules: vec![DynamicRule {
                selector: parse_selector(".btn:hover").unwrap(),
                declarations: vec![Declaration {
                    prop: "background-color".into(),
                    value: "#0000ff".into(),
                }],
            }],
        };
        let nodes = [TemplateNode {
            kind: NodeKind::Button,
            style: ResolvedStyle::default(),
            parent_idx: None,
            classes: vec!["btn".into()],
            id_attr: None,
            draggable: false,
            tabindex: None,
        }];
        let input = PackageInput {
            components: vec![("comp1", &nodes, &rules)],
            asset_manifest: &[],
        };
        let mut s = Stage::new_for_test();
        s.create_root("div", "").unwrap();
        s.load_package("bag", &crate::asset::write_package(&input)).unwrap();
        let before = s.scene.as_ref().unwrap().dynamic_rules.rules.len();
        s.instantiate("bag", "comp1").unwrap();
        s.instantiate("bag", "comp1").unwrap();
        let after = s.scene.as_ref().unwrap().dynamic_rules.rules.len();
        assert_eq!(after - before, 1, "同选择器规则去重，只加一份");
    }

    /// 多实例 :hover 独立性（review Important-2）：同组件 instantiate 两次 → 仅 hover 实例1
    /// 的按钮变蓝，实例2 按钮保持原色（无 rematch 串状态回归）。
    ///
    /// 设计契约（spec §4.4）：dynamic_rules 按 class 匹配、多实例共享；hit_test 返具体 NodeId →
    /// 各实例独立 :hover。此前 `instantiate_multi_instance_independent` 只验结构独立，:hover 行为
    /// 独立是设计推断——本测试把它钉成受测事实。
    ///
    /// 布局：scene_root(400x200) → 块流挂两实例（comp root=div 100x50，含 button.btn 100x50）。
    /// 实例1 的 btn 几何在 (0,0)-(100,50)，实例2 的 btn 在 (0,50)-(100,100)。Move 到 (50,25)
    /// 只命中 btn1。layout solve 仅 roots[0] 下沉——两实例必须挂同一 scene_root 子树下才被布局。
    #[cfg(feature = "parse")]
    #[test]
    fn instantiate_multi_instance_hover_independent() {
        use crate::parse::css::Declaration;
        use crate::parse::selector::parse_selector;
        use crate::style::dynamic::{DynamicRule, DynamicRuleTable};
        use crate::transform::Affine2Ext;

        // .btn:hover { background-color: #0000ff } —— 共享规则，按 class + 伪类匹配各实例
        let rules = DynamicRuleTable {
            rules: vec![DynamicRule {
                selector: parse_selector(".btn:hover").unwrap(),
                declarations: vec![Declaration {
                    prop: "background-color".into(),
                    value: "#0000ff".into(),
                }],
            }],
        };
        // 组件根 = Container(div) 100x50；子 = Button.btn 100x50（base 灰底，hover 蓝）
        let mut btn_style = ResolvedStyle::default();
        crate::scene::dynamic::apply_css(&mut btn_style, "width:100px;height:50px;background-color:#cccccc");
        let mut root_style = ResolvedStyle::default();
        crate::scene::dynamic::apply_css(&mut root_style, "width:100px;height:50px");
        let nodes = [
            TemplateNode {
                kind: NodeKind::Container,
                style: root_style,
                parent_idx: None,
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
            TemplateNode {
                kind: NodeKind::Button,
                style: btn_style,
                parent_idx: Some(0),
                classes: vec!["btn".into()],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
        ];
        let input = PackageInput {
            components: vec![("comp1", &nodes, &rules)],
            asset_manifest: &[],
        };

        let font_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
        let mut s = Stage::new(font_path, (400.0, 200.0)).unwrap();
        // scene_root 作唯一布局根（solve 仅 roots[0] 下沉）；两实例 append_child 挂其下，块流纵向堆叠
        let scene_root = s.create_root("div", "width:400px;height:200px").unwrap();
        s.load_package("bag", &crate::asset::write_package(&input)).unwrap();
        let i1 = s.instantiate("bag", "comp1").unwrap();
        let i2 = s.instantiate("bag", "comp1").unwrap();
        s.append_child(scene_root, i1).unwrap();
        s.append_child(scene_root, i2).unwrap();

        // 取两实例各自的 btn NodeId（comp root 的首个 Button 子）
        let btn1 = {
            let sc = s.scene.as_ref().unwrap();
            *sc.get(i1).unwrap().children.iter().find(|&&c| {
                matches!(sc.get(c).unwrap().kind, NodeKind::Button)
            }).unwrap()
        };
        let btn2 = {
            let sc = s.scene.as_ref().unwrap();
            *sc.get(i2).unwrap().children.iter().find(|&&c| {
                matches!(sc.get(c).unwrap().kind, NodeKind::Button)
            }).unwrap()
        };
        assert_ne!(btn1, btn2, "两实例 btn NodeId 不同");

        // warmup tick：hit_test 读上帧 world_transforms（1 帧延迟语义，首帧空 → 全 None）
        s.tick_and_render();
        // 校验两 btn 几何已分离（btn1 在 y≈0，btn2 在 y≈50，无重叠 → 命中不串）
        {
            let sc = s.scene.as_ref().unwrap();
            let (_x1, y1) = sc.world_transforms[btn1.index()].apply_point(0.0, 0.0);
            let (_x2, y2) = sc.world_transforms[btn2.index()].apply_point(0.0, 0.0);
            assert!(
                (y2 - y1).abs() >= 40.0,
                "两实例 btn 应纵向分离（y1={y1}, y2={y2}），否则 hover 命中会串"
            );
        }

        // hover 前基线：两 btn 都是灰底 #cccccc = [0.8,0.8,0.8,1.0]
        {
            let sc = s.scene.as_ref().unwrap();
            assert_eq!(
                sc.get(btn1).unwrap().style.background_color,
                Some([0.8, 0.8, 0.8, 1.0]),
                "hover 前 btn1 灰底"
            );
            assert_eq!(
                sc.get(btn2).unwrap().style.background_color,
                Some([0.8, 0.8, 0.8, 1.0]),
                "hover 前 btn2 灰底"
            );
        }

        // Move 到 (50,25) —— 落在 btn1 几何 (0,0)-(100,50) 内，btn2 在 y≈50 外
        s.set_input(&[crate::input::PointerEvent {
            kind: crate::input::PointerKind::Move,
            x: 50.0,
            y: 25.0,
            button: 0,
            pad: [0, 0],
            touch_id: -1,
        }]);
        s.tick_and_render();

        // 核心：btn1 变蓝（hover 命中 + rematch），btn2 保持灰（无串状态）
        let sc = s.scene.as_ref().unwrap();
        assert_eq!(
            sc.get(btn1).unwrap().style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            "btn1 被 hover → 变蓝"
        );
        assert_eq!(
            sc.get(btn2).unwrap().style.background_color,
            Some([0.8, 0.8, 0.8, 1.0]),
            "btn2 未被 hover → 保持灰（无 cross-talk）"
        );
        assert_ne!(
            sc.get(btn1).unwrap().style.background_color,
            sc.get(btn2).unwrap().style.background_color,
            "两实例 :hover 状态独立（btn1 蓝 / btn2 灰）"
        );
    }
}

/// D17 集成测试：打包期 PNG IHDR 尺寸 → load_package 建尺寸表 → measure 用真实尺寸。
/// 验证端到端链路：打包器填 AssetEntry.w/h → pkg.bin 存 → read_package 读 → Stage.image_sizes
/// 建 → solve 查表算 Image intrinsic（三档：CSS > 真实像素 > 64×64）。
#[cfg(test)]
mod d17_image_size_tests {
    use super::*;
    use crate::asset::{AssetEntry, PackageInput, TemplateNode};
    use crate::scene::NodeKind;
    use crate::style::resolved::ResolvedStyle;
    use taffy::style::Dimension;

    /// 辅助：建带 Image 子的 pkg（root Container + Image leaf，src=path）。
    /// AssetEntry 带真实 w/h（模拟打包器读 PNG IHDR 填）。
    fn make_pkg_with_image_size(src: &str, w: u32, h: u32) -> Vec<u8> {
        let mut img_style = ResolvedStyle::default();
        // align_self=FlexStart 防 column 容器 stretch 把 cross 轴宽拉满
        img_style.taffy_style.align_self = Some(taffy::style::AlignSelf::FlexStart);
        let nodes = [
            TemplateNode {
                kind: NodeKind::Container,
                style: ResolvedStyle::default(),
                parent_idx: None,
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
            TemplateNode {
                kind: NodeKind::Image { src: src.into() },
                style: img_style,
                parent_idx: Some(0),
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
        ];
        let rules = crate::style::dynamic::DynamicRuleTable::default();
        let manifest = [AssetEntry { path: src.into(), w, h }];
        let input = PackageInput {
            components: vec![("comp1", &nodes, &rules)],
            asset_manifest: &manifest,
        };
        crate::asset::write_package(&input)
    }

    /// D17 端到端：40×20 图打包进 pkg → load_package 建尺寸表 → instantiate → solve
    /// → Image measure 用真实 40×20（非 64×64 兜底）。
    #[test]
    fn load_package_builds_size_table_and_measure_uses_real_dims() {
        let pkg_bytes = make_pkg_with_image_size("icons/wide.png", 40, 20);
        let mut s = Stage::new_for_test();
        s.create_root("div", "width:300px;height:300px").unwrap();
        s.load_package("bag", &pkg_bytes).unwrap();
        // D17：load_package 后 Stage.image_sizes 含 path → (w,h)
        assert_eq!(s.image_size("icons/wide.png"), Some((40, 20)),
            "load_package 建尺寸表：path→(40,20)");

        let comp_root = s.instantiate("bag", "comp1").unwrap();
        s.append_child(s.scene.as_ref().unwrap().roots[0], comp_root).unwrap();
        s.tick_and_render();

        // Image 是 comp_root 的首个子（gather: root[0] + img[1]，img parent_idx=0）
        let scene = s.scene.as_ref().unwrap();
        let img_id = scene.get(comp_root).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        // 无 CSS 尺寸 → 用尺寸表真实像素 40×20（三档第二档）
        assert!((r.w - 40.0).abs() < 0.1, "measure 用真实 w=40（非 64 兜底），got {}", r.w);
        assert!((r.h - 20.0).abs() < 0.1, "measure 用真实 h=20（非 64 兜底），got {}", r.h);
    }

    /// D17：pkg 的 AssetEntry w/h=0（非 PNG / 读失败）→ 尺寸表无有效条目 → measure fallback 64×64。
    #[test]
    fn load_package_zero_dims_falls_back_to_64() {
        let pkg_bytes = make_pkg_with_image_size("icons/zero.png", 0, 0);
        let mut s = Stage::new_for_test();
        s.create_root("div", "width:300px;height:300px").unwrap();
        s.load_package("bag", &pkg_bytes).unwrap();
        // w/h=0 → image_size 返 None（filter 掉 0/0）
        assert_eq!(s.image_size("icons/zero.png"), None, "w/h=0 → None（fallback 64×64）");

        let comp_root = s.instantiate("bag", "comp1").unwrap();
        s.append_child(s.scene.as_ref().unwrap().roots[0], comp_root).unwrap();
        s.tick_and_render();

        let scene = s.scene.as_ref().unwrap();
        let img_id = scene.get(comp_root).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 64.0).abs() < 0.1, "w/h=0 → fallback w=64，got {}", r.w);
        assert!((r.h - 64.0).abs() < 0.1, "w/h=0 → fallback h=64，got {}", r.h);
    }

    /// D17：CSS 尺寸赢过真实像素（三档第一档）。
    /// 40×20 图 + CSS width:80px → w=80（CSS），height 等比 = 40（80×20/40，2:1 真实 aspect）。
    #[test]
    fn css_length_overrides_real_image_size() {
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.width = Dimension::Length(80.0);
        img_style.taffy_style.align_self = Some(taffy::style::AlignSelf::FlexStart);
        let nodes = [
            TemplateNode {
                kind: NodeKind::Container,
                style: ResolvedStyle::default(),
                parent_idx: None,
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
            TemplateNode {
                kind: NodeKind::Image { src: "icons/wide.png".into() },
                style: img_style,
                parent_idx: Some(0),
                classes: vec![],
                id_attr: None,
                draggable: false,
                tabindex: None,
            },
        ];
        let rules = crate::style::dynamic::DynamicRuleTable::default();
        let manifest = [AssetEntry { path: "icons/wide.png".into(), w: 40, h: 20 }];
        let input = PackageInput {
            components: vec![("comp1", &nodes, &rules)],
            asset_manifest: &manifest,
        };
        let pkg_bytes = crate::asset::write_package(&input);

        let mut s = Stage::new_for_test();
        s.create_root("div", "width:300px;height:300px").unwrap();
        s.load_package("bag", &pkg_bytes).unwrap();
        let comp_root = s.instantiate("bag", "comp1").unwrap();
        s.append_child(s.scene.as_ref().unwrap().roots[0], comp_root).unwrap();
        s.tick_and_render();

        let scene = s.scene.as_ref().unwrap();
        let img_id = scene.get(comp_root).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        // CSS width:80px 赢（三档第一档）；height 等比用真实 2:1 aspect = 80×20/40 = 40
        assert!((r.w - 80.0).abs() < 0.1, "CSS width 赢：w=80，got {}", r.w);
        assert!((r.h - 40.0).abs() < 0.1, "height 等比=40（80×20/40 真实 2:1），got {}", r.h);
    }

    /// D17：多包 load_package 合并尺寸表（path 全局唯一）。
    #[test]
    fn multi_package_merges_size_tables() {
        let pkg_a = make_pkg_with_image_size("icons/a.png", 10, 20);
        let pkg_b = make_pkg_with_image_size("icons/b.png", 30, 40);
        let mut s = Stage::new_for_test();
        s.load_package("a", &pkg_a).unwrap();
        s.load_package("b", &pkg_b).unwrap();
        assert_eq!(s.image_size("icons/a.png"), Some((10, 20)), "包 a 的 path 进表");
        assert_eq!(s.image_size("icons/b.png"), Some((30, 40)), "包 b 的 path 进表（多包合并）");
    }

    /// D17：重复 load 同名包 → 新包尺寸覆盖旧包（path 全局唯一，后写赢）。
    #[test]
    fn reload_package_overwrites_size_entry() {
        let pkg_v1 = make_pkg_with_image_size("icons/x.png", 10, 10);
        let pkg_v2 = make_pkg_with_image_size("icons/x.png", 50, 50);
        let mut s = Stage::new_for_test();
        s.load_package("bag", &pkg_v1).unwrap();
        assert_eq!(s.image_size("icons/x.png"), Some((10, 10)), "首次 load");
        s.load_package("bag", &pkg_v2).unwrap();
        assert_eq!(s.image_size("icons/x.png"), Some((50, 50)), "重 load 覆盖（新尺寸）");
    }
}
