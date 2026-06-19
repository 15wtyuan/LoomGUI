using System.Collections.Generic;
using NUnit.Framework;

namespace LoomGUI.Tests
{
    /// Blob v2 scaffold 校验（T1）。手搓最小 v2 header（镜像 loomgui_ffi_c/src/blob.rs v2 布局），
    /// 验：magic+version 校验生效（IsValid）、Version==2、ClipCount==0（T1 占位）、单 Mesh 节点
    /// 经 ReadMesh 仍正确解析（mesh_arena header 偏移重算正确）。
    ///
    /// 注意：Unity EditMode 在本任务环境无法 headless 执行；Rust blob.rs::TestView 的 v2 测试
    /// 是布局契约的权威自动门，本 C# 测试仅保证编译正确 + 逻辑（offset 计算）正确。
    public class FrameBlobV2Tests
    {
        /// 构造最小合法 v2 blob：1 节点 Mesh（4 verts/6 idx，顶点已 re-base 到本地）。
        /// layout 严格镜像 blob.rs::build_blob v2。
        static byte[] BuildMinimalV2Blob()
        {
            var b = new List<byte>();

            // header: magic, version=2, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(2u));
            b.AddRange(System.BitConverter.GetBytes(1u));

            // header_len = 3*4 + 13*4 + 2*4 + 2*4 + 2*4 = 88
            const int HeaderLen = 88;
            int colOff = HeaderLen;
            int[] offs = new int[13];
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4 };
            for (int i = 0; i < 13; i++) { offs[i] = colOff; colOff += elemSize[i]; }

            // mesh arena：4 verts / 6 idx。顶点 (0,0)(1,0)(1,1)(0,1)（已 re-base）。
            var arena = new List<byte>();
            int arenaStart = arena.Count;
            arena.AddRange(System.BitConverter.GetBytes(4));
            arena.AddRange(System.BitConverter.GetBytes(6));
            float[] vs = { 0f, 0f, 1f, 0f, 1f, 1f, 0f, 1f };
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(vs[v * 2])); arena.AddRange(System.BitConverter.GetBytes(vs[v * 2 + 1])); }
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(0f)); arena.AddRange(System.BitConverter.GetBytes(0f)); }
            for (int v = 0; v < 4; v++) { arena.AddRange(System.BitConverter.GetBytes(1f)); arena.AddRange(System.BitConverter.GetBytes(1f)); arena.AddRange(System.BitConverter.GetBytes(1f)); arena.AddRange(System.BitConverter.GetBytes(1f)); }
            uint[] ix = { 0, 1, 2, 0, 2, 3 };
            for (int k = 0; k < 6; k++) arena.AddRange(System.BitConverter.GetBytes(ix[k]));
            int arenaLen = arena.Count - arenaStart;

            // 13 列 offset + mesh/text/clip 三 arena off+len
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            int meshArenaOff = colOff;
            b.AddRange(System.BitConverter.GetBytes(meshArenaOff));
            b.AddRange(System.BitConverter.GetBytes(arenaLen));
            int textArenaOff = meshArenaOff + arenaLen;        // text 紧跟 mesh
            b.AddRange(System.BitConverter.GetBytes(textArenaOff));
            b.AddRange(System.BitConverter.GetBytes(0u));      // text_arena_len（T1 空）
            int clipOff = textArenaOff;                        // text 空，clip 紧跟 text
            b.AddRange(System.BitConverter.GetBytes(clipOff));
            b.AddRange(System.BitConverter.GetBytes(4u));      // clip_table_len（仅 clip_count）

            // 列数据（node 0）：node_id=1, parent=-1, visible=1, alpha=1, sort_key=1,
            // local=(5,6), mask=0, kind=1(Mesh), mesh_off=0, mesh_len=arenaLen, text_off=0, text_len=0
            b.AddRange(System.BitConverter.GetBytes(1u));        // node_id
            b.AddRange(System.BitConverter.GetBytes(-1));        // parent_id
            b.Add(1);                                            // visible
            b.AddRange(System.BitConverter.GetBytes(1f));        // alpha
            b.AddRange(System.BitConverter.GetBytes(1u));        // sort_key
            b.AddRange(System.BitConverter.GetBytes(5f));        // local_x
            b.AddRange(System.BitConverter.GetBytes(6f));        // local_y
            b.AddRange(System.BitConverter.GetBytes(0u));        // mask_context
            b.Add(1);                                            // payload_kind = Mesh
            b.AddRange(System.BitConverter.GetBytes(0u));        // mesh_off
            b.AddRange(System.BitConverter.GetBytes((uint)arenaLen)); // mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));        // text_off
            b.AddRange(System.BitConverter.GetBytes(0u));        // text_len

            b.AddRange(arena);
            // text_arena T1 空，跳过。
            // clip 表：仅 clip_count=0
            b.AddRange(System.BitConverter.GetBytes(0u));
            return b.ToArray();
        }

        [Test]
        public void V2BlobIsValidAndHasExpectedHeader()
        {
            var blob = new FrameBlob(BuildMinimalV2Blob());
            Assert.IsTrue(blob.IsValid, "v2 magic+version 应通过 IsValid");
            Assert.AreEqual(2u, blob.Version, "Version==2");
            Assert.AreEqual(1, blob.NodeCount);
        }

        [Test]
        public void ClipCountIsZeroInT1()
        {
            var blob = new FrameBlob(BuildMinimalV2Blob());
            Assert.AreEqual(0, blob.ClipCount, "T1: clip 表仅 clip_count=0，无 entries");
        }

        [Test]
        public void TextAccessorsArePlaceholderZero()
        {
            var blob = new FrameBlob(BuildMinimalV2Blob());
            Assert.AreEqual(0u, blob.TextOff(0), "T1: text_off 占位 0");
            Assert.AreEqual(0u, blob.TextLen(0), "T1: text_len 占位 0");
        }

        [Test]
        public void SingleMeshNodeParsesViaReadMesh()
        {
            var blob = new FrameBlob(BuildMinimalV2Blob());
            // mesh 仍应能正确解析（mesh_arena header 偏移重算正确、列数据未坏）。
            var mesh = blob.ReadMesh(0);
            Assert.AreEqual(4, mesh.Verts.Length, "vert_count=4");
            Assert.AreEqual(6, mesh.Idx.Length, "idx_count=6");
            // 顶点（已 re-base 到本地）：(0,0)(1,0)(1,1)(0,1)。
            Assert.AreEqual(0f, mesh.Verts[0].x);
            Assert.AreEqual(0f, mesh.Verts[0].y);
            Assert.AreEqual(1f, mesh.Verts[2].x);
            Assert.AreEqual(1f, mesh.Verts[2].y);
        }
    }
}
