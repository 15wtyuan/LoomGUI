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
├── loomgui_ffi_c/            # C ABI + csbindgen（v1a Phase 1 ✅ 已实现）
├── loomgui_unity/            # Unity 6.5 URP 后端（v1a Phase 1 ✅ 已实现）
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

### 2.8 FFI（loomgui_ffi_c，主文档 §14，v1a Phase 1）
- `extern "C"` 薄包装 + opaque `*mut StageHandle`；csbindgen 扫 `src/lib.rs` 生成 C# `Native` 类。
- ABI：`stage_new/free/load_html/tick/borrow_frame/shutdown`。string 走 UTF-8 `*const u8`+len；`borrow_frame(h, *mut usize) -> *const u8` 返 Rust 拥有的帧 blob（下 tick 失效；未 tick 返 null+len=0）。
- `StageHandle{ stage, frame_blob: Vec<u8> }`——tick 时 `build_blob` 覆写 frame_blob。
- `build_blob(&[RenderNode]) -> Vec<u8>`：SOA 公共头 11 列（node_id/parent_id(i32,-1=none)/visible/alpha/sort_key/local_x/local_y/mask_context/payload_kind/mesh_off/mesh_len）+ mesh arena（vert_count/idx_count/verts/uvs/colors/indices）。**mesh 顶点 re-base 到节点本地**（减 transform.x/y——C# 巢状 GO 靠 localPosition 定位）。全 LE。

### 2.9 Unity 后端（loomgui_unity，主文档 §14，v1a Phase 1）
- `FrameBlob`（BitConverter 解析 blob）→ `MirrorPool.Sync`（`Dictionary<uint,RenderObj>` O(n) stale-flag diff：标 stale→遍历命中清 stale/更新→余销毁；按 `parent_id` 巢状 GO，`localPosition=(local_x,local_y)`，`sortingOrder=sort_key`，mesh 上传，payload_kind 2/0 跳过）。
- `MaterialManager`：key=(program, texture, mask_context)，同 key 复用 Material。**tint×alpha 已 baked 进顶点色（Rust 侧）**，材质只带 texture+clip_box+blend（SrcAlpha/OneMinusSrcAlpha）。
- `LoomStage`（`[ExecuteAlways]` MonoBehaviour）：LateUpdate `tick→borrow_frame→Marshal.Copy→FrameBlob→MirrorPool.Sync`。根 GO `localScale=(sf,-sf,sf)`（shrink-to-fit sf=min(sw/dw,sh/dh) + y-flip 合一）+ `localPosition=(-sw/2,sh/2,0)`；UI 相机正交 `orthoSize=sh/2` `cullingMask=1<<6`(LoomUI) **独立于根**（不 SetParent）。shader `Cull Off`（根翻转 winding）。
- URP unlit shader：`col=tex2D×v.color`、`Cull Off`、`ZWrite Off`、`Blend[_Src][_Dst]` property、`CLIPPED` variant（rect mask `_ClipBox` discard，Phase 2 启用）。图片 v1a 占位 1×1 白贴图；Text Phase 2。

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

### 3.5 csbindgen 1.9（loomgui_ffi_c/build.rs + 生成 LoomGUIBindings.cs）
- 默认生成 **`internal`** 类型（`Native` 类、`StageHandle` 结构）→ 跨程序集（LoomGUI.Bindings→LoomGUI.Runtime）访问须 `[assembly: InternalsVisibleTo("LoomGUI.Runtime")]`（放 AssemblyInfo.cs）。
- 类型映射：`*const u8`→`byte*`、`*mut usize`→`nuint*`、opaque `*mut T`→`T*`（**类型化指针非 IntPtr**）。`csharp_use_function_pointer(false)` 切 Mono 模式。
- `CString::as_ptr()` 返 `*const c_char`(i8)，签名为 `*const u8` 时须 `as *const u8` cast。
- build.rs 跑两次（OUT_DIR 必成 `.expect`；Unity 目录那次失败要 `cargo:warning=` 勿 `let _ =` 吞错）。
- C# `fixed(T* p=&localVar)` **非法**（CS0213 "already fixed"）——局部栈上已固定，直接 `&localVar` 传；`fixed` 只 pin 托管对象（数组/string）。

### 3.6 Unity 6.5（6000.5）C# API
- `Object.GetInstanceID()` **废弃**→`GetEntityId()`，后者返 **`EntityId`（非 int）**，`EntityId→int` 隐式转换**也废弃**（"将来不能 int 表示"）。**绕开整条**：缓存 key 直接持 Object 引用（`Dictionary<Texture,...>`，Unity 对象引用同一性），别碰任何 id API。
- **EditMode 禁 `Object.Destroy`**（报 "Destroy may not be called from edit mode"），须 `DestroyImmediate`。`[ExecuteAlways]` 组件生产代码 EditMode 也跑 → `Application.isPlaying ? Destroy : DestroyImmediate`（Mesh 是独立 UnityEngine.Object，GO 销毁不连带，须显式销毁防泄漏）。
- `Camera.nearClipPlane` **须 >0**（负值抛异常）。
- Unity 开着**锁 native `.dll`**——重建/拷 `.dll` 须先关 Unity（锁文件 `Temp/UnityLockfile` 可能残留非真锁，以能否 `rm` 为准）。
- 生成物 gitignore：`.slnx`(6.5 解决方案)、csbindgen `.cs` 绑定及其 `.cs.meta`；`.dll`（`!**/Plugins/**/*.dll` 白名单）入库；`.meta` 是 Unity 资产元数据须入库。

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
**解决**：锁仓库内 `tests/fixtures/DejaVuSans.ttf` + `env!("CARGO_MANIFEST_DIR")`；fixture 用 ASCII（DejaVuSans 无 CJK）。
**教训**：测试产物跨平台一致就锁仓库内资源；CJK 渲染验证留 v1（需 CJK 字体）。

