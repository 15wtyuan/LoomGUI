# v1b.3 — 图集打包（shelf，最小可行 batching） 设计

- **日期**：2026-06-21
- **状态**：设计（待实现）
- **范围**：v1b 第三个子项目（C = 图集）。D 文本 CJK/多字体为后续 spec。
- **依据**：主设计 `docs/design/00-main-design.md` §8.2（rotated UV 修正）、§8.3（TextureView 去引擎化）、§8.5（FairyBatching 不合并 mesh）、§12.3（图集打包器内置 shelf/guillotine）、§12.4（refcount）；roadmap `docs/roadmap/v1-scope.md` §1-资源（图集 TextureView 优先级高）、§3-G7；v1b.2 spec `docs/superpowers/specs/2026-06-20-v1b-texture-design.md`（散图真纹理，本 spec 的 BASE）。
- **参考实现**：`temp/FairyGUI-unity`（`NTexture.cs` root/view 双角色 + `UIPackage.cs:78-85` AtlasSprite + `:887-924` 解析 + `:1271-1363` LoadAtlas/LoadImage）。

---

## 1. 背景与目标

v1b.2（merged main @ 691835a）落地散图真纹理：`<img src="x.png">` → Unity 每张散图各一个 `Texture2D` → core `TextureRegistry` 分配每 src 一个 tex_id → Image 按 tex_id 绑真纹理渲真像素。**但每张散图是独立 Texture2D**，N 张图 = N 个 MaterialManager material key = N 路独立（同图集才能批合的前提不成立）。

v1b.3 落地 roadmap「图集 TextureView（优先级高）」：**打包期**把散图 shelf 打包成一张 atlas.png + AtlasSprite 表（src→region）；**运行期**同图集所有 sprite 共享 1 张 atlas Texture2D → MaterialManager 对同 texture 返回同 Material 实例 → 触发 Unity 批合。

**目标**：`<img>` 散图在打包期合进 atlas.png；运行时同 atlas 的多 sprite 共享 1 Texture2D + 1 Material；core 持 UV region（去引擎化）；端到端可验 batching 条件具备。

---

## 2. 范围

**做**：
- 打包器 `loomgui_pkg` 加 `image` crate：读散图 PNG（解码 RGBA8+dims）→ shelf 打包 → 编码 atlas.png + AtlasSprite 表。
- `.pkg.bin` 格式 v1→**v2**：末尾加 AtlasSprite section（atlas filename/dims + 每 sprite region）。
- core `TexMeta` 演进加 `uv_min/uv_max`；`TextureRegistry` load 时从 AtlasSprite 表建（core 分配 atlas_tex_id，不再 runtime register）。
- core `render/mesh.rs::quad` 多接 uv_rect；Image 分支按 region 烤 UV（替代 v1b.2 全图 (0,0)-(1,1)）。
- FFI：删 v1b.2 的 `image_src_count/at` + `register_texture`；加 `atlas_count` + `atlas_info`。
- Unity `LoomStage.LoadTextures` → `LoadAtlas`（atlas_count/info→load atlas.png→`_texMap[tex_id]`）。
- **blob 格式不变（v3）**——per-vertex UV 已在 mesh_arena，atlas 子区 UV 只是不同值。

**不做（各自后续 spec / defer）**：
- **mesh 合并**（§8.5 现不做）——见 §3，甲-B 不含；真·保证 N→1 draw call 需它，单列 v1b.4。
- rotation（旋转打包）—— v1b.4+（UV 修正公式 §8.2 已记）。
- trim（去透明边 + originalSize/offset）—— v1b.4+。
- 多图集（总像素超 max texture size 拆 N 张）—— v1b.4+（甲-B 单图集）。
- alpha 纹理（`alpha_tex`）—— 需要时（桌面 RGBA 不需要）。
- refcount / `on_release` 卸载（§12.4）—— v1b.4+（甲-B atlas 随 Stage，OnDestroy 一并 Dispose，同 v1b.2 loose 风格）。
- POT / 纹理压缩 / mipmap —— NPOT 足够 v1（Unity Texture2D 支持 NPOT，UI 无 mipmap）。
- 全局 registry / 多 Stage 共享 / Addressables 异步 —— v1.x。
- inline 路径（`_usePackage=false`）的真纹理/atlas —— inline 无打包器 → 无 atlas → 白占位（同 v1b.2）。atlas 是 package 期产物，仅 `_usePackage=true` 可用。

---

## 3. 关键边界：batching 收益的诚实认知（mesh 不合并）

LoomGUI v1 **每节点独立 MeshRenderer**（`MirrorPool.cs:145-160` NewRenderObj 每 node 一个 GO+MeshFilter+MeshRenderer+Mesh；§8.5「不合并 mesh」）。**图集不直接保证 N 图→1 draw call。** 甲-B 实际收益：

