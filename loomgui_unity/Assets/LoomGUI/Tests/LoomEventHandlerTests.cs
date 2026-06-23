using NUnit.Framework;
using System;
using System.Runtime.InteropServices;

namespace LoomGUI.Tests
{
    public class LoomEventHandlerTests
    {
        // 辅助：手搓 handler + handle（家里机 EditMode 需真 .dll；本机写测代码不跑）
        // node_parent 由核心 FFI 提供（Task 2），手搓场景 root(0)>parent(1)>child(2)
        // 测假设 Native.loomgui_node_parent 已加载（.dll 在 Plugins）
        //
        // 路由测需 Stage handle + scene（root>parent>child）。家里机实现者用 LoomStage 装载手搓包，
        // SetHandle 后注册 listener、DispatchPending 喂手搓 LoomEvent[]。本机只写测骨架 + 断言意图。

        /// v1c.2-T4：listener 表 + DispatchPending 分流。AddListener(5, Click) → DispatchPending
        /// 一条 EventRecord(nodeId=5,Click) → 回调被触发。v1c.2 签名是 EventCallback(ctx)（非 Action<LoomEvent>）。
        [Test]
        public void DispatchPending_RoutesToListener()
        {
            var handler = new LoomEventHandler();
            uint received = 0;
            LoomGUI.EventCallback cb = ctx => received = ctx.target;
            handler.AddListener(5u, EventType.Click, cb);

            var evt = new LoomEvent { nodeId = 5, type = EventType.Click, x = 10f, y = 20f };
            int recSize = Marshal.SizeOf<LoomEvent>();
            IntPtr ptr = Marshal.AllocHGlobal(recSize);
            Marshal.StructureToPtr(evt, ptr, false);
            try
            {
                handler.DispatchPending(ptr, 1);
                Assert.AreEqual(5u, received, "listener 应收到 target=5 的 Click");
            }
            finally { Marshal.FreeHGlobal(ptr); }
        }

        /// 无 listener → DispatchPending 不抛（no-op）。RollOver 走 DirectDispatch 直派。
        [Test]
        public void DispatchPending_NoListener_NoOp()
        {
            var handler = new LoomEventHandler();
            var evt = new LoomEvent { nodeId = 99, type = EventType.RollOver, x = 0, y = 0 };
            int recSize = Marshal.SizeOf<LoomEvent>();
            IntPtr ptr = Marshal.AllocHGlobal(recSize);
            Marshal.StructureToPtr(evt, ptr, false);
            Assert.DoesNotThrow(() => handler.DispatchPending(ptr, 1));
            Marshal.FreeHGlobal(ptr);
        }

        /// 多事件 buffer + 仅匹配的 listener 触发（验 i*recSize 偏移读对 + 分流到 BubbleRoute/DirectDispatch）。
        [Test]
        public void DispatchPending_MultipleEvents_OnlyMatchedListenerFires()
        {
            var handler = new LoomEventHandler();
            uint clickNodeId = 0;
            int moveCount = 0;
            LoomGUI.EventCallback clickCb = ctx => clickNodeId = ctx.target;
            LoomGUI.EventCallback moveCb = ctx => moveCount++;
            handler.AddListener(7u, EventType.Click, clickCb);
            handler.AddListener(3u, EventType.Move, moveCb);

            // 手搓 3 条 EventRecord buffer：[nodeId=3,Move], [nodeId=7,Click], [nodeId=99,RollOver]
            int recSize = Marshal.SizeOf<LoomEvent>();
            IntPtr ptr = Marshal.AllocHGlobal(recSize * 3);
            Marshal.StructureToPtr(new LoomEvent { nodeId = 3, type = EventType.Move, x = 1f, y = 1f }, ptr, false);
            Marshal.StructureToPtr(new LoomEvent { nodeId = 7, type = EventType.Click, x = 2f, y = 2f }, ptr + recSize, false);
            Marshal.StructureToPtr(new LoomEvent { nodeId = 99, type = EventType.RollOver, x = 3f, y = 3f }, ptr + recSize * 2, false);
            try
            {
                handler.DispatchPending(ptr, 3);
                Assert.AreEqual(7u, clickNodeId, "Click listener(nodeId=7) 应触发");
                Assert.AreEqual(1, moveCount, "Move listener(nodeId=3) 应触发 1 次");
            }
            finally { Marshal.FreeHGlobal(ptr); }
        }

