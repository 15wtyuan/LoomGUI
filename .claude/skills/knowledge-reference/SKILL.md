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

> **★ 工作准则（牢记）：动任何机制前先对照 fgui 源码。** LoomGUI 的渲染/对象模型/批合/事件/动画/资源管线全面借鉴 FairyGUI（参考实现 `temp/FairyGUI-unity/`）。**实现任何功能前**先 grep/读 fgui 对应文件看它怎么做的，再定 LoomGUI 设计——避免走歪。本 session 因没先看 fgui 的 sortingOrder/rect-mask/MaterialManager，初版设计走了弯路（误用 z 排序、误以为 rect mask 要独立 GO、绘制序想复杂）。对照时注意 fgui 是 Built-in RP（URP/shader/材质 API 要适配，见 §3.5/3.6）。

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
- `Glyph { glyph_id, codepoint, x, y, bearing_x, bearing_y }` 绝对坐标。**codepoint**（v1a Phase 2 加）供引擎字体 API（Unity `GetCharacterInfo(char)` 按码点非 glyph_id）。
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
- `Stage::new(font_path, root_size)` → `load_inline(html, css)` → `tick_and_render()` → `FrameData{nodes:Vec<RenderNode>, clips:Vec<ClipEntry>}`（v1a Phase 2：clips=嵌套交集后的 clip 表）→ `render_json()`。
- 静态首帧：tick 接空输入、dt=0。

### 2.8 FFI（loomgui_ffi_c，主文档 §14，v1a Phase 1）
- `extern "C"` 薄包装 + opaque `*mut StageHandle`；csbindgen 扫 `src/lib.rs` 生成 C# `Native` 类。
- ABI：`stage_new/free/load_html/tick/borrow_frame/shutdown`。string 走 UTF-8 `*const u8`+len；`borrow_frame(h, *mut usize) -> *const u8` 返 Rust 拥有的帧 blob（下 tick 失效；未 tick 返 null+len=0）。
- `StageHandle{ stage, frame_blob: Vec<u8> }`——tick 时 `build_blob` 覆写 frame_blob。
- `build_blob(&FrameData) -> Vec<u8>`（**v2**, version=2）：SOA 公共头 **13 列**（v1 的 11 + `text_off`/`text_len`）+ mesh arena + **text_arena**（per text 节点 `font_size:u32|color:f32×4|glyph_count:u32|glyphs[{codepoint,pen_x,pen_y}]`）+ **clip 表**（`context_id→design rect`，嵌套交集）。magic+version 进 header，C# `FrameBlob.IsValid` 校验（防 stale v1 blob）。**mesh 顶点 re-base 到节点本地**（减 transform.x/y）。全 LE。改 blob 格式必重编+换 .dll（坑 10）。

### 2.9 Unity 后端（loomgui_unity，主文档 §14，v1a Phase 1）
- `FrameBlob`（BitConverter 解析 v2 blob，`IsValid` 校验 magic+version）→ `MirrorPool.Sync`（`Dictionary<uint,RenderObj>` O(n) stale-flag diff）。**flatten（Phase 2）**：所有 GO 挂**根**（非巢状——local_x/local_y 是绝对 design 坐标，巢状 SetParent 会双计父位置，坑见 §2.11/Phase 1 单节点未暴露），`localPosition=绝对`、`sortingOrder=sort_key`；kind=1 Mesh / kind=2 Text（→TextRasterizer）/ kind=0 跳过。**buffer 复用**：RenderObj 持可复用 List，`SetVertices(List)` 零 alloc（T7，500 节点压测）。
- `MaterialManager`：key=(program, texture, mask_context)——mask_context 进 key → 每 ctx 独立 Material 持各自 `_ClipBox`；ctx>0 → `EnableKeyword("CLIPPED")`（`#pragma multi_compile`）+ `SetClipBox`。**tint×alpha baked 进顶点色（Rust 侧）**，材质只带 texture+clip_box+blend。
- `LoomStage`（`[ExecuteAlways]` MonoBehaviour）：LateUpdate `tick→borrow_frame→Marshal.Copy→FrameBlob→MirrorPool.Sync`。根 GO `localScale=(sf,-sf,sf)`（shrink-to-fit sf=min(sw/dw,sh/dh) + y-flip 合一）+ `localPosition=(-sw/2,sh/2,0)`；UI 相机正交 `orthoSize=sh/2` `cullingMask=1<<6`(LoomUI) **独立于根**（不 SetParent）。shader `Cull Off`（根翻转 winding）。Phase 2：`[SerializeField] Font _font`（EnsureFont 兜底 AssetDatabase 加载 DejaVu）、`Font.textureRebuilt+=OnRebuilt`（OnDestroy 解绑）、`ResetStatics`（`SubsystemRegistration` 调 `loomgui_shutdown`+`TextRasterizer.ResetStatic`）、**Awake 清 root 下 loom_node 孤儿 GO**（ExecuteAlways 防累积，坑 11）。
- URP unlit shader：`col=tex2D×v.color`、`Cull Off`、`ZWrite Off`、`Blend[_Src][_Dst]` property、`CLIPPED` variant（rect mask `_ClipBox` discard，Phase 2 启用）。图片 v1a 占位 1×1 白贴图；**Text Phase 2 ✅**（font atlas）。