1. **多 sprite 共享 1 Material**：MaterialManager 对同 texture 返回同实例（`MirrorPool.cs:116` `mm.Get(program:0, tex, maskCtx)`）→ 触发 **SRP Batcher**（同 material 的 MeshRenderer 常量缓冲批合并，**CPU 效率↑**，draw call 数不变）。
2. **URP Dynamic Batching**（用户在 RenderPipelineAsset 勾选）：同 material + <300 顶点的小 quad（UI quad=4 顶点）自动合并 mesh → **draw call↓**。需 PlayMode FrameDebugger 实测确认（取决于 URP 配置）。
3. **纹理内存/绑定**：1 张 atlas vs N 散图（内存↓、GPU 纹理绑定切换↓、cache 局部性↑）。
4. **基础设施**：为「mesh 合并」（v1b.4）铺路——真·保证 N→1 draw call 需 MirrorPool 把同 material 批内 quad 合进一个 mesh，是更大改动 + §8.5 设计调整。

> **对比 fgui**：fgui `NTexture(root,region,rotated,originalSize,offset)` root/view 双角色 + 双层 refcount（`NTexture.cs:182,512-538`）+ 同 root → 同 MaterialManager → FairyBatching 排序相邻（`Container.cs:877-941`）。fgui 同样**不合并 mesh**，靠 SRP Batcher + 同 material。甲-B 是 fgui 图集模型的忠实子集：同数据流（atlas.png + 每 sprite region → 共享纹理 → 同 material），砍 rotation/trim/multi-atlas/refcount（皆空间/生命周期优化，v1 下 YAGNI）。

**结论**：甲-B 保证「同 atlas → 同 Material → Unity 批合条件具备」，**不保证** draw call 数；实际 draw call 取决于 SRP Batcher（CPU）+ URP Dynamic Batching（可能合并）。验收以 FrameDebugger 实测为准（同 material 必现；draw call↓ 需开 Dynamic Batching）。

---

## 4. 状态分布 + 数据流

```
打包期 (loomgui_pkg, build-time):
  html+css → scene（parse/style/build）
  → 收集 Image{src}（DFS 先序去重，保首次出现序）
  → 每 src 读 PNG：image::open(res_dir/src).to_rgba8() → (pixels, w, h)
  → shelf 打包：按高排序、atlas_w=max(512,最宽sprite)、逐行摆、超宽换行 → 每 sprite region(x,y,w,h) + atlas 总尺寸
  → blit 每 sprite 像素进 atlas buffer → image::save_buffer(<stem>.atlas.png, RGBA8)
  → AtlasSprite 表（src_idx + region）+ atlas filename/dims 写进 .pkg.bin v2 末段
  → 写 <stem>.pkg.bin + <stem>.atlas.png

运行期:
  Unity load_package(.pkg.bin bytes)
    → core read_package v2：解析 scene + AtlasSprite section
    → core build_registry：atlas 按序分 tex_id(从1)；每 sprite uv = region/atlas_dims；src→TexMeta{tex_id,uv_min,uv_max,w,h}
  Unity collect atlases:
    count = atlas_count(stage)
    foreach i: (srcPtr,srcLen,tid,aw,ah) = atlas_info(stage,i,...)
              src = UTF8(srcPtr,srcLen)                      // len-based 读（坑16，禁 NUL-scan）
              bytes = File.ReadAllBytes(Path.Combine(StreamingAssets, src))
              tex = new Texture2D(aw,ah); tex.LoadImage(bytes)
              _texMap[tid] = tex                             // 同 atlas 所有 sprite 共享此 tid
  tick (LateUpdate):
    render Image → registry.get(src)→{tex_id,uv_region}
      → Mesh{texture:tex_id, uvs: quad 按 uv_region 烤（4 角）}
      → blob v3（per-vertex UV 已在 mesh_arena，无格式改）
    Unity MirrorPool：blob.TexId(i)→_texMap[tid]→atlas Texture2D
      → mm.Get(0, atlas_tex, maskCtx) 同 texture→同 Material→SRP/Dynamic Batching
```

握手只在 load 时一次性（一屏 1 张 atlas，同步 File 读）。运行时 tick 零握手（纯查表）。

---

## 5. 打包器改动（loomgui_pkg，+image crate）

### 5.1 新依赖

`loomgui_pkg/Cargo.toml` 加 `image = "0.25"`（写本 spec 时最新稳定；implementer 取当前 stable）。**只在 packer**——core（loomgui_core）不碰像素，不加 `image`。`image` 提供 `open`/`DynamicImage::to_rgba8`/`save_buffer`，解码+合成+编码一步到位。

