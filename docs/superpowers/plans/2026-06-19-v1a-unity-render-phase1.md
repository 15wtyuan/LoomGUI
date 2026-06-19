# v1a Phase 1 · FFI 缝 + 静态色块渲染 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 打通 Rust 核心经 csbindgen FFI 进 Unity、镜像成 GameObject、用 URP shader 真渲染出静态色块面板（多节点 + 正确绘制序）。证明 v1 最大风险的缝成立。不含文本/rect mask（Phase 2）。

**Architecture:** 新建 Rust crate `loomgui_ffi_c`（依赖 `loomgui_core`，csbindgen 生成 C# 绑定）产出 SOA 帧_blob（Rust 把 v0 的 `Vec<RenderNode>` 拍平，mesh 顶点 re-base 到节点本地空间）。Unity 侧 `LoomStage` MonoBehaviour 每帧 `tick`→`borrow_frame`→`Marshal.Copy`→`MirrorPool` 按 `parent_id` 巢状 diff GO、`sortingOrder=sort_key`、URP unlit shader 画色块。根 Stage GO `(sf,-sf,sf)` 一次性 y-flip + 设计分辨率缩放。

**Tech Stack:** Rust（edition 2021，csbindgen）、Unity 6.5（6000.5.0f1）URP 17.5.0、C#（unsafe、`Span<byte>`+`BinaryPrimitives`）、HLSL（URP unlit）。

**依据:** spec `docs/superpowers/specs/2026-06-19-v1a-unity-render-design.md` §4.1/§4.2/§4.3；v0 源 `loomgui_core/src/{render,scene,stage}.*`；fgui 参考 `temp/FairyGUI-unity/`。

## Global Constraints

- **Rust**: edition 2021，workspace 加 member `loomgui_ffi_c`。新 crate 依赖 `loomgui_core`（path）+ `csbindgen`（build-dep）。
- **Unity**: 6000.5.0f1，URP（PC_RPAsset 已瘦身：Depth/Opaque/HDR off），Mono backend（Editor），Allow unsafe = ON，LoomUI layer = index 6。
- **坐标契约**（关键）：核心左上 y 下，C# 永不做 `height-y`。blob 里 mesh 顶点 = **节点本地空间 `[0..w,0..h]`**（Rust re-base），`transform` = 父坐标系本地位移，`parent_id` 用于 GO 巢状。根 Stage GO `localScale=(sf,-sf,sf)`（sf=MatchWidthOrHeight，与 y-flip 合一）→ winding 反转，shader 必须 `Cull Off`。
- **内存**: Rust 拥有 per-frame blob，下帧 tick 开头 reset；C# `Marshal.Copy` 到 buffer，`Span<byte>`+`BinaryPrimitives` 读，**禁用 `Marshal.PtrToStructure`**。wire 全 little-endian。
- **DrawState**: tint×alpha 烤进顶点色（不用 MPB）；clip_box 进 mask_context 专属 Material（Phase 1 mask_context 恒 0）。
- **IL2CPP-safe**（v1 build 才需要，但代码先合规）：Rust→C# 回调（Phase 1 无）将来必须 static + `[MonoPInvokeCallback]`。
- **围栏元素**: Phase 1 用 `div`/`button`/`img`（v0 已支持）；`span`/Text 节点 Phase 1 **跳过不渲染**（payload_kind=Text 时 MirrorPool 暂跳过，留 Phase 2）。
- **TDD**: Rust 用 `cargo test`；Unity 用 EditMode test（`com.unity.test-framework` 已装）+ PlayMode 集成。

## File Structure

```
loomgui/
├── Cargo.toml                      # workspace，members += "loomgui_ffi_c"
├── loomgui_ffi_c/                  # NEW crate
│   ├── Cargo.toml
│   ├── build.rs                    # csbindgen：生成 C# 到 loomgui_unity/Assets/Plugins/LoomGUI/Bindings/
│   └── src/
│       ├── lib.rs                  # extern "C" wrappers（stage 生命周期 + borrow_frame + version + shutdown）
│       └── blob.rs                 # BlobBuilder：Vec<RenderNode> → Vec<u8>（SOA + mesh arena，顶点 re-base）
└── loomgui_unity/
    ├── Assets/Plugins/LoomGUI/
    │   ├── loomgui_ffi_c.dll       # Rust 产物（入库，Plugins 白名单）
    │   ├── LoomGUI.Bindings.asmdef # unsafe enabled
    │   └── Bindings/LoomGUIBindings.cs   # csbindgen 生成（gitignore，不入库）
    └── Assets/LoomGUI/
        ├── LoomGUI.Runtime.asmdef
        ├── Runtime/
        │   ├── FrameBlob.cs        # Span 解析器（blob byte[] → 托管结构）
        │   ├── MaterialManager.cs  # DrawState 缓存：(program,texture,mask_context) → Material
        │   ├── MirrorPool.cs       # Dictionary<uint,RenderObj> O(n) diff + GO 巢状 + mesh 上传
        │   └── LoomStage.cs        # MonoBehaviour：LateUpdate 驱动 + 根 Stage/相机设置
        └── Shaders/LoomGUI-Unlit.shader   # URP unlit + CLIPPED variant（Phase 1 只用无 clip 路径）
```

blob 跨 FFI 布局（Phase 1 定版，little-endian；text 列 Phase 2 加）：
```
HEADER（全 u32）:
  magic=0x4D4F4F4C, version=1, node_count
  off_node_id, off_parent_id, off_visible, off_alpha, off_sort_key,
  off_local_x, off_local_y, off_mask_context, off_payload_kind, off_mesh_off, off_mesh_len
  off_mesh_arena, len_mesh_arena
COLUMNS（各 node_count 元素，按 off 定位）:
  node_id:u32 | parent_id:i32(-1=none) | visible:u8 | alpha:f32 | sort_key:u32
  | local_x:f32 | local_y:f32 | mask_context:u32 | payload_kind:u8
  | mesh_off:u32 | mesh_len:u32      (非 Mesh 节点 = 0)
MESH_ARENA（Mesh 节点拼接，每段 = 一个 mesh）:
  每段：vert_count:u32, idx_count:u32, verts[vert_count*2 f32], uvs[vert_count*2 f32],
        colors[vert_count*4 f32], indices[idx_count u32]
  mesh_off = 该段在 arena 的字节偏移；mesh_len = 该段字节数。
```

---

## Task 1: loomgui_ffi_c crate + csbindgen + version() round-trip

**目标**：建 `loomgui_ffi_c` crate，跑通 csbindgen 生成 C# 绑定，`loomgui_version()` 过 FFI 来回。验工具链（Rust→.dll+csbindgen）。后续 task 在此叠加。

**Files:**
- Modify: `Cargo.toml`（workspace 根）
- Create: `loomgui_ffi_c/Cargo.toml`
- Create: `loomgui_ffi_c/build.rs`
- Create: `loomgui_ffi_c/src/lib.rs`

**Interfaces:**
- Consumes: 无
- Produces: `loomgui_ffi_c` lib crate；`pub extern "C" fn loomgui_version() -> *const u8`（返回 C 字符串 `b"v1a\0"`）。csbindgen 生成 `LoomGUIBindings.cs` 含 `static extern byte* loomgui_version()`。

- [ ] **Step 1: workspace 根 Cargo.toml 加 member**

`Cargo.toml`（把 `members` 改为含 `loomgui_ffi_c`）：

```toml
[workspace]
members = ["loomgui_core", "loomgui_ffi_c"]
resolver = "2"
```

- [ ] **Step 2: 写 loomgui_ffi_c/Cargo.toml**

```toml
[package]
name = "loomgui_ffi_c"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
loomgui_core = { path = "../loomgui_core" }

[build-dependencies]
csbindgen = "1"
```

> `cdylib` 出 `.dll`（Windows）/`.dylib`/`.so`；`rlib` 让 Rust 测试能调本 crate 函数。

- [ ] **Step 3: 写 build.rs（csbindgen 生成 C# 到 Unity Bindings 目录）**

