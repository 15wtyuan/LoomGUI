# v1b.2 — 真纹理加载（散图 → Texture2D → TexId 注册） 设计

- **日期**：2026-06-20
- **状态**：设计（待实现）
- **范围**：v1b 第二个子项目（B）。C 图集打包 / D 文本 CJK 为各自后续 spec。
- **依据**：主设计 `docs/design/00-main-design.md` §8.3（TextureView）、§8.7（RenderNode Mesh payload）、§12.3（图集）、§12.4（refcount）、§14.3/§14.4（跨边界数据/资源加载）；roadmap `docs/roadmap/v1-scope.md` §3-G7、`docs/roadmap/v1x-deferred.md`；v1b.1 spec `docs/superpowers/specs/2026-06-20-v1b-packager-design.md`。
- **参考实现**：`temp/FairyGUI-unity`（`NTexture.cs` / `UIPackage.cs`，纹理对象模型）。

---

## 1. 背景与目标

v1b.1 让「HTML → 打包器 → `.pkg.bin` → 运行时加载渲染」端到端跑通，包路径与 inline 逐节点等价（验收 #6，merged main @ 5706a7b）。但 **Image 节点仍渲占位**：render 把 `texture` 字段填 `hash_str(src)`，blob 序列化时该字段被丢弃；Unity 所有 Image 用同一张 `Texture2D.whiteTexture` → 显示为白块。

v1b.2 落地 roadmap G7「纹理加载（磁盘→Unity→GPU→注册 TexId）」：`<img src="logo.png">` 在 Unity 显真像素而非白块，且无 CSS 尺寸时按真实像素布局。

**目标**：散图 PNG（外部文件）→ Unity 解码上传 GPU → 注册 TexId → Image 节点按 TexId 绑定真实 Texture2D 渲染；Image 内在尺寸支持真实像素。命中 G7。

---

## 2. 范围

**做**：
- core 新 `TextureRegistry`（`src→TexMeta{tex_id,w,h}`，per Stage）。
- core render/measure 接线 registry（Image → tex_id；内在尺寸真实像素）。
- FFI `register_texture` + `image_src_count` / `image_src_at`（collect）。
- blob 格式 v2→**v3**：SOA 加 `tex_id: u32` 列（14 列）。
- Unity：collect→load PNG→register→`tex_id→Texture2D` map；MirrorPool 按 tex_id 绑材质；FrameBlob 读 v3。

**不做（各自后续 spec / defer）**：
- 图集打包（shelf/guillotine + TextureView UV region）—— v1b.3（C）。v1b.2 散图每张独立 Texture2D，UV 全图 (0,0)-(1,1)。
- 纹理生命周期 refcount / `on_release` 卸载 —— 图集 C（loose 图随 Stage 存活，Stage 释放时一并 Dispose，同 v0 字体 leak 风格）。
- alpha 纹理（`alpha_tex`）—— Mesh payload 设计上保留 `Option`，v1b.2 恒 None。
- 多 Stage 纹理共享 / 全局 registry —— v1.x（`loomgui_shutdown` 全局 registry 注释留为愿景，v1b.2 per Stage）。
- Addressables/YooAsset 异步资源管线 —— v1.x（v1b.2 直接 File 同步读）。
- inline 路径的真纹理（`_usePackage=false`）—— v1b.2 inline 仍白占位（inline 是 dev 迭代；真纹理 PlayMode 验走 package 路径）。
- NPOT / 纹理压缩 / mipmap —— Unity `Texture2D.LoadImage` 默认处理，不额外配置。

---

## 3. 关键边界：TexId 是 FFI 代价，非 fgui 那样的对象引用

fgui `NTexture`（`temp/FairyGUI-unity/.../NTexture.cs`）是**进程内 C# 对象**，直接包 Unity `Texture2D`：`nativeTexture` 取 `_root._nativeTexture`，显示组件**直接持 NTexture 对象引用**，`_root` 是 NTexture（图集子 view 指根），`width/height` 从持有的 Texture2D 白拿。**fgui 没有整数 TexId、没有两张表、没有注册握手** —— 因为单进程，纹理按对象身份引用。

LoomGUI 的 Rust 核心 + Unity 后端跨 FFI，blob 必须用字节传每图节点标识；对象引用/进程内指针过不了 FFI → 必须用整数 id + 映射表。**这是「Rust 核心 + 可换后端」架构的固有成本，fgui 单进程不付。**

