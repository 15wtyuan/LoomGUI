# Task 8 Report — Unity Sprite 查询（path→Sprite + Sprite Atlas + 砍 LoadAtlas/_texMap）

**状态**：已完成（本机无 Unity，C# 语法自检 + grep 验证；家里机编译验证留 T12）
**日期**：2026-07-02

---

## 1. 实现内容

### 1.1 LoomStage.cs（重写）
- **砍**：`_texMap`、`_usePackage`、`_pkgFile`、`_html`、`_css`、`_stress500` 字段；`LoadHtml()`、`LoadPackage()`（private）、`LoadPackageFile()`、`LoadAtlas()`、`BuildStress500Fixture()` 方法。移除 atlas FFI 引用（`atlas_count`/`atlas_info`——T7 已从 LoomGUIBindings.cs 删绑定，T8 清掉 LoomStage 侧残留调用）。
- **新 API**：
  - `LoadPackage(string name, byte[] bytes) -> int`：转调 `loomgui_stage_load_package(h, name, name_len, bytes, bytes_len)`，多包共存。
  - `Instantiate(string pkg, string comp) -> uint`：转调 `loomgui_stage_instantiate(h, pkg, pkg_len, comp, comp_len)`，返 NodeId（0xFFFF_FFFF=失败）。
- **Awake 重构**：不再自动 load 单包建 scene。只建 stage + pool + SpriteResolver + camera + 配 transform。scene 由 driver 调 `CreateRoot` 建，内容由 `LoadPackage` + `Instantiate` 建。
- **新字段**：`SpriteResolver _sprites` + `[SerializeField] List<SpriteAtlas> _spriteAtlases`（Inspector 配）。Awake 注册进 SpriteResolver。
- **LateUpdate**：`_pool.Sync(blob, transform, _mm, _sprites, Texture2D.whiteTexture, _font)`（`_texMap` → `_sprites`）。
- **OnDestroy**：删 `_texMap` Dispose 块；加 `_sprites?.Clear()`（纯缓存，无 UnityEngine.Object 持有）。
- **`using UnityEngine.U2D;`** 加（SpriteAtlas）。

### 1.2 SpriteResolver.cs（新建）
- `Dictionary<string, Sprite> _cache` + `List<SpriteAtlas> _atlases` + `Sprite _missingSprite`。
- `GetSprite(path)`：缓存 → 遍历 `_atlases` 调 `atlas.GetSprite(Path.GetFileNameWithoutExtension(path))` → 命中缓存 + 返；全 miss → `_missingSprite`（可能 null，调用方 fallback 不崩）。miss 也缓存（避免每帧重复遍历）。
- `RegisterAtlas`/`RegisterAtlases`/`Clear`/`ClearCache`/`AtlasCount`。
- **path 命名规则**：取文件名去扩展（"icons/skin.png" → "skin"），对齐 Unity Sprite 资产名（SpriteAtlas.GetSprite 按 Sprite 名索引）。

### 1.3 MirrorPool.cs（按 path 取 Sprite）
- **Sync 签名改**：`Dictionary<uint, Texture2D> texMap` → `SpriteResolver sprites`。
- **Mesh 分支**：读 `blob.PathIdx(i)` → `blob.ReadPath(pathIdx)` → `sprites.GetSprite(path)` → `sp.texture` 作 Texture（path_idx=0/miss → fallback）。
- **UV 重映射**（`RemapMeshUvToSprite`）：blob mesh UV 是全图 [0,1]（T6 后核心不知图集），SpriteAtlas 把 Sprite 打进 atlas 子区 → 用 `sprite.rect + texture.width/height` 算 packed UV 子区，线性重映射：`packed_u = ru0 + blob_u*(ru1-ru0)`，v 同。保 blob 的 v 翻转（TL.v=1 → atlas 顶 rv1）。九宫格切片同基于 [0,1] blob UV → 同公式（slice 比例由 Rust 算进 blob UV）。
- 砍 `blob.TexId(i)` + `texMap.TryGetValue` 路径。

