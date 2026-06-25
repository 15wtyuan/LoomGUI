using System.Collections.Generic;
using NUnit.Framework;

namespace LoomGUI.Tests
{
    public class FrameBlobTests
    {
        // 手搓一个 1 节点 mesh blob（镜像 loomgui_ffi_c/src/blob.rs::build_blob v4 布局），
        // 验 FrameBlob 解析器把每列 + mesh arena 读回正确。
        [Test]
        public void ParsesOneMeshNode()
        {
            var b = new List<byte>();

            // header: magic, version=4, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(4u));
            b.AddRange(System.BitConverter.GetBytes(1u));

            // header 总长 = 3*4 + 18*4 + 6*4 = 108（v4：18 col + mesh/text/clip 三 arena 各 off+len）。
            int headerLen = 12 + 18 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 108
            int colOff = headerLen;
            int[] offs = new int[18];
            // 元素字节数顺序同 blob.rs columns（v4，18 列：含 mask_context + m_a..m_ty world matrix）。
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            for (int i = 0; i < 18; i++) { offs[i] = colOff; colOff += elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：1 mesh，4 verts / 6 idx。
            // verts 全 0（占位，本测不断言顶点值）；uvs/colors/indices 仅填位，只校验 ReadMesh 长度。
            var arena = new List<byte>();
            int arenaStart = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
            arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int k = 0; k < 6; k++) arena.AddRange(System.BitConverter.GetBytes(0u));
            int arenaLen = arena.Count - arenaStart;

            // 18 列 offset + mesh/text/clip 三 arena off+len（text_arena 空、clip 表仅 clip_count=0）
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));   // mesh_arena_off
            b.AddRange(System.BitConverter.GetBytes(arenaLen));   // mesh_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // text_arena_off（紧跟 mesh_arena）
            b.AddRange(System.BitConverter.GetBytes(0u));         // text_arena_len（T1 空）
            int clipOff = arenaOff + arenaLen;                    // text 空，clip 紧跟 text
            b.AddRange(System.BitConverter.GetBytes(clipOff));    // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));         // clip_table_len（仅 clip_count u32）

            // 列数据（node 0）：node_id=7, parent=-1, visible=1, alpha=1, sort_key=3,
            // mask_context=0, m_a=1 m_b=0 m_c=0 m_d=1 m_tx=10 m_ty=20（纯平移 world matrix）,
            // payload_kind=1(Mesh), mesh_off=0, mesh_len=arenaLen, text_off=0, text_len=0, tex_id=0
            b.AddRange(System.BitConverter.GetBytes(7u));        // col 0: node_id
            b.AddRange(System.BitConverter.GetBytes(-1));        // col 1: parent_id
            b.Add(1);                                            // col 2: visible
            b.AddRange(System.BitConverter.GetBytes(1f));        // col 3: alpha
            b.AddRange(System.BitConverter.GetBytes(3u));        // col 4: sort_key
            b.AddRange(System.BitConverter.GetBytes(0u));        // col 5: mask_context
            b.AddRange(System.BitConverter.GetBytes(1f));        // col 6: m_a（identity 2×2 左上 = 1）
            b.AddRange(System.BitConverter.GetBytes(0f));        // col 7: m_b
            b.AddRange(System.BitConverter.GetBytes(0f));        // col 8: m_c
            b.AddRange(System.BitConverter.GetBytes(1f));        // col 9: m_d（identity 2×2 右下 = 1）
            b.AddRange(System.BitConverter.GetBytes(10f));       // col 10: m_tx（平移 x = 原 local_x）
            b.AddRange(System.BitConverter.GetBytes(20f));       // col 11: m_ty（平移 y = 原 local_y）
            b.Add(1);                                            // col 12: payload_kind = Mesh
            b.AddRange(System.BitConverter.GetBytes(0u));        // col 13: mesh_off
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen)); // col 14: mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));        // col 15: text_off（T1 占位）
            b.AddRange(System.BitConverter.GetBytes(0u));        // col 16: text_len（T1 占位）
            b.AddRange(System.BitConverter.GetBytes(0u));        // col 17: tex_id（Mesh 占位 0）
            b.AddRange(arena);
            // clip 表：仅 clip_count=0
            b.AddRange(System.BitConverter.GetBytes(0u));

            var view = new FrameBlob(b.ToArray());
            Assert.IsTrue(view.IsValid, "v4 blob 应通过 magic+version 校验");
            Assert.AreEqual(4u, view.Version);
            Assert.AreEqual(1, view.NodeCount);
            Assert.AreEqual(0, view.ClipCount, "T1: clip_count=0");
            Assert.AreEqual(7u, view.NodeId(0));
            Assert.AreEqual(-1, view.ParentId(0));
            Assert.IsTrue(view.Visible(0));
            Assert.AreEqual(1f, view.Alpha(0));
            Assert.AreEqual(3u, view.SortKey(0));
            Assert.AreEqual(1f, view.Ma(0));
            Assert.AreEqual(0f, view.Mb(0));
            Assert.AreEqual(0f, view.Mc(0));
            Assert.AreEqual(1f, view.Md(0));
            Assert.AreEqual(10f, view.Mtx(0));
            Assert.AreEqual(20f, view.Mty(0));
            Assert.IsTrue(view.IsPureTranslation(0), "identity 2×2 → 纯平移");
            Assert.AreEqual(0u, view.MaskContext(0));
            Assert.AreEqual(1, view.PayloadKind(0));
            Assert.AreEqual(0u, view.TextOff(0), "T1: text_off 占位 0");
            Assert.AreEqual(0u, view.TextLen(0), "T1: text_len 占位 0");
            var mesh = view.ReadMesh(0);
            Assert.AreEqual(4, mesh.Verts.Length);
            Assert.AreEqual(4, mesh.Uvs.Length);
            Assert.AreEqual(4, mesh.Colors.Length);
            Assert.AreEqual(6, mesh.Idx.Length);
        }

        // 非 v4 blob（version=3 陈旧或 magic 错）应判 IsValid=false。
        [Test]
        public void RejectsStaleOrBadMagicBlob()
        {
            // version=3（陈旧）
            var v3 = new List<byte>();
            v3.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            v3.AddRange(System.BitConverter.GetBytes(3u));  // version=3
            v3.AddRange(System.BitConverter.GetBytes(0u));
            Assert.IsFalse(new FrameBlob(v3.ToArray()).IsValid, "v3 blob 应判 invalid");

            // magic 错
            var bad = new List<byte>();
            bad.AddRange(System.BitConverter.GetBytes(0xDEADBEEFu));
            bad.AddRange(System.BitConverter.GetBytes(4u));
            bad.AddRange(System.BitConverter.GetBytes(0u));
            Assert.IsFalse(new FrameBlob(bad.ToArray()).IsValid, "错 magic 应判 invalid");
        }
    }
}