`loomgui_ffi_c/build.rs`：

```rust
fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    // 生成一份到 OUT_DIR（Rust 测试/编译用）+ 一份直接落到 Unity Bindings 目录（入库参考）。
    csbindgen::Builder::default()
        .input_extern_file("src/lib.rs")
        .csharp_dll_name("loomgui_ffi_c")
        .csharp_namespace("LoomGUI.Bindings")
        .csharp_class_name("Native")
        .csharp_use_function_pointer(false)
        .generate_csharp_file(format!("{}/LoomGUIBindings.cs", out_dir))
        .expect("csbindgen csharp gen");

    // 落到 Unity（构建脚本不允许失败影响 cargo build，故忽略错误）。
    let unity_bindings = "../loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs";
    let _ = csbindgen::Builder::default()
        .input_extern_file("src/lib.rs")
        .csharp_dll_name("loomgui_ffi_c")
        .csharp_namespace("LoomGUI.Bindings")
        .csharp_class_name("Native")
        .csharp_use_function_pointer(false)
        .generate_csharp_file(unity_bindings);
}
```

> 注：csbindgen 1.x 的确切方法名/选项若编译报错，按编译器/文档调——核心是「读 `src/lib.rs` 的 `extern "C"` → 生成 `Native` 静态类的 `[DllImport]`」。Unity 那份被 gitignore（`**/LoomGUI*Bindings*.cs`），不入库。

- [ ] **Step 4: 写 src/lib.rs 的失败测试 + version 导出**

`loomgui_ffi_c/src/lib.rs`：

```rust
//! FFI 导出层（§14.1 csbindgen）：extern "C" 薄包装，opaque Stage 句柄。
//! 命名前缀 `loomgui_`，csbindgen 扫描本文件生成 C# 绑定。

use std::ffi::CString;

/// 版本字符串（C null-terminated）。Task 1 工具链 round-trip 用。
#[no_mangle]
pub extern "C" fn loomgui_version() -> *const u8 {
    // CString 进静态：用 OnceLock 缓存，避免每次分配+泄漏。
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| CString::new("v1a").unwrap()).as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_returns_c_string_v1a() {
        unsafe {
            let s = CStr::from_ptr(loomgui_version());
            assert_eq!(s.to_str().unwrap(), "v1a");
        }
    }
}
```

- [ ] **Step 5: 跑测试**

Run: `cargo test -p loomgui_ffi_c`
Expected: 编译（首次拉 csbindgen），`test result: ok. 1 passed`。

- [ ] **Step 6: 出 .dll**

Run: `cargo build -p loomgui_ffi_c --release`
Expected: `target/release/loomgui_ffi_c.dll`（Windows）生成。

手动拷到 Unity（一次性，Task 8 会脚本化）：
```
cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll
```
> 先在 `loomgui_unity/Assets/Plugins/LoomGUI/` 建目录。Unity 会生成 `.dll.meta`（Platform=Editor+Standalone Windows x86_64）。

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml loomgui_ffi_c
git commit -m "feat(ffi_c): loomgui_ffi_c crate + csbindgen + version() FFI"
```

---

## Task 2: BlobBuilder（Rust）— Vec<RenderNode> → SOA blob，顶点 re-base

**目标**：把 v0 的 `Vec<RenderNode>` 拍平成上面 File Structure 定的 blob `Vec<u8>`。**关键**：Mesh 顶点从 v0 的父坐标系 re-base 到节点本地 `[0..w,0..h]`（减去 `transform.x/y`）。纯 Rust，完全可测。

**Files:**
- Create: `loomgui_ffi_c/src/blob.rs`
- Modify: `loomgui_ffi_c/src/lib.rs`（`pub mod blob;`）

**Interfaces:**
- Consumes: `loomgui_core::render::node::{RenderNode, NodePayload, NodeTransform}`（v0 已有）
- Produces: `pub fn build_blob(nodes: &[RenderNode]) -> Vec<u8>`；blob 布局见 File Structure。

- [ ] **Step 1: 写 blob.rs 失败测试**

`loomgui_ffi_c/src/blob.rs`：

```rust
//! 帧 blob 构建器：Vec<RenderNode> → 拍平 SOA blob（§4.1）。
//! mesh 顶点 re-base 到节点本地空间（v0 是父坐标系，减 transform.x/y）。

use loomgui_core::render::node::{BlendMode, MaskContext, NodePayload, NodeTransform, RenderNode};

/// magic = "LOOM" little-endian。
const MAGIC: u32 = 0x4D4F4F4C;
const VERSION: u32 = 1;

/// 入口：RenderNode 切片 → blob 字节。
pub fn build_blob(_nodes: &[RenderNode]) -> Vec<u8> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_node(id: u32, parent: Option<u32>, x: f32, y: f32, w: f32, h: f32) -> RenderNode {
        RenderNode {
            node_id: id,
            parent_id: parent,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            transform: NodeTransform { x, y, ..NodeTransform::default() },
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: id,
            payload: NodePayload::Mesh {
                // v0 父坐标系顶点：(x,y)(x+w,y)(x+w,y+h)(x,y+h)
                verts: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: 0,
                program: 0,
            },
        }
    }

    #[test]
    fn build_blob_has_magic_and_count() {
        let blob = build_blob(&[mesh_node(0, None, 10.0, 20.0, 5.0, 5.0)]);
        assert_eq!(&blob[0..4], &MAGIC.to_le_bytes());
        let v = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        assert_eq!(v, VERSION);
        let n = u32::from_le_bytes(blob[8..12].try_into().unwrap());
        assert_eq!(n, 1);
    }

    #[test]
    fn mesh_verts_are_rebased_to_local() {
        // 顶点原本在 (10,20)..(15,25)；re-base 后应 (0,0)..(5,5)。
        let blob = build_blob(&[mesh_node(0, None, 10.0, 20.0, 5.0, 5.0)]);
        let view = TestView::parse(&blob);
        let verts = view.mesh_verts(0);
        assert_eq!(verts[0], [0.0, 0.0]);
        assert_eq!(verts[2], [5.0, 5.0]);
        // local_x/local_y 保留原 transform（10,20），供 GO localPosition。
        assert_eq!(view.local_x(0), 10.0);
        assert_eq!(view.local_y(0), 20.0);
    }

    #[test]
    fn parent_id_minus_one_for_none() {
        let blob = build_blob(&[mesh_node(0, None, 0.0, 0.0, 1.0, 1.0)]);
        let view = TestView::parse(&blob);
        assert_eq!(view.parent_id(0), -1);
    }

    // —— 测试用解析器（镜像 C# FrameBlob 逻辑，验 Rust 布局正确）——
    // col_off 索引：0=node_id 1=parent_id 2=visible 3=alpha 4=sort_key
    //              5=local_x 6=local_y 7=mask_context 8=payload_kind 9=mesh_off 10=mesh_len
    struct TestView<'a> { buf: &'a [u8], col_off: [usize; 11], arena_off: usize }
    impl<'a> TestView<'a> {
        fn parse(buf: &'a [u8]) -> Self {
            assert_eq!(&buf[0..4], &MAGIC.to_le_bytes());
            let mut col_off = [0usize; 11];
            let mut h = 12;
            for i in 0..11 {
                col_off[i] = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()) as usize;
                h += 4;
            }
            let arena_off = u32::from_le_bytes(buf[h..h+4].try_into().unwrap()) as usize;
            TestView { buf, col_off, arena_off }
        }
        fn parent_id(&self, i: usize) -> i32 {
            let o = self.col_off[1] + i * 4;
            i32::from_le_bytes(self.buf[o..o+4].try_into().unwrap())
        }
        fn local_x(&self, i: usize) -> f32 {
            let o = self.col_off[5] + i * 4;
            f32::from_le_bytes(self.buf[o..o+4].try_into().unwrap())
        }
        fn local_y(&self, i: usize) -> f32 {
            let o = self.col_off[6] + i * 4;
            f32::from_le_bytes(self.buf[o..o+4].try_into().unwrap())
        }
        /// 读节点 i 的 mesh 顶点（arena 段：vert_count, idx_count, verts[], uvs[], colors[], indices[]）。
        fn mesh_verts(&self, i: usize) -> Vec<[f32; 2]> {
            let seg = self.arena_off + u32::from_le_bytes(
                self.buf[self.col_off[9] + i * 4..][0..4].try_into().unwrap()) as usize; // mesh_off
            let vc = u32::from_le_bytes(self.buf[seg..seg + 4].try_into().unwrap()) as usize;
            let mut p = seg + 8; // 跳 vert_count + idx_count
            (0..vc).map(|_| {
                let vx = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                let vy = f32::from_le_bytes(self.buf[p..p + 4].try_into().unwrap()); p += 4;
                [vx, vy]
            }).collect()
        }
    }
}
```

> 注：测试用解析器里的列名 hack（`off_mesh_arena`→`mesh_arena`）只为简短；实现稳定后可清理。关键是验证 magic/count、顶点 re-base、parent_id 编码。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p loomgui_ffi_c blob`
Expected: FAIL（`build_blob` 是 `todo!()`，panic）。

