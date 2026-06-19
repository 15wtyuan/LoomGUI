//! FFI 导出层（§14.1 csbindgen）：extern "C" 薄包装，opaque Stage 句柄。
//! 命名前缀 `loomgui_`，csbindgen 扫描本文件生成 C# 绑定。

pub mod blob;

use std::ffi::CString;

/// 版本字符串（C null-terminated `b"v1a\0"`）。Task 1 工具链 round-trip 用。
///
/// 返回 `*const u8`（csbindgen 映射为 C# `byte*`）；CString::as_ptr 给的是
/// `*const c_char`（i8），这里 cast 对齐签名。OnceLock 缓存，避免每次分配+泄漏。
#[no_mangle]
pub extern "C" fn loomgui_version() -> *const u8 {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new("v1a").unwrap())
        .as_ptr() as *const u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_returns_c_string_v1a() {
        unsafe {
            let s = CStr::from_ptr(loomgui_version() as *const i8);
            assert_eq!(s.to_str().unwrap(), "v1a");
        }
    }
}