### 5.2 pack API 多接 res_dir

```rust
/// 打 .pkg.bin + 旁挂 atlas.png。res_dir = 解析 <img src> 的基准目录（CLI 传 html_path.parent()）。
/// 返回 (pkg_bytes, atlas_png_bytes, atlas_filename) —— 调用方写两个文件。
pub fn pack(html: &str, css: &str, root_size: (f32, f32), res_dir: &Path)
    -> Result<PackedPackage, String>
```
`PackedPackage { pkg_bytes: Vec<u8>, atlas_png: Vec<u8>, atlas_filename: String }`。CLI 写 `<stem>.pkg.bin` + `<stem>.atlas.png`（atlas_filename = `<stem>.atlas.png`，相对名进 .pkg.bin header，Unity 据此从 StreamingAssets 拼）。

> 为何返回 atlas_png bytes 而非 packer 内直接写文件：库测试要拿 bytes 验内容（decode 回查 region 像素），不碰磁盘。CLI 负责落盘。

### 5.3 shelf 打包算法（甲-B：无旋转/无 trim/NPOT/单图集）

```rust
struct PlacedSprite { src: String, x: u32, y: u32, w: u32, h: u32 }  // region in atlas px

/// shelf 打包。输入 (src, w, h) 列表（已 decode 得 dims）；输出 (atlas_w, atlas_h, Vec<PlacedSprite>)。
fn shelf_pack(sprites: &mut Vec<(String,u32,u32)>) -> (u32, u32, Vec<PlacedSprite>) {
    const DEFAULT_ATLAS_W: u32 = 512;
    // 按高降序（tall 先放，标准 shelf）。
    sprites.sort_by(|a, b| b.1.cmp(&a.1));   // 注：仅排序辅助 vec，不动 src→region 对应
    let atlas_w = sprites.iter().map(|(_,w,_)| *w).max().unwrap_or(0).max(DEFAULT_ATLAS_W);
    let mut placed = Vec::with_capacity(sprites.len());
    let mut x = 0u32; let mut y = 0u32; let mut shelf_h = 0u32;
    for (src, w, h) in sprites.iter() {
        if x + w > atlas_w {            // 超宽 → 换行
            y += shelf_h; x = 0; shelf_h = 0;
        }
        placed.push(PlacedSprite { src: src.clone(), x, y, w: *w, h: *h });
        x += w;
        shelf_h = shelf_h.max(*h);
    }
    let atlas_h = y + shelf_h;
    (atlas_w, atlas_h, placed)
}
```

- `atlas_w = max(最宽 sprite, 512)` → 保证最宽 sprite 单行放得下 + 小图能同行挤。
- NPOT atlas（不补 POT）。v1 demo 图少 → atlas 小（如 3 张 128² → atlas ≤ 512×~400）。
- **已知限制**：总像素超 max texture size（桌面 ~16368 或 Unity `Texture2D.maxTextureSize`）会失败。甲-B 单图集不拆分 → 超限报错（build-time）。多图集 v1b.4+。

### 5.4 atlas 像素合成

```rust
// 建 atlas buffer（RGBA8，atlas_w*atlas_h*4），初始透明黑。
let mut buf = vec![0u8; (atlas_w * atlas_h * 4) as usize];
for (sprite, placed) in sprites.iter().zip(placed.iter()) {
    let img = image::open(res_dir.join(&sprite.src))?.to_rgba8();  // 已 decode 缓存则复用
    for row in 0..sprite.h {
        for col in 0..sprite.w {
            let src_idx = ((row * sprite.w + col) * 4) as usize;
            let dst_idx = (((placed.y + row) * atlas_w + (placed.x + col)) * 4) as usize;
            buf[dst_idx..dst_idx+4].copy_from_slice(&img.get_pixel(col, row).0);
        }
    }
}
let atlas_png = image::save_buffer_to_memory(
    &image::ImageBuffer::from_raw(atlas_w, atlas_h, buf).unwrap(),
    image::ImageFormat::Png).unwrap();
```
（实现可调 API：`image::RgbaImage::from_raw` + `PngEncoder`。具体调用以 implementer 验证 `image` 0.25 实际 API 为准——记 §3 API 适配。）

### 5.5 缺图 build 时 fail

src 在 scene 但 `res_dir/src` 不可读 → `pack` 返 `Err("image not found: {src}")`。**打包期校验**（比 v1b.2 运行时白占位更早暴露作者错误）。不降级、不跳过。

---

## 6. .pkg.bin v1→v2（AtlasSprite section）