### 2.10 文字渲染链（v1a Phase 2，§9/§14）
- **Rust 笔位权威 + Unity 纯光栅**（偏离 fgui 的 advance/行高，§9.1 跨平台根）。blob text_arena 每 text 节点 = `font_size:u32|color:f32×4|glyph_count:u32|glyphs[{codepoint:u32,pen_x:f32,pen_y:f32}]`；pen=GO-local（content 偏移 Rust 烤进），pen_y=`line.y+line.baseline`，**不 re-base**（pen 已节点局部，与 mesh re-base 不同）。
- Unity `TextRasterizer.BuildMesh`：`RequestCharactersInTexture(串,font_size)` 填 atlas → 每 glyph `GetCharacterInfo((char)codepoint,font_size)` 取 UV 四角 + 像素 box(`minX/maxY/maxX/minY`)，quad 摆 `pen+box`（y-down：`top=pen_y−maxY,bottom=pen_y−minY`），顶点序 BL/TL/TR/BR 对齐 fgui `DrawGlyph`。**不用** `CharacterInfo.advance` / `fontSize×1.25` 行高。
- **必修坑**：`Font.textureRebuilt` 静态事件 → `s_fontVersion++` → MirrorPool 下帧检测版本变 → 强制 text 节点重 BuildMesh（atlas rebuild 后 glyph UV 变，不监听画花字）。照搬 fgui `DynamicFont.cs:356-375`。
- material key=(program=1, `font.material.mainTexture`, mask_context)；texture=动态字体 atlas。

### 2.11 rect mask `_ClipBox`（v1a Phase 2，§8.6/§14）
- Rust：batch DFS 算**嵌套 clip 交集**（祖先 clip 链累乘交，disjoint→零面积 Rect 非 None），emit clip 表 `context_id→绝对 design rect`；修 v0「只裁最内层」bug。mask_context 进 material key。
- Unity：design rect→world（**根 transform** `TransformPoint` 两角，非逐 clipper 矩阵——clip 是绝对 design 非 clipper-local）→ `_ClipBox=(-cx/hw,-cy/hh,1/hw,1/hh)`（照搬 fgui `UpdateContext.cs:105-156`）；零 half→safe-blank `(-2,-2,0,0)` 防 div0。MirrorPool 每 ctx 每帧首次 SetClipBox（fgui `firstMaterialInFrame`）。shader CLIPPED：`clipPos=TransformObjectToWorld(pos).xy×zw+xy`，`col.a*=step(max(abs(clipPos)),1)`。
- **坐标模型**：blob local_x/local_y + clip_rect 均**绝对 design**（layout 累加父 origin）；后端 flatten 挂根 GO，根 transform 一次性映射 design→world。nesting+父相对坐标留 v1c（transform 继承/事件）。