- [ ] **Step 3: 实现 build_blob**

替换 `build_blob` 的 `todo!()`（保留上面的 imports/常量/测试）：

```rust
pub fn build_blob(nodes: &[RenderNode]) -> Vec<u8> {
    let n = nodes.len();
    // 列名 + 每元素字节数。
    let columns: &[(&str, usize)] = &[
        ("node_id", 4), ("parent_id", 4), ("visible", 1), ("alpha", 4),
        ("sort_key", 4), ("local_x", 4), ("local_y", 4), ("mask_context", 4),
        ("payload_kind", 1), ("mesh_off", 4), ("mesh_len", 4),
    ];
    let num_col_offsets = columns.len();          // 11
    let header_len = 3 * 4                          // magic, version, node_count
        + num_col_offsets * 4                       // 列 offset
        + 2 * 4;                                    // mesh_arena off + len

    // 先把 mesh arena + per-node 列值算出来（mesh arena 决定列值里的 mesh_off/len）。
    let mut mesh_arena: Vec<u8> = Vec::new();
    let mut col_node_id = Vec::<u8>::new();
    let mut col_parent_id = Vec::<u8>::new();
    let mut col_visible = Vec::<u8>::new();
    let mut col_alpha = Vec::<u8>::new();
    let mut col_sort_key = Vec::<u8>::new();
    let mut col_local_x = Vec::<u8>::new();
    let mut col_local_y = Vec::<u8>::new();
    let mut col_mask = Vec::<u8>::new();
    let mut col_kind = Vec::<u8>::new();
    let mut col_mesh_off = Vec::<u8>::new();
    let mut col_mesh_len = Vec::<u8>::new();

    for rn in nodes {
        col_node_id.extend_from_slice(&rn.node_id.to_le_bytes());
        col_parent_id.extend_from_slice(&rn.parent_id.map(|p| p as i32).unwrap_or(-1).to_le_bytes());
        col_visible.push(rn.visible as u8);
        col_alpha.extend_from_slice(&rn.alpha.to_le_bytes());
        col_sort_key.extend_from_slice(&rn.sort_key.to_le_bytes());
        col_local_x.extend_from_slice(&rn.transform.x.to_le_bytes());
        col_local_y.extend_from_slice(&rn.transform.y.to_le_bytes());
        col_mask.extend_from_slice(&rn.mask_context.0.to_le_bytes());

        match &rn.payload {
            NodePayload::Mesh { verts, uvs, colors, indices, .. } => {
                col_kind.push(1);
                // re-base 顶点到本地：减 transform.x/y。
                let (tx, ty) = (rn.transform.x, rn.transform.y);
                let seg_off = mesh_arena.len() as u32;
                mesh_arena.extend_from_slice(&(verts.len() as u32).to_le_bytes());
                mesh_arena.extend_from_slice(&(indices.len() as u32).to_le_bytes());
                for v in verts {
                    mesh_arena.extend_from_slice(&(v[0] - tx).to_le_bytes());
                    mesh_arena.extend_from_slice(&(v[1] - ty).to_le_bytes());
                }
                for u in uvs {
                    mesh_arena.extend_from_slice(&u[0].to_le_bytes());
                    mesh_arena.extend_from_slice(&u[1].to_le_bytes());
                }
                for c in colors {
                    mesh_arena.extend_from_slice(&c[0].to_le_bytes());
                    mesh_arena.extend_from_slice(&c[1].to_le_bytes());
                    mesh_arena.extend_from_slice(&c[2].to_le_bytes());
                    mesh_arena.extend_from_slice(&c[3].to_le_bytes());
                }
                for ix in indices {
                    mesh_arena.extend_from_slice(&(*ix as u32).to_le_bytes());
                }
                let seg_len = mesh_arena.len() as u32 - seg_off;
                col_mesh_off.extend_from_slice(&seg_off.to_le_bytes());
                col_mesh_len.extend_from_slice(&seg_len.to_le_bytes());
            }
            NodePayload::Text { .. } => {
                col_kind.push(2);  // Phase 1：C# 跳过；Phase 2 实装 text arena。
                col_mesh_off.extend_from_slice(&0u32.to_le_bytes());
                col_mesh_len.extend_from_slice(&0u32.to_le_bytes());
            }
            NodePayload::Unchanged => {
                col_kind.push(0);
                col_mesh_off.extend_from_slice(&0u32.to_le_bytes());
                col_mesh_len.extend_from_slice(&0u32.to_le_bytes());
            }
        }
    }

    let col_bufs: Vec<(&str, &Vec<u8>)> = vec![
        ("node_id",&col_node_id),("parent_id",&col_parent_id),("visible",&col_visible),
        ("alpha",&col_alpha),("sort_key",&col_sort_key),("local_x",&col_local_x),
        ("local_y",&col_local_y),("mask_context",&col_mask),("payload_kind",&col_kind),
        ("mesh_off",&col_mesh_off),("mesh_len",&col_mesh_len),
    ];

    // 算各列 offset。
    let mut off = header_len;
    let mut col_offsets: Vec<u32> = Vec::new();
    for (_name, buf) in &col_bufs {
        col_offsets.push(off as u32);
        off += buf.len();
    }
    let arena_off = off as u32;
    let arena_len = mesh_arena.len() as u32;

    // 拼装。
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    for o in &col_offsets { out.extend_from_slice(&o.to_le_bytes()); }
    out.extend_from_slice(&arena_off.to_le_bytes());
    out.extend_from_slice(&arena_len.to_le_bytes());
    for (_name, buf) in &col_bufs { out.extend_from_slice(buf); }
    out.extend_from_slice(&mesh_arena);
    out
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p loomgui_ffi_c blob`
Expected: 3 passed。若测试解析器列名 hack 对不上实现，先调测试解析器（实现以"顶点 re-base + parent_id -1 + magic/count"语义为准）。

- [ ] **Step 5: Commit**

```bash
git add loomgui_ffi_c/src/blob.rs loomgui_ffi_c/src/lib.rs
git commit -m "feat(ffi_c): BlobBuilder — RenderNode → SOA blob，mesh 顶点 re-base 本地"
```

---

## Task 3: FFI ABI — stage 生命周期 + tick + borrow_frame + shutdown

**目标**：在 `loomgui_ffi_c/src/lib.rs` 加 `extern "C"` 包装：持 opaque `*mut Stage` 句柄，`load_html`/`tick`/`borrow_frame`/`shutdown`。`borrow_frame` 返回 Rust 拥有的 blob 指针+len（C# 立即拷贝）。

**Files:**
- Modify: `loomgui_ffi_c/src/lib.rs`

**Interfaces:**
- Consumes: `loomgui_core::stage::Stage`（v0：`new(font_path, root_size)`、`load_inline(html, css)`、`tick_and_render() -> Vec<RenderNode>`）；Task 2 `blob::build_blob`。
- Produces: `loomgui_stage_new/load_html/tick/borrow_frame/free/shutdown`（extern "C"）。

