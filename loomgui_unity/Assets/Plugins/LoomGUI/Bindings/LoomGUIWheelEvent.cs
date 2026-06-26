using System.Runtime.InteropServices;

namespace LoomGUI.Bindings
{
    /// v1d.5-T12：手补 C# 镜像（csbindgen 不为 use-imported 的 Rust #[repr(C)] struct 生成 stub
    /// ——只扫描 lib.rs 内 #[no_mangle] fn 签名，不追 use 路径；同 LoomGUIPointerEvent.cs 模式）。
    /// WheelEvent 被 loomgui_stage_set_wheel_input 签名引用但无 stub → 手动补。
    ///
    /// 字段序与 Rust loomgui_core::input::WheelEvent（#[repr(C)]）一致：
    ///   x: f32, y: f32, delta_x: f32, delta_y: f32
    /// StructLayout.Sequential（全 f32，无 padding）：
    ///   x@0(4) + y@4(4) + delta_x@8(4) + delta_y@12(4) = 16B
    /// 与 Rust repr(C) 一致。
    [StructLayout(LayoutKind.Sequential)]
    internal unsafe struct WheelEvent
    {
        public float x;        // design-space pointer x
        public float y;        // design-space pointer y
        public float delta_x;  // horizontal scroll delta (normalized ±1/notch)
        public float delta_y;  // vertical scroll delta (normalized ±1/notch)
    }
}