### 2.12 打包器 + 包格式（v1b.1，§12/§5.5）
- **`.pkg.bin` 是 Rust-internal**：`loomgui_pkg` 写、core runtime 读，**C# 永不解析**（Unity 只读文件→bytes→`load_package`）。与 frame blob（Rust↔C# 跨语言契约）本质不同——无需 C# reader/跨语言字节对齐，style 可直接 bincode 投影。
- v1 格式（扁平 + stringTable，LE）：Header 28B（magic `LPKG`=0x474B504C + version=1 + flags + nodeCount + stringCount + rootSizeX/Y）+ StringTable（u16 len+UTF8，**只** text content + image src）+ NodeBlock（每节点 parentIndex i32 + kind u8 + styleLen u32 + `bincode(ResolvedStyle)` blob + textIdx/srcIdx u16；NULL_IDX=0xFFFF；kind 0=Container/1=Button/2=Image/3=Text）。indexTable/压缩/分支推 formatVersion=2（v1x-deferred §6）。
- **StyleRecord = bincode(ResolvedStyle)**：taffy 开 `serde` feature（§3.7），ResolvedStyle/TextAlign 加 `Serialize/Deserialize/PartialEq` 派生——穷尽由派生保证（加字段编译期强制覆盖 encode/decode，R3≈0）。font_family 随 blob 走（不进 stringTable）。
- `asset::write_package(&Scene,root_size)->Vec<u8>` / `read_package(&[u8])->Result<(Scene,root_size),PkgError>`（常驻，不依赖 parse）。read 全 `Result` 无 panic 跨 FFI（Reader 截断保护）。版本协商：magic + formatVersion∈[1,1]（fgui 缺，主设计 §12.2 要）。
- **`Scene::build(&[(Option<usize>,NodeKind,ResolvedStyle)])`** 共享建树（常驻，不依赖 parse）——`build_scene`（parse 路径，gate）与 `read_package`（runtime 路径）共用，防建树逻辑分叉（R2）。NodeId=entries 下标；children 按 DFS 先序填。
- **parse feature gate**：`scraper`/`cssparser` optional + `parse` feature（default on）。gate 在 parse 后：`parse/` 模块 + `style::cascade` + `build_scene`/`gather_rec` + `Stage::load_inline` + 用 parse 的测。常驻：`ResolvedStyle`/`TextAlign`/`Scene::build`/`mapping`/layout/render/text/scene/stage(除 load_inline)/asset。`loomgui_ffi_c`：`load_package` 常驻、`load_html` gate；**dev .dll 仍带 parse**（PlayMode inline 迭代要），gate 价值=架构正确 + 将来精简 build。构建矩阵门：`cargo build -p loomgui_{core,ffi_c} --no-default-features` 皆编。
- **黄金等价测**（最强门）：`pkg→load_package→render_json` == `inline load_inline→render_json`（包路径与 inline 渲染逐节点等价，验收 #6）。fixture 覆盖 div/text/img/rect mask。
- `loomgui_pkg` CLI（不引 clap，`std::env::args`）：`pack(html,css,root_size)` = `parse_html→parse_css→resolve_styles→build_scene→write_package`。packager **不**加载字体/不 solve/不 render。

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
- 生成物 gitignore：`.slnx`(6.5 解决方案)、csbindgen `.cs` 绑定及其 `.cs.meta`；`.dll`（`!**/Plugins/**/*.dll` 白名单）入库；`.meta` 是 Unity 资产元数据须入库（**implementer 提 .cs 易漏 .meta**——Unity 关着时不生成，坑 13）。
- **动态字体 API**（Text 光栅）：`Font.RequestCharactersInTexture(string, fontSize, FontStyle)` 填 atlas（必先调，否则 GetCharacterInfo 恒 false）→ `Font.GetCharacterInfo(char, out CharacterInfo, fontSize, FontStyle) -> bool` 取 `minX/maxY/maxX/minY`（像素 box）+ `uvBottomLeft/uvTopLeft/uvTopRight/uvBottomRight`。`CharacterInfo.advance` 存在但 LoomGUI **不用**（Rust 笔位）。`Font.textureRebuilt` 是**静态**事件（register/OnDestroy 解绑防泄漏）。
- **`HideFlags.DontSaveInEditor`**：`[ExecuteAlways]` 程序生成 GO 标之防被存进场景（否则 EditMode dirty 场景 + Play/Stop 累积残留，坑 11）。
- **`Mesh.SetVertices(List)`/`SetUVs`/`SetColors`/`SetTriangles(List)` overload**：零 per-frame 数组 alloc（vs `SetVertices(Vector3[])`）。`List.Clear()` 保 Capacity，warm-up 后复用零 alloc。
- **shader keyword**：`#pragma multi_compile _ CLIPPED`（两 variant 都编，`EnableKeyword` 切换生效）**非** `shader_feature`（未启用的 variant 会被 strip → clip 静默失效）。