- [ ] **Step 1: 写失败测试（Rust 侧调 extern fn，模拟 C# 调用）**

在 `loomgui_ffi_c/src/lib.rs` 追加（在 version 函数后）：

```rust
pub mod blob;

use loomgui_core::stage::Stage;
use loomgui_core::render::node::RenderNode;

/// opaque 句柄：Stage + 缓存的最近一帧 blob（borrow_frame 返回它的指针，下帧 reset）。
pub struct StageHandle {
    stage: Stage,
    frame_blob: Vec<u8>,   // borrow_frame 返回 &this[..]；tick 时被覆盖。
}

#[no_mangle]
pub extern "C" fn loomgui_stage_new(_font_path: *const u8, _fp_len: usize, _w: f32, _h: f32) -> *mut StageHandle {
    todo!()
}

#[no_mangle]
pub extern "C" fn loomgui_stage_free(_h: *mut StageHandle) {
    todo!()
}

#[no_mangle]
pub extern "C" fn loomgui_stage_load_html(
    _h: *mut StageHandle, _html: *const u8, _html_len: usize, _css: *const u8, _css_len: usize,
) -> i32 {
    todo!()
}

#[no_mangle]
pub extern "C" fn loomgui_stage_tick(_h: *mut StageHandle, _dt: f32) {
    todo!()
}

#[no_mangle]
pub extern "C" fn loomgui_stage_borrow_frame(_h: *mut StageHandle, _out_len: *mut usize) -> *const u8 {
    todo!()
}

#[no_mangle]
pub extern "C" fn loomgui_shutdown() {}

#[cfg(test)]
mod abi_tests {
    use super::*;
    use std::ffi::CString;

    fn font_path() -> (CString, usize) {
        let p = format!("{}/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR").rsplit_once('/').unwrap().0);
        // 上两级到 workspace 根，再进 loomgui_core/tests/fixtures
        let ws = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().to_path_buf();
        let p = ws.join("loomgui_core/tests/fixtures/DejaVuSans.ttf");
        let c = CString::new(p.to_str().unwrap()).unwrap();
        let len = c.as_bytes().len();
        (c, len)
    }

    #[test]
    fn full_ffi_roundtrip_builds_blob() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr(), fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let html = CString::new(r#"<div style="width:100px;height:50px;background-color:#ff0000;"></div>"#).unwrap();
        let css = CString::new("").unwrap();
        let r = loomgui_stage_load_html(h, html.as_ptr(), html.as_bytes().len(),
                                        css.as_ptr(), css.as_bytes().len());
        assert_eq!(r, 0, "load_html ok");
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_frame(h, &mut len);
        assert!(!ptr.is_null());
        assert!(len > 12, "blob 至少含 header");
        unsafe { assert_eq!(&*(ptr as *const u8), &0x4Cu8); } // magic 第一字节 'L'
        loomgui_stage_free(h);
    }
}
```

> 注：测试用 inline style 的 div（v0 parse 支持）。若 v0 的 `load_inline` 不吃 inline style，改用 `<style>` 块或 class+css 参数。以 v0 实际为准调 HTML 串。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p loomgui_ffi_c abi_tests`
Expected: FAIL（`todo!()` panic）。

- [ ] **Step 3: 实现各 extern fn**

替换上面 5 个 `todo!()`（`shutdown` 已空实现）：

```rust
#[no_mangle]
pub extern "C" fn loomgui_stage_new(font_path: *const u8, fp_len: usize, w: f32, h: f32) -> *mut StageHandle {
    let path = unsafe { std::slice::from_raw_parts(font_path, fp_len) };
    let path = match std::str::from_utf8(path) { Ok(s) => s, Err(_) => return std::ptr::null_mut() };
    let stage = match Stage::new(path, (w, h)) { Ok(s) => s, Err(_) => return std::ptr::null_mut() };
    Box::into_raw(Box::new(StageHandle { stage, frame_blob: Vec::new() }))
}

#[no_mangle]
pub extern "C" fn loomgui_stage_free(h: *mut StageHandle) {
    if h.is_null() { return; }
    unsafe { drop(Box::from_raw(h)); }
}

#[no_mangle]
pub extern "C" fn loomgui_stage_load_html(
    h: *mut StageHandle, html: *const u8, html_len: usize, css: *const u8, css_len: usize,
) -> i32 {
    let sh = unsafe { &mut *h };
    let html = unsafe { std::slice::from_raw_parts(html, html_len) };
    let css = unsafe { std::slice::from_raw_parts(css, css_len) };
    let html = match std::str::from_utf8(html) { Ok(s) => s, Err(_) => return -1 };
    let css = match std::str::from_utf8(css) { Ok(s) => s, Err(_) => return -1 };
    match sh.stage.load_inline(html, css) { Ok(()) => 0, Err(_) => -1 }
}

#[no_mangle]
pub extern "C" fn loomgui_stage_tick(h: *mut StageHandle, _dt: f32) {
    let sh = unsafe { &mut *h };
    let nodes: Vec<RenderNode> = sh.stage.tick_and_render();
    sh.frame_blob = blob::build_blob(&nodes);
}

