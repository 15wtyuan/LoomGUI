using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace LoomGUI
{
    /// EventType 与 Rust loomgui_core::input::EVT_* 常量一致（:byte 对齐 event_type:u8）。
    public enum EventType : byte
    {
        Down = 0,
        Up = 1,
        Move = 2,
        Click = 3,
        RollOver = 4,
        RollOut = 5,
    }

    /// C# 镜像 Rust loomgui_core::input::EventRecord（#[repr(C)]）。
    /// 字段序：node_id:u32 @0 → event_type:u8 @4 → pad 3 → x:f32 @8 → y:f32 @12（sizeof=16）。
    /// 与 Rust ABI 一致（StructLayout.Sequential 默认 pack=0）。
    [StructLayout(LayoutKind.Sequential)]
    public struct LoomEvent
    {
        public uint nodeId;
        public EventType type;
        public float x;
        public float y;
    }

    /// v1c.2 事件路由阶段（Capture/Target/Bubble），对齐 DOM/W3C 模型 + fgui capture/bubble 双组。
    public enum Phase : byte { Capture = 0, Target = 1, Bubble = 2 }

    /// 业务回调签名（listener 收 EventContext 读命中/坐标/止冒泡）。对齐 fgui EventCallback1。
    public delegate void EventCallback(EventContext ctx);

    /// EventContext（照 fgui EventContext.cs，对象池复用）。
    public class EventContext
    {
        public uint target;            // 原始命中（EventRecord.node_id）
        public uint currentTarget;     // 路由当前节点
        public Phase phase;
        public EventType type;
        public float x, y;
        internal bool _stopsPropagation, _defaultPrevented;
        public void StopPropagation() => _stopsPropagation = true;
        public void PreventDefault() => _defaultPrevented = true;

        static readonly System.Collections.Generic.Stack<EventContext> _pool = new();
        internal static EventContext Get()
        {
            var ctx = _pool.Count > 0 ? _pool.Pop() : new EventContext();
            ctx._stopsPropagation = false; ctx._defaultPrevented = false;
            return ctx;
        }
        internal static void Return(EventContext ctx) => _pool.Push(ctx);

        [UnityEngine.RuntimeInitializeOnLoadMethod(UnityEngine.RuntimeInitializeLoadType.SubsystemRegistration)]
        static void ClearPool() => _pool.Clear();   // Domain reload 清池（照 fgui EventContext.cs:96-102）
    }

    /// EventBridge（照 fgui EventBridge.cs）：capture + bubble 两组多播委托。
    internal class EventBridge
    {
        EventCallback _bubble, _capture;
        public void Add(EventCallback cb) { _bubble -= cb; _bubble += cb; }       // -= 去重
        public void AddCapture(EventCallback cb) { _capture -= cb; _capture += cb; }
        public void Remove(EventCallback cb) => _bubble -= cb;
        public void RemoveCapture(EventCallback cb) => _capture -= cb;
        public bool isEmpty => _bubble == null && _capture == null;
        public void CallBubble(EventContext ctx) { if (_bubble != null) _bubble(ctx); }
        public void CallCapture(EventContext ctx) { if (_capture != null) _capture(ctx); }
    }

    /// C# 侧 listener 表 + 派发（对齐 fgui listener 在 C# 域，核心只产事件）。
    /// v1c.1 单 callback/type（AddListener 覆盖）；v1c.2 多 callback 用 List。
    public class LoomEventHandler
    {
        readonly Dictionary<uint, Dictionary<EventType, Action<LoomEvent>>> _listeners = new();

        public void AddListener(uint nodeId, EventType type, Action<LoomEvent> cb)
        {
            if (!_listeners.TryGetValue(nodeId, out var byType))
                _listeners[nodeId] = byType = new Dictionary<EventType, Action<LoomEvent>>();
            byType[type] = cb;
        }

        public void RemoveListener(uint nodeId, EventType type)
        {
            if (_listeners.TryGetValue(nodeId, out var byType))
                byType.Remove(type);
        }

        /// 读 EventRecord[] buffer → 逐条查表派发。ptr = loomgui_stage_borrow_events 返回。
        ///
        /// **v1c.1 诚实简化**：用 Marshal.PtrToStructure 读 EventRecord。桌面 Mono backend OK；
        /// IL2CPP 移动端有对齐坑（spec §14.3 禁用 Marshal.PtrToStructure），届时换
        /// `Span&lt;byte&gt;` + BinaryPrimitives / Unsafe.ReadUnaligned 读。v1c.1 桌面优先。
        public void DispatchPending(IntPtr ptr, int count)
        {
            if (ptr == IntPtr.Zero || count <= 0) return;
            int recSize = Marshal.SizeOf<LoomEvent>();
            for (int i = 0; i < count; i++)
            {
                var evt = Marshal.PtrToStructure<LoomEvent>(ptr + i * recSize);
                if (_listeners.TryGetValue(evt.nodeId, out var byType) && byType.TryGetValue(evt.type, out var cb))
                    cb(evt);
            }
        }
    }
}
