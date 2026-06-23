using System.Runtime.InteropServices;

namespace LoomGUI.Bindings
{
    /// v1c.3：手补 C# 镜像（csbindgen 不为 use-imported 的 Rust #[repr(C)] struct 生成 stub
    /// ——只扫描 lib.rs 内 #[no_mangle] fn 签名，不追 use 路径）。
    /// PointerEvent 被 loomgui_stage_set_input 签名引用但无 stub → 手动补，放 LoomGUI.Bindings
    /// 程序集（与 Native/StageHandle 同集，签名可解析）。
    ///
    /// 字段序与 Rust loomgui_core::input::PointerEvent（#[repr(C)], PointerKind repr(u8)）一致：
    ///   kind:u8, button:u8, pad:[u8;2], touch_id:i32, x:f32, y:f32
    /// StructLayout.Sequential（pad0/pad1 显式补两字节，对齐 Rust [u8;2]）：
    ///   kind@0(1) + button@1(1) + pad0@2(1) + pad1@3(1) + touch_id@4(4) + x@8(4) + y@12(4) = 16B
    /// 与 Rust repr(C) 一致（v1c.3：+touch_id，PointerKind repr(u8) 1B 判别）。
    [StructLayout(LayoutKind.Sequential)]
    internal unsafe struct PointerEvent
    {
        public byte kind;        // 0=Down,1=Up,2=Move（Rust PointerKind repr(u8)）
        public byte button;
        public byte pad0;        // 显式 pad（Rust [u8;2] 的两字节）
        public byte pad1;
        public int touch_id;     // v1c.3：-1=鼠标主指，>=0=触摸 fingerId
        public float x;
        public float y;
    }
}
