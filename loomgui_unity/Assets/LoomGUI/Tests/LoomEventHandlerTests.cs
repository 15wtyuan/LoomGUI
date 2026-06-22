using NUnit.Framework;
using System;
using System.Runtime.InteropServices;

namespace LoomGUI.Tests
{
    public class LoomEventHandlerTests
    {
        /// listener 表路由：AddListener(5, Click) → DispatchPending 一条 EventRecord(nodeId=5,Click)
        /// → 回调被触发，收到 nodeId=5。
        /// 用 Marshal.StructureToPtr 手搓 EventRecord buffer（镜像 borrow_events 返回的 POD[] 内存），
        /// 传给 DispatchPending（签名收 IntPtr + count）。
        [Test]
        public void DispatchPending_RoutesToListener()
        {
            var handler = new LoomEventHandler();
            uint received = 0;
            handler.AddListener(5u, EventType.Click, e => received = e.nodeId);

            var evt = new LoomEvent { nodeId = 5, type = EventType.Click, x = 10f, y = 20f };
            int recSize = Marshal.SizeOf<LoomEvent>();
            IntPtr ptr = Marshal.AllocHGlobal(recSize);
            Marshal.StructureToPtr(evt, ptr, false);
            try
            {
                handler.DispatchPending(ptr, 1);
                Assert.AreEqual(5u, received, "listener 应收到 nodeId=5 的 Click");
            }
            finally { Marshal.FreeHGlobal(ptr); }
        }

        /// 无 listener → DispatchPending 不抛（no-op）。
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

        /// 多事件 buffer + 仅匹配的 listener 触发（验 i*recSize 偏移读对）。
        [Test]
        public void DispatchPending_MultipleEvents_OnlyMatchedListenerFires()
        {
            var handler = new LoomEventHandler();
            uint clickNodeId = 0;
            int moveCount = 0;
            handler.AddListener(7u, EventType.Click, e => clickNodeId = e.nodeId);
            handler.AddListener(3u, EventType.Move, e => moveCount++);

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
    }
}