### 3.7 taffy 0.5.2 serde + bincode 1.x（style/resolved.rs + asset/mod.rs，v1b.1）
- taffy 0.5.2 有 **`serde` feature**：`Style`（style/mod.rs:189）及全部字段类型（geometry/dimension/flex/grid/alignment）都 `#[cfg_attr(feature="serde", derive(Serialize,Deserialize))]` + `#[serde(default)]`；`Style` 还派生 `PartialEq`。开 `taffy = { version="0.5", features=["serde"] }` 后，含 `taffy_style: taffy::style::Style` 的 `ResolvedStyle` 能整体 `#[derive(Serialize,Deserialize,PartialEq)]`。
- bincode 1.x：`bincode::serialize(&x)->Vec<u8>` / `bincode::deserialize::<T>(&bytes)`。`#[serde(default)]` 在 bincode（位置编码无缺字段概念）下透明。用于包格式的 StyleRecord——穷尽由 serde 派生保证，比手写枚举 taffy 30+ 字段稳健（R3≈0）。
- bincode 格式随 taffy/bincode 版本——升级时 bump 包 `formatVersion`。

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

### 坑 10：Rust 改 blob 格式后 .dll 没换 → C# 静默拒帧不渲染（v1a Phase 2）
**症状**：PlayMode **啥都不渲**（红块文字全无）、Console **干净无错**。
**根因**：Plugins 里 `.dll` 是旧的（Phase 1），产 **v1 blob（version=1）**；T1 起 C# `FrameBlob.IsValid` 只认 version==2 → `MirrorPool.Sync` 第 1 行 `if(!IsValid)return` **静默早退**。Unity 开着锁 .dll，编译后没换。
**解决**：`cargo build --release` → **关 Unity**（锁 .dll）→ `cp target/release/loomgui_ffi_c.dll Plugins/LoomGUI/` → 重开。
**教训**：**任何 Rust FFI 改动（尤其 blob/ABI 格式）后，PlayMode 验前必重编+换 .dll**。症状"全不渲+Console 干净"先怀疑 stale .dll（`md5sum` 对比 fresh build）。Unity 开着锁 .dll，换 .dll 必关 Unity。

### 坑 11：ExecuteAlways 程序生成 GO 累积泄漏（v1a Phase 2）
**症状**：Play/Stop 反复 + domain reload 后 `loom_node` GO 在 Hierarchy 累积、内存泄漏。
**根因**：`[ExecuteAlways]` EditMode 也跑，MirrorPool 产的 `loom_node` GO 挂 root 下被**存进场景**；每次 Awake 新 `_pool` 丢旧引用 → 孤儿 GO 清不掉。
**解决**：GO/Mesh 标 `HideFlags.DontSaveInEditor`（不入存盘）+ Awake 开头清 root 下 `loom_node` 孤儿 GO。
**教训**：ExecuteAlways + 程序生成 GO 必加 DontSave + 开局清孤儿；OnDestroy 的 `pool.Clear` 只清当前 run 的，跨 run 孤儿要 Awake 清。

### 坑 12：手搓 blob 测 fixture 写 AoS，多节点读串列（v1a Phase 2）
**症状**：EditMode 跑 `MirrorPoolFlattenTests` 报 `SetTriangles: idx 非三的倍数`。
**根因**：C# 手搓 2 节点 blob 写 **AoS**（node0 全字段、node1 全字段）但列 offset 按 1 节点 elemSize 递进、`FrameBlob` 读 **SOA**（列优先 `ColOff(idx)+i*elemSize`）→ node1 每字段读串一位，mesh_off 落到 node0 mesh_len → idx 读成垃圾。
**解决**：fixture 列 offset 按 `NodeCount×elemSize` 递进、数据列优先写（镜像 `blob.rs`）。单节点 fixture AoS≡SOA 不受影响（故 Phase 1 没暴露）。
**教训**：手搓 blob byte[] 测必 SOA 列优先，与 `blob.rs`/`FrameBlob` 一致；多节点才暴露（单节点掩盖）。

