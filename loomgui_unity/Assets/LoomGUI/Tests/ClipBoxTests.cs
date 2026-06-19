using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// T6: _ClipBox 纯数学测（§4.4，照搬 fgui UpdateContext.cs:105-156）。
    ///
    /// Unity EditMode 在本任务环境无法 headless 执行；此测试仅保证 ComputeClipBox 逻辑
    /// （design rect + 已知根 transform → 精确 _ClipBox 向量）正确。design→world 转换、
    /// safe-blank、符号约定（y-down design → y-up world）是 clip 正确性的核心，此处钉死。
    public class ClipBoxTests
    {
        /// 设计 rect {100,100,200,200}，根 scale=(1,-1,1) pos=0：
        ///   TL design (100,100) → world (100,-100)；BR design (300,300) → world (300,-300)。
        ///   center=(200,-200)；half=(|300-100|/2,|-300+100|/2)=(100,100)。
        ///   _ClipBox = (-cx/hw,-cy/hh,1/hw,1/hh) = (-200/100,-(-200)/100,1/100,1/100) = (-2,2,0.01,0.01)。
        [Test]
        public void ComputeClipBox_DesignRect_WorldCenterHalf()
        {
            var root = new GameObject("root").transform;
            root.localScale = new Vector3(1f, -1f, 1f);   // y-down design → y-up world
            root.position = Vector3.zero;

            // design rect {x=100,y=100,w=200,h=200}
            Vector4 box = ClipMath.ComputeClipBox(
                root,
                designX: 100f, designY: 100f, designW: 200f, designH: 200f);

            // 符号验：design y→world y 翻转，center.y=-200，故 _ClipBox.y=+2（正）。
            Assert.AreEqual(-2f, box.x, 1e-5f, "_ClipBox.x = -cx/hw");
            Assert.AreEqual(2f, box.y, 1e-5f, "_ClipBox.y = -cy/hh（cy=-200 → +2）");
            Assert.AreEqual(0.01f, box.z, 1e-5f, "_ClipBox.z = 1/hw");
            Assert.AreEqual(0.01f, box.w, 1e-5f, "_ClipBox.w = 1/hh");

            Object.DestroyImmediate(root.gameObject);
        }

        /// 根带平移（pos=(10,20)）+ scale=(1,-1,1)：TransformPoint 应正确叠加，
        /// _ClipBox 由 world center/half 决定（不手搓 (-sw/2,sh/2,sf) 公式）。
        [Test]
        public void ComputeClipBox_RootTranslated_TransformPointHonored()
        {
            var root = new GameObject("root").transform;
            root.localScale = new Vector3(1f, -1f, 1f);
            root.position = new Vector3(10f, 20f, 0f);

            // design {0,0,100,100}：
            //   TL (0,0) → world (10,20)；BR (100,100) → world (110,-80)。
            //   center=(60,-30)；half=(50,|−100|/2)=(50,50)。
            //   _ClipBox=(-60/50,-(-30)/50,1/50,1/50)=(-1.2,0.6,0.02,0.02)。
            Vector4 box = ClipMath.ComputeClipBox(root, 0f, 0f, 100f, 100f);
            Assert.AreEqual(-1.2f, box.x, 1e-4f);
            Assert.AreEqual(0.6f, box.y, 1e-4f);
            Assert.AreEqual(0.02f, box.z, 1e-5f);
            Assert.AreEqual(0.02f, box.w, 1e-5f);

            Object.DestroyImmediate(root.gameObject);
        }

        /// scale=(2,-2,2)（sf=2）：world half=scale×designHalf，_ClipBox.zw=1/(2×designHalf)。
        /// design {0,0,100,100}：TL→(0,0)，BR→(200,-200)；center=(100,-100)；half=(100,100)。
        /// _ClipBox=(-1,1,0.01,0.01)。
        [Test]
        public void ComputeClipBox_ScaleTwo_HalfScales()
        {
            var root = new GameObject("root").transform;
            root.localScale = new Vector3(2f, -2f, 2f);
            root.position = Vector3.zero;

            Vector4 box = ClipMath.ComputeClipBox(root, 0f, 0f, 100f, 100f);
            Assert.AreEqual(-1f, box.x, 1e-5f);
            Assert.AreEqual(1f, box.y, 1e-5f);
            Assert.AreEqual(0.01f, box.z, 1e-5f);
            Assert.AreEqual(0.01f, box.w, 1e-5f);

            Object.DestroyImmediate(root.gameObject);
        }

        /// 零面积（disjoint nested clip → empty）：half=0 → safe-blank (-2,-2,0,0)
        /// （fgui UpdateContext.cs:123-124）。clipPos=worldXY×(0,0)+(-2,-2)=(-2,-2)，
        /// max(abs)=2 > 1 → step(2,1)=0 → 全 discard（防除零）。
        [Test]
        public void ComputeClipBox_ZeroArea_SafeBlank()
        {
            var root = new GameObject("root").transform;
            root.localScale = new Vector3(1f, -1f, 1f);
            root.position = Vector3.zero;

            // w=0（零宽）→ half.x=0
            Vector4 box = ClipMath.ComputeClipBox(root, 10f, 10f, 0f, 100f);
            Assert.AreEqual(new Vector4(-2f, -2f, 0f, 0f), box, "_ClipBox.x=-2");
            Assert.AreEqual(0f, box.z, 1e-6f, "_ClipBox.z=0（safe-blank）");
            Assert.AreEqual(0f, box.w, 1e-6f, "_ClipBox.w=0（safe-blank）");

            Object.DestroyImmediate(root.gameObject);
        }

        /// h=0（零高）同样应产 safe-blank（half.y=0 分支）。
        [Test]
        public void ComputeClipBox_ZeroHeight_SafeBlank()
        {
            var root = new GameObject("root").transform;
            root.localScale = new Vector3(1f, -1f, 1f);
            root.position = Vector3.zero;

            Vector4 box = ClipMath.ComputeClipBox(root, 0f, 0f, 100f, 0f);
            Assert.AreEqual(0f, box.w, 1e-6f, "half.y=0 → _ClipBox.w=0");
            Assert.AreEqual(0f, box.z, 1e-6f, "safe-blank 整体 zw=0");

            Object.DestroyImmediate(root.gameObject);
        }
    }
}
