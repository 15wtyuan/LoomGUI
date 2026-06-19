fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    // 生成一份到 OUT_DIR（Rust 测试/编译用）+ 一份直接落到 Unity Bindings 目录（入库参考）。
    csbindgen::Builder::default()
        .input_extern_file("src/lib.rs")
        .csharp_dll_name("loomgui_ffi_c")
        .csharp_namespace("LoomGUI.Bindings")
        .csharp_class_name("Native")
        .csharp_use_function_pointer(false)
        .generate_csharp_file(format!("{}/LoomGUIBindings.cs", out_dir))
        .expect("csbindgen csharp gen");

    // 落到 Unity（构建脚本不允许失败影响 cargo build，故忽略错误）。
    let unity_bindings = "../loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs";
    let _ = csbindgen::Builder::default()
        .input_extern_file("src/lib.rs")
        .csharp_dll_name("loomgui_ffi_c")
        .csharp_namespace("LoomGUI.Bindings")
        .csharp_class_name("Native")
        .csharp_use_function_pointer(false)
        .generate_csharp_file(unity_bindings);
}
