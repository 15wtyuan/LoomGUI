using System.Collections.Generic;
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// atlas 打包下同 atlas 多 sprite 共享 1 Texture2D + 各自子区 UV 烘焙。
    /// 手搓 v4 双 Mesh 节点 blob（同 tex_id=1，不同 uv 子区）+ mock texMap{1→atlasTex}，
    /// 断言两节点 Mr.sharedMaterial.mainTexture == atlasTex（同实例 → MaterialManager 命中同 key →
    /// 共享 Material → 可 batch）且各自 Mesh.uv 匹配其子区 4 角（per-vertex UV 由 blob 写，
    /// MirrorPool 直拷）。
    public class AtlasMirrorPoolTests
    {
        /// 构造 v4 blob：N=2 Mesh 节点，同 tex_id=1，不同 uv 子区。
        /// 布局镜像 FrameBlob.cs / MirrorPoolTexIdTests（header 108 / 18 列 SOA / mesh arena）。
        /// 关键差异（vs MirrorPoolTexIdTests 单节点）：
        ///   - node_count=2；每列段长 = 2 × elemSize[i]。
        ///   - mesh arena 含 2 个独立 segment（vert_count+idx_count+verts+uvs+colors+indices），
        ///     各节点 mesh_off 指向自身 segment 在 arena 内的字节偏移。
        ///   - tex_id 列两节点都写 1（同 atlas）。
        ///   - uvs 写各自子区 4 角（左上/右下对角定义子矩形 → quad 4 角按 TL-BR 展开）。
        static byte[] TwoMeshBlobSameAtlas(
            (uint id, uint texId, float u0, float v0, float u1, float v1) n0,
            (uint id, uint texId, float u0, float v0, float u1, float v1) n1)
        {
            const int N = 2;
            var b = new List<byte>();
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu)); // magic
            b.AddRange(System.BitConverter.GetBytes(4u));           // version=4
            b.AddRange(System.BitConverter.GetBytes((uint)N));      // node_count=2

            const int HeaderLen = 12 + 18 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 108
            // v4 18 列 elemSize（镜像 FrameBlob 列序）
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            int colOff = HeaderLen;
            int[] offs = new int[18];
            for (int i = 0; i < 18; i++) { offs[i] = colOff; colOff += N * elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：2 个独立 segment（每节点一个）。
            var arena = new List<byte>();
            // segment 0
            int seg0Off = arena.Count;
            AppendQuadSegment(arena, n0.u0, n0.v0, n0.u1, n0.v1);
            int seg0Len = arena.Count - seg0Off;
            // segment 1
            int seg1Off = arena.Count;
            AppendQuadSegment(arena, n1.u0, n1.v0, n1.u1, n1.v1);
            int seg1Len = arena.Count - seg1Off;
            int arenaLen = arena.Count;

            // 18 列 col_offset
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // text_arena_off
            b.AddRange(System.BitConverter.GetBytes(0u));                  // text_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));                  // clip_table_len（仅 clip_count u32）

            // 列数据（SOA：每列 N 个节点连续；纯平移 world matrix @ 各自位置）。
            // col 0: node_id (u32) × N
            b.AddRange(System.BitConverter.GetBytes(n0.id));
            b.AddRange(System.BitConverter.GetBytes(n1.id));
            // col 1: parent_id (i32) × N（-1 = 无父）
            b.AddRange(System.BitConverter.GetBytes(-1));
            b.AddRange(System.BitConverter.GetBytes(-1));
            // col 2: visible (u8) × N
            b.Add(1); b.Add(1);
            // col 3: alpha (f32) × N
            b.AddRange(System.BitConverter.GetBytes(1f)); b.AddRange(System.BitConverter.GetBytes(1f));
            // col 4: sort_key (u32) × N
            b.AddRange(System.BitConverter.GetBytes(0u)); b.AddRange(System.BitConverter.GetBytes(0u));
            // col 5: mask_context (u32) × N（0 = 无裁剪）
            b.AddRange(System.BitConverter.GetBytes(0u)); b.AddRange(System.BitConverter.GetBytes(0u));
            // col 6: m_a (f32) × N（identity = 1）
            b.AddRange(System.BitConverter.GetBytes(1f)); b.AddRange(System.BitConverter.GetBytes(1f));
            // col 7: m_b (f32) × N（identity = 0）
            b.AddRange(System.BitConverter.GetBytes(0f)); b.AddRange(System.BitConverter.GetBytes(0f));
            // col 8: m_c (f32) × N（identity = 0）
            b.AddRange(System.BitConverter.GetBytes(0f)); b.AddRange(System.BitConverter.GetBytes(0f));
            // col 9: m_d (f32) × N（identity = 1）
            b.AddRange(System.BitConverter.GetBytes(1f)); b.AddRange(System.BitConverter.GetBytes(1f));
            // col 10: m_tx (f32) × N（节点 0 在原点，节点 1 错开避免重叠）
            b.AddRange(System.BitConverter.GetBytes(0f)); b.AddRange(System.BitConverter.GetBytes(10f));
            // col 11: m_ty (f32) × N
            b.AddRange(System.BitConverter.GetBytes(0f)); b.AddRange(System.BitConverter.GetBytes(0f));
            // col 12: payload_kind (u8) × N（1 = Mesh）
            b.Add(1); b.Add(1);
            // col 13: mesh_off (u32) × N（arena 内字节偏移）
            b.AddRange(System.BitConverter.GetBytes((uint)seg0Off));
            b.AddRange(System.BitConverter.GetBytes((uint)seg1Off));
            // col 14: mesh_len (u32) × N
            b.AddRange(System.BitConverter.GetBytes((uint)seg0Len));
            b.AddRange(System.BitConverter.GetBytes((uint)seg1Len));
            // col 15: text_off (u32) × N（Mesh 节点无 text）
            b.AddRange(System.BitConverter.GetBytes(0u)); b.AddRange(System.BitConverter.GetBytes(0u));
            // col 16: text_len (u32) × N
            b.AddRange(System.BitConverter.GetBytes(0u)); b.AddRange(System.BitConverter.GetBytes(0u));
            // col 17: tex_id (u32) × N（两节点同 atlas → tex_id=1）
            b.AddRange(System.BitConverter.GetBytes(n0.texId));
            b.AddRange(System.BitConverter.GetBytes(n1.texId));

            // mesh arena
            b.AddRange(arena);
            // clip_table：clip_count(u32)=0
            b.AddRange(System.BitConverter.GetBytes(0u));
            return b.ToArray();
        }

        /// 追加 1 个 quad mesh segment 到 arena（4 顶点 + 6 索引）。
        static void AppendQuadSegment(List<byte> arena, float u0, float v0, float u1, float v1)
        {
            arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
            arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
            // verts（本地 quad，4 顶点，design y-down：TL→TR→BR→BL）
            foreach (var v in new[] { (0f, 0f), (5f, 0f), (5f, 5f), (0f, 5f) })
            {
                arena.AddRange(System.BitConverter.GetBytes(v.Item1));
                arena.AddRange(System.BitConverter.GetBytes(v.Item2));
            }
            // uvs（与 verts 同序 TL→TR→BR→BL：TL=(u0,v0) TR=(u1,v0) BR=(u1,v1) BL=(u0,v1)）。
            foreach (var uv in new[] { (u0, v0), (u1, v0), (u1, v1), (u0, v1) })
            {
                arena.AddRange(System.BitConverter.GetBytes(uv.Item1));
                arena.AddRange(System.BitConverter.GetBytes(uv.Item2));
            }
            // colors（白）
            for (int i = 0; i < 4; i++)
            {
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
            }
            // indices（逐个 GetBytes——BitConverter 无 uint[] 重载）
            arena.AddRange(System.BitConverter.GetBytes(0u));
            arena.AddRange(System.BitConverter.GetBytes(1u));
            arena.AddRange(System.BitConverter.GetBytes(2u));
            arena.AddRange(System.BitConverter.GetBytes(0u));
            arena.AddRange(System.BitConverter.GetBytes(2u));
            arena.AddRange(System.BitConverter.GetBytes(3u));
        }

        [Test]
        public void AtlasSprites_ShareMaterial_AndBakeRegionUV()
        {
            var root = new GameObject("root");
            root.transform.localScale = new Vector3(1f, -1f, 1f);
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();

            // mock atlas：1 张 64×64 atlas.png，tex_id=1。
            var atlasTex = new Texture2D(64, 64);
            var texMap = new Dictionary<uint, Texture2D> { { 1u, atlasTex } };

            try
            {
                var blob = new FrameBlob(TwoMeshBlobSameAtlas(
                    n0: (id: 100u, texId: 1u, u0: 0f,   v0: 0f, u1: 0.5f, v1: 0.5f),
                    n1: (id: 200u, texId: 1u, u0: 0.5f, v0: 0f, u1: 1f,   v1: 0.5f)));
                Assert.IsTrue(blob.IsValid, "v4 blob 应 IsValid");
                Assert.AreEqual(2, blob.NodeCount, "blob 应含 2 节点");
                pool.Sync(blob, root.transform, mm, texMap, Texture2D.whiteTexture, null);

                // 两节点都应建出 loom_node GO（flatten：都挂 root）。
                Assert.AreEqual(2, root.transform.childCount, "应有 2 个 loom_node GO");

                // 节点 A
                var roA = root.transform.GetChild(0);
                var mrA = roA.GetComponent<MeshRenderer>();
                var mfA = roA.GetComponent<MeshFilter>();
                Assert.AreSame(atlasTex, mrA.sharedMaterial.mainTexture,
                    "节点 A tex_id=1 → 应绑 texMap[1] atlasTex（同实例）");

                // 节点 B
                var roB = root.transform.GetChild(1);
                var mrB = roB.GetComponent<MeshRenderer>();
                var mfB = roB.GetComponent<MeshFilter>();
                Assert.AreSame(atlasTex, mrB.sharedMaterial.mainTexture,
                    "节点 B tex_id=1 → 应绑同一 atlasTex（同实例）");

                // 关键断言：两节点同 atlas → MaterialManager key=(0, atlasTex, 0, false) 命中 →
                // sharedMaterial 是**同一 Material 实例**（batchable）。
                Assert.AreSame(mrA.sharedMaterial, mrB.sharedMaterial,
                    "同 atlas 两节点应共享同一 Material 实例（MaterialManager key 命中 → 可 batch）");

                // 各自 Mesh.uv 匹配其子区。blob UV 顶点序 TL→TR→BR→BL（镜像 Rust
                // render::mesh::quad），故 uv[0]=uv_min、uv[2]=uv_max。
                var uvA = mfA.sharedMesh.uv;
                Assert.AreEqual(4, uvA.Length, "节点 A 应有 4 顶点 uv");
                Assert.AreEqual(new Vector2(0f,   0f),   uvA[0], "节点 A uv[0] = uv_min (TL)");
                Assert.AreEqual(new Vector2(0.5f, 0.5f), uvA[2], "节点 A uv[2] = uv_max (BR)");

                var uvB = mfB.sharedMesh.uv;
                Assert.AreEqual(4, uvB.Length, "节点 B 应有 4 顶点 uv");
                Assert.AreEqual(new Vector2(0.5f, 0f),   uvB[0], "节点 B uv[0] = uv_min (TL)");
                Assert.AreEqual(new Vector2(1f,   0.5f), uvB[2], "节点 B uv[2] = uv_max (BR)");
            }
            finally
            {
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(atlasTex);
                Object.DestroyImmediate(root);
            }
        }
    }
}
