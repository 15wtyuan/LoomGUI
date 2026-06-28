using System.Collections.Generic;
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// MirrorPool 按 blob 的 tex_id 从 texMap 绑对 Texture2D。
    /// 手搓 v4 单节点 Mesh blob（tex_id=7）+ mock texMap{7→红 Texture2D}，
    /// 断言该节点 Mr.sharedMaterial.mainTexture == 红贴图（非 whiteTexture）。
    public class MirrorPoolTexIdTests
    {
        static byte[] SingleMeshBlobWithTexId(uint nodeId, uint texId)
        {
            var b = new List<byte>();
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu)); // magic
            b.AddRange(System.BitConverter.GetBytes(4u));           // version=4
            b.AddRange(System.BitConverter.GetBytes(1u));           // node_count=1

            const int HeaderLen = 12 + 18 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 108
            // v4 18 列 elemSize
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            int colOff = HeaderLen;
            int[] offs = new int[18];
            for (int i = 0; i < 18; i++) { offs[i] = colOff; colOff += 1 * elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：1 quad
            var arena = new List<byte>();
            int segOff = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
            arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
            foreach (var v in new[] { (0f, 0f), (5f, 0f), (5f, 5f), (0f, 5f) })
            {
                arena.AddRange(System.BitConverter.GetBytes(v.Item1));
                arena.AddRange(System.BitConverter.GetBytes(v.Item2));
            }
            foreach (var uv in new[] { (0f, 0f), (1f, 0f), (1f, 1f), (0f, 1f) })
            {
                arena.AddRange(System.BitConverter.GetBytes(uv.Item1));
                arena.AddRange(System.BitConverter.GetBytes(uv.Item2));
            }
            for (int i = 0; i < 4; i++)
            {
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
                arena.AddRange(System.BitConverter.GetBytes(1f));
            }
            // 索引：逐个 GetBytes（BitConverter 无 uint[] 重载，数组语法会被当 bool）。
            arena.AddRange(System.BitConverter.GetBytes(0u));
            arena.AddRange(System.BitConverter.GetBytes(1u));
            arena.AddRange(System.BitConverter.GetBytes(2u));
            arena.AddRange(System.BitConverter.GetBytes(0u));
            arena.AddRange(System.BitConverter.GetBytes(2u));
            arena.AddRange(System.BitConverter.GetBytes(3u));
            int arenaLen = arena.Count - segOff;

            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // text_arena_off
            b.AddRange(System.BitConverter.GetBytes(0u));                  // text_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));                  // clip_table_len

            // 列数据（单节点，SOA≡AoS；纯平移 world matrix）
            b.AddRange(System.BitConverter.GetBytes(nodeId)); // col 0: node_id
            b.AddRange(System.BitConverter.GetBytes(-1));     // col 1: parent_id
            b.Add(1);                                         // col 2: visible
            b.AddRange(System.BitConverter.GetBytes(1f));     // col 3: alpha
            b.AddRange(System.BitConverter.GetBytes(0u));     // col 4: sort_key
            b.AddRange(System.BitConverter.GetBytes(0u));     // col 5: mask_context
            b.AddRange(System.BitConverter.GetBytes(1f));     // col 6: m_a
            b.AddRange(System.BitConverter.GetBytes(0f));     // col 7: m_b
            b.AddRange(System.BitConverter.GetBytes(0f));     // col 8: m_c
            b.AddRange(System.BitConverter.GetBytes(1f));     // col 9: m_d
            b.AddRange(System.BitConverter.GetBytes(0f));     // col 10: m_tx
            b.AddRange(System.BitConverter.GetBytes(0f));     // col 11: m_ty
            b.Add(1);                                         // col 12: payload_kind=Mesh
            b.AddRange(System.BitConverter.GetBytes((uint)0));        // col 13: mesh_off
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen)); // col 14: mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));     // col 15: text_off
            b.AddRange(System.BitConverter.GetBytes(0u));     // col 16: text_len
            b.AddRange(System.BitConverter.GetBytes(texId));  // col 17: tex_id

            b.AddRange(arena);
            b.AddRange(System.BitConverter.GetBytes(0u));     // clip_count=0
            return b.ToArray();
        }

        [Test]
        public void TexId_Binds_Texture_From_TexMap()
        {
            var root = new GameObject("root");
            root.transform.localScale = new Vector3(1f, -1f, 1f);
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();

            // mock texMap：tex_id=7 → 红 Texture2D
            var redTex = new Texture2D(2, 2);
            var texMap = new Dictionary<uint, Texture2D> { { 7u, redTex } };

            try
            {
                var blob = new FrameBlob(SingleMeshBlobWithTexId(1u, 7u));
                Assert.IsTrue(blob.IsValid, "v4 blob 应 IsValid");
                pool.Sync(blob, root.transform, mm, texMap, Texture2D.whiteTexture, null);

                // 找到唯一 loom_node，断言其 material.mainTexture 是红贴图（非 whiteTexture）。
                Assert.AreEqual(1, root.transform.childCount);
                var mr = root.transform.GetChild(0).GetComponent<MeshRenderer>();
                Assert.AreSame(redTex, mr.sharedMaterial.mainTexture,
                    "tex_id=7 → 应绑 texMap[7] 红贴图");
            }
            finally
            {
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(redTex);
                Object.DestroyImmediate(root);
            }
        }

        /// tex_id=0（占位）或 texMap 缺该 tid → fallback（whiteTexture）。
        /// 验 fallback 分支不会误绑到 texMap 里的某张图。
        [Test]
        public void TexId_Zero_Or_Missing_FallsBack()
        {
            var root = new GameObject("root");
            root.transform.localScale = new Vector3(1f, -1f, 1f);
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();

            var redTex = new Texture2D(2, 2);
            // texMap 只含 tid=7；blob 用 tid=0 + tid=99（都不命中）→ 应 fallback。
            var texMap = new Dictionary<uint, Texture2D> { { 7u, redTex } };

            try
            {
                // tid=0：显式占位 → fallback
                var blob0 = new FrameBlob(SingleMeshBlobWithTexId(1u, 0u));
                Assert.IsTrue(blob0.IsValid);
                pool.Sync(blob0, root.transform, mm, texMap, Texture2D.whiteTexture, null);
                Assert.AreEqual(1, root.transform.childCount);
                var mr0 = root.transform.GetChild(0).GetComponent<MeshRenderer>();
                Assert.AreSame(Texture2D.whiteTexture, mr0.sharedMaterial.mainTexture,
                    "tex_id=0 → 应绑 fallback whiteTexture");

                // tid=99：texMap 不含 → fallback（Sync 复用同 GO，材质刷新到 fallback）
                var blob99 = new FrameBlob(SingleMeshBlobWithTexId(1u, 99u));
                pool.Sync(blob99, root.transform, mm, texMap, Texture2D.whiteTexture, null);
                var mr1 = root.transform.GetChild(0).GetComponent<MeshRenderer>();
                Assert.AreSame(Texture2D.whiteTexture, mr1.sharedMaterial.mainTexture,
                    "tex_id=99 缺 texMap → 应绑 fallback whiteTexture（非 redTex）");
            }
            finally
            {
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(redTex);
                Object.DestroyImmediate(root);
            }
        }
    }
}
