using System.Collections.Generic;
using NUnit.Framework;

namespace LoomGUI.Tests
{
    public class FrameBlobTests
    {
        // 手搓一个 1 节点 mesh blob（镜像 loomgui_ffi_c/src/blob.rs::build_blob 布局），
        // 验 FrameBlob 解析器把每列 + arena 读回正确。
        [Test]
        public void ParsesOneMeshNode()
        {
            var b = new List<byte>();

            // header: magic, version=1, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(1u));
            b.AddRange(System.BitConverter.GetBytes(1u));

            // header 总长 = 3*4 + 11*4 + 2*4 = 64。列 offset 从此起按 elemSize 递进。
            int headerLen = 12 + 11 * 4 + 2 * 4; // = 64
            int colOff = headerLen;
            int[] offs = new int[11];
            // 元素字节数顺序同 blob.rs columns。
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4 };
            for (int i = 0; i < 11; i++) { offs[i] = colOff; colOff += elemSize[i]; }
            int arenaOff = colOff;

            // arena：1 mesh，4 verts / 6 idx。
            // verts 全 0（占位，本测不断言顶点值）；uvs=identity (v,1-v) 仅填位；
            // colors=0；indices=0。只校验 ReadMesh 返回正确长度。
            var arena = new List<byte>();
            int arenaStart = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
            arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int k = 0; k < 6; k++) arena.AddRange(System.BitConverter.GetBytes(0u));
            int arenaLen = arena.Count - arenaStart;

            // 11 列 offset（offset from blob start）+ arena_off + arena_len
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));

            // 列数据（node 0）：node_id=7, parent=-1, visible=1, alpha=1, sort_key=3,
            // local_x=10, local_y=20, mask_context=0, payload_kind=1(Mesh),
            // mesh_off=0 (相对 arena 起始), mesh_len=arenaLen
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
            b.AddRange(arena);

            var view = new FrameBlob(b.ToArray());
            Assert.AreEqual(1, view.NodeCount);
            Assert.AreEqual(7u, view.NodeId(0));
            Assert.AreEqual(-1, view.ParentId(0));
            Assert.IsTrue(view.Visible(0));
            Assert.AreEqual(1f, view.Alpha(0));
            Assert.AreEqual(3u, view.SortKey(0));
            Assert.AreEqual(10f, view.LocalX(0));
            Assert.AreEqual(20f, view.LocalY(0));
            Assert.AreEqual(0u, view.MaskContext(0));
            Assert.AreEqual(1, view.PayloadKind(0));
            var mesh = view.ReadMesh(0);
            Assert.AreEqual(4, mesh.Verts.Length);
            Assert.AreEqual(4, mesh.Uvs.Length);
            Assert.AreEqual(4, mesh.Colors.Length);
            Assert.AreEqual(6, mesh.Idx.Length);
        }
    }
}
