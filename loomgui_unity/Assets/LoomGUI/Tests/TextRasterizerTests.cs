using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// TextRasterizer.BuildMesh 测试（§4.3 / T4）。验 glyph quad 数学：
    ///   quad_left=pen_x+minX, quad_right=pen_x+maxX,
    ///   quad_top=pen_y−maxY, quad_bottom=pen_y−minY（y-down；maxY 基线上方→减）
    /// 顶点序 BL,TL,TR,BR（对齐 fgui DrawGlyph）；索引每 quad 0,1,2,0,2,3；顶点色=color×alpha。
    ///
    /// Unity EditMode 在本任务环境无法 headless 执行——本测保证编译正确 + 逻辑（quad 数学、顶点序、
    /// 索引、顶点色）正确。预期值由 BuildMesh 内部用的同一 GetCharacterInfo 重新算出再比对，
    /// 故即使 DejaVu 不同版本的精确 minX/maxY 不同，只要 BuildMesh 数学正确即过。
    public class TextRasterizerTests
    {
        const string DejaVuPath = "Assets/LoomGUI/Fonts/DejaVuSans.ttf";

        static Font LoadDejaVu()
        {
#if UNITY_EDITOR
            var font = UnityEditor.AssetDatabase.LoadAssetAtPath<Font>(DejaVuPath);
            Assert.IsNotNull(font, $"DejaVu 字体应在 {DejaVuPath}（T4 拷入；.meta TrueTypeFontImporter）");
            return font;
#else
            Assert.Inconclusive("PlayMode/build 无 AssetDatabase——跳过");
            return null;
#endif
        }

        /// "A" @ pen (0, 20) → 1 glyph → 4 verts / 6 idx。
        [Test]
        public void BuildMesh_OneGlyph_ProducesOneQuad()
        {
            var font = LoadDejaVu();
            const int fontSize = 24;
            var glyphs = new[] { new GlyphData('A', 0f, 20f) };
            var mesh = TextRasterizer.BuildMesh(font, fontSize, Color.white, 1f, glyphs);

            Assert.AreEqual(4, mesh.Verts.Length, "1 glyph = 4 verts（BL,TL,TR,BR）");
            Assert.AreEqual(6, mesh.Idx.Length, "1 quad = 6 idx（0,1,2,0,2,3）");
        }

        /// quad 四角位置 == pen + box 数学（§4.3 step 4）。用 BuildMesh 内同源 GetCharacterInfo
        /// 重算期望，再逐顶点比对——锁顶点序 + 数学，不锁 DejaVu 具体数值（跨版本稳健）。
        [Test]
        public void BuildMesh_QuadCorners_MatchPenPlusBoxMath()
        {
            var font = LoadDejaVu();
            const int fontSize = 24;
            float penX = 0f, penY = 20f;
            var glyphs = new[] { new GlyphData('A', penX, penY) };
            var mesh = TextRasterizer.BuildMesh(font, fontSize, Color.white, 1f, glyphs);

            // 重算期望（同 BuildMesh 内源）。
            font.RequestCharactersInTexture("A", fontSize, FontStyle.Normal);
            Assert.IsTrue(font.GetCharacterInfo('A', out var info, fontSize, FontStyle.Normal),
                "GetCharacterInfo 应成功（已 RequestCharactersInTexture）");

            float pl = penX + info.minX;
            float pr = penX + info.maxX;
            float pt = penY - info.maxY;   // maxY 基线上方 → y-down 减
            float pb = penY - info.minY;   // minY 基线下方

            // 顶点序 BL,TL,TR,BR（fgui DrawGlyph 同序）。
            Assert.AreEqual(new Vector2(pl, pb), mesh.Verts[0], "BL = (pl, pb)");
            Assert.AreEqual(new Vector2(pl, pt), mesh.Verts[1], "TL = (pl, pt)");
            Assert.AreEqual(new Vector2(pr, pt), mesh.Verts[2], "TR = (pr, pt)");
            Assert.AreEqual(new Vector2(pr, pb), mesh.Verts[3], "BR = (pr, pb)");

            // 索引：0,1,2,0,2,3。
            Assert.AreEqual(0u, mesh.Idx[0]);
            Assert.AreEqual(1u, mesh.Idx[1]);
            Assert.AreEqual(2u, mesh.Idx[2]);
            Assert.AreEqual(0u, mesh.Idx[3]);
            Assert.AreEqual(2u, mesh.Idx[4]);
            Assert.AreEqual(3u, mesh.Idx[5]);
        }

        /// quad 宽度 == maxX − minX（pen 平移不改变 glyph 宽，只改位置）。
        [Test]
        public void BuildMesh_PenShift_MovesQuadNotResize()
        {
            var font = LoadDejaVu();
            const int fontSize = 24;
            font.RequestCharactersInTexture("A", fontSize, FontStyle.Normal);
            font.GetCharacterInfo('A', out var info, fontSize, FontStyle.Normal);
            float expectedW = info.maxX - info.minX;

            // pen (0,0) vs (100,0)：宽度应同。
            var m0 = TextRasterizer.BuildMesh(font, fontSize, Color.white, 1f, new[] { new GlyphData('A', 0f, 0f) });
            var m1 = TextRasterizer.BuildMesh(font, fontSize, Color.white, 1f, new[] { new GlyphData('A', 100f, 0f) });

            float w0 = m0.Verts[2].x - m0.Verts[0].x;   // TR.x − BL.x
            float w1 = m1.Verts[2].x - m1.Verts[0].x;
            Assert.AreEqual(expectedW, w0, 0.001f, "pen(0) 宽 == maxX−minX");
            Assert.AreEqual(expectedW, w1, 0.001f, "pen(100) 宽 == maxX−minX（平移不改尺寸）");
            // 平移：BL.x 差 100。
            Assert.AreEqual(100f, m1.Verts[0].x - m0.Verts[0].x, 0.001f, "pen 平移 100 → BL.x 平移 100");
        }

        /// 顶点色 = color × alpha（node_alpha 烤四顶点）。
        [Test]
        public void BuildMesh_VertexColor_IsColorTimesAlpha()
        {
            var font = LoadDejaVu();
            var color = new Color(1f, 0.5f, 0.25f, 1f);
            const float alpha = 0.6f;
            var mesh = TextRasterizer.BuildMesh(font, 24, color, alpha, new[] { new GlyphData('A', 0f, 0f) });

            var expected = color; expected.a *= alpha;   // BuildMesh：color.a *= alpha
            foreach (var c in mesh.Colors)
                Assert.AreEqual(expected, c, "四顶点色 = color × alpha（a 受 alpha 缩放，rgb 不变）");
        }

        /// 多 glyph（"AB"）→ 2 quad = 8 verts / 12 idx，第二 quad 索引基 = 4。
        [Test]
        public void BuildMesh_TwoGlyphs_TwoQuadsIndexedFromBase4()
        {
            var font = LoadDejaVu();
            var glyphs = new[] { new GlyphData('A', 0f, 20f), new GlyphData('B', 30f, 20f) };
            var mesh = TextRasterizer.BuildMesh(font, 24, Color.white, 1f, glyphs);

            Assert.AreEqual(8, mesh.Verts.Length, "2 glyph = 8 verts");
            Assert.AreEqual(12, mesh.Idx.Length, "2 quad = 12 idx");
            // 第二 quad 索引基 4：4,5,6,4,6,7
            Assert.AreEqual(4u, mesh.Idx[6]);
            Assert.AreEqual(5u, mesh.Idx[7]);
            Assert.AreEqual(6u, mesh.Idx[8]);
            Assert.AreEqual(4u, mesh.Idx[9]);
            Assert.AreEqual(6u, mesh.Idx[10]);
            Assert.AreEqual(7u, mesh.Idx[11]);
        }

        /// FontVersion 初值 0；OnRebuilt 自增。
        [Test]
        public void FontVersion_IncrementsOnRebuilt()
        {
            TextRasterizer.ResetStatic();
            Assert.AreEqual(0, TextRasterizer.FontVersion, "ResetStatic 后版本 0");
            TextRasterizer.OnRebuilt(null);
            Assert.AreEqual(1, TextRasterizer.FontVersion, "OnRebuilt 后版本 1");
            TextRasterizer.OnRebuilt(null);
            Assert.AreEqual(2, TextRasterizer.FontVersion, "二次 OnRebuilt → 2");
            TextRasterizer.ResetStatic();
            Assert.AreEqual(0, TextRasterizer.FontVersion, "ResetStatic 归零（Domain reload 语义）");
        }

        /// ResetStatic 契约（T8 / §4.3e Domain reload 保护）：任意非零版本 → 调一次归零；
        /// 再调一次仍 0（幂等）；多次 OnRebuilt 累积后归零仍生效。锁 SubsystemRegistration 复位语义。
        /// （SubsystemRegistration 属性本身 editor-triggered、EditMode 无法 headless 触发——本测
        /// 验 ResetStatic 行为本身；LoomStage.ResetStatics 的接线由 diff/code review 锁定。）
        [Test]
        public void ResetStatic_ZerosFontVersion_FromAnyNonzeroState()
        {
            // 预置一个非零状态（模拟若干次 atlas rebuild 累积）。
            TextRasterizer.OnRebuilt(null);
            TextRasterizer.OnRebuilt(null);
            TextRasterizer.OnRebuilt(null);
            Assert.AreNotEqual(0, TextRasterizer.FontVersion, "前置：三次 OnRebuilt 后版本非 0");

            // ResetStatic → 必须归零（Domain reload 后下帧视为全新版本基线）。
            TextRasterizer.ResetStatic();
            Assert.AreEqual(0, TextRasterizer.FontVersion, "ResetStatic 必须把版本归零");

            // 幂等：再调一次仍 0（不应变负 / 抛异常）。
            TextRasterizer.ResetStatic();
            Assert.AreEqual(0, TextRasterizer.FontVersion, "ResetStatic 幂等——已为 0 再调仍 0");

            // 归零后 OnRebuilt 从 0 重新累加（基线已复位，不是继续旧计数）。
            TextRasterizer.OnRebuilt(null);
            Assert.AreEqual(1, TextRasterizer.FontVersion, "归零后 OnRebuilt 从 1 重新计");

            TextRasterizer.ResetStatic(); // 测试隔离：结束归零，避免污染其他测。
        }
    }
}
