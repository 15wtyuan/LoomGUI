using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// 渲染树 → GameObject 镜像 diff。每帧 O(n)：标 stale → 遍历命中清 stale/更新 → 余销毁。
    /// flatten：所有 GO 挂 root；纯平移节点 localPosition=(Mtx,Mty) 绝对 design；非纯平移节点
    /// GO transform=identity + _ObjectMatrix uniform；sortingOrder=sort_key。
    /// parent_id 仍在 blob 列但渲染不用（事件系统再用）。
    /// Mesh 顶点已由 Rust re-base 到节点本地空间，此处按 (x,y,0) 上传。
    /// 渲染 payload_kind=1（Mesh）+ 2（Text）；Unchanged(0) 跳过。
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

        // buffer 复用（500 节点静态压测 GC 缓解）：每 RenderObj 持可复用 List，
        // UploadMesh 每帧 Clear+fill 后用 Mesh.SetVertices(List) 等 overload 上传——
        // List<T>.Clear() 保留 Capacity，故 warm-up 后零 per-frame 数组 alloc。
        public readonly List<Vector3> VList = new();
        public readonly List<Vector2> UvList = new();
        public readonly List<Color> CList = new();
        public readonly List<int> IList = new();
        // cached MaterialPropertyBlock for per-renderer _ObjectMatrix.
        // Lazy-init in non-pure-translation path; avoids shared material overwrite.
        public MaterialPropertyBlock Mpb;
    }

    public sealed class MirrorPool
    {
        readonly Dictionary<uint, RenderObj> _pool = new();
        int _lastFontVersion = -1;     // -1 → 首帧必不等，强制建/光栅；之后追 TextRasterizer.FontVersion
        // 每 ctx 每帧首次算一次 _ClipBox 并 SetClipBox。
        // Sync 开头清空；clip 表 entry 少（few ctx），每帧开销可忽略。
        readonly HashSet<uint> _clipsAppliedThisFrame = new();

        /// 当前镜像中的 GO 数量（=pool 中 node_id 数）。测试/调试用。
        public int Count => _pool.Count;

        public void Sync(FrameBlob blob, Transform root, MaterialManager mm,
                         SpriteResolver sprites, Texture fallback, Font font)
        {
            // 防御：陈旧/非当前 blob 直接早退（magic+version 校验）。不做清理——上一帧的 GO
            // 维持不动比误销毁更安全；调用方应自检 IsValid 再 Sync。
            if (!blob.IsValid) return;

            // font atlas rebuild 检测：版本变 → 本帧所有 text 节点强制重 BuildMesh
            // （glyph UV 变，缓存 mesh 作废）。
            bool fontDirty = _lastFontVersion != TextRasterizer.FontVersion;

            // ① 全标 stale
            foreach (var kv in _pool) kv.Value.Stale = true;
            // 本帧 clip 应用集清空（per-ctx-per-frame 一次性算 _ClipBox）。
            _clipsAppliedThisFrame.Clear();

            // ② 遍历节点
            int n = blob.NodeCount;
            for (int i = 0; i < n; i++)
            {
                if (!blob.Visible(i)) continue;
                byte kind = blob.PayloadKind(i);
                // Unchanged(0) = 静态帧节点。dirty 保证此刻 world/alpha/sort/mask/payload
                // 全不变 → 清 stale 保留上帧 GO、跳过上传。
                if (kind == 0)
                {
                    if (_pool.TryGetValue(blob.NodeId(i), out var unchangedRo))
                        unchangedRo.Stale = false;
                    continue;
                }
                if (kind != 1 && kind != 2) continue;  // 未知 kind 防御跳过

                uint id = blob.NodeId(i);
                if (!_pool.TryGetValue(id, out var ro))
                {
                    ro = NewRenderObj(root);
                    ro.LastNodeId = id;
                    _pool[id] = ro;
                }
                ro.Stale = false;
                ro.IsText = kind == 2;

                // flatten：所有节点挂 root。
                // pure 和非 pure 统一 GO localPosition=(Mtx,Mty)（world translate 进 GO transform）。
                // 非纯平移的 scale/rotate 进 _ObjectMatrix（无 translate）。这样 renderer.bounds = GO.worldTransform ×
                // Mesh.bounds 自动 world（culling 正确），不需 mutate Mesh.bounds 做 translate hack（mutate mesh 资产，
                // 非 pure→pure 切回时 bounds 双 translate → frustum culling 误剔 → 字消失）。
                ro.Go.transform.SetParent(root, false);
                bool pure = blob.IsPureTranslation(i);
                ro.Go.transform.localPosition = new Vector3(blob.Mtx(i), blob.Mty(i), 0f);
                ro.Go.transform.localRotation = Quaternion.identity;
                ro.Go.transform.localScale = Vector3.one;

                ro.Mr.sortingOrder = (int)blob.SortKey(i);

                uint maskCtx = blob.MaskContext(i);
                float nodeAlpha = blob.Alpha(i);

                // mc>0 节点本帧首次见 → 读 clip 表 design rect，转 world，算 _ClipBox，
                // SetClipBox 到该 ctx 的 per-context Material。
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
                    // mesh 上传（顶点已 re-base 到本地）。
                    var seg = blob.ReadMesh(i);
                    UploadMesh(ro, seg);
                    ro.Mesh.RecalculateBounds();
                    ro.LastFontVersion = TextRasterizer.FontVersion;
                    // v1.4-a T8：按 path_idx 取 path → SpriteResolver.GetSprite → Sprite.texture + 打包 UV。
                    //   path_idx=0（纯色无图）/ path 查不到 Sprite → fallback（whiteTexture）。
                    //   blob mesh UV 是全图 [0,1]（T6 后核心不知图集，写全图 UV）；
                    //   SpriteAtlas 把 Sprite 打进 atlas 子区 → 用 sprite.rect + texture 尺寸重映射 UV
                    //   到 atlas 子区（保 blob 的 v 翻转：blob TL.v=1 → atlas 顶 rv1）。
                    uint pathIdx = blob.PathIdx(i);
                    Sprite sp = null;
                    Texture tex = fallback;
                    if (pathIdx != 0 && sprites != null)
                    {
                        string path = blob.ReadPath(pathIdx);
                        if (!string.IsNullOrEmpty(path))
                        {
                            sp = sprites.GetSprite(path);
                            if (sp != null) tex = sp.texture;
                        }
                    }
                    // Sprite 命中 → 把 mesh UV 重映射到 Sprite 在 atlas 内的子区（packed UV）。
                    //   blob UV 全 [0,1]（T6）；Sprite.rect 是 atlas 内像素矩形（Unity y-up）。
                    //   packed_u = ru0 + blob_u*(ru1-ru0)；packed_v = rv0 + blob_v*(rv1-rv0)。
                    //   blob 的 v 已翻转（TL.v=1→atlas 顶 rv1），线性重映射保翻转。
                    //   九宫格切片 mesh UV 同基于 [0,1] → 同公式正确（slice 比例由 Rust 算进 blob UV）。
                    if (sp != null && sp.texture != null)
                    {
                        RemapMeshUvToSprite(ro, sp, sp.texture);
                    }
                    // program 来自 blob（v5 第 19 列）：0=img/无图 Container，2=Container+bg-image（CSS 合成，坑 79）。
                    var mat = mm.Get((int)blob.Program(i), tex, maskCtx, !pure);
                    if (!pure)
                    {
                        // _ObjectMatrix 只 scale/rotate（translate 进 GO localPosition，renderer.bounds 自动 world）。
                        var m = Matrix4x4.identity;
                        m[0, 0] = blob.Ma(i); m[0, 1] = blob.Mc(i);
                        m[1, 0] = blob.Mb(i); m[1, 1] = blob.Md(i);
                        SetObjectMatrix(ro, m);
                    }
                    // v1.3 ColorFilter（program=3=filter 无图 / 4=filter+bg-image 双 keyword）：
                    // 矩阵 20 float 拆 5 Vector MPB SetVector。漏 program=4 → cf-demo 滤镜不生效（全青色，验收坑）。
                    if (blob.Program(i) == 3 || blob.Program(i) == 4)
                    {
                        SetColorFilterMatrix(ro, blob.ColorMatrix(i));
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
                            // _ObjectMatrix 只 scale/rotate（translate 进 GO localPosition）。
                            var m = Matrix4x4.identity;
                            m[0, 0] = blob.Ma(i); m[0, 1] = blob.Mc(i);
                            m[1, 0] = blob.Mb(i); m[1, 1] = blob.Md(i);
                            SetObjectMatrix(ro, m);
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

        /// buffer 复用：从 MeshSegment 填 ro 持有的可复用 List，再走 SetVertices(List) 等 overload。
        /// List<T>.Clear() 保留 Capacity → warm-up 后每帧零数组 alloc。kind=1（mesh）与 kind=2（text BuildMesh 产出）同走此路径。
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

        /// v1.4-a T8：把 mesh UV（blob 写全图 [0,1]，T6 后核心不知图集）重映射到 Sprite 在 atlas 内的子区。
        /// SpriteAtlas 把 Sprite 打进 atlas 纹理子区 → 需用 sprite.rect + texture 尺寸算 packed UV。
        ///   packed_u = ru0 + blob_u*(ru1-ru0)；packed_v = rv0 + blob_v*(rv1-rv0)。
        /// blob UV 已 v 翻转（TL.v=1 → atlas 顶 rv1），线性重映射保翻转不二次翻转。
        /// 九宫格切片同基于 [0,1] blob UV → 同公式（slice 比例由 Rust 算进 blob UV）。
        ///
        /// 直接改 ro.Mesh 的 UV（SetUVs 后 in-place 重写）——避免再 SetUVs 一次（Mesh 已持数据）。
        /// 用 Mesh.GetUVs 读回 List，原地改，SetUVs 写回（比重建 List 省 alloc——但每帧 image 节点少，简单优先）。
        static void RemapMeshUvToSprite(RenderObj ro, Sprite sp, Texture2D tex)
        {
            if (sp == null || tex == null) return;
            float tw = tex.width;
            float th = tex.height;
            if (tw <= 0f || th <= 0f) return;
            var r = sp.rect;
            float ru0 = r.xMin / tw, ru1 = r.xMax / tw;
            float rv0 = r.yMin / th, rv1 = r.yMax / th;
            float du = ru1 - ru0, dv = rv1 - rv0;

            var uvs = new List<Vector2>();
            ro.Mesh.GetUVs(0, uvs);
            for (int i = 0; i < uvs.Count; i++)
            {
                uvs[i] = new Vector2(ru0 + uvs[i].x * du, rv0 + uvs[i].y * dv);
            }
            ro.Mesh.SetUVs(0, uvs);
        }

        /// _ObjectMatrix 经 MPB 传 shader。SetMatrix 对 CBUFFER 内非 Properties 字段不生效（MPB 只覆盖
        /// material property）→ 拆 4 Vector（Properties 对应）SetVector 覆盖，shader vert 重组 float4x4。
        /// HLSL float4x4(v0..v3) 是 row-major 构造，故传 m.GetRow(0..3)（行），不是 GetColumn。
        /// m 只含 scale/rotate（translate 进 GO localPosition=(Mtx,Mty)）。mul(objM, v).xy = (Ma·x+Mc·y, Mb·x+Md·y)；
        /// worldPos = TransformObjectToWorld(designWorld) = root × GO × designWorld（GO.position 提供 translate）。
        static void SetObjectMatrix(RenderObj ro, in Matrix4x4 m)
        {
            ro.Mpb ??= new MaterialPropertyBlock();
            ro.Mpb.SetVector("_ObjM0", m.GetRow(0));
            ro.Mpb.SetVector("_ObjM1", m.GetRow(1));
            ro.Mpb.SetVector("_ObjM2", m.GetRow(2));
            ro.Mpb.SetVector("_ObjM3", m.GetRow(3));
            ro.Mr.SetPropertyBlock(ro.Mpb);
        }

        /// ColorFilter 矩阵（20 float）拆 5 Vector MPB SetVector：_CF0..3（矩阵行）+ _CFOff（offset）。
        /// 照搬 fgui UpdateMatrix（MPB per-renderer 覆盖，不拆 Material）。
        static void SetColorFilterMatrix(RenderObj ro, float[] m)
        {
            ro.Mpb ??= new MaterialPropertyBlock();
            ro.Mpb.SetVector("_CF0", new Vector4(m[0],  m[1],  m[2],  m[3]));
            ro.Mpb.SetVector("_CF1", new Vector4(m[5],  m[6],  m[7],  m[8]));
            ro.Mpb.SetVector("_CF2", new Vector4(m[10], m[11], m[12], m[13]));
            ro.Mpb.SetVector("_CF3", new Vector4(m[15], m[16], m[17], m[18]));
            ro.Mpb.SetVector("_CFOff", new Vector4(m[4], m[9], m[14], m[19]));
            ro.Mr.SetPropertyBlock(ro.Mpb);
        }

        public void Clear()
        {
            foreach (var kv in _pool) TearDown(kv.Value);
            _pool.Clear();
        }

        // Edit-mode-safe 销毁：LoomStage 挂 [ExecuteAlways]，Sync/Clear 会在 Edit mode 跑；
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