### 坑 13：implementer 提 .cs 漏 .meta（v1a Phase 2）
**症状**：合 main 后 4 个新 `.cs` 的 `.meta` 没入库（ClipMath/ClipBoxTests/FrameBlobV2Tests/MirrorPoolFlattenTests）。
**根因**：Unity 关着时 import 不生成 `.meta`；subagent 提 .cs 时 Unity 未开 → .meta 后生成、未 add。TextRasterizer.cs.meta 这次提了（那次 Unity 开着），其余漏。
**解决**：合 main 后补提交漏的 .meta；或 implementer 提前确保 Unity 开过一次生成 .meta。
**教训**：Unity `.meta` 随 `.cs` 入库；subagent 流程里提 .cs 后 controller 验 `.meta` 是否齐（Unity 生成的资产元数据，缺则 GUID 不稳）。

### 坑 14：Unity GetCharacterInfo 要 codepoint 非 glyph_id（v1a Phase 2）
**症状**：text_arena 只有 ttf `glyph_id`，Unity 取不到字。
**根因**：Unity `Font.GetCharacterInfo(char, ...)` 按 **Unicode 码点**，非 ttf glyph_id；ttf glyph_id 是字体内部字形索引。
**解决**：Rust `Glyph` 加 `codepoint:u32`（`measure_text` 遍历 char 时填），text_arena 送 codepoint，Unity `(char)codepoint` 调 GetCharacterInfo。
**教训**：引擎字体 API 多按码点；核心 Glyph 须同时持 glyph_id（ttf 直连后端）+ codepoint（引擎字体 API）。

### 坑 15：新二进制格式 magic 撞既有格式（v1b.1）
**症状**：v1b.1 包格式初拟 magic `"LOOM"`(0x4D4F4F4C)，与 frame blob 的 `MAGIC`（blob.rs）**完全相同**。
**根因**：两种格式独立，但 magic 是唯一识别码；撞了 magic→校验形同虚设（误传 frame blob 给 `load_package` 会过 magic 检查再挂错）。
**解决**：包改独立 magic `"LPKG"`(0x474B504C，磁盘字节 `4C 50 4B 47`)。planning 期 grep 现有 magic 才发现。
**教训**：新增二进制格式先 `grep -r 'MAGIC\|0x4D4F4F4C' src/` 确认 magic 唯一；formatVersion 是「同格式的版本」，magic 是「这是哪种格式」，两者正交。

## 6. 调试/验证技巧

- **★ 实现 v1+ 后端/渲染/对象模型前，先参考 `temp/FairyGUI-unity/` 源码**（对照机制、避免走歪——本 session 因没先看 fgui 的 sortingOrder/rect-mask/MaterialManager，初版设计走了弯路：误用 z 排序、误以为 rect mask 要独立 GO、把绘制序想复杂）。
- `cargo test -p loomgui_core`：全量（v0 ~52 测试）。
- **feature gate 构建矩阵**（v1b.1）：`cargo build -p loomgui_core --no-default-features` + `-p loomgui_ffi_c --no-default-features` 皆编（证 runtime 可无 parser）；`cargo build -p loomgui_pkg`（带 parse）。
- **打包器冒烟**（v1b.1）：`cargo run -p loomgui_pkg -- in.html in.css -o out.pkg.bin -w 1080 -h 1920`；前 4 字节 `4c 50 4b 47`="LPKG"。
- `cargo run --example v0_snapshot`：端到端产 `v0_snapshot.json`。
- insta 快照：`INSTA_UPDATE=always cargo test --test snapshot` 首次接受，再裸跑锁定。
- 字体路径：`format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"))`。
- 改 `ResolvedStyle` 默认/映射后，跑 layout + snapshot 测试看布局变化。
- taffy 布局调试：看 `Node.layout_rect`（solve 回写的绝对坐标）。
- 查 crate 实际 API：`~/.cargo/registry/src/<crate>-<ver>/src/`。
- Rust→Unity 闭环：改 Rust 后 `cargo build -p loomgui_ffi_c --release` → 关 Unity → `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/`。
- Unity 验证：Test Runner EditMode（`Window→General→Test Runner`）；PlayMode 看 Game 视图渲染；PlayMode 前确认 `.dll` 是最新版。
- 跨语言 round-trip：Rust `build_blob` ↔ C# `FrameBlob` 靠手搓 blob byte[] 的 EditMode 测互验（blob 布局是 Rust↔C# 契约，两端须字节级一致；改列/偏移必同步）。**手搓多节点 fixture 必 SOA 列优先**（坑 12，单节点掩盖 AoS 错）。
- **stale .dll 诊断**：PlayMode **全不渲 + Console 干净** → `md5sum target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`，不等 = stale（Rust 改 blob/ABI 格式没换 .dll，坑 10）。
- **T7 perf 基线**：500 节点静态 ~5-8ms/帧（120-200fps，无卡顿过 §9.3）。成本 = 朴素每帧全量重传（Rust 没 dirty/Unchanged 跳过 + `ReadMesh` per-frame alloc 数组）；优化（dirty 跳静态≈0 + ArrayPool 冷帧≤2ms）归 v1e。
- PlayMode 验前 checklist：① Rust 改过 → 重编+换 .dll（关 Unity）② LoomStage `_font` 赋值 ③ Console 看红字 ④ Hierarchy 看 GO 不累积。

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