### 6.1 header
- `PKG_FORMAT_VERSION` 1→**2**。`MIN_VERSION=2, MAX_VERSION=2`。旧 v1 包 `TooNew(1)` 拒（sample 须用新 packer 重打）。
- 无迁移器（fgui 式内联兼容待多版本累积；v1b.x-deferred §6）。

### 6.2 末尾追加 AtlasSprite section（在 NodeBlock 之后）

```
AtlasSection:
  atlas_count: u32                     // 甲-B 恒 1（多图集 v1b.4+ 才 >1）
  for each atlas (甲-B 1 个):
    filename_idx: u16                  // into stringTable（如 "loom.atlas.png"）；相对名
    width:  u32                        // atlas 像素宽
    height: u32                        // atlas 像素高
  sprite_count: u32
  for each sprite:
    src_idx: u16                       // into stringTable（该 Image 的 src，已由 NodeBlock interning）
    x: u32, y: u32, w: u32, h: u32     // region in atlas pixels（y-down，image crate 行0=顶）
```

- **无 atlas_idx per sprite**：甲-B 单图集，所有 sprite 属 atlas 0。多图集（v1b.4+）加 `atlas_idx:u16` + bump version。
- **无 rotated/offset/originalSize**：甲-B 无旋转/trim。
- stringTable 复用现有 interning（src 已在 NodeBlock intern；filename 新 intern）。

### 6.3 read_package v2

`read_package(bytes) -> Result<(Scene, (f32,f32), AtlasSection), PkgError>`。多返 `AtlasSection`（或 core 内建 `build_registry` 直接消费）。`Scene::build` 不变。

```rust
pub struct AtlasSection {
    pub atlases: Vec<AtlasInfo>,        // 甲-B len=1
    pub sprites: Vec<AtlasSprite>,      // 全部 sprite（甲-B 都属 atlas 0）
}
pub struct AtlasInfo { pub filename: String, pub width: u32, pub height: u32 }
pub struct AtlasSprite { pub src: String, pub x: u32, pub y: u32, pub w: u32, pub h: u32 }
```

---

## 7. core 改动（loomgui_core）

### 7.1 `asset/texture.rs` — TexMeta 演进 + registry load 时建

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexMeta {
    pub tex_id: u32,        // atlas root tex_id（同图集 sprite 共享）；0=未注册哨兵
    pub uv_min: [f32; 2],   // sprite 在 atlas 内 UV 左上（核心 y-down 约定，[0,1]）
    pub uv_max: [f32; 2],   // UV 右下
    pub width: u32,         // sprite 原始像素宽（measure；甲-B = region.w，无 trim）
    pub height: u32,        // sprite 原始像素高
}

#[derive(Debug, Default)]
pub struct TextureRegistry { map: HashMap<String, TexMeta> }