### 1.4 FrameBlob.cs（v7）
- `ExpectedVersion` 6 → 7。
- 列 17 注释 `tex_id` → `path_idx`（u32，1-based path 表索引，0=纯色）。
- 新增 path string table header：`PathTableOff`（@116）/`PathTableLen`（@120）。
- `TexId(i)` → `PathIdx(i)`。
- 新增 `PathCount` + `ReadPath(uint idx)`：idx=0→null；idx>0→读 path_table 内第 idx 条 length-prefixed UTF-8。越界→null（不崩）。镜像 Rust `blob.rs::read_path`。
- 加 `using System.Text;`（Encoding.UTF8）。

### 1.5 附带改动（保编译）
- **LoomShowcaseDriver.cs**：`OnDynLoadMail`/`OnDynLoadShowcase` 调 `LoadPackageFile`（已砍）→ 改 TODO-T11 stub（Log + 注释）。T11 重写为多包 instantiate。
- **LoomEventHandler.cs**：注释提 `LoadPackageFile` → 改「业务 driver 切界面前」。
- **DynamicTreeDemo.cs**：注释提「LoomStage.Awake 先用 inline _html/_css」→ 改「CreateRoot 即建场景根」。
- **测试**：`MirrorPoolTexIdTests.cs`/`AtlasMirrorPoolTests.cs` 重写为 T8 占位（旧 v4 tex_id 测试退役，完整 round-trip 留 T12）；`MirrorPoolTests.cs`/`MirrorPoolFlattenTests.cs`/`MergeMirrorPoolTests.cs` 的 `Sync(... texMap ...)` → `Sync(... null ...)`（texMap=空字典 fallback 路径，null SpriteResolver 同效果）。注：这些测试 blob 仍 v4 → v7 FrameBlob 拒绝 → IsValid=false → 测试断言会 fail（运行时，非编译错）—— T12 重写为 v7 blob。

---

## 2. path→Sprite 工作流

```
Rust build_blob (v7)
  Image 节点 → image_path intern 进 path string table → 写 path_idx（1-based，0=纯色）
  mesh UV = 全图 [0,1]（T6 后核心不知图集，不写 atlas 子区 UV）
  ↓
FrameBlob (C#)
  PathIdx(i) → ReadPath(idx) → path 字符串（如 "icons/skin.png"）
  ↓
SpriteResolver.GetSprite(path)
  缓存命中 → 返 Sprite
  缓存 miss → 遍历 List<SpriteAtlas> 调 atlas.GetSprite("skin")（文件名去扩展）
  全 miss → MissingSprite（null 时 MirrorPool 走 fallback whiteTexture，不崩）
  ↓
MirrorPool
  Sprite.texture → MaterialManager.Get(tex)（同 atlas 多 sprite 共享 texture → 同 Material → batchable）
  RemapMeshUvToSprite：blob UV [0,1] → sprite.rect 子区 packed UV
```

---

## 3. MirrorPool 读 path_idx 流程

```csharp
uint pathIdx = blob.PathIdx(i);          // 列 17，u32
if (pathIdx != 0 && sprites != null) {
    string path = blob.ReadPath(pathIdx);  // path table 第 idx 条 length-prefixed UTF-8
    if (!string.IsNullOrEmpty(path)) {
        Sprite sp = sprites.GetSprite(path);
        if (sp != null) tex = sp.texture;
    }
}
if (sp != null && sp.texture != null) RemapMeshUvToSprite(ro, sp, sp.texture);
```

---

## 4. Self-review

### 4.1 已验证
- **grep 无残留**：`atlas_count`/`atlas_info`/`_texMap`/`LoadAtlas`/`_usePackage`/`_pkgFile`/`tex_id`/`TexId`/`LoadHtml`/`LoadPackageFile` 在 Runtime/ 下仅注释提及，无代码引用。
- **所有 `Sync(...)` 调用**（Runtime + Tests）签名匹配新 `SpriteResolver` 参数。
- **FrameBlob v7 layout** 与 Rust `blob.rs` 镜像：header 124B（加 path_table off+len @116/120）、列 17=path_idx、path table layout（count + length-prefixed entries）、ReadPath 逻辑同 Rust `read_path`。
- **FFI 签名匹配**：`LoadPackage`/`Instantiate` 调用的 `loomgui_stage_load_package(h, name, name_len, bytes, bytes_len)` / `loomgui_stage_instantiate(h, pkg, pkg_len, comp, comp_len)` 与 LoomGUIBindings.cs T7 绑定一致。
- **UV 重映射公式**：blob UV 已 v 翻转（T6 后全图 [0,1]），sprite.rect 子区线性重映射保翻转。九宫格切片同基于 [0,1] → 同公式。
- **C# 语法**：`fixed` 语句、`unsafe`、`nuint` cast、`?? `null 合并、`List<Vector2>` GetUVs/SetUVs——均合法。

