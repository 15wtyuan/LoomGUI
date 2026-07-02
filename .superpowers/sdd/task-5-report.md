# Task 5 Report: Stage instantiate（克隆组件子树 + 伪类规则合并）

## 实现内容

### 1. `Stage::instantiate(pkg, component) -> Result<NodeId, String>`（stage.rs）
从 `packages[pkg].components[component]` 克隆一棵 ComponentTemplate 子树进当前 scene，返回组件根 NodeId（孤立，parent=None，调用方 `append_child` 挂载或 push 进 `scene.roots`）。

**内部流程（spec §4.4）：**
1. `scene.as_mut().ok_or("no scene (create_root first)")?` — scene 必须已存在。
2. `packages.get(pkg).and_then(|p| p.components.get(component)).cloned()` — clone 出 template 避开 packages + scene 双借（两者都在 self 上）。
3. 遍历 `template.nodes`，逐个建 live Node：
   - 调 `create_node_from_template(scene, tn.kind, tn.style)` 建 Node（slotmap 分配 NodeId + clip_rect + dirty_text + id 回填）。
   - 回填 `classes/id_attr/draggable/tabindex`（create_node_from_template 不填这些，同 create_node）。
   - 按 `parent_idx` 串子树（`parent.children.push(child)` + `child.parent = Some(parent)`）；根（parent_idx=None）不串，记录返回。
4. 伪类规则合并去重：遍历 `template.dynamic_rules.rules`，相同选择器（`ParsedSelector == rule.selector`）不重复加进 `scene.dynamic_rules.rules`。

### 2. `create_node_from_template(scene, kind, base_style) -> NodeId`（scene/dynamic.rs）
T5 新增的节点构造辅助，与 v1.3+ `create_node` 同构（clip_rect 派生 / dirty_text / slotmap insert / id 回填），但跳过 CSS parse——style 已在 ComponentTemplate.nodes[i].style 烘焙好。直接用传入 style 作 `base_style`（源）+ `style = base_style.clone()`（派生，下帧 rematch 从 base 起算）。

**create_node 复用适配**：brief 担心的「create_node 紧耦合 CSS parse」不成立——create_node 的节点构造部分（kind→Node 结构 + clip_rect + dirty_text + slotmap insert + id 回填）与 CSS parse（kind_from_tag + apply_css）是清晰分离的两段。抽出 `create_node_from_template` 吃已 bake 的 `NodeKind + ResolvedStyle`，复用全部节点构造逻辑，零重复。

### 3. `Stage::ensure_scene()` 私有辅助（pub(crate)）
scene 不存在则建空骨架（首次 `create_root`/`create_node` 调用时初始化）。spec §4.2「scene 初始由 create_root 建」——原 create_root/create_node 要求 scene 已存在（鸡生蛋问题），现 ensure_scene 首次调用建空骨架，幂等。`pub(crate)` 供集成测试直接初始化场景（黄金等价测试 instantiate 后 push 孤立根进 scene.roots，不套额外 stage_root）。

### 4. `ParsedSelector`/`Compound`/`Declaration` 加 `PartialEq`（style/dynamic.rs）
instantiate 伪类规则去重需选择器相等比较。给三个结构 derive `PartialEq`（结构相等 = 同选择器，含 raw/compound/specificity）。bincode 序列化不受影响（已有 Serialize/Deserialize）。

## 测试

### T5 新增 instantiate 单元测试（5 个，stage.rs `instantiate_tests` 模块）
- `instantiate_clones_subtree_returns_orphan_root` — 克隆 2 节点子树，根 parent=None（孤立），child.parent=root，scene 节点数 +2。
- `instantiate_multi_instance_independent` — 同组件两次 instantiate，NodeId 不同，各自独立子树（child 不串）。
- `instantiate_missing_pkg_or_comp_errors` — 包/组件不存在 → Err。
- `instantiate_without_scene_errors` — scene=None → Err。
- `instantiate_merges_dynamic_rules_dedup`（parse-gated）— :hover 规则 instantiate 两次 → scene.dynamic_rules 只多 1 份（去重）。

### T5 新增 create_node_from_template 单元测试（3 个，scene/dynamic.rs）
- `create_node_from_template_uses_baked_style` — base_style = 传入 baked style，style=base_style.clone()，clip_rect for overflow:hidden，dirty_mesh=true。
- `create_node_from_template_text_marks_dirty_text` — Text 节点 dirty_text=true（同 create_node）。
- `create_node_from_template_id_is_live` — 返回 NodeId live（scene.get 可查）。

### T4 ignore 测试重写恢复（3 个，stage.rs `tests` 模块，UN-ignore）
**原 T4 ignore 原因**：load_package 不再建 scene（进资源池），包路径无 scene 可渲染。
**T5 改写**：load_package → instantiate("scene") → push 进 scene.roots → render，与 inline 路径（load_inline_for_test → render）等价对比。