impl TextureRegistry {
    pub fn get(&self, src: &str) -> Option<TexMeta> { self.map.get(src).copied() }
    /// load_package 从 AtlasSprite 表建时插入。
    pub fn insert(&mut self, src: &str, meta: TexMeta) { self.map.insert(src.into(), meta); }
    pub fn clear(&mut self) { self.map.clear(); }
    pub fn len(&self) -> usize { self.map.len() }
}
```

**删除** v1b.2 的 `register(&mut self, src, w, h) -> u32` + `next_id` 字段——atlas 模式 core 在 `load_package` 时从 AtlasSprite 表分配 tex_id（atlas 按序 1,2,…），无 runtime register 握手。v1b.2 的 register 单测改写为 insert + load 流程测（见 §13）。

### 7.2 `build_registry`（asset 层，load_package 调）

```rust
/// 从 AtlasSection 建 TextureView registry。甲-B 单图集：atlas[0] 得 tex_id=1，所有 sprite 属它。
/// （多图集 v1b.4+ 需 sprite 带 atlas_idx 才能分流；届时改回 per-atlas 循环。）
pub fn build_registry(section: &AtlasSection) -> TextureRegistry {
    debug_assert!(!section.atlases.is_empty(), "v2 包至少 1 atlas");
    let mut reg = TextureRegistry::default();
    let atlas = &section.atlases[0];
    let atlas_tex_id = 1u32;                          // 甲-B：唯一 atlas → tex_id 1
    let aw = atlas.width as f32; let ah = atlas.height as f32;
    for spr in &section.sprites {
        let uv_min = [spr.x as f32 / aw, spr.y as f32 / ah];
        let uv_max = [(spr.x + spr.w) as f32 / aw, (spr.y + spr.h) as f32 / ah];
        reg.insert(&spr.src, TexMeta {
            tex_id: atlas_tex_id, uv_min, uv_max, width: spr.w, height: spr.h,
        });
    }
    reg
}
```

> 甲-B 硬绑 atlas[0]（单图集）。tex_id=1 与 FFI `atlas_info(0)` 报的 `(i+1)=1` 两侧约定一致（§8.2），Unity `_texMap[1]` 对得上。

### 7.3 Stage 集成

`Stage` 加 `textures: TextureRegistry` 字段（替换 v1b.2 同名字段，类型演进）。`load_package`：`read_package` → `(scene, root_size, atlas_section)` → `self.textures = build_registry(&atlas_section)`。`load_inline`：`self.textures.clear()`（inline 无 atlas → 空 registry → Image tex_id=0 白占位）。`tick_and_render` 把 `&self.textures` 传 `solve` + `build_render_nodes`（同 v1b.2 接线）。

### 7.4 `render/mesh.rs::quad` 多接 uv_rect

```rust
/// 生成 quad。uv_min/uv_max 指定 UV 区间（v1b.2 全图 = [0,0],[1,1]；atlas sprite = 子区）。
pub fn quad(rect: &Rect, color: [f32; 4], uv_min: [f32; 2], uv_max: [f32; 2])
    -> (Vec<[f32;2]>, Vec<[f32;2]>, Vec<[f32;4]>, Vec<u32>)
{
    let verts = vec![[rect.x,rect.y],[rect.x+rect.w,rect.y],[rect.x+rect.w,rect.y+rect.h],[rect.x,rect.y+rect.h]];
    let (umin,vmin) = (uv_min[0],uv_min[1]); let (umax,vmax) = (uv_max[0],uv_max[1]);
    // vert 顺序 TL,TR,BR,BL ↔ sprite 角 TL,TR,BR,BL（核心 y-down，与 v1b.2 convention 一致）
    let uvs = vec![[umin,vmin],[umax,vmin],[umax,vmax],[umin,vmax]];
    let colors = vec![color; 4];
    let indices = vec![0,1,2,0,2,3];
    (verts, uvs, colors, indices)
}
```
v1b.2 调用点（Container/Button 背景色块）改传 `[0.0,0.0],[1.0,1.0]`（纯色块全图 UV，tex_id=0 白贴图 × bg 色 = 纯色，行为不变）。

### 7.5 `render/mod.rs` — Image 分支按 region 烤 UV

```rust
NodeKind::Image { src } => {
    let (tex_id, uv_min, uv_max) = match textures.get(src) {
        Some(m) => (m.tex_id, m.uv_min, m.uv_max),
        None => (0u32, [0.0,0.0], [1.0,1.0]),   // 未注册哨兵：白占位（tex_id=0，UV 无关）
    };
    let (v, uvs, col, idx) = crate::render::mesh::quad(rect, [1.0,1.0,1.0,1.0], uv_min, uv_max);
    rn.payload = NodePayload::Mesh { verts: v, uvs, colors: col, indices: idx, texture: tex_id, program: 0 };
}
```
Container/Button 分支：`quad(rect, bg, [0,0],[1,1])`（全图 UV）。

### 7.6 measure（不变逻辑，dims 源换）

三档优先级（v1b.2 已实现）：`CSS Dimension::Length > registry.get(src).width/height > 64×64`。甲-B `TexMeta.width/height` 现来自 atlas 表（= sprite 原始像素，无 trim）。**measure 代码零改**（字段名 width/height 不变）。

### 7.7 UV 方向约定（packer / core / Unity 三方）

- packer region 用 **PNG 像素空间**（y-down，`image` crate 行 0 = 图顶）。
- core `uv_min = [region.x/aw, region.y/ah]`（y-down，region.y 从图顶起算）—— **沿用 v1b.2 已 PlayMode 验证的 convention**（v1b.2 全图 UV `(0,0)-(1,1)` + root 一次性 y-flip 已验证方向正确；甲-B 仅缩到子区，convention 不变）。
- vert→UV 角映射：TL→uv_min, TR→(umax,vmin), BR→uv_max, BL→(umin,vmax)（§7.4）。
- **PlayMode 必验 sprite 方向不倒/不镜像**（风险 R1）。

---

## 8. FFI（loomgui_ffi_c）

### 8.1 删 v1b.2 loose 模式 FFI

删 `loomgui_stage_image_src_count` / `loomgui_stage_image_src_at` / `loomgui_stage_register_texture`（loose 散图模型被 atlas 取代）。csbindgen 重生成 `Native`（惯例 gitignored）。

### 8.2 加 atlas collect FFI

```c
// atlas 数量（甲-B 恒 1）。
uintptr_t loomgui_stage_atlas_count(StageHandle* h);