        /// v1c.2-T3：对象池复用。Get → Return → Get 拿回同一实例。
        /// 注意：EventContext.Get() 只重置 _stopsPropagation/_defaultPrevented，不重置 payload（target 等）。
        [Test]
        public void EventContext_Pool_ReusesInstances()
        {
            var a = LoomGUI.EventContext.Get();
            LoomGUI.EventContext.Return(a);
            var b = LoomGUI.EventContext.Get();
            Assert.AreSame(a, b, "Return 后 Get 复用同实例（池复用）");
            LoomGUI.EventContext.Return(b);
        }

        /// v1c.2-T3：EventBridge 多播 —— Add 两个 callback → CallBubble 都触发；
        /// Remove 一个后只剩另一个。
        [Test]
        public void EventBridge_AddMultipleCallbacks_AllInvoked()
        {
            var bridge = new LoomGUI.EventBridge();
            int hits = 0;
            LoomGUI.EventCallback cb1 = _ => hits++;
            LoomGUI.EventCallback cb2 = _ => hits++;
            bridge.Add(cb1); bridge.Add(cb2);
            bridge.CallBubble(null);
            Assert.AreEqual(2, hits, "多播：两个 cb 都调");
            bridge.Remove(cb1);
            hits = 0; bridge.CallBubble(null);
            Assert.AreEqual(1, hits, "Remove cb1 后只 cb2");
        }

        // ===== v1c.2-T4 路由测骨架（家里机补 handle 装载 + 跑）=====
        // 下述测需 Stage handle + scene（root(0)>parent(1)>child(2)）。家里机用 LoomStage 装载
        // 手搓包，SetHandle 后注册 listener、DispatchPending 喂手搓 LoomEvent[]。
        // 本机写测骨架 + 断言意图；家里机补全 handle 装载（参考既有测的 setup 风格）。

        /// child(2) Down → bubble: child(2) Target, parent(1) Bubble, root(0) Bubble 都收。
        /// 断言：3 个节点 listener 都被调，phase 序 Capture(root)>Target(child)>Bubble(parent/root)。
        [Test]
        public void BubbleRoute_ReachesAllAncestors()
        {
            // 家里机：
            //   var stage = BuildStage("root>parent>child");  // node_id: root=0, parent=1, child=2
            //   var h = new LoomEventHandler();
            //   h.SetHandle((IntPtr)stage.NativeHandle);
            //   var hits = new System.Collections.Generic.List<(uint node, Phase phase)>();
            //   h.AddCapture(0, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // root capture（反向，根先）
            //   h.AddCapture(1, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // parent capture
            //   h.AddCapture(2, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // child capture
            //   h.AddListener(2, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // child target(bubble)
            //   h.AddListener(1, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // parent bubble
            //   h.AddListener(0, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // root bubble
            //   DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, x = 0, y = 0 });
            //   // capture（反向）：root(0,Capture) → parent(1,Capture) → child(2,Capture)
            //   // bubble（正向）：child(2,Target) → parent(1,Bubble) → root(0,Bubble)
            //   Assert.AreEqual(6, hits.Count, "3 节点 × 2 阶段(capture+bubble) = 6 hits");
            //   Assert.AreEqual((0u, Phase.Capture), hits[0], "capture 根先");
            //   Assert.AreEqual((2u, Phase.Target), hits[3], "bubble target 是 child");
            //   Assert.AreEqual((0u, Phase.Bubble), hits[5], "bubble 根最后");
            Assert.Ignore("家里机：需 Stage handle + scene（root>parent>child）+ .dll，本机无 Unity 跳过");
        }