此外，本 spec 选了「真实尺寸测量」（§5），core 的 measure 要每 src 的 w/h → core **必须**有 `src→dims` 表。该表不是为 blob 加的，是为测量加的（fgui 也需维度，只是 in-process 从 Texture2D 白拿）。故 `register(src, w, h)` 的真正理由是**报维度**，tex_id 顺带分配。

> 推论：即便 blob 改带 src 串（非 tex_id），core 仍要 register（为维度）。既然表在，blob 带 tex_id（u32）比带 src 串（变长需 arena+offset）又便宜又紧凑。故「registry + tex_id blob」是选了真实尺寸测量后的自洽最简，非画蛇添足。

---

## 4. 状态分布 + 注册握手

TexId 是 core 拥有的整数；GPU 纹理全在 Unity。两张表分居两侧，靠注册握手粘合：

| 侧 | 持有 | 不持有 |
|---|---|---|
| **core（.dll 内，per Stage）** | `TextureRegistry { src → TexMeta{tex_id,w,h}, next_id }` | Texture2D、PNG bytes |
| **Unity（per LoomStage）** | `Dictionary<tex_id, Texture2D> _texMap` | src→tex_id（由 core 管） |

**注册握手（LoomStage Awake，`load_package` 之后、首 tick 之前）**：

```
1. Unity load_package(bytes)
     → core 解析建 scene（含 Image{src} 节点）；textures.clear() 已在 load_package 内做
2. Unity collect:
     count = image_src_count(stage)
     foreach i in 0..count: src = UTF8( image_src_at(stage, i, &len) )
3. foreach src:
     path = Path.Combine(streamingAssetsPath, src)
     bytes = File.ReadAllBytes(path)              // try/catch — 缺图跳过
     tex = new Texture2D(2,2); tex.LoadImage(bytes)   // 返 bool，失败跳过
     tex_id = register_texture(stage, src, tex.width, tex.height)   // core 分配
     _texMap[tex_id] = tex
4. Unity tick（LateUpdate）：
     render 查 registry(src→tex_id) → blob 带 tex_id → Unity 查 _texMap → 绑材质
```

握手只在 load 时一次性（一屏数张～数十张 PNG，同步 File 读）。运行时 tick 零握手（纯查表）。

---

## 5. core 改动