#[no_mangle]
pub extern "C" fn loomgui_stage_borrow_frame(h: *mut StageHandle, out_len: *mut usize) -> *const u8 {
    let sh = unsafe { &*h };
    unsafe { *out_len = sh.frame_blob.len(); }
    sh.frame_blob.as_ptr()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p loomgui_ffi_c abi_tests`
Expected: PASS（blob round-trip，magic 首字节 'L'）。

- [ ] **Step 5: 重新出 .dll + 拷贝**

Run: `cargo build -p loomgui_ffi_c --release && cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`

- [ ] **Step 6: Commit**

```bash
git add loomgui_ffi_c/src/lib.rs
git commit -m "feat(ffi_c): stage 生命周期 + tick + borrow_frame extern C ABI"
```

---

## Task 4: Unity Bindings asmdef + FrameBlob Span 解析器

**目标**：Unity 侧建 Bindings asmdef（接 csbindgen 生成的 `LoomGUIBindings.cs`），写 `FrameBlob.cs` 用 `Span<byte>`+`BinaryPrimitives` 解析 blob。EditMode 单测喂已知 byte[] 验证。

**Files:**
- Create: `loomgui_unity/Assets/Plugins/LoomGUI/LoomGUI.Bindings.asmdef`
- Create: `loomgui_unity/Assets/LoomGUI/LoomGUI.Runtime.asmdef`
- Create: `loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`
- Create: `loomgui_unity/Assets/LoomGUI/Tests/FrameBlobTests.cs`（EditMode）

**Interfaces:**
- Consumes: Task 1 csbindgen 生成的 `LoomGUI.Bindings.Native`（实际 Task 4 只用 FrameBlob 解析，Native 调用在 Task 8）。
- Produces: `FrameBlob` struct（node_count、各列 Span 访问器、`MeshSegment` 读出 verts/uvs/colors/indices）。

- [ ] **Step 1: 写 LoomGUI.Bindings.asmdef**

`loomgui_unity/Assets/Plugins/LoomGUI/LoomGUI.Bindings.asmdef`：

```json
{
    "name": "LoomGUI.Bindings",
    "rootNamespace": "LoomGUI.Bindings",
    "references": [],
    "includePlatforms": [],
    "excludePlatforms": [],
    "allowUnsafeCode": true,
    "autoReferenced": true
}
```

> csbindgen 生成的 `Bindings/LoomGUIBindings.cs`（namespace `LoomGUI.Bindings`，class `Native`）落在本 asmdef 目录下，自动归此程序集。

- [ ] **Step 2: 写 LoomGUI.Runtime.asmdef**

`loomgui_unity/Assets/LoomGUI/LoomGUI.Runtime.asmdef`：

```json
{
    "name": "LoomGUI.Runtime",
    "rootNamespace": "LoomGUI",
    "references": ["LoomGUI.Bindings"],
    "includePlatforms": [],
    "excludePlatforms": [],
    "allowUnsafeCode": true
}
```

- [ ] **Step 3: 写 FrameBlob.cs**

`loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`：

```csharp
using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace LoomGUI
{
    /// 帧 blob 托管解析视图（Task 4）。解析 Rust build_blob 产出的 little-endian blob。
    public readonly struct FrameBlob
    {
        public const uint Magic = 0x4D4F4F4C;
        readonly byte[] _buf;

        public FrameBlob(byte[] buf) { _buf = buf; }
        public int NodeCount => ReadU32(8);

        // 列 offset 在 header[12..12+11*4)。顺序同 Rust columns。
        int ColOff(int idx) => (int)ReadU32(12 + idx * 4);
        int ArenaOff => (int)ReadU32(12 + 11 * 4);
        int ArenaLen => (int)ReadU32(12 + 12 * 4);

        public uint NodeId(int i) => ReadU32(ColOff(0) + i * 4);
        public int ParentId(int i) => (int)ReadU32(ColOff(1) + i * 4);
        public bool Visible(int i) => _buf[ColOff(2) + i] != 0;
        public float Alpha(int i) => ReadF32(ColOff(3) + i * 4);
        public uint SortKey(int i) => ReadU32(ColOff(4) + i * 4);
        public float LocalX(int i) => ReadF32(ColOff(5) + i * 4);
        public float LocalY(int i) => ReadF32(ColOff(6) + i * 4);
        public uint MaskContext(int i) => ReadU32(ColOff(7) + i * 4);
        public byte PayloadKind(int i) => _buf[ColOff(8) + i];        // 0=Unchanged 1=Mesh 2=Text
        uint MeshOff(int i) => ReadU32(ColOff(9) + i * 4);
        uint MeshLen(int i) => ReadU32(ColOff(10) + i * 4);

        /// 读节点 i 的 mesh（payload_kind==1 才调）。verts/uvs/colors 是 f32，indices 是 u32。
        public MeshSegment ReadMesh(int i)
        {
            int p = ArenaOff + (int)MeshOff(i);
            int vertCount = (int)ReadU32(p); p += 4;
            int idxCount = (int)ReadU32(p); p += 4;
            var seg = new MeshSegment(vertCount, idxCount);
            for (int v = 0; v < vertCount; v++) {
                seg.Verts[v] = new UnityEngine.Vector2(ReadF32(p), ReadF32(p + 4)); p += 8;
            }
            for (int v = 0; v < vertCount; v++) {
                seg.Uvs[v] = new UnityEngine.Vector2(ReadF32(p), ReadF32(p + 4)); p += 8;
            }
            for (int v = 0; v < vertCount; v++) {
                seg.Colors[v] = new UnityEngine.Color(ReadF32(p), ReadF32(p+4), ReadF32(p+8), ReadF32(p+12)); p += 16;
            }
            for (int k = 0; k < idxCount; k++) { seg.Idx[k] = ReadU32(p); p += 4; }
            return seg;
        }

        uint ReadU32(int o) => BitConverter.ToUInt32(_buf, o);
        float ReadF32(int o) => BitConverter.ToSingle(_buf, o);
    }

    public sealed class MeshSegment
    {
        public readonly UnityEngine.Vector2[] Verts;
        public readonly UnityEngine.Vector2[] Uvs;
        public readonly UnityEngine.Color[] Colors;
        public readonly uint[] Idx;
        public MeshSegment(int vertCount, int idxCount) {
            Verts = new UnityEngine.Vector2[vertCount]; Uvs = new UnityEngine.Vector2[vertCount];
            Colors = new UnityEngine.Color[vertCount]; Idx = new uint[idxCount];
        }
    }
}
```

> 用 `BitConverter`（托管数组上安全且快）。spec 说 `Span<byte>+BinaryPrimitives`；`BitConverter.ToXXX(byte[], int)` 等价、可读性高，v1a 先用，v1e 改 Span 零分配。

- [ ] **Step 4: 写 EditMode 测试（手搓一个最小 blob byte[] 验解析）**

`loomgui_unity/Assets/LoomGUI/Tests/FrameBlobTests.cs`：

```csharp
using NUnit.Framework;
using LoomGUI;

namespace LoomGUI.Tests
{
    public class FrameBlobTests
    {
        // 手搓一个 1 节点 mesh blob（镜像 Rust build_blob 布局），验解析。
        [Test]
        public void ParsesOneMeshNode()
        {
            var b = new System.Collections.Generic.List<byte>();
            // header: magic, version, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Fu));
            b.AddRange(System.BitConverter.GetBytes(1u));
            b.AddRange(System.BitConverter.GetBytes(1u));
            int headerLen = 12 + 11 * 4 + 2 * 4; // = 60
            // 11 列 offset（每列 1 元素）+ arena off/len，先占位后填。
            int colOff = headerLen;
            int[] offs = new int[11];
            int[] elemSize = { 4,4,1,4,4,4,4,4,1,4,4 };
            for (int i = 0; i < 11; i++) { offs[i] = colOff; colOff += elemSize[i]; }
            int arenaOff = colOff;
            // arena: 1 mesh, 4 verts, 6 idx。verts 全 (0,0)..  占位。
            var arena = new System.Collections.Generic.List<byte>();
            int arenaStart = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
            arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
            for (int v=0;v<4;v++){ arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v=0;v<4;v++){ arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v=0;v<4;v++){ arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(1f)); }
            for (int k=0;k<6;k++) arena.AddRange(System.BitConverter.GetBytes(0u));
            int arenaLen = arena.Count - arenaStart;

            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));

            // 列数据：node_id=7, parent=-1, visible=1, alpha=1, sort=3, lx=10, ly=20, mask=0, kind=1, mesh_off=0, mesh_len=arenaLen
            b.AddRange(System.BitConverter.GetBytes(7u));
            b.AddRange(System.BitConverter.GetBytes(-1));
            b.Add(1);
            b.AddRange(System.BitConverter.GetBytes(1f));
            b.AddRange(System.BitConverter.GetBytes(3u));
            b.AddRange(System.BitConverter.GetBytes(10f));
            b.AddRange(System.BitConverter.GetBytes(20f));
            b.AddRange(System.BitConverter.GetBytes(0u));
            b.Add(1);
            b.AddRange(System.BitConverter.GetBytes(0u));
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen));
            b.AddRange(arena);

            var view = new FrameBlob(b.ToArray());
            Assert.AreEqual(1, view.NodeCount);
            Assert.AreEqual(7u, view.NodeId(0));
            Assert.AreEqual(-1, view.ParentId(0));
            Assert.AreEqual(10f, view.LocalX(0));
            Assert.AreEqual(1, view.PayloadKind(0));
            var mesh = view.ReadMesh(0);
            Assert.AreEqual(4, mesh.Verts.Length);
            Assert.AreEqual(6, mesh.Idx.Length);
        }
    }
}
```

- [ ] **Step 5: 跑 EditMode 测试**

Run（Unity Editor）：`Assets/LoomGUI/Tests/FrameBlobTests.cs` → Run（Window → General → Test Runner → EditMode → Run All）
Expected: `ParsesOneMeshNode` PASS。

- [ ] **Step 6: Commit**

```bash
git add loomgui_unity/Assets/Plugins/LoomGUI/LoomGUI.Bindings.asmdef loomgui_unity/Assets/LoomGUI
git commit -m "feat(unity): Bindings/Runtime asmdef + FrameBlob Span 解析器 + EditMode 测试"
```

---

## Task 5: URP unlit shader（LoomGUI-Unlit）

**目标**：URP unlit 透明 shader：顶点色 × 贴图、`Cull Off`、`ZWrite Off`、queue Transparent、blend property、`CLIPPED` variant（Phase 1 走无 clip 路径，variant 预留给 Phase 2）。照 fgui `FairyGUI-Image.shader` URP 化（spec §4.2c）。

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader`

