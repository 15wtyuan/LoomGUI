---
name: knowledge-reference
description: >
  Use when working on LoomGUI (loomgui_core Rust 核心、HTML/CSS DSL、taffy flexbox、
  ttf-parser 文本测量、RenderNode 渲染树、FFI、Unity/Godot 后端), researching LoomGUI
  architecture/mechanisms/design decisions, hitting taffy 0.5 / ttf-parser 0.20 / cssparser 0.34
  API issues, looking up pitfalls/debug techniques/known issues, or needing project implementation
  context before starting work. 项目实操知识库（踩坑/API/调试/机制实现），随开发累积。
---

# LoomGUI 知识参考

LoomGUI 项目实操知识库：架构索引、各层机制、依赖 API 适配踩坑、AI 可预测性约束、踩坑记录、调试技巧、已知问题。

## 0. 本文件 vs docs 分工（先读这个）

| 文档 | 性质 | 何时读 |
|---|---|---|
| `docs/design/00-main-design.md` | **设计权威契约**（v1 真相源，710 行） | 查设计意图/契约（§4 围栏 §5 parse §6 Node §7 taffy §8 render §9 text） |
| `docs/roadmap/v1-scope.md` | 范围 | 查 v1 干什么/围栏冻结/胶水任务 |
| `docs/roadmap/v1x-deferred.md` | defer 草稿 | 查 v1.x/v2 机制 |
| `docs/review/` | 五轮对抗审查归档 | 追溯决策来源 |
| **本 skill** | **实操知识库** | 查「怎么干 + 坑在哪 + API 怎么用」 |

开工先读本 skill 知道实操上下文，设计查 docs/design。两者互补不重复。

## 1. 架构

LoomGUI = 跨引擎游戏 UI 框架（对标 FairyGUI）。**Rust 核心（loomgui_core，引擎无关）+ 多引擎后端（Unity 首发）**。

**核心动机**：AI 驱动界面拼装——HTML 作 DSL，让 AI 既能编辑（文本）又能预测渲染（AI 对 HTML/CSS 强先验）。**AI 可预测性是 DSL 决策的首要准则**（见 §4），背离浏览器语义的 divergence 须谨慎评估。

### 1.1 workspace（主文档 §16）
```
loomgui/                      # workspace
├── loomgui_core/             # lib，引擎无关（v0 已实现，砍 event/anim）
│   ├── src/{parse,style,layout,scene,render,text}/ + stage.rs
│   ├── examples/v0_snapshot.rs
│   └── tests/{snapshot.rs, fixtures/DejaVuSans.ttf, snapshots/}
├── loomgui_pkg/              # 打包器（v1 第一阶段）
├── loomgui_ffi_c/            # C ABI（v1）
├── loomgui_unity/            # csbindgen + Unity 后端（v1）
└── loomgui_editor/           # 编辑器（v2+）
```

### 1.2 数据流（单向无环）
```
HTML/CSS → parse(ElementTree/StyleSheet) → style(ResolvedStyle) → scene(Node 树)
        → text(measure → TextLayout) → layout(taffy solve → layout_rect)
        → render(Vec<RenderNode>) → stage.tick → render_nodes JSON
```

### 1.3 渲染树契约（主文档 §8）
`RenderNode` = 公共头 + payload enum（`Unchanged`/`Mesh`/`Text`）。描述**渲染意图**，不规定引擎机制（后端自选 stencil/Material/canvas_item）。
- glyph 绝对坐标（§9.2）：核心已累加 advance + align 偏移，后端拼 quad 零累加。
- 纹理加载是**后端职责**，核心只持 TexId + UV。

## 2. 各层机制要点

### 2.1 parse（§5）
- `scraper` HTML + `cssparser` CSS + 自写 ~100 行选择器匹配器（不用 selectors crate，围栏窄）。
- `ElementData { tag, classes, id, text, attrs: HashMap<String,String>, children, parent }`。
- 行内混排（文本+元素同在）解析期报错。
- `match_element` 返回 specificity **降序**（元组 `(id数, class数, tag数)`）—— 下游 cascade 注意排序方向（坑 6）。