### 5.1 `loomgui_core/src/asset/texture.rs`（新，常驻不 gate）

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct TexMeta {
    pub tex_id: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct TextureRegistry {
    map: HashMap<String, TexMeta>,
    next_id: u32,            // 从 1 起；0 保留 = 未注册哨兵
}

impl Default for TextureRegistry {
    fn default() -> Self { Self { map: HashMap::new(), next_id: 1 } }
}

impl TextureRegistry {
    /// 注册：core 分配 tex_id（src 幂等，同 src 二次调用返回同 id）。返回 tex_id(>=1)。
    pub fn register(&mut self, src: &str, w: u32, h: u32) -> u32 {
        if let Some(m) = self.map.get(src) { return m.tex_id; }
        let id = self.next_id;
        self.next_id += 1;
        self.map.insert(src.to_string(), TexMeta { tex_id: id, width: w, height: h });
        id
    }
    pub fn get(&self, src: &str) -> Option<TexMeta> { self.map.get(src).copied() }
    pub fn clear(&mut self) { self.map.clear(); self.next_id = 1; }   // 重载用
}
```

`asset/mod.rs` 加 `pub mod texture;`（或 `pub use texture::*;`）。

### 5.2 Stage 集成

`Stage` 加字段 `textures: TextureRegistry`。`load_package` 与 `load_inline` 开头先 `self.textures.clear()`（重载 → tex_id 重分配，Unity 侧重建 `_texMap`）。

### 5.3 render 接线（`render/mod.rs`，Image 分支）

`build_render_nodes` 多接一个 `&TextureRegistry` 参数。Image 分支：

```rust
NodeKind::Image { src } => {
    let tex_id = textures.get(src).map(|m| m.tex_id).unwrap_or(0);  // 0=哨兵
    let (verts, uvs, colors, idx) = crate::render::mesh::quad(rect, [1.0,1.0,1.0,1.0]);  // 白 tint
    rn.payload = NodePayload::Mesh {
        verts, uvs, colors, indices: idx,
        texture: tex_id,   // 原 hash_str(src) 占位 → 真实 tex_id（0=未注册）
        program: 0,
    };
}
```

UV 仍 (0,0)-(1,1)（散图全图；图集 C 才改 UV region）。`tick_and_render` 把 `&self.textures` 传入。

### 5.4 measure 接线（`layout/mod.rs`，Image 内在尺寸优先级）

`solve` 的 measure 闭包捕获 `&TextureRegistry`。Image 内在尺寸优先级（保 AI 可预测）：

```
CSS width/height (Dimension::Length)  →  用声明值（显式作者意图赢）
否则 registry.get(src)                 →  真实像素 w/h
否则                                    →  64×64 兜底
```

现状是 `if Length { v } else { 64 }`；改为三档。`measure_context(node, &textures)` 多接 registry 参数。

> 优先级「CSS 声明 > 真实像素 > 64×64」：AI 写 `<img src>` 不带尺寸时得自然大小，显式 CSS 尺寸仍赢。

---

## 6. FFI（`loomgui_ffi_c/src/lib.rs`，常驻不 gate）

```c
// 注册：core 分配 tex_id（src 幂等），存 w/h 供 measure。返回 tex_id(>=1)。
uint32_t loomgui_stage_register_texture(StageHandle* h,
                                        const uint8_t* src, uintptr_t src_len,
                                        uint32_t w, uint32_t h);

// load_package 后：core 遍历 scene 返回去重 src 列表（Unity 据此知道加载哪些 PNG）。
uintptr_t loomgui_stage_image_src_count(StageHandle* h);
// 返回第 i 个 src 的 UTF-8 串指针；*out_len = 串字节长。OOB → null。串归 Stage 拥有，下次调用/tick 前有效。
const uint8_t* loomgui_stage_image_src_at(StageHandle* h, uintptr_t index, uintptr_t* out_len);
```

`src_at` 复用既有 `*const u8 + len` 串返回模式（与 `load_html` / `load_package` 一致），**不引入新打包格式**。csbindgen 重生成 `Native` 绑定（惯例 gitignored，不入库）。

register 实现：`h.stage.textures.register(str::from_utf8(src_slice), w, h)`，UTF-8 切片用 `slice::from_raw_parts` + 长度边界，`from_utf8` 失败 → 返回 0（哨兵，不 panic）。collect 实现：`src_count`/`src_at` 首次调用时遍历当前 scene，按 **DFS 先序**收集 `NodeKind::Image{src}` 的 src 去重（保首次出现顺序）并缓存于 Stage；`load_package`/`load_inline` 失效缓存（随 `textures.clear()` 一并，下次 collect 重建）。`src_at(i)` 从缓存读第 i 项，OOB 返 null。count 与 at 共用同一缓存，故顺序一致。

---

## 7. blob v3（跨语言契约；坑 10 必重编换 .dll）

### 7.1 header
- `version` 2→**3**。magic 不变（frame blob 自己的 magic）。C# `FrameBlob.IsValid` 改认 `==3`。

### 7.2 SOA 13→14 列，末尾追加 `tex_id: u32`
```
现有 13 列: node_id parent_id visible alpha sort_key local_x local_y mask_context
            payload_kind mesh_off mesh_len text_off text_len
新增列 13:  tex_id (u32)        // 4 字节/节点
```

每节点 tex_id：
- Image（payload_kind=1）→ 解析后的 tex_id（未注册 = 0）。
- Container / Button（payload_kind=1 纯色块）→ 0。
- Text（payload_kind=2）→ 0（文字纹理走 font material 路径，不经此列）。

### 7.3 shader / 材质零改
现有 shader `col = tex2D(_MainTex, uv) * v.color` **已天然支持两路**：
- 纯色块：tex_id=0 → 白贴图 × bg 色（顶点色） = 纯色。
- Image：tex_id=N → 真贴图 × 白 tint = 真图。

**shader、MaterialManager 均零改**（MaterialManager key=(program,texture,maskContext) 已多纹理；不同 texture 自然不同 Material 实例）。唯一变化：Unity 按 tex_id 绑对 Texture2D。

### 7.4 测试维护
- `blob.rs`（Rust）：`build_blob` 发 v3 + 14 列；`COLUMNS` 加 `("tex_id", 4)`。
- `FrameBlob.cs`（C#）：加 `ReadTexId(int i) -> uint`（列 13），`IsValid` 认 v3。
- 手搓 blob fixture 升 v3 + 14 列：`MirrorPoolFlattenTests` / `FrameBlobV2Tests`（坑 12 SOA 列优先；单节点 fixture AoS≡SOA 不受影响，但 version 字段 + 列 offset 须改）。

---

## 8. Unity 接线

### 8.1 LoomStage.cs（`_usePackage=true` 分支，Awake）
```
load_package(bytes)                              // 内含 textures.clear()
count = Native.loomgui_stage_image_src_count(stage)
for i in 0..count:
    (srcPtr, srcLen) = Native.image_src_at(stage, i)
    src = UTF8(srcPtr, srcLen)
    path = Path.Combine(Application.streamingAssetsPath, src)
    try { bytes = File.ReadAllBytes(path) } catch { Debug.LogError($"[LoomGUI] texture not found: {src}"); continue }
    tex = new Texture2D(2, 2)
    if (!tex.LoadImage(bytes)) { Debug.LogError($"[LoomGUI] bad png: {src}"); continue }
    tid = Native.loomgui_stage_register_texture(stage, srcUTF8, tex.width, tex.height)
    if (tid != 0) _texMap[tid] = tex        // 0=哨兵（非 UTF-8 src），不入表
// tick 循环不变
```
- 新字段：`Dictionary<uint, Texture2D> _texMap = new();`（LoomStage 持有）。
- `MirrorPool.Sync` 调用：把 `_texMap` + `Texture2D.whiteTexture`（fallback）作参数传（替代现 `placeholder` 参数）。
- `OnDestroy`：`foreach t in _texMap: Object.DestroyImmediate(t)`（EditMode）/ `Destroy`（PlayMode），`_texMap.Clear()`。

### 8.2 MirrorPool.cs
签名：`Sync(FrameBlob blob, Transform root, MaterialManager mm, Dictionary<uint,Texture2D> texMap, Texture fallback, Font font)`。kind==1 Mesh 节点：
```csharp
uint tid = blob.ReadTexId(i);
Texture tex = (tid != 0 && texMap.TryGetValue(tid, out var t)) ? t : fallback;
ro.Mr.sharedMaterial = mm.Get(program: 0, tex, maskCtx);
```
kind==2 Text：不变（font material 路径）。kind==0：跳过。

### 8.3 FrameBlob.cs
加列 13 读取（`ReadTexId`），`IsValid` 认 version==3。

### 8.4 MaterialManager.cs / shader
零改。

### 8.5 inline 路径（`_usePackage=false`）
v1b.2 不做 collect+register → registry 空 → Image tex_id=0 → 白占位（同今天）。真纹理 PlayMode 验走 `_usePackage=true`。

---

## 9. 错误处理

- **PNG 缺失**（src 在 scene 但 StreamingAssets 无此文件）：`File.ReadAllBytes` 抛 → try/catch → `Debug.LogError` → 不注册 → tex_id=0 → 白占位。**优雅降级不 crash**。
- **LoadImage 失败**（坏 PNG / 非图）：`tex.LoadImage` 返 false / 尺寸 0 → 同上 log + 不注册 + 白占位。
- **src 未注册就 tick**（注册前先 LateUpdate，或注册失败）：registry.get→None → tex_id=0 → 白占位（静默，非错，=「纹理还没到」）。
- **重复 register 同 src**：core 幂等返回同 tex_id；Unity `_texMap[tid]` 覆盖同 key，不泄漏。
- **register 收非 UTF-8 src**：core `from_utf8` 失败 → 返回 0 哨兵，不 panic。
- **core 侧**：register/collect 全 HashMap/slice 边界安全，**无 unwrap/panic 跨 FFI**。
- **stale v2 .dll**（坑 10）：`IsValid` 认 v3 → 旧 .dll 产 v2 blob 被拒 → 静默不渲（Console 干净）。诊断 `md5sum target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`。**PlayMode 验前必重编换 .dll（关 Unity）**。

---

## 10. 测试与验收

### Rust 单测（`core::asset::texture`）
- register 分配单调 tex_id（1,2,3…）；0 永不被分配（哨兵）。
- src 幂等：同 src 二次 register 返回同 tex_id，`next_id` 不前进。
- `get` 命中返回 TexMeta，未命中 None。
- `clear` 后 `next_id` 回 1、map 空、再 register 从 1 起。

### Rust 集成（render + measure）
- render：注册 `src→tex_id` 后，Image 节点 `RenderNode.Mesh.texture == tex_id`；未注册 src → `texture == 0`。
- measure 优先级三档各一测：
  - CSS `width:100px` + 真实 200×200 → 用 100（声明赢）。
  - 无 CSS + 注册真实 200×100 → 用 200×100（真实像素）。
  - 无 CSS + 未注册 → 64×64。

### 黄金等价（更新 v1b.1 的，最强门）
`pkg→load_package→register(图)→render_json` == `inline load_inline→register(图)→render_json`。注册一张 fixture 图：调 `register("x.png", 200, 100)` mock dims（**core register 只收 u32 w/h，不需真 PNG 解码**），让两路径都过真实 tex_id 路径，证明包路径与 inline 在带纹理下仍逐节点等价。

### blob v3 契约
- `build_blob` 产 version==3 + 14 列。
- C# 手搓 fixture 升 v3 + 14 列（`MirrorPoolFlattenTests` / `FrameBlobV2Tests`，坑 12）。
- 新增 EditMode 测：v3 blob 含 tex_id 列，MirrorPool 按 tex_id 从 mock `_texMap` 绑对 Texture。

### 构建矩阵（R1 门，v1b.1 延续）
`cargo build -p loomgui_core --no-default-features` + `-p loomgui_ffi_c --no-default-features` 皆编（`texture.rs` / register / collect 常驻不依赖 parse）。

### Unity PlayMode（批次，押用户）
sample 场景加一张真 PNG（如 128×128 彩图）放 `Assets/StreamingAssets/`，HTML `<img src="该文件名">`，`_usePackage=true`：
- Image 显示**真像素**（非白块）。
- 无 CSS dims 时按**真实像素**布局（如 128×128）。
- 缺图 src → 白占位 + Console 红字。
- 重编换 .dll 流程验过（坑 10）。

### 命中 roadmap G7
散图→Unity→GPU→注册 TexId，达成。

---

## 11. 风险

- **R1（最高）blob v3 跨语言契约 + stale .dll**：14 列 + version bump，Rust `build_blob` 与 C# `FrameBlob` 须字节级同步；PlayMode 前 stale v2 .dll 会静默不渲（坑 10）。缓解：手搓 blob fixture 互验（坑 12 SOA 列优先）；PlayMode 前 md5sum + 关 Unity 换 .dll。
- **R2 measure 优先级回归**：三档优先级改错 → 无尺寸图走样或声明尺寸失效。缓解：measure 三档单测 + 黄金等价。
- **R3 PNG 来源/路径**：src 当 StreamingAssets 相对路径，Windows/移动端路径分隔符、StreamingAssets 移动端不可 `File.ReadAllBytes`（需 UnityWebRequest）—— 移动端 defer v1.x，v1b.2 桌面 `File.ReadAllBytes` 足够。缓解：spec 明示桌面优先，移动端 noted defer。
- **R4 ExecuteAlways GO/Texture2D 泄漏**：重载场景 `_texMap` 旧 Texture2D 不 Dispose → 泄漏（坑 11 同源）。缓解：`OnDestroy` Dispose 全部 + Awake 清旧（同 loom_node 孤儿清理模式）。

---

## 12. 与主设计 / roadmap 对齐

- **不改 `docs/design/00-main-design.md`**：§8.3 TextureView / §8.7 Mesh payload(带 texture) / §12.3 图集 / §12.4 refcount / §14.3-14.4 资源加载 均已记；v1b.2 是其「散图（非图集）+ 无 refcount」首批实现。图集（TextureView UV region + refcount）留 v1b.3，主设计不矛盾。
- `knowledge-reference`：实现后用 session-summary 记新机制（§2 asset 层加纹理注册 + §2.8 blob v3 + §3 Unity 纹理加载 API）+ 新坑（如有）+ ledger（v1b.2 ✅）。

---

## 13. 非目标 / defer 清单

| 项 | 去向 |
|---|---|
| 图集打包（shelf/guillotine + TextureView UV region） | v1b.3（C） |
| 纹理 refcount / on_release 卸载 | v1b.3（C，loose 图随 Stage） |
| alpha 纹理（alpha_tex） | 图集/字体需要时 |
| 多 Stage 纹理共享 / 全局 registry | v1.x |
| Addressables/YooAsset 异步管线 | v1.x |
| inline 路径真纹理 | v1b.x（dev 路径，低优） |
| 移动端 StreamingAssets（UnityWebRequest 取 bytes） | v1.x（v1b.2 桌面 File.ReadAllBytes） |
| NPOT / 纹理压缩 / mipmap 配置 | 按需（LoadImage 默认够 v1b.2） |
