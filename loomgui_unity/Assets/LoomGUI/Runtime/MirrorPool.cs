using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// 渲染树 → GameObject 镜像 diff（§14.6）。每帧 O(n)：标 stale → 遍历命中清 stale/更新 → 余销毁。
    /// flatten：所有 GO 挂 root（§4.2）；纯平移节点 localPosition=(Mtx,Mty) 绝对 design；非纯平移节点
    /// GO transform=identity + _ObjectMatrix uniform；sortingOrder=sort_key。
    /// parent_id 仍在 blob 列但 Phase 2 渲染不用（v1c 事件再用）。
    /// Mesh 顶点已由 Rust（blob.rs）re-base 到节点本地空间，此处按 (x,y,0) 上传。
    /// Phase 2：渲染 payload_kind=1（Mesh）+ 2（Text）；Unchanged(0) 跳过。
    sealed class RenderObj
    {
        public GameObject Go;
        public MeshFilter Mf;
        public MeshRenderer Mr;
        public Mesh Mesh;
        public bool Stale;
        public uint LastNodeId;       // 复用 GO 时校验
        public bool IsText;            // kind=2：font atlas rebuild 时需重光栅
        // -1 哨兵：新建 RenderObj 的 text 节点首帧必 BuildMesh（即使 FontVersion==0）。
        public int LastFontVersion = -1;

        // §4.5 buffer 复用（500 节点静态压测 GC 缓解）：每 RenderObj 持可复用 List，
        // UploadMesh 每帧 Clear+fill 后用 Mesh.SetVertices(List) 等 overload 上传——
        // List<T>.Clear() 保留 Capacity，故 warm-up 后零 per-frame 数组 alloc。
        // 替代 naive `new Vector3[]/Color[]/int[]`（Phase 1 review Minor：500 节点×3 数组/帧 GC）。
        // 全量 ArrayPool（含 ReadMesh 数组 alloc）冷帧零 GC 留 v1e。
        public readonly List<Vector3> VList = new();
        public readonly List<Vector2> UvList = new();
        public readonly List<Color> CList = new();
        public readonly List<int> IList = new();
    }

    public sealed class MirrorPool
    {
        readonly Dictionary<uint, RenderObj> _pool = new();
        int _lastFontVersion = -1;     // -1 → 首帧必不等，强制建/光栅；之后追 TextRasterizer.FontVersion
        // §4.4：每 ctx 每帧首次（fgui firstMaterialInFrame 模式）算一次 _ClipBox 并 SetClipBox。
        // Sync 开头清空；clip 表 entry 少（few ctx），每帧开销可忽略。
        readonly HashSet<uint> _clipsAppliedThisFrame = new();

        /// 当前镜像中的 GO 数量（=pool 中 node_id 数）。测试/调试用。
        public int Count => _pool.Count;

        public void Sync(FrameBlob blob, Transform root, MaterialManager mm,
                         Dictionary<uint, Texture2D> texMap, Texture fallback, Font font)
        {
            // 防御：陈旧/非 v4 blob 直接早退（§4.1 magic+version 校验）。不做清理——上一帧的 GO
            // 维持不动比误销毁更安全；调用方应自检 IsValid 再 Sync。
            if (!blob.IsValid) return;

            // font atlas rebuild 检测（§4.3 必修坑）：版本变 → 本帧所有 text 节点强制重 BuildMesh
            // （glyph UV 变，缓存 mesh 作废）。照 fgui Stage.cs:828 重跑帧语义。
            bool fontDirty = _lastFontVersion != TextRasterizer.FontVersion;

            // ① 全标 stale
            foreach (var kv in _pool) kv.Value.Stale = true;
            // §4.4：本帧 clip 应用集清空（per-ctx-per-frame 一次性算 _ClipBox）。
            _clipsAppliedThisFrame.Clear();

            // ② 遍历节点
            int n = blob.NodeCount;
            for (int i = 0; i < n; i++)
            {
                if (!blob.Visible(i)) continue;
                byte kind = blob.PayloadKind(i);
                if (kind != 1 && kind != 2) continue;  // Mesh(1)/Text(2)；Unchanged(0) 跳过

                uint id = blob.NodeId(i);
                if (!_pool.TryGetValue(id, out var ro))
                {
                    ro = NewRenderObj(root);
                    ro.LastNodeId = id;
                    _pool[id] = ro;
                }
                ro.Stale = false;
                ro.IsText = kind == 2;

                // flatten（§4.2）：所有节点挂 root（避免巢状双计父）。
                // v4：world matrix Affine2 双路径——
                //   纯平移（IsPureTranslation）：GO localPosition=(Mtx,Mty) + identity rotation/scale。
                //   非纯平移：GO transform=identity，blob matrix 进 _ObjectMatrix uniform（shader × matrix）。
                ro.Go.transform.SetParent(root, false);
                bool pure = blob.IsPureTranslation(i);
                if (pure)
                {
                    ro.Go.transform.localPosition = new Vector3(blob.Mtx(i), blob.Mty(i), 0f);
                    ro.Go.transform.localRotation = Quaternion.identity;
                    ro.Go.transform.localScale = Vector3.one;
                }
                else
                {
                    ro.Go.transform.localPosition = Vector3.zero;
                    ro.Go.transform.localRotation = Quaternion.identity;
                    ro.Go.transform.localScale = Vector3.one;
                }

                ro.Mr.sortingOrder = (int)blob.SortKey(i);

                uint maskCtx = blob.MaskContext(i);
                float nodeAlpha = blob.Alpha(i);

                // §4.4：mc>0 节点本帧首次见 → 读 clip 表 design rect，转 world，算 _ClipBox，
                // SetClipBox 到该 ctx 的 per-context Material（fgui firstMaterialInFrame）。
                // 必须在 mm.Get 之前调，使新建 Material 时即带 box；同 ctx 后续节点跳过（HashSet 去重）。
                if (maskCtx > 0u && _clipsAppliedThisFrame.Add(maskCtx))
                {
                    if (blob.ClipRect(maskCtx, out float dx, out float dy, out float dw, out float dh))
                    {
                        Vector4 clipBox = ClipMath.ComputeClipBox(root, dx, dy, dw, dh);
                        mm.SetClipBox(maskCtx, clipBox);
                    }
                    // ClipRect miss（表里无该 ctx）→ 不 SetClipBox；material 仍按 mc 建（CLIPPED variant
                    // + 默认 _ClipBox=0,0,1,1 → 全保留，clip 无效但不崩；正常 flow 表必含所有 mc>0 ctx）。
                }

                if (kind == 1)
                {
                    // mesh 上传（Rust 已 re-base 顶点到本地）。
                    var seg = blob.ReadMesh(i);
                    UploadMesh(ro, seg);
                    ro.Mesh.RecalculateBounds();
                    ro.LastFontVersion = TextRasterizer.FontVersion;
                    // v1b.2：按 tex_id 从 texMap 绑真纹理；0/缺失 → fallback（白占位）。
                    uint tid = blob.TexId(i);
                    Texture tex = (tid != 0 && texMap.TryGetValue(tid, out var t)) ? (Texture)t : fallback;
                    var mat = mm.Get(program: 0, tex, maskCtx, !pure);
                    if (!pure)
                    {
                        var m = Matrix4x4.identity;
                        m[0, 0] = blob.Ma(i); m[0, 1] = blob.Mc(i); m[0, 3] = blob.Mtx(i);
                        m[1, 0] = blob.Mb(i); m[1, 1] = blob.Md(i); m[1, 3] = blob.Mty(i);
                        mat.SetMatrix("_ObjectMatrix", m);
                    }
                    ro.Mr.sharedMaterial = mat;
                }
                else  // kind == 2 (Text)
                {
                    // font atlas rebuild 或首次 → 重 BuildMesh（glyph UV 变，旧 mesh 作废）。
                    bool needRebuild = fontDirty || ro.LastFontVersion != TextRasterizer.FontVersion;
                    if (needRebuild)
                    {
                        blob.ReadText(i, out int fontSize, out Color textColor, out GlyphData[] glyphs);
                        var seg = TextRasterizer.BuildMesh(font, fontSize, textColor, nodeAlpha, glyphs);
                        UploadMesh(ro, seg);
                        ro.Mesh.RecalculateBounds();
                        ro.LastFontVersion = TextRasterizer.FontVersion;
                    }
                    // text program=1，texture=font atlas。font.material.mainTexture（atlas rebuild 后引用更新）。
                    // font 可能为 null（caller 未注入）→ 跳材质以免 NRE；测试用 BuildMesh 直接验。
                    if (font != null)
                    {
                        var tmat = mm.Get(program: 1, font.material.mainTexture, maskCtx, !pure);
                        if (!pure)
                        {
                            var m = Matrix4x4.identity;
                            m[0, 0] = blob.Ma(i); m[0, 1] = blob.Mc(i); m[0, 3] = blob.Mtx(i);
                            m[1, 0] = blob.Mb(i); m[1, 1] = blob.Md(i); m[1, 3] = blob.Mty(i);
                            tmat.SetMatrix("_ObjectMatrix", m);
                        }
                        ro.Mr.sharedMaterial = tmat;
                    }
                }
            }

            if (fontDirty) _lastFontVersion = TextRasterizer.FontVersion;

            // ③ 余 stale 销毁
            var dead = new List<uint>();
            foreach (var kv in _pool) if (kv.Value.Stale) dead.Add(kv.Key);
            foreach (var id in dead) { TearDown(_pool[id]); _pool.Remove(id); }
        }

        static RenderObj NewRenderObj(Transform root)
        {
            var go = new GameObject("loom_node");
            // ExecuteAlways 下镜像 GO 是运行时派生产物，标 DontSaveInEditor 防被存进场景
            // （否则 EditMode Sync 产出的 GO 会 dirty 场景、Play/Stop 与 domain reload 累积残留）。
            go.hideFlags = HideFlags.DontSaveInEditor;
            go.transform.SetParent(root, false);
            go.layer = root.gameObject.layer;  // LoomUI
            var mf = go.AddComponent<MeshFilter>();
            var mr = go.AddComponent<MeshRenderer>();
            var mesh = new Mesh { indexFormat = UnityEngine.Rendering.IndexFormat.UInt32 };
            mesh.hideFlags = HideFlags.DontSaveInEditor;  // Mesh 是独立 Object，也别存盘
            mesh.MarkDynamic();
            mf.sharedMesh = mesh;
            return new RenderObj { Go = go, Mf = mf, Mr = mr, Mesh = mesh };
        }

        /// §4.5 buffer 复用：从 MeshSegment 填 ro 持有的可复用 List，再走 SetVertices(List) 等 overload。
        /// List<T>.Clear() 保留 Capacity → warm-up 后每帧零数组 alloc（naive 每帧 new Vector3[]/Color[]/int[]
        /// 在 500 节点 ×3 数组/帧 是主要 GC 源）。kind=1（mesh）与 kind=2（text BuildMesh 产出）同走此路径。
        /// 注意：SetVertices(List) 要求 list 长度 == 顶点数；Clear()+Add 精确填到 Verts.Length 即满足。
        static void UploadMesh(RenderObj ro, MeshSegment seg)
        {
            int vc = seg.Verts.Length;
            // Clear 保留 capacity，再填（避免每帧 new List / new 数组）。
            var v = ro.VList; v.Clear();
            var uv = ro.UvList; uv.Clear();
            var c = ro.CList; c.Clear();
            var idx = ro.IList; idx.Clear();
            // 预扩一次（首次或更大 mesh 时）；后续 Clear 不收缩，零 alloc。
            if (v.Capacity < vc) { v.Capacity = vc; uv.Capacity = vc; c.Capacity = vc; }
            int ic = seg.Idx.Length;
            if (idx.Capacity < ic) idx.Capacity = ic;

            for (int i = 0; i < vc; i++)
            {
                v.Add(new Vector3(seg.Verts[i].x, seg.Verts[i].y, 0f));
                uv.Add(seg.Uvs[i]);
                c.Add(seg.Colors[i]);
            }
            for (int i = 0; i < ic; i++) idx.Add((int)seg.Idx[i]);

            ro.Mesh.Clear();                 // Unity 要求 SetVertices 前清空，否则顶点数变更报错
            ro.Mesh.SetVertices(v);
            ro.Mesh.SetUVs(0, uv);
            ro.Mesh.SetColors(c);
            ro.Mesh.SetTriangles(idx, 0);
        }

        public void Clear()
        {
            foreach (var kv in _pool) TearDown(kv.Value);
            _pool.Clear();
        }

        // Edit-mode-safe 销毁：T8 LoomStage 挂 [ExecuteAlways]，Sync/Clear 会在 Edit mode 跑；
        // Object.Destroy 在 Edit mode 非法（须 DestroyImmediate）。
        static void TearDown(RenderObj ro)
        {
            DestroyObj(ro.Mesh);   // new Mesh() 是独立 UnityEngine.Object，须显式销毁，否则泄漏
            DestroyObj(ro.Go);
        }

        static void DestroyObj(Object o)
        {
            if (o == null) return;
            if (Application.isPlaying) Object.Destroy(o);
            else Object.DestroyImmediate(o);
        }
    }
}
