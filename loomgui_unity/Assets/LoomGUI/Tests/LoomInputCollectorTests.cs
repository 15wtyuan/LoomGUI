using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    public class LoomInputCollectorTests
    {
        // screen (100,50) in 200x100 screen → design (54, 48) for rootSize (108,96) with y-flip
        //   design_x = screen_x / screen_w * root_w = 100/200*108 = 54
        //   design_y = root_h - (screen_y / screen_h * root_h) = 96 - (50/100*96) = 96-48 = 48
        // v1d.1：新 5-arg 签名，area=全屏 + useSafeArea=false 保持 v1c 行为（safe==full → 零回归）。
        [Test]
        public void ScreenToDesign_MapsCorrectly()
        {
            var design = LoomInputCollector.ScreenToDesign(
                new Vector2(100f, 50f), new Vector2Int(200, 100), new Vector2(108f, 96f),
                new Rect(0, 0, 200, 100), false);
            Assert.AreEqual(54f, design.x, 0.01f, "design_x = screen_x/screen_w*root_w");
            Assert.AreEqual(48f, design.y, 0.01f, "design_y = root_h - screen_y/screen_h*root_h（y-flip）");
        }

        // screen (0, 100) 左上（Unity 左下原点，y=100=顶部）→ design (0, 0)
        //   验 y-flip：Unity 顶部（y=screen_h）↦ LoomGUI 左上（design_y=0）
        [Test]
        public void ScreenToDesign_TopLeftScreen_IsTopLeftDesign()
        {
            var design = LoomInputCollector.ScreenToDesign(
                new Vector2(0f, 100f), new Vector2Int(200, 100), new Vector2(108f, 96f),
                new Rect(0, 0, 200, 100), false);
            Assert.AreEqual(0f, design.x, 0.01f);
            Assert.AreEqual(0f, design.y, 0.01f, "screen 顶部 → design y=0（左上原点）");
        }

        // screen 底部（y=0）↦ design 底部（design_y=root_h）—— y-flip 对称验。
        [Test]
        public void ScreenToDesign_BottomScreen_IsBottomDesign()
        {
            var design = LoomInputCollector.ScreenToDesign(
                new Vector2(0f, 0f), new Vector2Int(200, 100), new Vector2(108f, 96f),
                new Rect(0, 0, 200, 100), false);
            Assert.AreEqual(0f, design.x, 0.01f);
            Assert.AreEqual(96f, design.y, 0.01f, "screen 底部 → design y=root_h");
        }
    }
}
