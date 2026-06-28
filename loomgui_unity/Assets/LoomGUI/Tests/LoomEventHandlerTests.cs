using NUnit.Framework;
using System;
using System.Runtime.InteropServices;
using LoomGUI.Bindings;

namespace LoomGUI.Tests
{
    public unsafe class LoomEventHandlerTests
    {
        // 路由测需 Stage handle + scene（root>parent>child）。

        // BuildStage 装载 root>parent>child（node_id: root=0,parent=1,child=2）。
        // font_path 需真路径（DejaVuSans.ttf）——core stage_new 解析字体，null/0 会返 null。
        // 需传 Application.streamingAssetsPath 或 Assets/LoomGUI/Fonts/DejaVuSans.ttf 的 UTF-8 字节。
        static (IntPtr stage, LoomEventHandler handler) BuildStage()
        {
            // font_path 占位：未补真路径时 stage_new 会因 font 解析失败返 null。
            // 补法：System.IO.Path.Combine(Application.streamingAssetsPath, "DejaVuSans.ttf")
            //       byte[] fp = Encoding.UTF8.GetBytes(fontPath); fixed(byte* fpp=fp) stage = Native.loomgui_stage_new(fpp, (nuint)fp.Length, 200f, 200f);
            byte[] fontPathBytes = null; // 占位：需补真路径
            StageHandle* stagePtr;
            fixed (byte* fp = fontPathBytes)
            {
                stagePtr = Native.loomgui_stage_new(fp, (nuint)(fontPathBytes?.Length ?? 0), 200f, 200f);
            }
            // 若 stagePtr == null，说明 font_path 占位未补——补真路径后重试。
            // 注意：不能用 Assert.IsNotNull(stagePtr, ...) —— 指针装箱后恒非 null（boxed 0 也是对象），
            // 是 no-op；stagePtr 是 StageHandle*，用 != null 真比较。
            Assert.IsTrue(stagePtr != null, "BuildStage: stage_new 返 null（font_path 占位未补？需填真路径）");

            string html = "<div class=\"root\"><div class=\"parent\"><div class=\"child\"></div></div></div>";
            string css = ".root{width:200px;height:200px;}.parent{width:100px;height:100px;}.child{width:50px;height:50px;}";
            byte[] htmlBytes = System.Text.Encoding.UTF8.GetBytes(html);
            byte[] cssBytes = System.Text.Encoding.UTF8.GetBytes(css);
            fixed (byte* hp = htmlBytes, cp = cssBytes)
            {
                int r = Native.loomgui_stage_load_html(stagePtr, hp, (nuint)htmlBytes.Length, cp, (nuint)cssBytes.Length);
                Assert.AreEqual(0, r, "BuildStage: load_html 失败");
            }
            var handler = new LoomEventHandler();
            handler.SetHandle((IntPtr)stagePtr);
            return ((IntPtr)stagePtr, handler);
        }

        // 手搓单条 LoomEvent → marshal ptr → DispatchPending。释放 ptr。
        static void DispatchOne(LoomEventHandler handler, LoomEvent evt)
        {
            int recSize = Marshal.SizeOf<LoomEvent>();
            IntPtr ptr = Marshal.AllocHGlobal(recSize);
            Marshal.StructureToPtr(evt, ptr, false);
            try { handler.DispatchPending(ptr, 1); }
            finally { Marshal.FreeHGlobal(ptr); }
        }

        /// listener 表 + DispatchPending 分流。AddListener(5, Click) → DispatchPending
        /// 一条 EventRecord(nodeId=5,Click) → 回调被触发。
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

        /// 对象池复用。Get → Return → Get 拿回同一实例。
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

        /// EventBridge 多播 —— Add 两个 callback → CallBubble 都触发；Remove 一个后只剩另一个。
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

        // ===== 路由测 =====
        // 下述测需 Stage handle + scene（root(0)>parent(1)>child(2)）。BuildStage 装载手搓 html/css，
        // SetHandle 后注册 listener、DispatchOne 喂手搓 LoomEvent。