### 2.2 style（§5）
- `ResolvedStyle` = `taffy_style` + 视觉字段（bg-color/border/opacity/color/font-*/text-align...）+ `order: i32`。
- `resolve_styles` 自顶向下递归；继承白名单 8 字段（color/font-size/font-family/font-weight/line-height/letter-spacing/text-align/white-space）从父继承。
- `apply_decl`：CSS 声明 → taffy/视觉字段，**无条件覆盖默认**（时序：default → apply_decl）。
- **默认 flex-direction = Column**（见 §4 约束 1）。

### 2.3 scene（§6）
- `Node { id, parent, kind, style, taffy_id, layout_rect, clip_rect, children, dirty_mesh, dirty_text }`。
- `NodeKind`: `Container` / `Button` / `Image{src}` / `Text{content}`。
- div/button 裸文本 → Text 子节点（§4.2，文本是 flex item）。
- `overflow:hidden` → `clip_rect = Some`（layout solve 后填实际框）。

### 2.4 text（§9）
- ttf-parser 度量 + 贪心断行（unicode-linebreak UAX#14 留 v1.x）→ `TextLayout` SOA 三表（lines/runs/glyphs）。
- `Glyph { glyph_id, x, y, bearing_x, bearing_y }` 绝对坐标。
- v0 砍 rustybuzz/BiDi/fallback，仅 ASCII+CJK（CJK 需 CJK 字体；v0 fixture 用 ASCII）。

### 2.5 layout（§7）
- taffy 0.5 集成（**API 见 §3.1，与草稿差异大**）。
- MeasureFunc：文本调 `measure_text`，图片用声明尺寸或占位 64×64。
- `solve` 就地写 `layout_rect`（绝对坐标，父 origin 累加）+ `clip_rect`。

### 2.6 render（§8）
- `build_render_nodes`：Container/Button→Mesh quad(背景色)，Image→Mesh quad(占位 tex_id=hash(src))，Text→TextLayout 装 Text payload。
- `assign_sort_keys`：DFS 单计数器 sort_key，clip 的 Container 是 BatchingRoot 开新 mask_context。
- v0 保序（无 FairyBatching AABB 重排，v1.x 优化）。

### 2.7 stage
- `Stage::new(font_path, root_size)` → `load_inline(html, css)` → `tick_and_render()` → `Vec<RenderNode>` → `render_json()`。
- 静态首帧：tick 接空输入、dt=0。

## 3. 依赖 API 适配踩坑（v0 最大教训）

> **plan/brief 写的 API 草稿常与实际 crate 版本不符**。遇编译错按本节对照，**勿硬改依赖版本**，按 crate 实际源码（`~/.cargo/registry/src/<crate>-<ver>/src/`）调。

### 3.1 taffy 0.5（layout/mod.rs）
- **无 `MeasureFunc::Boxed`**。用 `TaffyTree<NodeContext>` + `new_leaf_with_context(style, ctx)` + `compute_layout_with_measure(root, Size::MAX_CONTENT, FnMut)`。
- measure 闭包签名：`FnMut(Size<Option<f32>>, Size<AvailableSpace>, NodeId, Option<&mut NodeContext>, &Style) -> Size<f32>`。`known.width` 是 `Option<f32>`（Some=约束宽，None=不限）。
- **闭包可借 `&font`**（FnMat 调用期存活，非 `'static`）→ **不需要 `Arc<Font>`**（v0 一度误判要 Arc，实际单 FnMut 借用合法）。
- `Size::MAX` → `Size::MAX_CONTENT`。
- 根 size setter 用 `Dimension::Length`（`Style.size` 是 `Size<Dimension>`）。
- `Style` **无 `order` 字段**（CSS order 无法存 taffy；留 `ResolvedStyle.order` 待 v1 消费）。

### 3.2 ttf-parser 0.20（text/layout.rs）
- **`glyph_hor_advance(GlyphId) -> Option<u16>`**（非 `glyph_advance_width`，返回 u16 非 i16）。
- **kerning 在 `kern::Subtable`**：`face.tables().kern.subtables` 遍历（取 horizontal + 非状态机子表），`.glyphs_kerning(GlyphId, GlyphId) -> Option<i16>`。`Subtables` 是 `Copy`。
- `glyph_index(ch) -> Option<GlyphId>`（`GlyphId(pub u16)`）。
- `glyph_bounding_box(GlyphId) -> Option<Rect{i16}>`。bearing 用 `x_min`/`y_max`。
- `ascender()/descender()/line_gap()/units_per_em()` 在 `Face` 上。
- `Face::parse(&'static [u8], 0)`——v0 用 `Box::leak` 拿 `'static`（单字体 OK，多字体 v1 换 owned wrapper）。

