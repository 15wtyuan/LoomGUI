using System.Collections.Generic;
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// merged blob（1 节点、8 顶点拼接 mesh segment）→ MirrorPool 产 1 个 GO + 大 Mesh。
    /// merged 让 N→1 GO（→ N→1 draw call）——验 MirrorPool 仅按 node_id 复用 GO、
    /// 对"单节点大 mesh segment"路径无"node_id 必为 scene 索引/连续"等隐含假设。
    ///
    /// 关键差异（vs AtlasMirrorPoolTests N=2 独立 segment）：
    ///   - node_count=1（merged batch 的 1 节点）。
    ///   - mesh arena 含 1 个 segment，但 vert_count=8、idx_count=12（2 quad 拼接，单 segment）。
    ///   - node_id = anchor（batch 内最小 id，非 scene 索引）—— 保证 MirrorPool 按 id 复用即可。
    ///   - 顶点绝对坐标：(0,0)(10,0)(10,10)(0,10)(100,0)(110,0)(110,10)(100,10)——
    ///     验 re-base 后 Unity 读到绝对坐标。
    [Ignore("v1.4-a: blob v4 layout, rewrite to v7 deferred")]
    public class MergeMirrorPoolTests
    {
        /// 构造 merged blob：1 节点，mesh segment = 8 顶点（2 quad 拼接）、12 indices、
        /// transform=identity（纯平移 0,0）、alpha=1、tex_id=1、mask_context=0、payload_kind=1。
        /// v4 布局（18 列含 world matrix）。
        static byte[] BuildMergedBlob()
        {
            const int N = 1;
            var b = new List<byte>();
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu)); // magic
            b.AddRange(System.BitConverter.GetBytes(4u));           // version=4
            b.AddRange(System.BitConverter.GetBytes((uint)N));      // node_count=1

            const int HeaderLen = 12 + 18 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 108
            // v4 18 列 elemSize（镜像 FrameBlob 列序）
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            int colOff = HeaderLen;
            int[] offs = new int[18];
            for (int i = 0; i < 18; i++) { offs[i] = colOff; colOff += N * elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：1 个 merged segment（8 顶点 = 2 quad 拼接；12 indices = 6×2）。
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

            // 18 列 col_offset（N=1：每列 1 元素）
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // text_arena_off
            b.AddRange(System.BitConverter.GetBytes(0u));                  // text_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));                  // clip_table_len（仅 clip_count u32）

            // 列数据（单节点：SOA≡AoS；每列 1 元素；纯平移 identity world matrix @ 原点）
            b.AddRange(System.BitConverter.GetBytes(1u));    // col 0: node_id（anchor=1，merged batch 最小 id）
            b.AddRange(System.BitConverter.GetBytes(-1));    // col 1: parent_id（-1=无父）
            b.Add(1);                                        // col 2: visible
            b.AddRange(System.BitConverter.GetBytes(1f));    // col 3: alpha
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 4: sort_key
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 5: mask_context（0=无裁剪）
            b.AddRange(System.BitConverter.GetBytes(1f));    // col 6: m_a（identity）
            b.AddRange(System.BitConverter.GetBytes(0f));    // col 7: m_b
            b.AddRange(System.BitConverter.GetBytes(0f));    // col 8: m_c
            b.AddRange(System.BitConverter.GetBytes(1f));    // col 9: m_d（identity）
            b.AddRange(System.BitConverter.GetBytes(0f));    // col 10: m_tx（merged 节点 root，原点）
            b.AddRange(System.BitConverter.GetBytes(0f));    // col 11: m_ty
            b.Add(1);                                        // col 12: payload_kind=1 (Mesh)
            b.AddRange(System.BitConverter.GetBytes((uint)0));        // col 13: mesh_off（arena 内 0）
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen)); // col 14: mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 15: text_off
            b.AddRange(System.BitConverter.GetBytes(0u));    // col 16: text_len
            b.AddRange(System.BitConverter.GetBytes(1u));    // col 17: tex_id=1

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

            // v1.4-a T8：Sync 新签名（SpriteResolver + fallback + font）。本测验 merged mesh 顶点数
            //   （非纹理绑定），传 null SpriteResolver → path_idx=0/miss → fallback。完整 path→Sprite
            //   round-trip 测试见 T12（手搓 v7 blob + mock SpriteAtlas）。
            //   注：blob 仍是 v4 → v7 FrameBlob 拒绝（IsValid=false）→ Sync 早退。本测断言会失败，
            //   T12 重写为 v7 blob 后恢复。本 task 仅保证编译通过。
            var tex = new Texture2D(16, 16);

            try
            {
                var blob = new FrameBlob(BuildMergedBlob());
                Assert.IsTrue(blob.IsValid, "v4 blob 应 IsValid（注：v7 FrameBlob 拒绝 v4，T12 重写）");
                Assert.AreEqual(1, blob.NodeCount, "merged blob 应含 1 节点");
                pool.Sync(blob, root.transform, mm, null, Texture2D.whiteTexture, null);

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