### 坑 9：color_tint 把背景色块涂黑（v1a T8）
**症状**：PlayMode 红背景块渲成黑色。
**根因**：v0 `ResolvedStyle::default().color=[0,0,0,1]`（CSS `color` 默认黑，是**前景/文本色**）；blob 烘焙 `bg×color_tint×alpha` 把红背景乘成不透明黑。
**解决**：build_blob **不乘 color_tint**——顶点色 = background_color，仅 `alpha×node opacity`。color_tint 是文本色（Phase 2 文本用）。
**教训**：mesh colors 已是最终色（bg-color / 图片白），别再叠 color_tint；§4.2b「tint×alpha 烘焙」指**文本/图片** tint，非背景色块。

## 6. 调试/验证技巧

- **★ 实现 v1+ 后端/渲染/对象模型前，先参考 `temp/FairyGUI-unity/` 源码**（对照机制、避免走歪——本 session 因没先看 fgui 的 sortingOrder/rect-mask/MaterialManager，初版设计走了弯路：误用 z 排序、误以为 rect mask 要独立 GO、把绘制序想复杂）。
- `cargo test -p loomgui_core`：全量（v0 ~52 测试）。
- `cargo run --example v0_snapshot`：端到端产 `v0_snapshot.json`。
- insta 快照：`INSTA_UPDATE=always cargo test --test snapshot` 首次接受，再裸跑锁定。
- 字体路径：`format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"))`。
- 改 `ResolvedStyle` 默认/映射后，跑 layout + snapshot 测试看布局变化。
- taffy 布局调试：看 `Node.layout_rect`（solve 回写的绝对坐标）。
- 查 crate 实际 API：`~/.cargo/registry/src/<crate>-<ver>/src/`。
- Rust→Unity 闭环：改 Rust 后 `cargo build -p loomgui_ffi_c --release` → 关 Unity → `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/`。
- Unity 验证：Test Runner EditMode（`Window→General→Test Runner`）；PlayMode 看 Game 视图渲染；PlayMode 前确认 `.dll` 是最新版。
- 跨语言 round-trip：Rust `build_blob` ↔ C# `FrameBlob` 靠手搓 blob byte[] 的 EditMode 测互验（blob 布局是 Rust↔C# 契约，两端须字节级一致；改列/偏移必同步）。

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

**v1a Phase 1 ✅ 完成（merged main @ 7920bbd）**：FFI crate（loomgui_ffi_c：csbindgen + SOA blob）+ Unity 6.5 URP 后端镜像（FrameBlob/MirrorPool/MaterialManager/LoomStage/shader）。静态色块在 Unity Game 视图真渲染——**v1 最大风险缝闭合**。spec `docs/superpowers/specs/2026-06-19-v1a-unity-render-design.md`、plan `docs/superpowers/plans/2026-06-19-v1a-unity-render-phase1.md`。

**v1a Phase 1 defer → Phase 2**（v1a 只渲静态色块）：
- rect mask：`_ClipBox` shader discard + mask_context 材质 + CLIPPED keyword（shader variant 已预留）。
- 文本：`Font.RequestCharactersInTexture`+`GetCharacterInfo` 取 UV/bbox（**丢弃 Unity advance**，位置按 Rust TextLayout）+ `Font.textureRebuilt` 监听 atlas rebuild 重取 UV。
- 500 节点静态压测。
- Domain reload 完整保护：`[RuntimeInitializeOnLoadMethod(SubsystemRegistration)]` 调 `loomgui_shutdown` + 清 C# 缓存（v1a 仅占位）。

**v1a Phase 1 perf defer → v1e**（spec §2 显式 defer）：
- `MirrorPool.UploadMesh` 每帧分配 `Vector3[]/Color[]/int[]`（改 List 池化 + `SetVertices(List)`）。
- shader 非 CLIPPED 路径无条件算 clipPos（fgui 用 `#ifdef` 守卫）。
- ArrayPool 帧拷贝（v1a 先 `new byte[]`）、冷帧/换页帧 FFI ≤2ms。

**v1 其余 defer（v0 起，未动）**：
- 打包器 loomgui_pkg + 真纹理加载（v1b，G1/G7）。
- event/命中/输入（v1c，G4）、anim GTween/ScrollPane（v1d，§11/§12.7）。
- NativeHost/virtualization/shape mask：v1.x。

完整 defer 表见各 spec §7；v1a Phase 1 实现 ledger 见 `.git/sdd/progress.md`。

## 维护

每次 LoomGUI 开发/修复后，用 `session-summary` skill 把新踩坑/机制/调试技巧总结进本文件（§5 加坑、§3 加 API、§7 更 ledger）。本 skill 与代码一起提交。
