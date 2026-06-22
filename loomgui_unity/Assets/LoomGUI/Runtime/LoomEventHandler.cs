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
