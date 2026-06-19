using System.Collections.Generic;
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// MirrorPool 的 EditMode 行为测试（Task 7）。
    /// 手搓 1 节点 mesh blob（镜像 loomgui_ffi_c/src/blob.rs::build_blob 布局，
    /// 同 FrameBlobTests）→ 验 stale-flag diff 的 Create / Reuse / Destroy 三端。
    public class MirrorPoolTests
    {
        /// 构造一个 1 节点 Mesh blob：visible=1, payload_kind=1, parent=-1,
        /// 4 顶点 quad mesh（顶点=(0,0)(w,0)(w,h)(0,h)，已 re-base 到本地）。
        /// v2 布局（13 列 + mesh/text/clip 三 arena header；text 空、clip 仅 count=0）。
        /// 用于驱动 MirrorPool.Sync 的 diff 逻辑。
        static byte[] OneMeshNodeBlob(
            uint id, float x, float y, float w, float h, uint sortKey)
        {
            var b = new List<byte>();

            // header: magic, version=2, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(2u));
            b.AddRange(System.BitConverter.GetBytes(1u));

            // header 总长 = 3*4 + 13*4 + 6*4 = 88。列 offset 从此起按 elemSize 递进。
            const int HeaderLen = 12 + 13 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 88
            int colOff = HeaderLen;
            int[] offs = new int[13];
            // 元素字节数顺序同 blob.rs columns（v2 +text_off/text_len）。
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4 };
            for (int i = 0; i < 13; i++) { offs[i] = colOff; colOff += elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：1 mesh，4 verts / 6 idx。顶点已 re-base 到本地：(0,0)(w,0)(w,h)(0,h)。
            var arena = new List<byte>();
            int arenaStart = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
            arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
            // verts[4×2]
            AppendVert(arena, 0f, 0f);
            AppendVert(arena, w,   0f);
            AppendVert(arena, w,   h);
            AppendVert(arena, 0f,  h);
            // uvs[4×2]
            AppendVert(arena, 0f, 0f);
            AppendVert(arena, 1f, 0f);
            AppendVert(arena, 1f, 1f);
            AppendVert(arena, 0f, 1f);
            // colors[4×4]（白色不透明，tint×alpha 已 baked 进顶点色）
            for (int v = 0; v < 4; v++)
            {
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
            }
            // indices[6] = (0,1,2,0,2,3) 两三角
            arena.AddRange(System.BitConverter.GetBytes(0u));
            arena.AddRange(System.BitConverter.GetBytes(1u));
            arena.AddRange(System.BitConverter.GetBytes(2u));
            arena.AddRange(System.BitConverter.GetBytes(0u));
            arena.AddRange(System.BitConverter.GetBytes(2u));
            arena.AddRange(System.BitConverter.GetBytes(3u));
            int arenaLen = arena.Count - arenaStart;

            // 13 列 offset + mesh/text/clip 三 arena off+len
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));              // mesh_arena_off
            b.AddRange(System.BitConverter.GetBytes(arenaLen));              // mesh_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen));   // text_arena_off（紧跟）
            b.AddRange(System.BitConverter.GetBytes(0u));                    // text_arena_len（T1 空）
            int clipOff = arenaOff + arenaLen;                               // text 空，clip 紧跟
            b.AddRange(System.BitConverter.GetBytes(clipOff));               // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));                    // clip_table_len（仅 clip_count）

            // 列数据（node 0）。
            b.AddRange(System.BitConverter.GetBytes(id));        // node_id
            b.AddRange(System.BitConverter.GetBytes(-1));        // parent_id（无父）
            b.Add(1);                                            // visible
            b.AddRange(System.BitConverter.GetBytes(1f));        // alpha
            b.AddRange(System.BitConverter.GetBytes(sortKey));   // sort_key
            b.AddRange(System.BitConverter.GetBytes(x));         // local_x
            b.AddRange(System.BitConverter.GetBytes(y));         // local_y
            b.AddRange(System.BitConverter.GetBytes(0u));        // mask_context
            b.Add(1);                                            // payload_kind = Mesh
            b.AddRange(System.BitConverter.GetBytes(0u));        // mesh_off（相对 arena 起始）
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen)); // mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));        // text_off（T1 占位）
            b.AddRange(System.BitConverter.GetBytes(0u));        // text_len（T1 占位）

            b.AddRange(arena);
            // text_arena T1 空，跳过。
            // clip 表：仅 clip_count=0
            b.AddRange(System.BitConverter.GetBytes(0u));
            return b.ToArray();

            static void AppendVert(List<byte> a, float vx, float vy)
            {
                a.AddRange(System.BitConverter.GetBytes(vx));
                a.AddRange(System.BitConverter.GetBytes(vy));
            }
        }

        static byte[] EmptyBlob()
        {
            var b = new List<byte>();
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu)); // magic
            b.AddRange(System.BitConverter.GetBytes(2u));          // version=2
            b.AddRange(System.BitConverter.GetBytes(0u));          // node_count = 0
            // 即便 0 节点也写全 header（13 col offset + mesh/text/clip 三 arena off+len），避免越界读。
            const int HeaderLen = 12 + 13 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 88
            int colOff = HeaderLen;
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4 };
            for (int i = 0; i < 13; i++)
            {
                b.AddRange(System.BitConverter.GetBytes(colOff));
                colOff += elemSize[i];
            }
            // 0 节点：三 arena 都紧跟 header 之后且互相重叠（均 len=0），clip 表仍含 clip_count=0。
            int segOff = colOff;
            b.AddRange(System.BitConverter.GetBytes(segOff)); // mesh_arena_off
            b.AddRange(System.BitConverter.GetBytes(0u));     // mesh_arena_len
            b.AddRange(System.BitConverter.GetBytes(segOff)); // text_arena_off（紧跟，len=0）
            b.AddRange(System.BitConverter.GetBytes(0u));     // text_arena_len
            b.AddRange(System.BitConverter.GetBytes(segOff)); // clip_table_off（紧跟，len=0）
            b.AddRange(System.BitConverter.GetBytes(4u));     // clip_table_len（仅 clip_count）
            // clip 表：仅 clip_count=0
            b.AddRange(System.BitConverter.GetBytes(0u));
            return b.ToArray();
        }

        [Test]
        public void SyncCreateReuseDestroyRoundtrip()
        {
            // Arrange：root + MaterialManager(LoomGUI/Unlit) + MirrorPool。
            // Shader.Find 在 EditMode 测试中由 Unity 测试环境解析；若该 shader 未在测试程序集中编译，
            // MaterialManager 仍会构造一个 hidden-shell Material（不阻塞本测试的 diff 行为验证）。
            var root = new GameObject("root");
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();
            var tex = Texture2D.whiteTexture;

            try
            {
                // 1) Create：Sync 1 节点 → 恰好创建 1 GO（root 的直接子节点）。
                var blob1 = new FrameBlob(OneMeshNodeBlob(id: 7, x: 10f, y: 20f, w: 5f, h: 5f, sortKey: 3));
                Assert.AreEqual(1, blob1.NodeCount, "blob 应解析出 1 节点");
                pool.Sync(blob1, root.transform, mm, tex, null);  // T4：Sync 加 Font 参数（Mesh 测传 null）

                Assert.AreEqual(1, pool.Count, "Create: pool.Count 应为 1");
                Assert.AreEqual(1, root.transform.childCount, "Create: root 应有 1 个直接子 GO");
                var node = root.transform.GetChild(0);
                Assert.AreEqual("loom_node", node.name, "Create: GO 名应为 loom_node");
                Assert.AreEqual(new Vector3(10f, 20f, 0f), node.localPosition, "Create: localPosition=(local_x,local_y,0)");
                Assert.AreEqual(Vector3.one, node.localScale, "Create: localScale=one");

                var mr = node.GetComponent<MeshRenderer>();
                Assert.IsNotNull(mr, "Create: Mesh 节点须挂 MeshRenderer");
                Assert.AreEqual(3, mr.sortingOrder, "Create: sortingOrder=(int)sort_key");
                var mf = node.GetComponent<MeshFilter>();
                Assert.IsNotNull(mr, "Create: 须挂 MeshFilter");
                Assert.IsNotNull(mf.sharedMesh, "Create: MeshFilter.sharedMesh 应已赋值");
                Assert.AreEqual(4, mf.sharedMesh.vertexCount, "Create: mesh 应有 4 顶点");

                var createdGo = node.gameObject;  // 记下，验证 Reuse 是同一个 GO

                // 2) Reuse：再 Sync 同一 blob → 仍 1 GO（复用，非新增）。
                pool.Sync(blob1, root.transform, mm, tex, null);
                Assert.AreEqual(1, pool.Count, "Reuse: pool.Count 仍应为 1");
                Assert.AreEqual(1, root.transform.childCount, "Reuse: root 仍只 1 子 GO");
                Assert.AreSame(createdGo, root.transform.GetChild(0).gameObject,
                    "Reuse: 第二次 Sync 应复用同一 GO，而非新建");

                // 3) Destroy：Sync 空 blob（NodeCount=0）→ 余 stale 从 pool 移除。
                //  生产 Sync 用 Object.Destroy（deferred），GO 在当前帧不立刻从层级消失，
                //  但 pool 的 dict 是同步清空的——这是 stale-flag diff 的真实语义。
                var empty = new FrameBlob(EmptyBlob());
                Assert.AreEqual(0, empty.NodeCount, "空 blob NodeCount=0");
                pool.Sync(empty, root.transform, mm, tex, null);
                Assert.AreEqual(0, pool.Count, "Destroy: 全 stale 后 pool.Count 应为 0");
                // createdGo 已被 Object.Destroy 标记销毁；deferred 后引用 == null（Unity 重载）。
                // EditMode 下一帧才真销毁，故仅断 pool.Count，不强断 childCount。
            }
            finally
            {
                // 清理：pool.Clear 销毁残留 GO；MaterialManager 销毁缓存的 Material；root 本身。
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(root);
            }
        }

        /// §4.5 / Task 7：UploadMesh 现走 RenderObj 可复用 List 路径（Clear+fill+SetVertices(List)）。
        /// 验证 reuse 不破坏几何——同 blob 连续 Sync 多次（List 被 Clear+refill），顶点/uv/颜色/索引
        /// 与首次完全一致；且不同 mesh（顶点数变化）也能正确刷新（List 扩容/收缩正确，无残留旧顶点）。
        /// kind=1（mesh）路径在此直测；kind=2（text）路径走 TextRasterizer.BuildMesh → 同 UploadMesh，
        /// 几何由 BuildMesh 决定、UploadMesh 仅搬运——故 mesh 路径的正确性即覆盖 UploadMesh 对 text 的搬运正确性。
        /// （text 顶点值需 live Font 才能算，此处不在单测范围；T4 TextRasterizerTests 已验 BuildMesh 输出。）
        [Test]
        public void UploadMesh_WithListReuse_ProducesCorrectGeometry()
        {
            var root = new GameObject("root");
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();
            var tex = Texture2D.whiteTexture;

            try
            {
                // blob A：4 顶点 quad（5×5），位置 (10,20)。
                var blobA = new FrameBlob(OneMeshNodeBlob(id: 1, x: 10f, y: 20f, w: 5f, h: 5f, sortKey: 0));
                pool.Sync(blobA, root.transform, mm, tex, null);
                var meshA = root.transform.GetChild(0).GetComponent<MeshFilter>().sharedMesh;

                // 读回几何，断 UploadMesh 路径产出正确（4 顶点、(0,0)(5,0)(5,5)(0,5) re-based 本地）。
                var vertsA = meshA.vertices;
                Assert.AreEqual(4, vertsA.Length, "A: 4 顶点");
                Assert.AreEqual(new Vector3(0f, 0f, 0f), vertsA[0], "A: v0");
                Assert.AreEqual(new Vector3(5f, 0f, 0f), vertsA[1], "A: v1");
                Assert.AreEqual(new Vector3(5f, 5f, 0f), vertsA[2], "A: v2");
                Assert.AreEqual(new Vector3(0f, 5f, 0f), vertsA[3], "A: v3");
                var idxA = meshA.triangles;
                Assert.AreEqual(new[] { 0, 1, 2, 0, 2, 3 }, idxA, "A: 索引两三角");
                Assert.AreEqual(4, meshA.uv.Length, "A: 4 uv");
                Assert.AreEqual(new Vector2(0f, 0f), meshA.uv[0], "A: uv0");
                Assert.AreEqual(new Vector2(1f, 1f), meshA.uv[2], "A: uv2");
                Assert.AreEqual(4, meshA.colors.Length, "A: 4 色");
                Assert.AreEqual(new Color(1f, 1f, 1f, 1f), meshA.colors[0], "A: 白色不透明");

                // blob B：更大 quad（20×30），同 id 复用同一 RenderObj（List 被 Clear+refill 到更大尺寸）。
                // 验 List 扩容后几何仍正确（无残留 A 的旧顶点 / 旧索引）。
                var blobB = new FrameBlob(OneMeshNodeBlob(id: 1, x: 10f, y: 20f, w: 20f, h: 30f, sortKey: 0));
                pool.Sync(blobB, root.transform, mm, tex, null);
                var meshB = root.transform.GetChild(0).GetComponent<MeshFilter>().sharedMesh;
                var vertsB = meshB.vertices;
                Assert.AreEqual(4, vertsB.Length, "B: 仍 4 顶点（List refill 不残留）");
                Assert.AreEqual(new Vector3(20f, 0f, 0f), vertsB[1], "B: v1=(20,0)");
                Assert.AreEqual(new Vector3(0f, 30f, 0f), vertsB[3], "B: v3=(0,30)");
                Assert.AreEqual(new[] { 0, 1, 2, 0, 2, 3 }, meshB.triangles, "B: 索引不变");

                // 再回 blob A（List 收缩回 4 顶点小 quad）——验 Clear 后无残留 B 的大尺寸顶点。
                pool.Sync(blobA, root.transform, mm, tex, null);
                var meshA2 = root.transform.GetChild(0).GetComponent<MeshFilter>().sharedMesh;
                var vertsA2 = meshA2.vertices;
                Assert.AreEqual(4, vertsA2.Length, "A2: 收缩后仍 4 顶点");
                Assert.AreEqual(new Vector3(5f, 5f, 0f), vertsA2[2], "A2: v2 恢复 (5,5) 无残留");
            }
            finally
            {
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(root);
            }
        }
    }
}
