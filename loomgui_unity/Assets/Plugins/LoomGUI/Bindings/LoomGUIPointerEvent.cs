using System.Runtime.InteropServices;

namespace LoomGUI.Bindings
{
    /// v1c.1：csbindgen 不为 use-imported 的 Rust #[repr(C)] struct 生成 C# stub
    /// （只扫描 lib.rs 内 #[no_mangle] fn 签名，不追 use 路径）。
    /// PointerEvent 被 loomgui_stage_set_input 签名引用但无 stub → 手动补 C# 镜像，
    /// 放 LoomGUI.Bindings 程序集（与 Native/StageHandle 同集，签名可解析）。
    ///
    /// 字段序与 Rust loomgui_core::input::PointerEvent（#[repr(C)]）一致：
    ///   kind:u8, x:f32, y:f32, button:u8
    /// StructLayout.Sequential 默认 pack=0 → 按各自对齐 padding：
    ///   u8 @0 → pad 3 → f32 @4 → f32 @8 → u8 @12 → pad 3 → sizeof=16
    /// 与 Rust repr(C) 一致（Rust 同样按字段对齐 padding）。
    [StructLayout(LayoutKind.Sequential)]
    internal unsafe struct PointerEvent
    {
        public byte kind;    // 0=Down,1=Up,2=Move（Rust PointerKind）
        public float x;
        public float y;
        public byte button;
    }
}
