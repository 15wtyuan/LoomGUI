using System.Runtime.InteropServices;

namespace LoomGUI.Bindings
{
    /// v1d.2：手补 C# 镜像（csbindgen 不为 use-imported 的 Rust #[repr(C)] struct 生成 stub
    /// ——只扫描 lib.rs 内 #[no_mangle] fn 签名，不追 use 路径；同 LoomGUIPointerEvent.cs 模式）。
    /// KeyEvent 被 loomgui_stage_set_key_input 签名引用但无 stub → 手动补。
    ///
    /// 字段序与 Rust loomgui_core::input::KeyEvent（#[repr(C)]）一致：
    ///   key_code:u32, modifiers:u8, is_down:bool, pad:[u8;2]
    /// StructLayout.Sequential（pad0/pad1 显式补两字节，对齐 Rust [u8;2]）：
    ///   key_code@0(4) + modifiers@4(1) + is_down@5(1) + pad0@6(1) + pad1@7(1) = 8B
    /// 字段名（key_code/modifiers/is_down/pad0/pad1）与 LoomInputCollector.CollectKeys 消费侧一致。
    [StructLayout(LayoutKind.Sequential)]
    internal unsafe struct KeyEvent
    {
        public uint key_code;   // KeyCode 枚举值（Unity KeyCode 转 u32）
        public byte modifiers;  // bit0=shift / bit1=ctrl / bit2=alt
        public bool is_down;    // true=keydown；false=keyup
        public byte pad0;       // 显式 pad（Rust [u8;2] 两字节）
        public byte pad1;
    }
}
