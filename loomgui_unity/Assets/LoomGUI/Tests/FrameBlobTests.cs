using System.Collections.Generic;
using NUnit.Framework;

namespace LoomGUI.Tests
{
    public class FrameBlobTests
    {
        // 手搓一个 1 节点 mesh blob（镜像 loomgui_ffi_c/src/blob.rs::build_blob v2 布局），
        // 验 FrameBlob 解析器把每列 + mesh arena 读回正确。
        [Test]
        public void ParsesOneMeshNode()
        {
            var b = new List<byte>();

            // header: magic, version=3, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(3u));
            b.AddRange(System.BitConverter.GetBytes(1u));

            // header 总长 = 3*4 + 14*4 + 6*4 = 92（v3：14 col + mesh/text/clip 三 arena 各 off+len）。
            int headerLen = 12 + 14 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 92
            int colOff = headerLen;
            int[] offs = new int[14];
            // 元素字节数顺序同 blob.rs columns（v3，末尾 tex_id u32）。
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            for (int i = 0; i < 14; i++) { offs[i] = colOff; colOff += elemSize[i]; }
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

            // 14 列 offset + mesh/text/clip 三 arena off+len（text_arena 空、clip 表仅 clip_count=0）
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));   // mesh_arena_off
            b.AddRange(System.BitConverter.GetBytes(arenaLen));   // mesh_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen)); // text_arena_off（紧跟 mesh_arena）
            b.AddRange(System.BitConverter.GetBytes(0u));         // text_arena_len（T1 空）
            int clipOff = arenaOff + arenaLen;                    // text 空，clip 紧跟 text
            b.AddRange(System.BitConverter.GetBytes(clipOff));    // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));         // clip_table_len（仅 clip_count u32）

            // 列数据（node 0）：node_id=7, parent=-1, visible=1, alpha=1, sort_key=3,
            // local_x=10, local_y=20, mask_context=0, payload_kind=1(Mesh),
            // mesh_off=0 (相对 arena 起始), mesh_len=arenaLen, text_off=0, text_len=0, tex_id=0
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
            b.AddRange(System.BitConverter.GetBytes(0u));   // text_off（T1 占位）
            b.AddRange(System.BitConverter.GetBytes(0u));   // text_len（T1 占位）
            b.AddRange(System.BitConverter.GetBytes(0u));   // tex_id（v1b.2，Mesh 占位 0）
            b.AddRange(arena);
            // clip 表：仅 clip_count=0
            b.AddRange(System.BitConverter.GetBytes(0u));

            var view = new FrameBlob(b.ToArray());
            Assert.IsTrue(view.IsValid, "v3 blob 应通过 magic+version 校验");
            Assert.AreEqual(3u, view.Version);
            Assert.AreEqual(1, view.NodeCount);
            Assert.AreEqual(0, view.ClipCount, "T1: clip_count=0");
            Assert.AreEqual(7u, view.NodeId(0));
            Assert.AreEqual(-1, view.ParentId(0));
            Assert.IsTrue(view.Visible(0));
            Assert.AreEqual(1f, view.Alpha(0));
            Assert.AreEqual(3u, view.SortKey(0));
            Assert.AreEqual(10f, view.LocalX(0));
            Assert.AreEqual(20f, view.LocalY(0));
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

        // 非 v2 blob（version=1 陈旧或 magic 错）应判 IsValid=false。
        [Test]
        public void RejectsStaleOrBadMagicBlob()
        {
            // version=1（陈旧）
            var v1 = new List<byte>();
            v1.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            v1.AddRange(System.BitConverter.GetBytes(1u));  // version=1
            v1.AddRange(System.BitConverter.GetBytes(0u));
            Assert.IsFalse(new FrameBlob(v1.ToArray()).IsValid, "v1 blob 应判 invalid");

            // magic 错
            var bad = new List<byte>();
            bad.AddRange(System.BitConverter.GetBytes(0xDEADBEEFu));
            bad.AddRange(System.BitConverter.GetBytes(2u));
            bad.AddRange(System.BitConverter.GetBytes(0u));
            Assert.IsFalse(new FrameBlob(bad.ToArray()).IsValid, "错 magic 应判 invalid");
        }
    }
}