        /// child bubble 回调 StopPropagation → parent/root bubble 不收；
        /// 但 capture 阶段（root→child 反向）已全跑（照 fgui line 302-311 不检查 stop）。
        [Test]
        public void StopPropagation_BreaksBubbleButNotCapture()
        {
            // 家里机：
            //   var stage = BuildStage("root>parent>child");
            //   var h = new LoomEventHandler(); h.SetHandle((IntPtr)stage.NativeHandle);
            //   int parentHits = 0, rootHits = 0, childCaptureHits = 0, rootCaptureHits = 0;
            //   h.AddListener(2, EventType.Down, c => c.StopPropagation());           // child bubble 止
            //   h.AddCapture(2, EventType.Down, c => childCaptureHits++);             // child capture（应跑）
            //   h.AddCapture(0, EventType.Down, c => rootCaptureHits++);              // root capture（应跑）
            //   h.AddListener(1, EventType.Down, c => parentHits++);                  // parent bubble（不收）
            //   h.AddListener(0, EventType.Down, c => rootHits++);                    // root bubble（不收）
            //   DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, x = 0, y = 0 });
            //   Assert.AreEqual(1, childCaptureHits, "capture 阶段不检查 stop，child capture 跑");
            //   Assert.AreEqual(1, rootCaptureHits, "capture 阶段不检查 stop，root capture 跑");
            //   Assert.AreEqual(0, parentHits, "child StopPropagation 后 parent bubble 不收");
            //   Assert.AreEqual(0, rootHits, "child StopPropagation 后 root bubble 不收");
            Assert.Ignore("家里机：需 Stage handle + scene（root>parent>child）+ .dll，本机无 Unity 跳过");
        }

        /// RollOver(child) → 只 child 收，parent/root 不收（直派非 bubble）。
        [Test]
        public void RollOver_DirectDispatch_NoBubble()
        {
            // 家里机：
            //   var stage = BuildStage("root>parent>child");
            //   var h = new LoomEventHandler(); h.SetHandle((IntPtr)stage.NativeHandle);
            //   int childHits = 0, parentHits = 0, rootHits = 0;
            //   h.AddListener(2, EventType.RollOver, c => childHits++);
            //   h.AddListener(1, EventType.RollOver, c => parentHits++);
            //   h.AddListener(0, EventType.RollOver, c => rootHits++);
            //   DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.RollOver, x = 0, y = 0 });
            //   Assert.AreEqual(1, childHits, "DirectDispatch：child 收");
            //   Assert.AreEqual(0, parentHits, "RollOver 不沿链，parent 不收");
            //   Assert.AreEqual(0, rootHits, "RollOver 不沿链，root 不收");
            Assert.Ignore("家里机：需 Stage handle + scene（root>parent>child）+ .dll，本机无 Unity 跳过");
        }

        /// root AddCapture(Down) → capture 阶段先于 child Target 收。
        [Test]
        public void AddCapture_FiresInCapturePhaseBeforeTarget()
        {
            // 家里机：
            //   var stage = BuildStage("root>parent>child");
            //   var h = new LoomEventHandler(); h.SetHandle((IntPtr)stage.NativeHandle);
            //   var order = new System.Collections.Generic.List<string>();
            //   h.AddCapture(0, EventType.Down, c => order.Add("root-capture"));   // root capture
            //   h.AddListener(2, EventType.Down, c => order.Add("child-target"));  // child target(bubble)
            //   DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, x = 0, y = 0 });
            //   Assert.AreEqual(2, order.Count);
            //   Assert.AreEqual("root-capture", order[0], "capture 阶段（反向从根）先于 target");
            //   Assert.AreEqual("child-target", order[1], "target(bubble) 后");
            Assert.Ignore("家里机：需 Stage handle + scene（root>parent>child）+ .dll，本机无 Unity 跳过");
        }

        /// RemoveListener(nodeId, type, cb) 后 cb 不再被调。
        [Test]
        public void DelegateRemove_StopsReceiving()
        {
            // 家里机：
            //   var stage = BuildStage("root>parent>child");
            //   var h = new LoomEventHandler(); h.SetHandle((IntPtr)stage.NativeHandle);
            //   int hits = 0;
            //   LoomGUI.EventCallback cb = c => hits++;
            //   h.AddListener(2, EventType.Down, cb);
            //   DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, x = 0, y = 0 });
            //   Assert.AreEqual(1, hits, "remove 前收 1 次");
            //   h.RemoveListener(2, EventType.Down, cb);
            //   DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, x = 0, y = 0 });
            //   Assert.AreEqual(1, hits, "remove 后不再收（仍 1）");
            Assert.Ignore("家里机：需 Stage handle + scene（root>parent>child）+ .dll，本机无 Unity 跳过");
        }
    }
}
