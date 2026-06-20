using System.Collections.Generic;
using NUnit.Framework;

namespace LoomGUI.Tests
{
    /// Blob v3 scaffold 校验（T1）。手搓最小 v3 header（镜像 loomgui_ffi_c/src/blob.rs v3 布局），
    /// 验：magic+version 校验生效（IsValid）、Version==3、ClipCount==0（T1 占位）、单 Mesh 节点
    /// 经 ReadMesh 仍正确解析（mesh_arena header 偏移重算正确）。
    ///
    /// 注意：Unity EditMode 在本任务环境无法 headless 执行；Rust blob.rs::TestView 的 v3 测试
    /// 是布局契约的权威自动门，本 C# 测试仅保证编译正确 + 逻辑（offset 计算）正确。
    public class FrameBlobV2Tests
    {
        /// 构造最小合法 v3 blob：1 节点 Mesh（4 verts/6 idx，顶点已 re-base 到本地）。
        /// layout 严格镜像 blob.rs::build_blob v3。
        static byte[] BuildMinimalV2Blob()
        {
            var b = new List<byte>();

            // header: magic, version=3, node_count=1
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(3u));
            b.AddRange(System.BitConverter.GetBytes(1u));

            // header_len = 3*4 + 14*4 + 2*4 + 2*4 + 2*4 = 92
            const int HeaderLen = 92;
            int colOff = HeaderLen;
            int[] offs = new int[14];
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            for (int i = 0; i < 14; i++) { offs[i] = colOff; colOff += elemSize[i]; }

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
            Assert.IsTrue(blob.IsValid, "v3 magic+version 应通过 IsValid");
            Assert.AreEqual(3u, blob.Version, "Version==3");
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

        /// T6：ClipRect 从 clip 表读 ctx → design rect。镜像 blob.rs::read_clips。
        /// 构造含 2 entries（ctx=1 rect{10,20,100,50}；ctx=2 rect{5,5,30,40}）的 blob，验线性扫描命中。
        static byte[] BuildBlobWithClips((uint ctx, float x, float y, float w, float h)[] clips)
        {
            var b = new List<byte>();
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));  // magic
            b.AddRange(System.BitConverter.GetBytes(3u));           // version=3（v1b.2）
            b.AddRange(System.BitConverter.GetBytes(1u));           // node_count

            // header_len = 3*4 + 14*4 + 2*4 + 2*4 + 2*4 = 92（v3：14 col + 三 arena 各 off+len）。
            const int HeaderLen = 92;
            int colOff = HeaderLen;
            int[] offs = new int[14];
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            for (int i = 0; i < 14; i++) { offs[i] = colOff; colOff += elemSize[i]; }

            // mesh arena：空（vc=0,ic=0），只为占位（本测不验 mesh）。
            var arena = new List<byte>();
            arena.AddRange(System.BitConverter.GetBytes(0));  // vc
            arena.AddRange(System.BitConverter.GetBytes(0));  // ic

            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            int meshOff = colOff;
            b.AddRange(System.BitConverter.GetBytes(meshOff));
            b.AddRange(System.BitConverter.GetBytes((uint)arena.Count));
            int textOff = meshOff + arena.Count;
            b.AddRange(System.BitConverter.GetBytes(textOff));
            b.AddRange(System.BitConverter.GetBytes(0u));     // text_arena_len
            int clipOff = textOff;
            uint clipLen = 4u + (uint)(clips.Length * 20);
            b.AddRange(System.BitConverter.GetBytes(clipOff));
            b.AddRange(System.BitConverter.GetBytes(clipLen));

            // 列（1 节点，mask_context=clips[0].ctx 仅占位）
            uint nodeCtx = clips.Length > 0 ? clips[0].ctx : 0u;
            b.AddRange(System.BitConverter.GetBytes(1u));     // node_id
            b.AddRange(System.BitConverter.GetBytes(-1));     // parent
            b.Add(1);                                         // visible
            b.AddRange(System.BitConverter.GetBytes(1f));     // alpha
            b.AddRange(System.BitConverter.GetBytes(0u));     // sort_key
            b.AddRange(System.BitConverter.GetBytes(0f));     // local_x
            b.AddRange(System.BitConverter.GetBytes(0f));     // local_y
            b.AddRange(System.BitConverter.GetBytes(nodeCtx));// mask_context
            b.Add(0);                                         // payload_kind=Unchanged（跳渲染）
            b.AddRange(System.BitConverter.GetBytes(0u));     // mesh_off
            b.AddRange(System.BitConverter.GetBytes(0u));     // mesh_len
            b.AddRange(System.BitConverter.GetBytes(0u));     // text_off
            b.AddRange(System.BitConverter.GetBytes(0u));     // text_len
            b.AddRange(System.BitConverter.GetBytes(0u));     // tex_id（v1b.2，Unchanged=0）

            b.AddRange(arena);
            // clip 表
            b.AddRange(System.BitConverter.GetBytes((uint)clips.Length));
            foreach (var c in clips)
            {
                b.AddRange(System.BitConverter.GetBytes(c.ctx));
                b.AddRange(System.BitConverter.GetBytes(c.x));
                b.AddRange(System.BitConverter.GetBytes(c.y));
                b.AddRange(System.BitConverter.GetBytes(c.w));
                b.AddRange(System.BitConverter.GetBytes(c.h));
            }
            return b.ToArray();
        }

        [Test]
        public void ClipRect_ReadsEntryByCtx_LinearScan()
        {
            var blob = new FrameBlob(BuildBlobWithClips(new[] {
                (1u, 10f, 20f, 100f, 50f),
                (2u, 5f, 5f, 30f, 40f),
            }));
            Assert.AreEqual(2, blob.ClipCount, "2 clip entries");

            Assert.IsTrue(blob.ClipRect(1u, out float x1, out float y1, out float w1, out float h1));
            Assert.AreEqual(10f, x1); Assert.AreEqual(20f, y1); Assert.AreEqual(100f, w1); Assert.AreEqual(50f, h1);

            Assert.IsTrue(blob.ClipRect(2u, out float x2, out float y2, out float w2, out float h2));
            Assert.AreEqual(5f, x2); Assert.AreEqual(5f, y2); Assert.AreEqual(30f, w2); Assert.AreEqual(40f, h2);
        }

        [Test]
        public void ClipRect_MissingCtx_ReturnsFalse()
        {
            var blob = new FrameBlob(BuildBlobWithClips(new[] { (1u, 10f, 20f, 100f, 50f) }));
            Assert.IsFalse(blob.ClipRect(999u, out float x, out float y, out float w, out float h),
                "未入表的 ctx 应返回 false（调用方跳过 SetClipBox）");
            Assert.AreEqual(0f, x, "miss 时 out 置 0");
        }
    }
}