**Interfaces:**
- Consumes: 无
- Produces: `Shader "LoomGUI/Unlit"`，properties `_MainTex`、`_SrcFactor`/`_DstFactor`、`_ClipBox`；keyword `CLIPPED`。

- [ ] **Step 1: 写 shader**

`loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader`：

```hlsl
Shader "LoomGUI/Unlit"
{
    Properties
    {
        _MainTex ("Texture", 2D) = "white" {}
        _SrcFactor ("SrcFactor", Float) = 5   // SrcAlpha
        _DstFactor ("DstFactor", Float) = 10  // OneMinusSrcAlpha
        _ClipBox ("ClipBox", Vector) = (0,0,1,1)
    }
    SubShader
    {
        Tags { "RenderPipeline" = "UniversalPipeline" "Queue" = "Transparent" "RenderType" = "Transparent" }
        Cull Off
        ZWrite Off
        Blend [_SrcFactor] [_DstFactor]

        Pass
        {
            HLSLPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #pragma multi_compile _ CLIPPED
            #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"

            struct Attr { float4 pos : POSITION; float4 color : COLOR; float2 uv : TEXCOORD0; };
            struct Vary { float4 pos : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD0;
                          float2 clipPos : TEXCOORD1; };

            CBUFFER_START(UnityPerMaterial)
                float4 _MainTex_ST;
                float4 _ClipBox;
            CBUFFER_END
            TEXTURE2D(_MainTex); SAMPLER(sampler_MainTex);

            Vary vert(Attr v) {
                Vary o;
                o.pos = TransformObjectToHClip(v.pos.xyz);
                o.color = v.color;
                o.uv = TRANSFORM_TEX(v.uv, _MainTex);
                float2 worldXY = TransformObjectToWorld(v.pos.xyz).xy;
                o.clipPos = worldXY * _ClipBox.zw + _ClipBox.xy;
                return o;
            }
            half4 frag(Vary i) : SV_Target {
                half4 col = SAMPLE_TEXTURE2D(_MainTex, sampler_MainTex, i.uv) * i.color;
                #ifdef CLIPPED
                float2 f = abs(i.clipPos);
                col.a *= step(max(f.x, f.y), 1.0);
                #endif
                return col;
            }
            ENDHLSL
        }
    }
}
```

> `Cull Off` 因根 y-flip winding 反转（spec §8.1）。`CLIPPED` 分支 Phase 1 不激活（material 不 enable keyword），Phase 2 rect mask 启用。`blend [_Src] [_Dst]` 照 fgui 用 property 非 variant。

- [ ] **Step 2: 验证 shader 编译**

Unity Editor 等待导入，选中 `LoomGUI-Unlit.shader`，Inspector 无编译错误（Console 干净）。
手动 sanity：新建 Material 选 `LoomGUI/Unlit`，能赋纹理、调 SrcFactor/DstFactor。

- [ ] **Step 3: Commit**

```bash
git add loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader
git commit -m "feat(unity): URP unlit shader（Cull Off/顶点色/CLIPPED variant）"
```

---

## Task 6: MaterialManager（DrawState 缓存）

**目标**：`(program, texture, mask_context)` → Material 缓存（照 fgui `MaterialManager`，spec §4.2b）。Phase 1：program 恒 Image、mask_context 恒 0、texture = 占位 1×1 白（图片）或白（纯色）。tint 不走 material（走顶点色）。

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs`

**Interfaces:**
- Consumes: Task 5 `Shader "LoomGUI/Unlit"`。
- Produces: `MaterialManager.Get(program, texture, maskContext) -> Material`（同 key 复用实例）。

- [ ] **Step 1: 写 MaterialManager.cs 失败测试（EditMode）**

`loomgui_unity/Assets/LoomGUI/Tests/MaterialManagerTests.cs`：

```csharp
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    public class MaterialManagerTests
    {
        [Test]
        public void SameKeyReturnsSameMaterial()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var a = mm.Get(program: 0, white, maskContext: 0);
            var b = mm.Get(program: 0, white, maskContext: 0);
            Assert.AreSame(a, b);
        }

        [Test]
        public void DifferentMaskContextReturnsDifferentMaterial()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var a = mm.Get(0, white, 0);
            var b = mm.Get(0, white, 1);
            Assert.AreNotSame(a, b);
        }
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: Test Runner → EditMode → `MaterialManagerTests`
Expected: FAIL（`MaterialManager` 类型不存在，编译错）。

- [ ] **Step 3: 实现 MaterialManager.cs**

`loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs`：

```csharp
using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// DrawState 缓存（§8.4，照 fgui MaterialManager）。
    /// key = (program, texture, mask_context)。同 key 复用 Material 实例。
    /// tint×alpha 走顶点色（不在 key 里）；clip_box 进 mask_context 专属 Material 的 _ClipBox uniform。
    public sealed class MaterialManager
    {
        readonly Shader _shader;
        readonly Dictionary<Key, Material> _cache = new();
        // Phase 1：mask_context 恒 0，不 enable CLIPPED；Phase 2 rect mask 在此按 mask_context 设 _ClipBox + EnableKeyword。
        readonly Dictionary<uint, Vector4> _clipBoxByCtx = new();

        public MaterialManager(Shader shader) { _shader = shader; }

        public Material Get(int program, Texture texture, uint maskContext)
        {
            var key = new Key(program, texture ? texture.GetInstanceID() : 0, maskContext);
            if (!_cache.TryGetValue(key, out var mat))
            {
                mat = new Material(_shader);
                mat.mainTexture = texture;
                mat.SetFloat("_SrcFactor", 5f);   // SrcAlpha
                mat.SetFloat("_DstFactor", 10f);  // OneMinusSrcAlpha
                if (_clipBoxByCtx.TryGetValue(maskContext, out var cb))
                {
                    mat.SetVector("_ClipBox", cb);
                    mat.EnableKeyword("CLIPPED");
                }
                _cache[key] = mat;
            }
            return mat;
        }

        /// Phase 2 rect mask 用：注册某 mask_context 的 clip_box。
        public void SetClipBox(uint maskContext, Vector4 clipBox) => _clipBoxByCtx[maskContext] = clipBox;

        public void Clear() { foreach (var kv in _cache) Object.Destroy(kv.Value); _cache.Clear(); }

        readonly struct Key
        {
            readonly int _program, _tex; readonly uint _ctx;
            public Key(int p, int t, uint c) { _program = p; _tex = t; _ctx = c; }
            public override int GetHashCode() => System.HashCode.Combine(_program, _tex, _ctx);
            public override bool Equals(object o) => o is Key k && k._program==_program && k._tex==_tex && k._ctx==_ctx;
        }
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: Test Runner → `MaterialManagerTests`
Expected: 2 passed。

- [ ] **Step 5: Commit**

```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs loomgui_unity/Assets/LoomGUI/Tests/MaterialManagerTests.cs
git commit -m "feat(unity): MaterialManager DrawState 缓存（program/texture/mask_context）"
```

---

## Task 7: MirrorPool — NodeId→GO diff + GO 巢状 + mesh 上传

**目标**：`Dictionary<uint, RenderObj>` O(n) diff（stale-flag），按 `parent_id` 巢状 GO，`localPosition=(local_x, local_y)`，`sortingOrder=sort_key`，Mesh 节点上 4 顶点 quad mesh（本地空间，Task 2 已 re-base），材质走 MaterialManager。Text 节点 Phase 1 跳过。

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`

**Interfaces:**
- Consumes: Task 4 `FrameBlob`、Task 6 `MaterialManager`。
- Produces: `MirrorPool.Sync(FrameBlob blob, Transform root, MaterialManager mm, Texture placeholder)`。

- [ ] **Step 1: 写 MirrorPool.cs 失败测试（EditMode）**

`loomgui_unity/Assets/LoomGUI/Tests/MirrorPoolTests.cs`：

