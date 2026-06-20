//! FFI 导出层（§14.1 csbindgen）：extern "C" 薄包装，opaque Stage 句柄。
//! 命名前缀 `loomgui_`，csbindgen 扫描本文件生成 C# 绑定。

pub mod blob;

use std::ffi::CString;
use loomgui_core::stage::Stage;

/// 版本字符串（C null-terminated `b"v1b.2\0"`）。Task 1 工具链 round-trip 用。
///
/// 返回 `*const u8`（csbindgen 映射为 C# `byte*`）；CString::as_ptr 给的是
/// `*const c_char`（i8），这里 cast 对齐签名。OnceLock 缓存，避免每次分配+泄漏。
#[no_mangle]
pub extern "C" fn loomgui_version() -> *const u8 {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new("v1b.2").unwrap())
        .as_ptr() as *const u8
}

/// opaque 句柄：Stage + 缓存的最近一帧 blob（borrow_frame 返回它的指针，下帧 reset）。
pub struct StageHandle {
    stage: Stage,
    frame_blob: Vec<u8>, // borrow_frame 返回 &this[..]；tick 时被覆盖。
}

/// 创建 Stage 句柄。`font_path` 为 UTF-8 字节（指针+len），失败返回 null。
#[no_mangle]
pub extern "C" fn loomgui_stage_new(
    font_path: *const u8,
    fp_len: usize,
    w: f32,
    h: f32,
) -> *mut StageHandle {
    if font_path.is_null() {
        return std::ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(font_path, fp_len) };
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let stage = match Stage::new(path, (w, h)) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    Box::into_raw(Box::new(StageHandle {
        stage,
        frame_blob: Vec::new(),
    }))
}

/// null-safe 释放 Stage 句柄。
#[no_mangle]
pub extern "C" fn loomgui_stage_free(h: *mut StageHandle) {
    if h.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(h));
    }
}