// 第 i 个 atlas 信息。返 src 指针（atlas filename UTF-8，**无尾 NUL** + *out_src_len = 字节长）；
// *out_tex_id = core 分配的 atlas tex_id；*out_w/*out_h = atlas 像素尺寸。OOB → null。
// 串归 Stage 拥有，下次调用/tick 前有效（同 v1b.2 image_src_at 串契约，坑16）。
const uint8_t* loomgui_stage_atlas_info(StageHandle* h, uintptr_t index,
    uint32_t* out_tex_id, uint32_t* out_w, uint32_t* out_h, uintptr_t* out_src_len);
```

实现：`atlas_count` = `h.stage.atlases.len()`（Stage 缓存 load_package 解析的 AtlasSection.atlases）。`atlas_info(i)`：OOB→null；否则返 `atlases[i].filename.as_ptr()` + `*out_src_len=len` + `*out_tex_id=(i+1)` + `*out_w/h`。**len-based 串契约**（坑16：`String::as_ptr()` 无尾 NUL，C# 必 `Encoding.UTF8.GetString(ptr,len)`，禁 NUL-scan）。

> tex_id 由 core 在 load 时按 atlas 序分配（`build_registry` §7.2 的 `(idx+1)`），FFI `atlas_info` 报同序的 `(i+1)`——两侧约定一致，Unity `_texMap[tid]` 对得上。

---

## 9. blob（不变 v3）

**blob 格式零改**（version 仍 3，14 列，header_len=92，arena 布局全同 v1b.2）。atlas 子区 UV 是 mesh_arena 内 per-vertex UV 的**不同值**（替代 v1b.2 的 (0,0)-(1,1)），走既有 `uvs[]` 写入路径（`blob.rs:79-82`）。C# `FrameBlob`/`MirrorPool` 零改。

> **stale .dll 失败模式变化**（坑10）：blob version 不变（仍3）→ 旧 .dll 产的 v3 blob 仍被 C# 接受，但旧 render 烤的是 (0,0)-(1,1) 全图 UV → atlas sprite 采样整张 atlas（错区域，非静默）。且旧 .dll 读不了 v2 .pkg.bin（TooNew 拒）。故 **PlayMode 前必重编换 .dll + 重打 sample**（v2 包）。md5 诊断同坑10。

---

## 10. Unity 接线

### 10.1 LoomStage.cs（`_usePackage=true` 分支，Awake）

v1b.2 `LoadTextures()`（collect loose PNGs）→ 改名 `LoadAtlas()`：

```csharp
void LoadAtlas() {
    _texMap.Clear();
    if (_stage == null) return;
    nuint count; unsafe { count = Native.loomgui_stage_atlas_count(_stage); }
    for (nuint i = 0; i < count; i++) {
        byte* p = null; nuint srcLen = 0; uint tid = 0; uint aw = 0; uint ah = 0;
        unsafe { p = Native.loomgui_stage_atlas_info(_stage, i, &tid, &aw, &ah, &srcLen); }
        if (p == null || srcLen == 0) continue;
        string src = Encoding.UTF8.GetString(p, (int)srcLen);          // len-based（坑16）
        string path = System.IO.Path.Combine(Application.streamingAssetsPath, src);
        byte[] bytes;
        try { bytes = System.IO.File.ReadAllBytes(path); }
        catch (System.Exception e) { Debug.LogError($"[LoomStage] atlas not found: {src} ({e.Message})"); continue; }
        var tex = new Texture2D((int)aw, (int)ah);
        if (!tex.LoadImage(bytes)) {
            Debug.LogError($"[LoomStage] bad atlas png: {src}");
            if (Application.isPlaying) UnityEngine.Object.Destroy(tex);
            else UnityEngine.Object.DestroyImmediate(tex);
            continue;
        }
        _texMap[tid] = tex;     // tid 由 core 分配（= i+1）；同 atlas 所有 sprite 共享
    }
}
```
Awake 在 `_usePackage` 分支调 `LoadAtlas()`（替代 v1b.2 `LoadTextures()`）。`OnDestroy` Dispose `_texMap` 全部（同 v1b.2，ExecuteAlways 双路径 Destroy/DestroyImmediate——坑18 全限定 `UnityEngine.Object`）。

### 10.2 MirrorPool.cs / FrameBlob.cs / MaterialManager.cs / shader — 零改

- `MirrorPool.Sync`：已按 `blob.TexId(i)` 绑 `_texMap[tid]`（v1b.2 实现），`UploadMesh` 已应用 blob per-vertex UV（`MirrorPool.cs:182`）——atlas 子区 UV 自动生效。
- `FrameBlob`：v3 不变。
- `MaterialManager.Get(program, tex, maskCtx)`：同 texture → 同 Material 实例（v1b.2 已就位）→ atlas 多 sprite 共享 → SRP/Dynamic Batching 触发。
- shader `col = tex2D(_MainTex, uv) * v.color`：零改。

### 10.3 内容管线（变化）

StreamingAssets 放 **`.pkg.bin` + `.atlas.png`**（atlas_filename 在 .pkg.bin 内，Unity 据此拼路径）。**散图 PNG 不再需要**（已烤进 atlas）。v1b.2 的 `loom_sample.png`（散图）→ v1b.3 改为 atlas.png（packer 产出）。

### 10.4 inline 路径（`_usePackage=false`）

`LoadAtlas` 不调（inline 无打包器 → core `load_inline` 后 registry 空）→ Image tex_id=0 → 白占位（同 v1b.2）。atlas PlayMode 验走 `_usePackage=true`。

---

## 11. 错误处理

- **缺图（打包期）**：`pack` 返 `Err`（§5.5）——build-time fail，不降级。
- **atlas.png 缺失/坏**（runtime）：`File.ReadAllBytes` 抛 / `LoadImage` 返 false → try/catch → `LogError` → 不入 `_texMap` → 该 atlas tex_id 缺 → 所有引用它的 sprite `texMap.TryGetValue` miss → fallback 白占位。**优雅降级不 crash**。
- **src 在 scene 但不在 AtlasSprite 表**（理论上不发生——packer 收集所有 Image src 入表）：registry.get→None → tex_id=0 → 白占位（静默）。
- **atlas 总像素超 max texture size**：build-time atlas 尺寸超大 → `Texture2D.LoadImage` 可能失败或 Unity 裁剪 → 白占位/视觉错。甲-B 不拆分（多图集 defer）。packer 可加 build-time 检查（atlas_w/h 超 16384 → Err）。
- **stale v2 .pkg.bin vs 旧 .dll**：旧 .dll `read_package` 拒 v2（TooNew）→ load 失败 → 场景空（§9）。md5 诊断同坑10。
- **core 侧**：FFI/shelf/build_registry 全 slice 边界安全，**无 unwrap/panic 跨 FFI**。

---

## 12. 测试与验收

### Rust 单测 — packer（loomgui_pkg）
- **shelf 非重叠 + 不出界**：给定 dims 集合，所有 placed region 两两不相交 + 全在 `[0,atlas_w)×[0,atlas_h)` 内。
- **shelf 放得下最宽 sprite**：`atlas_w >= max(sprite.w)`。
- **PNG round-trip**：encode atlas → decode → 每 sprite region 内像素 == 原 PNG 对应像素（用 `image` decode 回查）。
- **pack 端到端**：html（含 2-3 `<img>`）+ css + fixtures PNGs → `pack` → 得 pkg_bytes + atlas_png；assert atlas_png 非空 + pkg_bytes v2 magic/version。
- **缺图 fail**：html 引用不存在的 src → `pack` 返 Err。

### Rust 单测 — core
- **`build_registry`**：单图集 AtlasSection → 所有 sprite tex_id=1；uv_min/uv_max = region/atlas_dims 精确值。
- **TexMeta insert/get/clear**（替换 v1b.2 register 测）。
- **render Image**：注册 src→{tex_id=1, uv_region} 后，`RenderNode.Mesh.texture==1` + 4 个 uv == `[umin,vmin],[umax,vmin],[umax,vmax],[umin,vmax]`；未注册 src → texture==0 + uv 全图。
- **render Container/Button**：uv 仍 (0,0)-(1,1)（纯色块行为不变）。
- **measure 三档**（逻辑同 v1b.2，dims 源换）：CSS 100px + 真实 200 → 用 100；无 CSS + 注册 200×100 → 200×100；无 CSS + 未注册 → 64×64。

### Rust 集成 — .pkg.bin v2
- **atlas section round-trip**：write_package（含 AtlasSection）→ read_package → atlas_count/filename/dims + 每 sprite region 全等。
- **version 拒绝**：v1 包（version=1）→ `TooNew(1)`（MIN=2）。
- **v1b.2 golden test 处理**：v1b.2 的 pkg-vs-inline golden（带 register）**移除**（inline 无 atlas，不再等价）。替换为 **pkg render 快照（insta）**：load package（含 atlas）→ build_render_nodes → snapshot RenderNode JSON（含 tex_id + uv per Image），锁 atlas UV 输出。

### blob（不变）
- 现有 blob v3 测全绿（format 未改；mesh UV round-trip 测仍过，值变 (0,0)-(1,1)→子区不影响 round-trip 测结构）。

### FFI（loomgui_ffi_c）
- `atlas_count`/`atlas_info` round-trip：load v2 包 → count==1 → info(0) 返 filename + tex_id==1 + dims 正确；OOB→null。
- 删除的 v1b.2 image_src/register 测一并删。

### 构建矩阵（R1 门）
`cargo build -p loomgui_core --no-default-features` + `-p loomgui_ffi_c --no-default-features` 皆编（atlas 代码常驻不依赖 parse）。`-p loomgui_pkg`（需 parse feature + image）单独编。

### Unity EditMode
- 新 EditMode 测：手搓 v3 blob 含 atlas 子区 UV + mock `_texMap` → MirrorPool 绑 atlas Texture2D + mesh.uv == 子区 4 角。

### Unity PlayMode（批次，押用户）
sample 场景：HTML 含 **2-3 张不同 src 的 `<img>`** + 对应 fixtures PNGs → packer 产出 `.pkg.bin` + `.atlas.png`（committed 进 StreamingAssets）→ `_usePackage=true`：
- 3 张图全显、**方向正**（不倒/不镜像）。
- **FrameDebugger**：3 个 Image MeshRenderer 共享同 1 Material（同 atlas texture）→ SRP Batcher 批合（CPU）。
- 开 URP Dynamic Batching → draw call < 3（可能合并）。
- 重编换 .dll + 重打 v2 sample 流程验过（坑10）。

### 命中 roadmap
G7（v1b.2 已达）扩展：散图→**图集**→Unity→GPU→共享 TexId。图集 TextureView（roadmap §1-资源「优先级高」）首批落地。

---

## 13. 风险

- **R1（最高）UV 方向三方约定**：packer region（y-down 像素）/ core UV（v1b.2 convention）/ Unity Texture2D（PNG LoadImage）三方任一处 flip 错 → sprite 倒/镜像/错区域。缓解：沿用 v1b.2 已 PlayMode 验证 convention（仅缩子区，不改方向语义）；render UV 四角单测；PlayMode 必验方向。
- **R2 .pkg.bin v2 + stale .dll/.pkg（坑10）**：v2 包旧 .dll 拒；新 .dll 旧 v1 包拒（TooNew）；stale .dll + v2 包 → load 失败或错 UV。缓解：bump version、重打 sample、重编 .dll、PlayMode 前 md5sum。
- **R3 shelf 正确性**：重叠/OOB → 像素互踩/越界。缓解：shelf 非重叠+不出界单测。
- **R4 `image` crate 新 dep**：版本 API 差异（save_buffer_to_memory 等）。缓解：仅 packer 用；implementer 验 `image` 0.25 实际 API（记 §3 API 适配）。
- **R5 batching 收益不如预期**：mesh 不合并 → draw call 不一定↓（仅 SRP Batcher CPU）。缓解：spec §3 已诚实告知；验收以 FrameDebugger「同 Material」为底线（draw call↓ 需开 Dynamic Batching，best-effort）。
- **R6 ExecuteAlways 泄漏**：`_texMap` atlas Texture2D 不 Dispose → 泄漏（坑11/18 同源）。缓解：OnDestroy Dispose 全部 + Awake 清旧（v1b.2 模式，全限定 UnityEngine.Object）。

---

## 14. 与主设计 / roadmap 对齐

- **不改 `docs/design/00-main-design.md`**：§8.2 rotated UV 修正公式、§8.3 TextureView、§8.5 不合并 mesh、§12.3 打包器图集、§12.4 refcount 均已记；v1b.3 是其「shelf 无旋转/trim + 单图集 + 无 refcount」首批实现。rotation/trim/multi-atlas/refcount 留 v1b.4+，主设计不矛盾。
- `knowledge-reference`：实现后用 session-summary 记新机制（§2 asset 层 atlas/TextureView + §3 image crate / Unity atlas 加载 API）+ 新坑（如有：UV 方向三方约定、image crate API）+ ledger（v1b.3 ✅；v0 → v1b.3 演进）。

---

## 15. 非目标 / defer 清单

| 项 | 去向 |
|---|---|
| mesh 合并（MirrorPool 合同 material 批内 quad → 真 N→1 draw call） | v1b.4（§8.5 设计调整） |
| rotation（旋转打包 + UV 修正 §8.2） | v1b.4+ |
| trim（去透明边 + originalSize/offset 还原） | v1b.4+ |
| 多图集（超 max texture size 拆 N 张 + atlas_idx per sprite） | v1b.4+ |
| alpha 纹理（alpha_tex） | 需要时 |
| refcount / on_release 卸载（§12.4） | v1b.4+（atlas 随 Stage） |
| POT / 纹理压缩 / mipmap | 按需 |
| 全局 registry / 多 Stage 共享 | v1.x |
| Addressables/YooAsset 异步管线 | v1.x |
| inline 路径 atlas | v1b.x（dev 路径，低优） |
| .pkg.bin 迁移器链（v1→v2 兼容） | v1b.x-deferred §6（多版本累积后） |