### 3.3 cssparser 0.34（parse/css.rs）
- **不能用 NestingParser + parse_one_rule 草稿**。
- `DeclParser` 需实现三 trait：`DeclarationParser + QualifiedRuleParser + AtRuleParser`（`RuleBodyItemParser` 要求三者）。
- 用 `StyleSheetParser` 迭代器替代 `parse_one_rule` 循环。
- `parse_block` 参数是 `ParserState` 非 `SourcePosition`。
- v0 不解析 @ 规则（`AtRuleParser` 默认拒）。

### 3.4 scraper 0.19（parse/dom.rs）
- `Html::parse_document` → `select("body")` → `children()` 迭代。
- `ElementRef::value()` 取 Element，`.attrs()` 取属性迭代。
- `<img>` 是 void 元素（无闭合标签），src 从 `attrs` 取非 text。

## 4. AI 可预测性核心约束（首要准则，勿违背）

> LoomGUI 根本目的 = AI 驱动界面拼装。HTML 作 DSL 让 AI 能编辑+预测渲染。以下约束是 AI 可预测性的根基，违背即损害核心目的。

1. **div 默认 `flex-direction: column`**（§4.1）。`ResolvedStyle::default()` 设 `FlexDirection::Column`（taffy 默认是 Row！）。CSS 显式 flex-direction 无条件覆盖。AI 对 div「垂直堆叠」的先验成立。
2. **div 永远是 flex 容器，只装 flex item**。无浏览器 block/inline flow。文本+图混排进 `<l-rich>`（v1.x）。
3. **div/button 裸文本 → Text 子节点**（§4.2）。`<div>标题</div>` 产出 Container + Text 子「标题」（文本是 flex item），**不丢弃**。
4. **行内混排报错**（文本+元素同在）。解析期 Err，提示用 span/l-rich。
5. **围栏外元素报错不降级**。parse 白名单 `[div/span/img/button/l-container]`，其它 Err。
6. **margin 不折叠**（flex 语义）。子项间距用 `gap`，别用 margin（margin 在 LoomGUI 求和、Chrome block flow 折叠，Chrome 预览会骗 AI）。
7. **glyph 绝对坐标**（§9.2）。后端拼 quad 零累加。
8. **坐标系左上原点、y 向下**（§8.1）。核心代码无 `height-y` 翻转（翻转在后端根 Stage 一次性）。

## 5. v0 踩坑记录

### 坑 1：taffy 0.5 `MeasureFunc::Boxed` 不存在
**症状**：layout brief 写 `MeasureFunc::Boxed` 闭包，编译报无此变体。
**根因**：taffy 0.5.2 改用 `TaffyTree<NodeContext>` + `compute_layout_with_measure` FnMut 分发。
**解决**：见 §3.1。`Arc<Font>` carry 作废（FnMut 借用合法）。
**教训**：brief 的 API 草稿是起点非权威，按编译器 + crate 实际版本调。

### 坑 2：ttf-parser 0.20 advance/kerning API 改名
**症状**：`glyph_advance_width`/`kerning_for` 编译失败。
**根因**：0.20 改名 `glyph_hor_advance`（返 u16）；kerning 移到 `kern::Subtable`。
**解决**：见 §3.2。
**教训**：ttf-parser 跨版本 API 变动大，查 `~/.cargo/registry` 源码确认。

### 坑 3：默认 flex-direction 没落地 column
**症状**：未显式写 flex-direction 的 div 水平排列，违反 §4.1。
**根因**：`ResolvedStyle::default()` 用 taffy `Style::DEFAULT`（flex_direction=Row）。实现者一度用测试 CSS 掩盖（加 flex-direction:column 让测试过），final review 抓出。
**解决**：Default impl 设 Column；CSS 显式声明无条件覆盖（时序 default→apply_decl）。加 `default_div_is_column` 回归测试。
**教训**：AI 可预测性核心约束必须在**默认值层**落地，不能靠测试 fixture 掩盖。

### 坑 4：围栏外元素静默降级 Text
**症状**：`<video>` 等围栏外 tag 被当 Text 节点。
**根因**：scene 层 `_ => NodeKind::Text` fallback。
**解决**：parse 层白名单报错（执法点在 parse 非 scene），scene 删 fallback 改 `unreachable!`。
**教训**：「报错不降级」类约束执法点要对（parse），下游信任输入。