**v1a Phase 2 ✅ 完成（merged main @ 9889afa）**：Text（Rust 笔位+Unity 光栅+textureRebuilt，§2.10）+ rect mask（嵌套交集+_ClipBox，§2.11）+ 500 节点压测（buffer 复用）+ Domain reload（ResetStatics 接 shutdown）。**v0 fixture（div+文本+img+rect mask）在 Unity 真渲，500 节点静态无卡顿，进出 Play 不 crash**。spec `docs/superpowers/specs/2026-06-19-v1a-unity-render-phase2-design.md`、plan `docs/superpowers/plans/2026-06-19-v1a-unity-render-phase2.md`。踩坑：.dll 重编换（坑 10）、ExecuteAlways GO 泄漏（坑 11）、SOA fixture（坑 12）、codepoint（坑 14）。

**v1a Phase 2 defer → v1e**（perf，spec §4.5 / §7）：
- **静态帧朴素全量重传**：Rust 没做 dirty/`Unchanged` 跳过（每帧对所有节点 emit Mesh）→ MirrorPool 每帧重传全部。优化：Rust dirty 跟踪 emit Unchanged → 静态帧≈0。T7 基线 ~5-8ms/500 节点。
- `FrameBlob.ReadMesh` 仍 per-frame alloc `MeshSegment` 数组（UploadMesh List 复用 T7 已做，ReadMesh 没做）→ ArrayPool 化。
- `TextRasterizer.BuildMesh` per-rebuild alloc 4 List（text-heavy 场景才痛，T7 未触）。
- shader 非 CLIPPED 路径无条件算 clipPos（fgui `#ifdef` 守卫）；ArrayPool 帧拷贝、冷帧/换页帧 FFI ≤2ms。
- Font `Box::leak`（真进程级泄漏，~700KB/Stage）缓存化——×20 域重载测**未现显著增长**，按 <5MB 阈值**推 v1e**（非阻塞）。
- 坐标 nesting+父相对（transform 继承/事件）→ v1c。

**v1b.1 ✅ 完成（merged main @ 5706a7b）**：打包器 `loomgui_pkg` CLI + `.pkg.bin` v1 格式（Rust-internal，§2.12）+ `Stage::load_package` + `Scene::build` 共享建树 + parse feature gate（runtime 可无 parser）+ FFI `loomgui_stage_load_package` + Unity `_usePackage` 接线。**包路径渲染 == inline 渲染（黄金等价），PlayMode 验过**——验收 #6 达成。spec `docs/superpowers/specs/2026-06-20-v1b-packager-design.md`、plan `docs/superpowers/plans/2026-06-20-v1b-packager.md`。踩坑：magic 撞 frame blob（坑 15）。

**v1b 拆分**：A 打包器/二进制包/加载器（v1b.1 ✅）、**B 真纹理加载（散图→Texture2D→TexId，v1b.2 = 下一个）**、C 图集打包、D 文本 CJK/多字体（text_arena 升三表）。各自后续 spec。

**v1 其余 defer（v0 起，未动）**：
- 真纹理加载（v1b.2/B，G7）—— **下一个**。
- event/命中/输入（v1c，G4）、anim GTween/ScrollPane（v1d，§11/§12.7）。
- NativeHost/virtualization/shape mask：v1.x。

完整 defer 表见各 spec §7；v1a Phase 1 实现 ledger 见 `.git/sdd/progress.md`。

## 维护

每次 LoomGUI 开发/修复后，用 `session-summary` skill 把新踩坑/机制/调试技巧总结进本文件（§5 加坑、§3 加 API、§7 更 ledger）。本 skill 与代码一起提交。