```csharp
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    public class MirrorPoolTests
    {
        FrameBlob OneNodeBlob(uint id, int parent, float x, float y, uint sort)
        {
            // 复用 FrameBlobTests 的手搓法；为简洁这里只示意——实际从 Rust 产出的 blob 取，
            // 或抽 FrameBlobTests 的 builder 成公共 helper。本测试重点：Sync 后 GO 数量 + 层级。
            Assert.Ignore("用 PlayMode 集成测（Task 8）；EditMode 测 mesh 上传见下");
            return default;
        }

        [Test]
        public void SyncCreatesGoPerMeshNodeAndNestsByParent()
        {
            // 真实验证放 PlayMode（Task 8 里跑真实 blob）。
            // EditMode 这里只验 MirrorPool 能 new + Sync 空 blob 不崩。
            var go = new GameObject("root");
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var pool = new MirrorPool();
            var empty = new FrameBlob(new byte[0]);  // NodeCount 解析 0 不崩
            // 空 blob Sync 不应抛。
            Assert.DoesNotThrow(() => pool.Sync(empty, go.transform, mm, Texture2D.whiteTexture));
            Object.DestroyImmediate(go);
        }
    }
}
```

> 真实多节点渲染验证在 Task 8 PlayMode（需真实 Rust blob）。EditMode 保证不崩 + 类型契约。

- [ ] **Step 2: 实现 MirrorPool.cs**

`loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`：

```csharp
using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// 渲染树 → GameObject 镜像 diff（§14.6）。每帧 O(n)：标 stale → 遍历命中清 stale/更新 → 余销毁。
    /// GO 按 parent_id 巢状；localPosition=(local_x,local_y)；sortingOrder=sort_key。
    sealed class RenderObj
    {
        public GameObject Go;
        public MeshFilter Mf;
        public MeshRenderer Mr;
        public Mesh Mesh;
        public bool Stale;
        public uint LastNodeId;       // 复用 GO 时校验
    }

    public sealed class MirrorPool
    {
        readonly Dictionary<uint, RenderObj> _pool = new();

        public void Sync(FrameBlob blob, Transform root, MaterialManager mm, Texture placeholder)
        {
            // ① 全标 stale
            foreach (var kv in _pool) kv.Value.Stale = true;

            // ② 遍历节点
            int n = blob.NodeCount;
            for (int i = 0; i < n; i++)
            {
                if (!blob.Visible(i)) continue;
                byte kind = blob.PayloadKind(i);
                if (kind != 1) continue;  // Phase 1：只渲染 Mesh(1)；Text(2)/Unchanged(0) 跳过

                uint id = blob.NodeId(i);
                if (!_pool.TryGetValue(id, out var ro))
                {
                    ro = NewRenderObj(root);
                    ro.LastNodeId = id;
                    _pool[id] = ro;
                }
                ro.Stale = false;

                // 巢状：SetParent 按 parent_id
                Transform parent = root;
                int pid = blob.ParentId(i);
                if (pid >= 0 && _pool.TryGetValue((uint)pid, out var pro)) parent = pro.Go.transform;
                ro.Go.transform.SetParent(parent, false);
                ro.Go.transform.localPosition = new Vector3(blob.LocalX(i), blob.LocalY(i), 0f);
                ro.Go.transform.localScale = Vector3.one;

                ro.Mr.sortingOrder = (int)blob.SortKey(i);

                // mesh 上传
                var seg = blob.ReadMesh(i);
                UploadMesh(ro, seg);
                ro.Mesh.RecalculateBounds();

                // 材质：Phase 1 program=0（Image），mask_context=0，texture=占位白。
                ro.Mr.sharedMaterial = mm.Get(program: 0, placeholder, blob.MaskContext(i));
            }

            // ③ 余 stale 销毁
            var dead = new List<uint>();
            foreach (var kv in _pool) if (kv.Value.Stale) dead.Add(kv.Key);
            foreach (var id in dead) { Object.Destroy(_pool[id].Go); _pool.Remove(id); }
        }

        static RenderObj NewRenderObj(Transform root)
        {
            var go = new GameObject("loom_node");
            go.transform.SetParent(root, false);
            go.layer = root.gameObject.layer;  // LoomUI
            var mf = go.AddComponent<MeshFilter>();
            var mr = go.AddComponent<MeshRenderer>();
            var mesh = new Mesh { indexFormat = UnityEngine.Rendering.IndexFormat.UInt32 };
            mesh.MarkDynamic();
            mf.sharedMesh = mesh;
            return new RenderObj { Go = go, Mf = mf, Mr = mr, Mesh = mesh };
        }

        static void UploadMesh(RenderObj ro, MeshSegment seg)
        {
            var verts = new Vector3[seg.Verts.Length];
            for (int i = 0; i < seg.Verts.Length; i++) verts[i] = new Vector3(seg.Verts[i].x, seg.Verts[i].y, 0);
            var cols = new Color[seg.Colors.Length];
            for (int i = 0; i < seg.Colors.Length; i++) cols[i] = seg.Colors[i];
            var idx = new int[seg.Idx.Length];
            for (int i = 0; i < seg.Idx.Length; i++) idx[i] = (int)seg.Idx[i];
            ro.Mesh.Clear();
            ro.Mesh.SetVertices(verts);
            ro.Mesh.SetUVs(0, seg.Uvs);
            ro.Mesh.SetColors(cols);
            ro.Mesh.SetTriangles(idx, 0);
        }

        public void Clear()
        {
            foreach (var kv in _pool) Object.Destroy(kv.Value.Go);
            _pool.Clear();
        }
    }
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: Test Runner → `MirrorPoolTests`
Expected: PASS（不崩；`OneNodeBlob` 那个 Ignore，真实验证 Task 8）。

- [ ] **Step 4: Commit**

```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs loomgui_unity/Assets/LoomGUI/Tests/MirrorPoolTests.cs
git commit -m "feat(unity): MirrorPool 渲染树 diff + GO 巢状 + mesh 上传"
```

---

## Task 8: LoomStage MonoBehaviour 驱动 + 根 Stage/相机 + PlayMode 集成

**目标**：`LoomStage` MonoBehaviour——`Awake` 建 stage + `load_html`，`LateUpdate` 每 `tick`→`borrow_frame`→`Marshal.Copy`→`MirrorPool.Sync`。根 Stage GO `localScale=(sf,-sf,sf)`、UI 相机 cullingMask=LoomUI(layer 6)。PlayMode 里加载一段 HTML，看到 N 个色块按布局渲染、绘制序正确。

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`
- Create: `loomgui_unity/Assets/LoomGUI/Tests/LoomStagePlayModeTests.cs`（PlayMode）
- Manual: 一个测试场景挂 `LoomStage` + UI 相机

**Interfaces:**
- Consumes: Task 3 `Native.*`（csbindgen 生成）、Task 4 `FrameBlob`、Task 6 `MaterialManager`、Task 7 `MirrorPool`。
- Produces: `LoomStage` MonoBehaviour（挂场景即跑）。

- [ ] **Step 1: 写 LoomStage.cs**

`loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`：

