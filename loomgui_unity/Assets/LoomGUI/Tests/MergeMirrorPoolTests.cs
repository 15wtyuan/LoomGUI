using System.Collections.Generic;
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// v1b.4：merged blob（1 节点、8 顶点拼接 mesh segment）→ MirrorPool 产 1 个 GO + 大 Mesh。
    /// 对照：v1b.3 AtlasMirrorPoolTests 两节点 blob → 2 个 GO；v1b.2 MirrorPoolTexIdTests 单节点 4 顶点。
    /// merged 让 N→1 GO（→ N→1 draw call）——本测验证 MirrorPool 仅按 node_id 复用 GO、
    /// 对“单节点大 mesh segment”路径无“node_id 必为 scene 索引/连续”等隐含假设。
    ///
    /// 关键差异（vs AtlasMirrorPoolTests N=2 独立 segment）：
    ///   - node_count=1（merged batch 的 1 节点）。
    ///   - mesh arena 含 1 个 segment，但 vert_count=8、idx_count=12（2 quad 拼接，单 segment）。
    ///   - node_id = anchor（batch 内最小 id，非 scene 索引）—— spec §8 保证 MirrorPool 零改（按 id 复用）。
    ///   - 顶点绝对坐标：(0,0)(10,0)(10,10)(0,10)(100,0)(110,0)(110,10)(100,10)——
    ///     照搬 T3 Rust fixture 值，验 re-base 减 0 后 Unity 读到绝对坐标。
    public class MergeMirrorPoolTests
    {
        /// 构造 merged blob：1 节点，mesh segment = 8 顶点（2 quad 拼接）、12 indices、
        /// transform=(0,0)、alpha=1、tex_id=1、mask_context=0、payload_kind=1。
        /// 参考 FrameBlob.cs 14 列布局 + mesh arena segment 布局；模板照 AtlasMirrorPoolTests /
        /// MirrorPoolTexIdTests（单节点变体）。
        static byte[] BuildMergedBlob()
        {
            const int N = 1;
            var b = new List<byte>();
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu)); // magic
            b.AddRange(System.BitConverter.GetBytes(3u));           // version=3
            b.AddRange(System.BitConverter.GetBytes((uint)N));      // node_count=1

            const int HeaderLen = 12 + 14 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 92
            // 14 列 elemSize（镜像 FrameBlob 列序）：node_id(u32) parent_id(i32) visible(u8)
            // alpha(f32) sort_key(u32) local_x(f32) local_y(f32) mask_context(u32) payload_kind(u8)
            // mesh_off(u32) mesh_len(u32) text_off(u32) text_len(u32) tex_id(u32)
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            int colOff = HeaderLen;
            int[] offs = new int[14];
            for (int i = 0; i < 14; i++) { offs[i] = colOff; colOff += N * elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：1 个 merged segment（8 顶点 = 2 quad 拼接；12 indices = 6×2）。
            // segment 布局：vert_count(u32) idx_count(u32) verts[vc×2 f32] uvs[vc×2 f32]
            //              colors[vc×4 f32] indices[idx_count u32]。
            // 顶点绝对坐标（design，y-down，TL→TR→BR→BL per quad；照搬 T3 Rust fixture 值，
            // 验 re-base 减 0 后 Unity 读到绝对坐标）：
            //   quad A：(0,0)(10,0)(10,10)(0,10)
            //   quad B：(100,0)(110,0)(110,10)(100,10)
            // uvs：单位子区 0..1（per-quad）；colors：白色不透明。
            // indices：quad A 0,1,2,0,2,3 + quad B 4,5,6,4,5,6（同 segment 连续顶点编号）。
            var arena = new List<byte>();
            int segOff = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(8));  // vert_count
            arena.AddRange(System.BitConverter.GetBytes(12)); // idx_count
            // verts（8 × 2 f32 = 64B；design y-down，TL→TR→BR→BL per quad）
            foreach (var v in new[] {
                (0f,   0f),  (10f,  0f),  (10f,  10f),  (0f,   10f),    // quad A
                (100f, 0f),  (110f, 0f),  (110f, 10f),  (100f, 10f)     // quad B
            })
            {
                arena.AddRange(System.BitConverter.GetBytes(v.Item1));
                arena.AddRange(System.BitConverter.GetBytes(v.Item2));
            }
            // uvs（8 × 2 f32；每 quad 子区 0..1 TL→TR→BR→BL，镜像 AtlasMirrorPoolTests 单 quad 套路）
            foreach (var uv in new[] {
                (0f, 0f), (1f, 0f), (1f, 1f), (0f, 1f),  // quad A
                (0f, 0f), (1f, 0f), (1f, 1f), (0f, 1f)   // quad B
            })
            {
                arena.AddRange(System.BitConverter.GetBytes(uv.Item1));
                arena.AddRange(System.BitConverter.GetBytes(uv.Item2));
            }
            // colors（8 × 4 f32 = 128B；白色不透明）
            for (int i = 0; i < 8; i++)
            {
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
            }
            // indices（12 × u32；quad A: 0,1,2,0,2,3 + quad B: 4,5,6,4,5,6）
            foreach (var idx in new uint[] {
                0, 1, 2, 0, 2, 3,   // quad A
                4, 5, 6, 4, 5, 6    // quad B
            })
            {
                arena.AddRange(System.BitConverter.GetBytes(idx));
            }
            int arenaLen = arena.Count - segOff;

            // 14 列 col_offset（N=1：每列 1 元素）
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // text_arena_off
            b.AddRange(System.BitConverter.GetBytes(0u));                  // text_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));                  // clip_table_len（仅 clip_count u32）

            // 列数据（单节点：SOA≡AoS；每列 1 元素）
            b.AddRange(System.BitConverter.GetBytes(1u));    // col 0: node_id（anchor=1，merged batch 最小 id）
            b.AddRange(System.BitConverter.GetBytes(-1));    // col 1: parent_id（-1=无父）
            b.Add(1);                                        // col 2: visible
            b.AddRange(System.BitConverter.GetBytes(1f));    // col 3: alpha
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 4: sort_key
            b.AddRange(System.BitConverter.GetBytes(0f));    // col 5: local_x（merged 节点 root，原点）
            b.AddRange(System.BitConverter.GetBytes(0f));    // col 6: local_y
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 7: mask_context（0=无裁剪）
            b.Add(1);                                        // col 8: payload_kind=1 (Mesh)
            b.AddRange(System.BitConverter.GetBytes((uint)0));        // col 9:  mesh_off（arena 内 0）
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen)); // col 10: mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 11: text_off
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 12: text_len
            b.AddRange(System.BitConverter.GetBytes(1u));    // col 13: tex_id=1

            // mesh arena
            b.AddRange(arena);
            // clip_table：clip_count(u32)=0
            b.AddRange(System.BitConverter.GetBytes(0u));
            return b.ToArray();
        }

        [Test]
        public void MergedBlobProducesSingleGoWithEightVerts()
        {
            var root = new GameObject("root");
            root.transform.localScale = new Vector3(1f, -1f, 1f);
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();

            var tex = new Texture2D(16, 16);
            var texMap = new Dictionary<uint, Texture2D> { { 1u, tex } };

            try
            {
                var blob = new FrameBlob(BuildMergedBlob());
                Assert.IsTrue(blob.IsValid, "v3 blob 应 IsValid");
                Assert.AreEqual(1, blob.NodeCount, "merged blob 应含 1 节点");
                pool.Sync(blob, root.transform, mm, texMap, Texture2D.whiteTexture, null);

                // merged 1 节点 → 1 个 loom_node GO（非 2）。
                var nodes = System.Array.FindAll(root.GetComponentsInChildren<MeshRenderer>(true),
                    mr => mr.gameObject.name == "loom_node");
                Assert.AreEqual(1, nodes.Length, "merged blob → 1 GO（非 2）—— MirrorPool 按 node_id 复用，无 scene 索引假设");

                // 该 GO 的 Mesh 有 8 顶点（2 quad 拼接）。
                var mf = nodes[0].GetComponent<MeshFilter>();
                Assert.AreEqual(8, mf.sharedMesh.vertexCount, "merged mesh 8 顶点（2 quad 拼接到单 segment）");
            }
            finally
            {
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(tex);
                Object.DestroyImmediate(root);
            }
        }
    }
}