/// 装载 HTML+CSS 文本（指针+len）。0=ok，-1=err。null/非 UTF-8 返回 -1。
///
/// **parse-gated：**本函数走核心 HTML/CSS 解析路径，`--no-default-features` 关掉 parse 时不存在。
/// 包加载路径走 `loomgui_stage_load_package`（常驻，不 gate）。
#[cfg(feature = "parse")]
#[no_mangle]
pub extern "C" fn loomgui_stage_load_html(
    h: *mut StageHandle,
    html: *const u8,
    html_len: usize,
    css: *const u8,
    css_len: usize,
) -> i32 {
    if h.is_null() || html.is_null() || css.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let html_bytes = unsafe { std::slice::from_raw_parts(html, html_len) };
    let css_bytes = unsafe { std::slice::from_raw_parts(css, css_len) };
    let html = match std::str::from_utf8(html_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let css = match std::str::from_utf8(css_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    match sh.stage.load_inline(html, css) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// 装载二进制包（spec §12/§13）。bytes = .pkg.bin（指针+len）。0=ok，-1=err。
/// null 句柄/空指针返回 -1。包是 Rust-internal，C# 只透传 bytes（不解析）。
///
/// **常驻（不 gate）：**包格式是 runtime 的稳定入口，不依赖 parse feature——
/// `--no-default-features` 构建的 .dll 仍有本函数（Unity 用 default 带 parse 的 dev .dll）。
#[no_mangle]
pub extern "C" fn loomgui_stage_load_package(
    h: *mut StageHandle,
    bytes: *const u8,
    len: usize,
) -> i32 {
    if h.is_null() || bytes.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let bytes = unsafe { std::slice::from_raw_parts(bytes, len) };
    match sh.stage.load_package(bytes) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// 注册纹理：core 分配 tex_id（src 幂等），存 w/h 供 measure。返回 tex_id(>=1)；
/// 非 UTF-8 src / null → 返回 0（哨兵，不 panic）。
#[no_mangle]
pub extern "C" fn loomgui_stage_register_texture(
    h: *mut StageHandle,
    src: *const u8,
    src_len: usize,
    width: u32,
    height: u32,
) -> u32 {
    if h.is_null() || src.is_null() { return 0; }
    let sh = unsafe { &mut *h };
    let bytes = unsafe { std::slice::from_raw_parts(src, src_len) };
    match std::str::from_utf8(bytes) {
        Ok(s) => sh.stage.textures.register(s, width, height),
        Err(_) => 0,
    }
}

/// collect：scene 中 Image src 去重后的数量（DFS 先序）。
#[no_mangle]
pub extern "C" fn loomgui_stage_image_src_count(h: *const StageHandle) -> usize {
    if h.is_null() { return 0; }
    let sh = unsafe { &*h };
    sh.stage.image_srcs().len()
}

/// collect：第 index 个 Image src 的 UTF-8 串指针；*out_len = 字节长。
/// OOB / null → null。串归 Stage 拥有，下次 load 前有效。
#[no_mangle]
pub extern "C" fn loomgui_stage_image_src_at(
    h: *const StageHandle,
    index: usize,
    out_len: *mut usize,
) -> *const u8 {
    if h.is_null() { return std::ptr::null(); }
    let sh = unsafe { &*h };
    let srcs = sh.stage.image_srcs();
    if index >= srcs.len() { return std::ptr::null(); }
    let s = &srcs[index];
    if !out_len.is_null() { unsafe { *out_len = s.len(); } }
    s.as_ptr()
}

/// 跑一帧 tick_and_render → build_blob 写入缓存。v0 无 dt 语义（忽略）。
#[no_mangle]
pub extern "C" fn loomgui_stage_tick(h: *mut StageHandle, _dt: f32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    let frame = sh.stage.tick_and_render();
    sh.frame_blob = blob::build_blob(&frame);
}

/// 借出最近一帧 blob：写 len 到 out_len，返回 Rust 拥有缓存指针（下 tick 失效）。
/// null 句柄或未 tick 过返回 null + len=0。
#[no_mangle]
pub extern "C" fn loomgui_stage_borrow_frame(
    h: *mut StageHandle,
    out_len: *mut usize,
) -> *const u8 {
    if h.is_null() {
        if !out_len.is_null() {
            unsafe { *out_len = 0 };
        }
        return std::ptr::null();
    }
    let sh = unsafe { &*h };
    // 未 tick 过：frame_blob 是空 Vec，as_ptr() 返回非空悬挂哨兵（违反"未 tick→null"契约）。
    // 显式判空 → null + len=0，与 null-handle 分支一致。
    if sh.frame_blob.is_empty() {
        if !out_len.is_null() {
            unsafe { *out_len = 0 };
        }
        return std::ptr::null();
    }
    if !out_len.is_null() {
        unsafe { *out_len = sh.frame_blob.len() };
    }
    sh.frame_blob.as_ptr()
}

/// 全局 shutdown（Domain reload hook）。C# `LoomStage.ResetStatics`（SubsystemRegistration）
/// 调用本函数——即使当前核心无全局态，hook 必须存在：v1b 引入 global texture/font registry
/// 时此处自动清，无需再改接线。
///
/// **v1a：near-no-op（诚实）。**核心无全局 native 态——Stage 是 per-handle（`loomgui_stage_free`
/// drop 全部 Stage 拥有的内存）。
///
/// **注意：Font 的 `Box::leak`（`text/layout.rs:76`）是真泄漏**——`bytes.clone()` 后 leak 取
/// `'static` 切片喂 ttf-parser Face，原 Vec 虽被 `_bytes` 持有但与 leaked 切片**不是同一份**，
/// Stage drop 时 `_bytes` 释放的是 clone 来源而非 leaked 副本。每次 Stage 创建（`loomgui_stage_new`
/// → Font）都 leak 一份字体字节。这是 v0 已知简化（§4.6），**不可由 shutdown 回收**（leak 切片
/// 无 handle 跟踪）——除非 Stage 句柄侧记录 leaked ptr 并在此处显式 `Box::from_raw` 释放，但
/// v1a 不做（×20 域重载测的内存观测将决定是否 Phase 2 内做字体缓存化为进程单例）。
///
/// **v1b：**全局 texture/font registry（进程级单例缓存化后）将在此清——届时填实现。
#[no_mangle]
pub extern "C" fn loomgui_shutdown() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_returns_c_string_v1b() {
        unsafe {
            let s = CStr::from_ptr(loomgui_version() as *const i8);
            assert_eq!(s.to_str().unwrap(), "v1b.2");
        }
    }
}

#[cfg(test)]
mod abi_tests {
    use super::*;
    use std::ffi::CString;

    /// 字体路径：CARGO_MANIFEST_DIR = loomgui_ffi_c/，字体在
    /// ../loomgui_core/tests/fixtures/DejaVuSans.ttf（仓库内 v0 测试字体）。
    fn font_path() -> (CString, usize) {
        let p = format!(
            "{}/../loomgui_core/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let c = CString::new(p).unwrap();
        let len = c.as_bytes().len();
        (c, len)
    }

    #[cfg(feature = "parse")]
    #[test]
    fn full_ffi_roundtrip_builds_blob() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let html = CString::new(
            r#"<div style="width:100px;height:50px;background-color:#ff0000;"></div>"#,
        )
        .unwrap();
        let css = CString::new("").unwrap();
        let r = loomgui_stage_load_html(
            h,
            html.as_ptr() as *const u8,
            html.as_bytes().len(),
            css.as_ptr() as *const u8,
            css.as_bytes().len(),
        );
        assert_eq!(r, 0, "load_html ok");
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_frame(h, &mut len);
        assert!(!ptr.is_null());
        assert!(len > 12, "blob 至少含 header");
        unsafe {
            assert_eq!(&*(ptr as *const u8), &0x4Cu8); // magic 第一字节 'L'
        }
        loomgui_stage_free(h);
    }

    /// load_package FFI：手搓 scene（不走 parse）→ write_package → FFI 装载 → tick → blob。
    /// 与 load_html 路径解耦（parse feature off 时仍可用）。
    #[test]
    fn load_package_builds_blob_from_package() {
        use loomgui_core::asset::write_package;
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let (fp, fplen) = font_path();
        // 手搓 scene（不走 parse），打成包
        let entries = vec![
            (None, NodeKind::Container, ResolvedStyle::default()),
            (Some(0), NodeKind::Text { content: "hi".into() }, ResolvedStyle::default()),
        ];
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0), &loomgui_core::asset::AtlasSection::default());

        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        assert!(!h.is_null());
        let r = loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len());
        assert_eq!(r, 0, "load_package ok");
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_frame(h, &mut len);
        assert!(!ptr.is_null() && len > 12, "tick 后应有 blob");
        loomgui_stage_free(h);
    }

    /// 契约：从未 tick 过的句柄 borrow_frame 必须返回 null + len=0
    /// （空 Vec::as_ptr() 是非空悬挂哨兵，Fix-1 显式判空锁住"未 tick→null"契约）。
    #[test]
    fn borrow_frame_never_ticked_returns_null() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let mut len = 1usize; // 故意非 0，确认被覆写为 0
        let ptr = loomgui_stage_borrow_frame(h, &mut len);
        assert!(ptr.is_null(), "未 tick 过 borrow_frame 必须 null");
        assert_eq!(len, 0, "未 tick 过 out_len 必须 0");
        loomgui_stage_free(h);
    }

    /// register_texture：core 分配单调 tex_id（src 幂等）；collect 返 scene 的 Image src。
    #[test]
    fn register_texture_and_collect_image_srcs() {
        use loomgui_core::asset::write_package;
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;

        let (fp, fplen) = font_path();
        // 手搓含两个 Image（src 不同）+ 一个 Container 的 scene。
        let entries = vec![
            (None, NodeKind::Container, ResolvedStyle::default()),
            (Some(0), NodeKind::Image { src: "a.png".into() }, ResolvedStyle::default()),
            (Some(0), NodeKind::Image { src: "b.png".into() }, ResolvedStyle::default()),
            (Some(0), NodeKind::Image { src: "a.png".into() }, ResolvedStyle::default()), // 重复 src
        ];
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0), &loomgui_core::asset::AtlasSection::default());

        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        assert!(!h.is_null());
        assert_eq!(loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len()), 0);

        // collect：去重后 2 个 src（a.png, b.png），DFS 先序。
        // 注意 image_src_at 返 String::as_ptr（非 null-terminated）+ *out_len，
        // 故用 slice::from_raw_parts + from_utf8 读，不能用 CStr（String 无 trailing \0）。
        let count = loomgui_stage_image_src_count(h);
        assert_eq!(count, 2, "去重后 2 个 Image src");
        let mut len0 = 0usize;
        let p0 = loomgui_stage_image_src_at(h, 0, &mut len0);
        let s0 = unsafe { std::str::from_utf8(std::slice::from_raw_parts(p0, len0)).unwrap() };
        assert_eq!(s0, "a.png");
        let mut len1 = 0usize;
        let p1 = loomgui_stage_image_src_at(h, 1, &mut len1);
        let s1 = unsafe { std::str::from_utf8(std::slice::from_raw_parts(p1, len1)).unwrap() };
        assert_eq!(s1, "b.png");
        // OOB → null
        assert!(loomgui_stage_image_src_at(h, 99, &mut len0).is_null());

        // register：单调 tex_id，src 幂等。
        let a = "a.png";
        let b = "b.png";
        let id1 = loomgui_stage_register_texture(h, a.as_ptr() as *const u8, a.len(), 10, 20);
        let id2 = loomgui_stage_register_texture(h, b.as_ptr() as *const u8, b.len(), 30, 40);
        let id3 = loomgui_stage_register_texture(h, a.as_ptr() as *const u8, a.len(), 999, 999); // 幂等
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 1, "同 src 幂等返回同 id");
        assert_ne!(id1, 0);

        loomgui_stage_free(h);
    }
}