### 坑 5：div/button 裸文本被丢弃
**症状**：`<div>标题</div>` 无文本输出。
**根因**：scene `build_rec` 对 Container 不处理 `el.text`。
**解决**：`build_text_child` 生成 Text 子节点（继承父 8 文本字段，size=Auto 不污染高度）。
**教训**：spec §4.2「Text 叶子是 Container 子节点」——裸文本该成 flex item 子节点，非丢弃。

### 坑 6：cascade specificity 排序方向反
**症状**：多规则命中时低 specificity 胜。
**根因**：`match_element` 返回降序，直接顺序 apply 让低优先级后写覆盖高优先级（反了）。
**解决**：`resolve_styles` 加 `sort_by_key` 升序（高 specificity 后 apply 胜，稳定排序保同级 source 顺序）。
**教训**：接 `match_element` 时核对排序方向；CSS cascade 是高 specificity 胜。

### 坑 7：后代选择器只查直接父
**症状**：`div.a span` 在 `<div class=a><div><span>` 不命中。
**根因**：`matches()` 只查 parent 不递归祖先；`Combinator` 字段未用。
**解决**：`match_compound_chain` 递归祖先（Descendant 沿父链搜+回溯，Child 只直接父）。附带修 `parse_selector` 空格降级 Child bug。
**教训**：围栏声明「后代/子代」选择器就要真实现递归祖先。

### 坑 8：snapshot 绑系统 arial.ttf
**症状**：Linux CI（DejaVuSans 无 arial）snapshot 漂移。
**根因**：测试字体试系统路径。
**解决**：锁仓库内 `tests/fixtures/DejaVuSans.ttf` + `env!(CARGO_MANIFEST_DIR)`；fixture 用 ASCII（DejaVuSans 无 CJK）。
**教训**：测试产物跨平台一致就锁仓库内资源；CJK 渲染验证留 v1（需 CJK 字体）。

## 6. 调试/验证技巧

- `cargo test -p loomgui_core`：全量（v0 ~52 测试）。
- `cargo run --example v0_snapshot`：端到端产 `v0_snapshot.json`。
- insta 快照：`INSTA_UPDATE=always cargo test --test snapshot` 首次接受，再裸跑锁定。
- 字体路径：`format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"))`。
- 改 `ResolvedStyle` 默认/映射后，跑 layout + snapshot 测试看布局变化。
- taffy 布局调试：看 `Node.layout_rect`（solve 回写的绝对坐标）。
- 查 crate 实际 API：`~/.cargo/registry/src/<crate>-<ver>/src/`。

## 7. 已知问题/未完成（v0 ledger）

**v0 占位 → v1.x 优化**：
- mask_context id = counter+1 不稳定（节点增删抖动）。
- sort_key 无 FairyBatching AABB 重排（v0 保序）。
- 断行贪心非 UAX#14（CJK kinsoku 留 v1.x）。
- baseline 未对 Chrome 校准（§9.1 实现期调）。
- Font 用 `Box::leak` 不释放（多字体 v1 换 owned）。
- tex_id 16 位 hash 碰撞（单页面无忧）。
- grayed 恒 false / BlendMode 仅 Normal。
- CSS order 排序跳过（taffy 0.5 无 order 字段，DOM 序）。
- border_width 仅取 top（非均匀 border）。
- opacity % 语义（`50%`→50.0 非 0.5，brief 原行为）。

**v0 defer → v1 阶段**：
- 打包器 loomgui_pkg（v1 第一阶段，G1）：v0 内存直通。
- 纹理加载（后端职责，G7）：v0 占位 tex_id。
- FFI csbindgen + SOA arena（G11）：v0 纯 Rust。
- event/命中/输入（G4）：v0 静态。
- anim GTween/ScrollPane（§11/§12.7）：v0 静态。
- Unity 后端镜像（G9-G14）：v0 无引擎。
- NativeHost/virtualization/shape mask：v1.x。

完整 defer 表见 `docs/superpowers/specs/2026-06-18-v0-skeleton-design.md` §7。

## 维护

每次 LoomGUI 开发/修复后，用 `session-summary` skill 把新踩坑/机制/调试技巧总结进本文件（§5 加坑、§3 加 API、§7 更 ledger）。本 skill 与代码一起提交。
