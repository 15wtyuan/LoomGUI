using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    public class LoomInputCollectorTests
    {
        // safe==全屏零回归验。用 aspect-matched rootSize（screen 2:1 ↔ root 2:1）→
        // sf=1、offX=0、offYTop=sh → 纯 y-flip 恒等映射（screen↔design 仅 y 翻转）。
        // 即"无刘海"屏语义：render 与 input 都退化为全屏映射。
        [Test]
        public void ScreenToDesign_MapsCorrectly()
        {
            // screen (100,50) in 200x100, root 200x100 (aspect-matched) → sf=1 → design (100, 50) y-flip = (100, 50)
            //   offX=0+(200-200*1)*0.5=0；offYTop=100；dx=(100-0)/1=100；dy=(100-50)/1=50
            var design = LoomInputCollector.ScreenToDesign(
                new Vector2(100f, 50f), new Vector2Int(200, 100), new Vector2(200f, 100f),
                new Rect(0, 0, 200, 100), false);
            Assert.AreEqual(100f, design.x, 0.01f, "aspect-matched sf=1 → design_x = screen_x");
            Assert.AreEqual(50f, design.y, 0.01f, "design_y = sh - screen_y（y-flip，sf=1）");
        }

        // screen (0, 100) 左上（Unity 左下原点，y=100=顶部）→ design (0, 0)
        //   验 y-flip：Unity 顶部（y=screen_h）↦ LoomGUI 左上（design_y=0）
        [Test]
        public void ScreenToDesign_TopLeftScreen_IsTopLeftDesign()
        {
            var design = LoomInputCollector.ScreenToDesign(
                new Vector2(0f, 100f), new Vector2Int(200, 100), new Vector2(200f, 100f),
                new Rect(0, 0, 200, 100), false);
            Assert.AreEqual(0f, design.x, 0.01f);
            Assert.AreEqual(0f, design.y, 0.01f, "screen 顶部 → design y=0（左上原点）");
        }

        // screen 底部（y=0）↦ design 底部（design_y=root_h）—— y-flip 对称验。
        [Test]
        public void ScreenToDesign_BottomScreen_IsBottomDesign()
        {
            var design = LoomInputCollector.ScreenToDesign(
                new Vector2(0f, 0f), new Vector2Int(200, 100), new Vector2(200f, 100f),
                new Rect(0, 0, 200, 100), false);
            Assert.AreEqual(0f, design.x, 0.01f);
            Assert.AreEqual(100f, design.y, 0.01f, "screen 底部 → design y=root_h");
        }

        // 刘海屏 round-trip 回归（render 前向 + ScreenToDesign 逆 → 原设计点）。
        // 用 ComputeRootTransform 同款前向公式把 design 映到 screen，再 ScreenToDesign 映回，
        // 断言 round-trip 误差 < epsilon——触控↔渲染对齐的根本保证。
        //
        // 场景：screenSize=(400,800)、rootSize=(200,400)、safe area=(40,0,320,800)（左侧 40px 刘海）。
        //   sf = min(320/200, 800/400) = 1.6（width-binding）
        //   rendered span = 320 × 640；safe 区 320×800 → 水平填满、垂直留白 160（上下各 80）
        //   offX = 40；offYTop = 800
        //   前向：screen.x = 40 + dx*1.6；screen.y = 800 - dy*1.6
        //   逆：  dx = (screen.x - 40)/1.6；dy = (800 - screen.y)/1.6 → 恒等回原 dx,dy ✓
        [Test]
        public void ScreenToDesign_NotchedSafeArea_RoundTrip()
        {
            var screenSize = new Vector2Int(400, 800);
            var rootSize = new Vector2(200f, 400f);
            var area = new Rect(40f, 0f, 320f, 800f);   // 左侧 40px 刘海
            // 与 ComputeRootTransform 同一公式（静态重算，避免依赖 Screen.safeArea）。
            float dw = rootSize.x, dh = rootSize.y;
            float sf = Mathf.Min(area.width / dw, area.height / dh);   // = 1.6
            float offX = area.x + (area.width - dw * sf) * 0.5f;       // = 40
            float offYTop = area.y + area.height;                      // = 800

            // 测多个设计点：四角 + 中心 + 刘海边缘。
            Vector2[] designPoints = new[]
            {
                new Vector2(0f, 0f),       // 左上（span 左上，恰在刘海右沿）
                new Vector2(200f, 0f),     // 右上
                new Vector2(0f, 400f),     // 左下
                new Vector2(200f, 400f),   // 右下
                new Vector2(100f, 200f),   // 中心
                new Vector2(50f, 350f),    // 刘海右沿附近
            };
            foreach (var d in designPoints)
            {
                // 前向：design → screen（ComputeRootTransform 同款）
                var screen = new Vector2(offX + d.x * sf, offYTop - d.y * sf);
                // 逆：screen → design
                var back = LoomInputCollector.ScreenToDesign(screen, screenSize, rootSize, area, true);
                Assert.AreEqual(d.x, back.x, 0.001f, $"round-trip dx 失败（design={d}, screen={screen}）");
                Assert.AreEqual(d.y, back.y, 0.001f, $"round-trip dy 失败（design={d}, screen={screen}）");
            }
        }
    }
}