        /// child(2) Down → bubble: child(2) Target, parent(1) Bubble, root(0) Bubble 都收。
        /// 断言：3 个节点 listener 都被调，phase 序 Capture(root)>Target(child)>Bubble(parent/root)。
        [Test]
        public void BubbleRoute_ReachesAllAncestors()
        {
            var (stage, h) = BuildStage();
            var hits = new System.Collections.Generic.List<(uint node, LoomGUI.Phase phase)>();
            h.AddCapture(0, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // root capture（反向，根先）
            h.AddCapture(1, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // parent capture
            h.AddCapture(2, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // child capture
            h.AddListener(2, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // child target(bubble)
            h.AddListener(1, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // parent bubble
            h.AddListener(0, EventType.Down, c => hits.Add((c.currentTarget, c.phase)));  // root bubble
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            // capture（反向）：root(0,Capture) → parent(1,Capture) → child(2,Capture)
            // bubble（正向）：child(2,Target) → parent(1,Bubble) → root(0,Bubble)
            Assert.AreEqual(6, hits.Count, "3 节点 × 2 阶段(capture+bubble) = 6 hits");
            Assert.AreEqual((0u, LoomGUI.Phase.Capture), hits[0], "capture 根先");
            Assert.AreEqual((2u, LoomGUI.Phase.Target), hits[3], "bubble target 是 child");
            Assert.AreEqual((0u, LoomGUI.Phase.Bubble), hits[5], "bubble 根最后");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// child bubble 回调 StopPropagation → parent/root bubble 不收；
        /// 但 capture 阶段（root→child 反向）已全跑。
        [Test]
        public void StopPropagation_BreaksBubbleButNotCapture()
        {
            var (stage, h) = BuildStage();
            int parentHits = 0, rootHits = 0, childCaptureHits = 0, rootCaptureHits = 0;
            h.AddListener(2, EventType.Down, c => c.StopPropagation());           // child bubble 止
            h.AddCapture(2, EventType.Down, c => childCaptureHits++);             // child capture（应跑）
            h.AddCapture(0, EventType.Down, c => rootCaptureHits++);              // root capture（应跑）
            h.AddListener(1, EventType.Down, c => parentHits++);                  // parent bubble（不收）
            h.AddListener(0, EventType.Down, c => rootHits++);                    // root bubble（不收）
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(1, childCaptureHits, "capture 阶段不检查 stop，child capture 跑");
            Assert.AreEqual(1, rootCaptureHits, "capture 阶段不检查 stop，root capture 跑");
            Assert.AreEqual(0, parentHits, "child StopPropagation 后 parent bubble 不收");
            Assert.AreEqual(0, rootHits, "child StopPropagation 后 root bubble 不收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// RollOver(child) → 只 child 收，parent/root 不收（直派非 bubble）。
        [Test]
        public void RollOver_DirectDispatch_NoBubble()
        {
            var (stage, h) = BuildStage();
            int childHits = 0, parentHits = 0, rootHits = 0;
            h.AddListener(2, EventType.RollOver, c => childHits++);
            h.AddListener(1, EventType.RollOver, c => parentHits++);
            h.AddListener(0, EventType.RollOver, c => rootHits++);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.RollOver, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(1, childHits, "DirectDispatch：child 收");
            Assert.AreEqual(0, parentHits, "RollOver 不沿链，parent 不收");
            Assert.AreEqual(0, rootHits, "RollOver 不沿链，root 不收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// root AddCapture(Down) → capture 阶段先于 child Target 收。
        [Test]
        public void AddCapture_FiresInCapturePhaseBeforeTarget()
        {
            var (stage, h) = BuildStage();
            var order = new System.Collections.Generic.List<string>();
            h.AddCapture(0, EventType.Down, c => order.Add("root-capture"));   // root capture
            h.AddListener(2, EventType.Down, c => order.Add("child-target"));  // child target(bubble)
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(2, order.Count);
            Assert.AreEqual("root-capture", order[0], "capture 阶段（反向从根）先于 target");
            Assert.AreEqual("child-target", order[1], "target(bubble) 后");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// RemoveListener(nodeId, type, cb) 后 cb 不再被调。
        [Test]
        public void DelegateRemove_StopsReceiving()
        {
            var (stage, h) = BuildStage();
            int hits = 0;
            LoomGUI.EventCallback cb = c => hits++;
            h.AddListener(2, EventType.Down, cb);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(1, hits, "remove 前收 1 次");
            h.RemoveListener(2, EventType.Down, cb);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(1, hits, "remove 后不再收（仍 1）");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        // ===== capture / move / multitouch 测 =====

        /// CaptureTouch 设 _touchCapture；BubbleRoute 消费即清（cap/bub 各记录一节点）。
        /// 验：root AddCapture(Down, CaptureTouch) + child AddListener(Down, CaptureTouch)
        ///     → DispatchOne(Down on child) 后 _captureNodeCap=root, _captureNodeBub=child，
        ///     核心收到两次 add_touch_monitor（同 touch_id）。
        /// 注：核心侧观测 API 无直接断言——这里验不抛 + 两次 CaptureTouch 调用都执行（handler 内部转 add_touch_monitor）。
        [Test]
        public void CaptureTouch_SetsFlag_ConsumedOnCapAndBub()
        {
            var (stage, h) = BuildStage();
            int captureCalls = 0;
            // root capture + child bubble 各调 CaptureTouch；不抛即过（标志消费 + 转发核心 add_touch_monitor）。
            h.AddCapture(0, EventType.Down, c => { c.CaptureTouch(); captureCalls++; });   // root capture 调 CaptureTouch
            h.AddListener(2, EventType.Down, c => { c.CaptureTouch(); captureCalls++; });  // child bubble 调 CaptureTouch
            Assert.DoesNotThrow(() =>
                DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 }));
            Assert.AreEqual(2, captureCalls, "cap + bub 两个节点都调 CaptureTouch（消费 _touchCapture）");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// Move 走 DirectDispatch（不再 BubbleRoute）：只命中节点收，不沿链。
        /// 验：root>parent>child 场景，child Move → 只 child 收，parent/root 不收。
        [Test]
        public void Move_DirectDispatch_NoBubble()
        {
            var (stage, h) = BuildStage();
            int childHits = 0, parentHits = 0, rootHits = 0;
            h.AddListener(2, EventType.Move, c => childHits++);
            h.AddListener(1, EventType.Move, c => parentHits++);
            h.AddListener(0, EventType.Move, c => rootHits++);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Move, clickCount = 0, touch_id = -1, x = 5f, y = 5f });
            Assert.AreEqual(1, childHits, "Move 直派：child 收");
            Assert.AreEqual(0, parentHits, "Move 不沿链，parent 不收");
            Assert.AreEqual(0, rootHits, "Move 不沿链，root 不收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// 两触摸（touch_id=0,1）Down 在同一节点（child=2）→ EventContext.touchId 各自正确 + 互不干扰。
        /// 验：listener 收到的 ctx.touchId 与 EventRecord.touch_id 一致。
        [Test]
        public void MultiTouch_DistinctTouchId()
        {
            var (stage, h) = BuildStage();
            int recTouchId = -99;
            h.AddListener(2, EventType.Down, c => recTouchId = c.touchId);
            // 喂 touch_id=0 的 Down on child(2)
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = 0, x = 0, y = 0 });
            Assert.AreEqual(0, recTouchId, "touch_id=0 的 Down → ctx.touchId=0");
            // 喂 touch_id=1 的 Down on child(2)
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = 1, x = 0, y = 0 });
            Assert.AreEqual(1, recTouchId, "touch_id=1 的 Down → ctx.touchId=1");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// RemoveTouchMonitor(nodeId) 主动释放——转发核心 remove（业务拖拽结束调）。
        /// 验：不抛（核心侧 remove 幂等）；后续 Move 不再派到该节点。
        [Test]
        public void RemoveTouchMonitor_NoThrow_FreesCapture()
        {
            var (stage, h) = BuildStage();
            Assert.DoesNotThrow(() => h.RemoveTouchMonitor(2), "remove 不存在的 monitor 应 no-op");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        // ===== StopImmediate + 双击 clickCount 测 =====

        /// StopImmediatePropagation 止同节点剩余监听器（StopPropagation 不会）。
        /// 单节点链（node 2，无祖先需 BuildStage 的多节点链；但用 BuildStage 保持一致性）。
        [Test]
        public void StopImmediate_StopsSiblingListenersOnSameNode()
        {
            var (stage, h) = BuildStage();
            int hit1 = 0, hit2 = 0;
            h.AddListener(2, EventType.Down, c => { hit1++; c.StopImmediatePropagation(); });   // 第一个：止同节点剩余
            h.AddListener(2, EventType.Down, c => { hit2++; });                                  // 同节点第二个：不应跑
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Down, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(1, hit1, "第一个 listener 跑");
            Assert.AreEqual(0, hit2, "StopImmediate 止同节点第二个 listener");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// 双击 clickCount 透传 EventContext（core 算 2，C# 读 ctx.clickCount=2）。
        /// 手搓 Click 事件 clickCount=2 → listener 收 ctx.clickCount=2 + isDoubleClick=true。
        [Test]
        public void DoubleClick_ClickCount_ReachesEventContext()
        {
            var (stage, h) = BuildStage();
            byte recvCount = 0; bool recvDouble = false;
            h.AddListener(2, EventType.Click, c => { recvCount = c.clickCount; recvDouble = c.isDoubleClick; });
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.Click, clickCount = 2, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(2, recvCount, "clickCount=2 透传");
            Assert.IsTrue(recvDouble, "isDoubleClick=true");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        // ===== drag/longpress BubbleRoute 路由测 =====

        /// DragStart 走 BubbleRoute——child(2) DragStart → child Target + parent(1) + root(0) Bubble 都收。
        [Test]
        public void DragStart_BubbleRoute_ReachesAncestors()
        {
            var (stage, h) = BuildStage();
            int childHits = 0, parentHits = 0, rootHits = 0;
            h.AddListener(2, EventType.DragStart, c => childHits++);
            h.AddListener(1, EventType.DragStart, c => parentHits++);
            h.AddListener(0, EventType.DragStart, c => rootHits++);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.DragStart, clickCount = 0, touch_id = -1, x = 5f, y = 5f });
            Assert.AreEqual(1, childHits, "DragStart bubble：child 收");
            Assert.AreEqual(1, parentHits, "DragStart bubble：parent 收");
            Assert.AreEqual(1, rootHits, "DragStart bubble：root 收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// LongPress 走 BubbleRoute——child(2) LongPress → 祖先链都收。
        [Test]
        public void LongPress_BubbleRoute_ReachesAncestors()
        {
            var (stage, h) = BuildStage();
            int childHits = 0, parentHits = 0, rootHits = 0;
            h.AddListener(2, EventType.LongPress, c => childHits++);
            h.AddListener(1, EventType.LongPress, c => parentHits++);
            h.AddListener(0, EventType.LongPress, c => rootHits++);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.LongPress, clickCount = 0, touch_id = -1, x = 0, y = 0 });
            Assert.AreEqual(1, childHits, "LongPress bubble：child 收");
            Assert.AreEqual(1, parentHits, "LongPress bubble：parent 收");
            Assert.AreEqual(1, rootHits, "LongPress bubble：root 收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        // ===== keydown/focusin BubbleRoute 路由测 =====

        /// KeyDown 走 BubbleRoute——child(2) KeyDown → child Target + parent(1) + root(0) Bubble 都收。
        /// key_code 复用 touch_id=13（Return），modifiers=0（EventRecord pad[0] @6，非 key 事件=0 正确）。
        [Test]
        public void KeyDown_BubbleRoute_ReachesAncestors()
        {
            var (stage, h) = BuildStage();
            int childHits = 0, parentHits = 0, rootHits = 0;
            h.AddListener(2, EventType.KeyDown, c => childHits++);
            h.AddListener(1, EventType.KeyDown, c => parentHits++);
            h.AddListener(0, EventType.KeyDown, c => rootHits++);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.KeyDown, clickCount = 0, modifiers = 0, touch_id = 13, x = 0f, y = 0f });
            Assert.AreEqual(1, childHits, "KeyDown bubble：child 收");
            Assert.AreEqual(1, parentHits, "KeyDown bubble：parent 收");
            Assert.AreEqual(1, rootHits, "KeyDown bubble：root 收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }

        /// FocusIn 走 BubbleRoute——child(2) FocusIn → child + parent(1) + root(0) 都收。
        [Test]
        public void FocusIn_BubbleRoute_ReachesAncestors()
        {
            var (stage, h) = BuildStage();
            int childHits = 0, parentHits = 0, rootHits = 0;
            h.AddListener(2, EventType.FocusIn, c => childHits++);
            h.AddListener(1, EventType.FocusIn, c => parentHits++);
            h.AddListener(0, EventType.FocusIn, c => rootHits++);
            DispatchOne(h, new LoomEvent { nodeId = 2, type = EventType.FocusIn, clickCount = 0, modifiers = 0, touch_id = 0, x = 0f, y = 0f });
            Assert.AreEqual(1, childHits, "FocusIn bubble：child 收");
            Assert.AreEqual(1, parentHits, "FocusIn bubble：parent 收");
            Assert.AreEqual(1, rootHits, "FocusIn bubble：root 收");
            Native.loomgui_stage_free((StageHandle*)stage);
        }
    }
}