- `package_load_renders_identical_to_inline`（黄金等价）— inline HTML `<div class="c"><span>hi</span><img src="logo.png"></div>` + CSS 的 render_json == 包路径 render_json（逐字等价）。测试辅助 `pkg_bytes_from_inline(html, css)` 模仿打包器提取（gather_rec 同构：tag→kind、resolve_styles 烘焙 style、classes/id/draggable/tabindex 从 ElementData 取）。
- `set_input_hover_emits_rollover_and_rematch` — 按钮 + :hover 规则打成包，instantiate 后 Move 到按钮 → RollOver + 伪类重匹配变蓝。
- `set_node_disabled_inhibits_click` — 按钮打成包，instantiate 后 set_node_disabled(true) → Down+Up 不产 Click。

## TDD 证据
1. **Step 1（写失败测试）**：5 个 instantiate 测试 + 3 个重写测试加入 stage.rs。
2. **Step 2（跑测试确认失败）**：`cargo test -p loomgui_core instantiate_` → 编译失败（`no method named 'instantiate' found`，E0599 × 多处）。
3. **Step 3（实现）**：加 `create_node_from_template` + `Stage::instantiate` + `ensure_scene` + `PartialEq` derives。
4. **Step 4（跑测试确认通过）**：
   - `cargo test -p loomgui_core instantiate_` → 5 passed。
   - `cargo test -p loomgui_core create_node_from_template` → 3 passed。
   - `cargo test -p loomgui_core package_load_renders_identical_to_inline` → 1 passed（黄金等价）。
   - `cargo test -p loomgui_core set_input_hover_emits_rollover_and_rematch set_node_disabled_inhibits_click` → 2 passed。
5. **Step 5（全量回归）**：`cargo test -p loomgui_core` → 492 passed, 0 failed, 0 ignored。`cargo test --workspace` → 全过，loomgui_ffi_c 6 ignored（T7 FFI 待恢复）。

## 文件变更
- `loomgui_core/src/stage.rs`（+488/-20）：instantiate + ensure_scene + 5 instantiate 测试 + 3 重写测试 + pkg_bytes_from_inline/gather_template_nodes 测试辅助。
- `loomgui_core/src/scene/dynamic.rs`（+85）：create_node_from_template + 3 单元测试。
- `loomgui_core/src/style/dynamic.rs`（+8/-4）：ParsedSelector/Compound/Declaration derive PartialEq。
- `.superpowers/sdd/progress.md`：T5 complete 记录。

## 自审

### 设计正确性
- **复用 create_node 节点构造**：create_node_from_template 与 create_node 零逻辑分叉（同 clip_rect/dirty_text/slotmap insert/id 回填），仅入参不同（baked kind+style vs kind_str+css）。符合 spec「复用 v1.3+ create_node 逻辑」。
- **伪类规则去重**：用 `ParsedSelector ==` 结构相等（raw + compound + specificity）。同选择器（如 `.btn:hover`）多次 instantiate 只加一份规则。规则按 class 匹配，多实例共享；hit_test 返具体 NodeId → 各实例独立命中 :hover（spec §4.4-4）。
- **多实例独立**：每次 instantiate 分配新 NodeId（slotmap insert），子树独立串接，不串。`instantiate_multi_instance_independent` 测试验证。
- **scene 必须已存在**：instantiate 首行 `scene.as_mut().ok_or(...)`，`instantiate_without_scene_errors` 测试验证。
- **id_attr 多实例约定限制**：find_node_by_id（Scene::find_by_id_attr）返首个匹配，不做去重（YAGNI，spec §4.4-6）。

### 黄金等价测试的关键决策
- **不套额外 stage_root**：包路径若 `create_root("div","")` + `append_child` 会多一层 wrapper → 与 inline 路径节点树不同构 → render_json 不等。改为 `ensure_scene()` + instantiate + `scene.roots.push(comp_root)`，让组件根直接作 scene 根（同 inline 路径 `.c` div 作根）。这符合 spec D4「返回孤立根 NodeId，调用方 append_child 挂载」——push 进 roots 是 append_child 到 scene 的等价（根无父）。
- **pkg_bytes_from_inline 测试辅助**：模仿 loomgui_pkg 打包器提取逻辑（gather_rec 同构），把 inline HTML+CSS 序列化成包。验证 instantiate 克隆子树后几何/样式与 inline 同构。

### concerns
1. **ensure_scene 改变 create_root/create_node 语义**：原 create_root 在 scene=None 时报 "no scene"；现自动建空骨架。这是 spec §4.2「scene 初始由 create_root 建」的正确实现（原 T4 实现是 spec 违反的临时态）。无测试依赖原报错行为（grep 确认），零回归。
2. **field_reassign_with_default clippy 警告**：gather_template_nodes 复制 gather_rec 的 auto-Text-子继承模式（`let mut ts = ResolvedStyle::default(); ts.color = ...`），触发 1 个 clippy 警告。与现有 gather_rec（node.rs）同模式（全库 131 个同类警告），保持一致性不修。
3. **T7 FFI 6 ignore 测试**：loomgui_ffi_c 的 6 个 #[ignore] 测试（load_package 不建 scene / atlas 砍）留 T7 FFI 改写恢复，不在 T5 范围。

## 结论
T5 完成。instantiate 克隆组件子树进 scene + 伪类规则合并去重，3 个 T4 ignore 测试重写恢复（黄金等价 + hover + disabled），492 core 测试全过，ignored 9→6（FFI 6 留 T7）。零回归。
