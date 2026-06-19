using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// 渲染树 → GameObject 镜像 diff（§14.6）。每帧 O(n)：标 stale → 遍历命中清 stale/更新 → 余销毁。
    /// GO 按 parent_id 巢状；localPosition=(local_x,local_y)；sortingOrder=sort_key。
    /// Mesh 顶点已由 Rust（Task 2 blob.rs）re-base 到节点本地空间，此处按 (x,y,0) 上传。
    /// Phase 1：只渲染 payload_kind=1（Mesh）；Text(2)/Unchanged(0) 跳过。
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

        /// 当前镜像中的 GO 数量（=pool 中 node_id 数）。测试/调试用。
        public int Count => _pool.Count;

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
            foreach (var id in dead) { TearDown(_pool[id]); _pool.Remove(id); }
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
            ro.Mesh.Clear();                 // Unity 要求 SetVertices 前清空，否则顶点数变更报错
            ro.Mesh.SetVertices(verts);
            ro.Mesh.SetUVs(0, seg.Uvs);
            ro.Mesh.SetColors(cols);
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