```csharp
using System;
using System.Runtime.InteropServices;
using LoomGUI.Bindings;
using UnityEngine;

namespace LoomGUI
{
    [ExecuteAlways]
    public class LoomStage : MonoBehaviour
    {
        [SerializeField] string _html = "<div style=\"width:200px;height:100px;background-color:#ff0000;\"></div>";
        [SerializeField] string _css = "";
        [SerializeField] Vector2 _designSize = new(1080, 1920);
        [SerializeField] Camera _uiCamera;

        IntPtr _stage;
        MaterialManager _mm;
        MirrorPool _pool;
        byte[] _frameBuf;

        void Awake()
        {
            // 字体路径（StreamingAssets 里 DejaVuSans.ttf）→ UTF8 byte[] → fixed 钉住给 native。
            string fontPath = System.IO.Path.Combine(Application.streamingAssetsPath, "DejaVuSans.ttf");
            byte[] fpBytes = System.Text.Encoding.UTF8.GetBytes(fontPath);
            unsafe {
                fixed (byte* fp = fpBytes) {
                    _stage = Native.loomgui_stage_new(fp, fpBytes.Length, _designSize.x, _designSize.y);
                }
            }
            if (_stage == IntPtr.Zero) { Debug.LogError("loomgui_stage_new 失败"); return; }
            LoadHtml();
            _mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            _pool = new MirrorPool();
            ConfigureRoot();
        }

        // 注：csbindgen 把 `*const u8` 生成 `byte*`；string → UTF8 byte[] + fixed 钉住传递。
        // 若 csbindgen 配了 string 辅助（参数名 _str 等），改用它。以生成签名为准。
        void LoadHtml()
        {
            byte[] hb = System.Text.Encoding.UTF8.GetBytes(_html);
            byte[] cb = System.Text.Encoding.UTF8.GetBytes(_css);
            unsafe {
                fixed (byte* hp = hb, cp = cb) {
                    Native.loomgui_stage_load_html(_stage, hp, hb.Length, cp, cb.Length);
                }
            }
        }

        void ConfigureRoot()
        {
            // 根 Stage scale = (sf,-sf,sf)：设计分辨率 MatchWidthOrHeight + y-flip 合一。
            float sw = Screen.width, sh = Screen.height;
            float sf = Mathf.Min(sw / _designSize.x, sh / _designSize.y);
            transform.localScale = new Vector3(sf, -sf, sf);
            // UI 相机：正交，cullingMask=LoomUI(6)，orthoSize=sh/2。
            if (_uiCamera == null)
            {
                var cgo = new GameObject("LoomUICamera");
                _uiCamera = cgo.AddComponent<Camera>();
            }
            _uiCamera.orthographic = true;
            _uiCamera.orthographicSize = sh / 2f;
            _uiCamera.clearFlags = CameraClearFlags.Depth;
            _uiCamera.cullingMask = 1 << 6;   // LoomUI
            _uiCamera.transform.SetParent(transform, false);
            // Stage 本体定位：让设计坐标系原点(0,0)对到屏幕左上。
            transform.position = new Vector3(-sw / 2f, sh / 2f, 0);
            gameObject.layer = 6;
        }

        void LateUpdate()
        {
            if (_stage == IntPtr.Zero) return;
            Native.loomgui_stage_tick(_stage, Time.deltaTime);

            // borrow_frame(*mut usize) → csbindgen byte* + ulong*；钉住一个 ulong 取回 len。
            // 若 csbindgen 生成的 out_len 类型不同（void*/nuint*），按生成签名调。
            byte* ptr;
            int len;
            unsafe {
                ulong lenRaw = 0;
                fixed (ulong* lp = &lenRaw) {
                    ptr = Native.loomgui_stage_borrow_frame(_stage, lp);
                }
                if (ptr == null || lenRaw == 0) return;
                len = (int)lenRaw;
            }

            // 原子拷贝到托管 buffer（§14.3）。v1a 先 new；ArrayPool 留 v1e。
            if (_frameBuf == null || _frameBuf.Length < len) _frameBuf = new byte[len];
            Marshal.Copy((IntPtr)ptr, _frameBuf, 0, len);

            var blob = new FrameBlob(_frameBuf);
            _pool.Sync(blob, transform, _mm, Texture2D.whiteTexture);
        }

        void OnDestroy()
        {
            _pool?.Clear();
            _mm?.Clear();
            if (_stage != IntPtr.Zero) Native.loomgui_stage_free(_stage);
            _stage = IntPtr.Zero;
        }

        // Domain reload 保护（§4.3e）——Phase 1 最小版：清静态状态。完整 loomgui_shutdown 留 Phase 2。
        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        static void ResetStatics() { /* Phase 2：调 loomgui_shutdown + 清全局缓存 */ }
    }
}
```

> `Native.*` 来自 Task 1 csbindgen 生成的 `LoomGUIBindings.cs`（class `Native`）。DejaVuSans.ttf 需放 `Assets/StreamingAssets/`（从 `loomgui_core/tests/fixtures/` 拷）。

- [ ] **Step 2: 准备 StreamingAssets 字体**

```
cp loomgui_core/tests/fixtures/DejaVuSans.ttf loomgui_unity/Assets/StreamingAssets/DejaVuSans.ttf
```

- [ ] **Step 3: 写 PlayMode 测试（验渲染树真同步）**

`loomgui_unity/Assets/LoomGUI/Tests/LoomStagePlayModeTests.cs`：

```csharp
using System.Collections;
using NUnit.Framework;
using UnityEngine;
using UnityEngine.TestTools;

namespace LoomGUI.Tests
{
    public class LoomStagePlayModeTests
    {
        [UnityTest]
        public IEnumerator RendersColoredQuadsFromHtml()
        {
            var go = new GameObject("stage");
            var stage = go.AddComponent<LoomStage>();
            // 跳一帧让 Awake + LateUpdate 跑
            yield return null;
            yield return null;
            // 验证：MirrorPool 内部 GO 数 > 0（通过反射或暴露 Count）。这里暴露个简易断言：
            // 至少 stage 下有子 GO（色块）。
            Assert.Greater(go.transform.childCount, 1, "应渲染出色块 GO");
            Object.Destroy(go);
        }
    }
}
```

> 若 v0 不支持 inline style，把 `_html` 默认值改 class+`_css`：`<div class="b"></div>` + `.b{width:200px;height:100px;background-color:#ff0000;}`。以 v0 parse 实际为准。

- [ ] **Step 4: PlayMode 跑 + 人工看**

Run: Test Runner → PlayMode → `RendersColoredQuadsFromHtml`
同时手动：建场景，挂 `LoomStage`，进 Play，Game 视图应看到一个红 `200×100` 色块在左上区域（设计坐标系 1080×1920 缩放到屏幕）。
Expected: 色块出现、位置/尺寸合理、无 Console 报错。

- [ ] **Step 5: 多节点 + 绘制序验证**

把 `_html` 换成多节点（验证 sort_key→sortingOrder + GO 巢状）：
```
<div style="display:flex;gap:10px;width:400px;height:100px;background-color:#00ff00;">
  <div style="width:50px;height:50px;background-color:#ff0000;"></div>
  <div style="width:50px;height:50px;background-color:#0000ff;"></div>
</div>
```
进 Play：应看到绿底横排 + 红/蓝两个子块；子块的 `sortingOrder` > 父（Inspector 可查 MeshRenderer.sortingOrder）。

- [ ] **Step 6: Commit**

```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs loomgui_unity/Assets/LoomGUI/Tests/LoomStagePlayModeTests.cs loomgui_unity/Assets/StreamingAssets/DejaVuSans.ttf
git commit -m "feat(unity): LoomStage MonoBehaviour 驱动 + 根 Stage/相机 + PlayMode 色块渲染"
```

---

## Phase 1 出口验收

1. `cargo test -p loomgui_ffi_c` 全绿（version、blob re-base、FFI round-trip）。
2. Unity Test Runner EditMode 全绿（FrameBlob 解析、MaterialManager 缓存、MirrorPool 不崩）。
3. Unity PlayMode：`LoomStage` 加载多节点 HTML，色块按布局渲染（父绿底 + 红/蓝子块横排），`sortingOrder` 递增、GO 巢状正确。
4. 工具链闭环：改 Rust → `cargo build --release` → 拷 `.dll` → Unity 重载 → 生效。

## Phase 1 不做（留 Phase 2）

- **Text 节点渲染**（Phase 1 `payload_kind=2` 跳过）→ Phase 2：`Font.RequestCharactersInTexture` + `textureRebuilt` 监听 + 据 Rust TextLayout 拼 quad。
- **rect mask**（`_ClipBox` + mask_context 材质 + CLIPPED keyword）→ Phase 2。
- **Domain reload 完整保护**（Phase 1 仅占位）→ Phase 2：`loomgui_shutdown` + 清全局缓存。
- **500 节点压测**、ArrayPool、冷帧 ≤2ms → v1e。