### 4.2 未验证（家里机编译可能暴露）
- **本机无 Unity toolchain，无法编译 C#**。可能的编译错：
  - `Sprite.texture` 返回类型（应为 Texture2D，传 `RemapMeshUvToSprite(Sprite, Texture2D)` 应匹配）。
  - `SpriteAtlas.GetSprite(string)` API（Unity 2019.4+，应可用）。
  - `Mesh.GetUVs(0, List<Vector2>)` / `SetUVs(0, List<Vector2>)` overload（Unity 2017.3+，应可用）。
- **运行时**（家里机 PlayMode T12 验）：
  - UV 重映射方向（v 翻转）—— 若 sprite 显示上下颠倒，调 RemapMeshUvToSprite 的 v 公式（swap rv0/rv1）。本机无法验。
  - SpriteAtlas.GetSprite 的 name 匹配规则——若 path "icons/skin.png" 的 Sprite 名不是 "skin"（如带目录前缀），需调 GetSprite 的 name 提取逻辑。T12 配 SpriteAtlas 时验。
  - 旧 v4/v6 测试 fail（blob version 不匹配）—— T12 重写。
  - LoomShowcaseDriver 场景缺 scene（Awake 不再自动建）—— T11 driver 重写后建。

### 4.3 设计决策
- **UV 重映射用 sprite.rect + texture 尺寸**（非 Sprite.uv）：sprite.rect 是 atlas 内像素矩形（Unity y-up，稳定），不依赖 Sprite.uv 的顶点序假设（Sprite.uv 的顺序与内部 packed 表示相关，不可靠）。线性重映射 blob [0,1] UV → 子区，保 v 翻转。
- **miss path 也缓存**：避免每帧重复遍历 atlas 查同一条 miss path（性能）。
- **SpriteResolver 独立类**（非进 LoomStage）：单一职责，便于 T9 面板/T12 测试注入 mock。
- **MissingSprite 可 null**：未注入时 MirrorPool 走 fallback whiteTexture（spec 要求"不崩"，null 透传满足）。

---

## 5. 文件清单

**改动**：
- `loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`（重写：砍 atlas + 新 LoadPackage/Instantiate）
- `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`（按 path 取 Sprite + UV 重映射）
- `loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`（v7：path_idx + ReadPath + path table header）
- `loomgui_unity/Assets/LoomGUI/Runtime/LoomShowcaseDriver.cs`（LoadPackageFile 调用 → T11 stub）
- `loomgui_unity/Assets/LoomGUI/Runtime/LoomEventHandler.cs`（注释清理）
- `loomgui_unity/Assets/LoomGUI/Examples/DynamicTreeDemo.cs`（注释清理）
- `loomgui_unity/Assets/LoomGUI/Tests/MirrorPoolTexIdTests.cs`（T8 占位）
- `loomgui_unity/Assets/LoomGUI/Tests/AtlasMirrorPoolTests.cs`（T8 占位）
- `loomgui_unity/Assets/LoomGUI/Tests/MirrorPoolTests.cs`（Sync 签名适配）
- `loomgui_unity/Assets/LoomGUI/Tests/MirrorPoolFlattenTests.cs`（Sync 签名适配）
- `loomgui_unity/Assets/LoomGUI/Tests/MergeMirrorPoolTests.cs`（Sync 签名适配）

**新建**：
- `loomgui_unity/Assets/LoomGUI/Runtime/SpriteResolver.cs`

---

## 6. 遗留 / 下游
- **T11**：重写 LoomShowcaseDriver 为多包 instantiate 架构（LoadPackage + Instantiate + AppendChild）；建 scene 骨架。
- **T12**：重写 v4/v6 测试为 v7 blob + mock SpriteAtlas/Sprite（path_idx round-trip + UV 重映射验证）；家里机编译验 C# 语法；PlayMode 验 UV 方向 + SpriteAtlas name 匹配。
- **T9**：Unity 包管理面板（配 SpriteAtlas 资产 + 打包）。
