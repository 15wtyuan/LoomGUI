//! FFI 导出层（§14.1 csbindgen）：extern "C" 薄包装，opaque Stage 句柄。
//! 命名前缀 `loomgui_`，csbindgen 扫描本文件生成 C# 绑定。

pub mod blob;

use std::ffi::CString;
use loomgui_core::input::{EventRecord, KeyEvent, PointerEvent};
use loomgui_core::scene::NodeId;
use loomgui_core::stage::Stage;

/// 版本字符串（C null-terminated `b"v1d.3\0"`）。Task 1 工具链 round-trip 用。
///
/// 返回 `*const u8`（csbindgen 映射为 C# `byte*`）；CString::as_ptr 给的是
/// `*const c_char`（i8），这里 cast 对齐签名。OnceLock 缓存，避免每次分配+泄漏。
#[no_mangle]
pub extern "C" fn loomgui_version() -> *const u8 {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new("v1d.3").unwrap())
        .as_ptr() as *const u8
}

/// opaque 句柄：Stage + 缓存的最近一帧 blob（borrow_frame 返回它的指针，下帧 reset）。
pub struct StageHandle {
    stage: Stage,
    frame_blob: Vec<u8>, // borrow_frame 返回 &this[..]；tick 时被覆盖。
    dump_blob: CString, // v1d.3：dump_scene 缓存（Rust 拥有）
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
        dump_blob: CString::new("").unwrap(),
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

/// atlas 数量（甲-B 恒 1，无图 scene = 0）。
#[no_mangle]
pub extern "C" fn loomgui_stage_atlas_count(h: *const StageHandle) -> usize {
    if h.is_null() { return 0; }
    let sh = unsafe { &*h };
    sh.stage.atlases.len()
}

/// 第 i 个 atlas 信息。返 atlas filename UTF-8 串指针（**无尾 NUL** + *out_src_len=字节长）；
/// *out_tex_id = core 分配的 atlas tex_id（= i+1）；*out_w/*out_h = atlas 像素尺寸。
/// OOB / null → null。串归 Stage 拥有，下次 load 前有效（坑16 len-based 读契约）。
#[no_mangle]
pub extern "C" fn loomgui_stage_atlas_info(
    h: *const StageHandle,
    index: usize,
    out_tex_id: *mut u32,
    out_w: *mut u32,
    out_h: *mut u32,
    out_src_len: *mut usize,
) -> *const u8 {
    if h.is_null() { return std::ptr::null(); }
    let sh = unsafe { &*h };
    if index >= sh.stage.atlases.len() { return std::ptr::null(); }
    let a = &sh.stage.atlases[index];
    unsafe {
        if !out_tex_id.is_null() { *out_tex_id = (index as u32) + 1; }   // atlas[0]→tex_id 1
        if !out_w.is_null() { *out_w = a.width; }
        if !out_h.is_null() { *out_h = a.height; }
        if !out_src_len.is_null() { *out_src_len = a.filename.len(); }
    }
    a.filename.as_ptr()
}

/// 跑一帧 tick_and_render → build_blob 写入缓存。v1c.4：dt 累积进 time_s（双击窗口，C# 传 unscaledDeltaTime）。
#[no_mangle]
pub extern "C" fn loomgui_stage_tick(h: *mut StageHandle, dt: f32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    sh.stage.advance_time(dt);
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

/// v1d.3：dump 整树 JSON（调试）。返 Rust 拥有的 UTF-8 C 串 + len；下 tick 失效。
#[no_mangle]
pub extern "C" fn loomgui_stage_dump_scene(h: *mut StageHandle, out_len: *mut usize) -> *const u8 {
    if h.is_null() || out_len.is_null() { return std::ptr::null(); }
    let handle = unsafe { &mut *h };
    let json = match &handle.stage.scene {
        Some(scene) => loomgui_core::dump::dump_scene_json(scene),
        None => String::from("[]"),
    };
    handle.dump_blob = CString::new(json).unwrap_or_else(|_| CString::new("[]").unwrap());
    let bytes = handle.dump_blob.as_bytes_with_nul();
    unsafe { *out_len = bytes.len(); }
    handle.dump_blob.as_ptr() as *const u8
}

/// 注入本帧指针事件（扁平 PointerEvent 数组）。tick 前调。
/// null/len=0 = 本帧无输入事件（清空 pending_input，hover diff 仍跑——指针位置沿用上帧 last_pos）。
///
/// **常驻（不 gate）：**输入是 runtime 稳定入口，`--no-default-features` 构建的 .dll 仍有本函数。
#[no_mangle]
pub extern "C" fn loomgui_stage_set_input(
    h: *mut StageHandle,
    events: *const PointerEvent,
    len: usize,
) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    if events.is_null() || len == 0 {
        sh.stage.set_input(&[]);
        return;
    }
    let evs = unsafe { std::slice::from_raw_parts(events, len) };
    sh.stage.set_input(evs);
}

/// 拉取本帧事件 SOA（pull，同 borrow_frame 语义）。返 `last_events` 的 `as_ptr` + 写 len。
/// null 句柄或未 tick（last_events 空）→ null + len=0。指针下 tick 失效。
///
/// **常驻（不 gate）：**事件是 runtime 稳定入口。EventRecord 是 `#[repr(C)]` POD，
/// C 侧按 `len * sizeof(EventRecord)` 切片读。
#[no_mangle]
pub extern "C" fn loomgui_stage_borrow_events(
    h: *const StageHandle,
    out_len: *mut usize,
) -> *const u8 {
    if h.is_null() {
        if !out_len.is_null() {
            unsafe { *out_len = 0 };
        }
        return std::ptr::null();
    }
    let sh = unsafe { &*h };
    let events: &[EventRecord] = sh.stage.last_events();
    if events.is_empty() {
        if !out_len.is_null() {
            unsafe { *out_len = 0 };
        }
        return std::ptr::null();
    }
    if !out_len.is_null() {
        unsafe { *out_len = events.len() };
    }
    events.as_ptr() as *const u8
}

/// UI 挡住时游戏不响应点击（§10.6）。= 任一活跃槽 last_hit 非空且非根（v1c.3 多指：鼠标 slot0 + 已分配触摸槽）。
/// null 句柄 → false。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_is_pointer_on_ui(h: *const StageHandle) -> bool {
    if h.is_null() {
        return false;
    }
    let sh = unsafe { &*h };
    sh.stage.is_pointer_on_ui()
}

/// 业务设节点 disabled 状态（伪类源 + active/click 抑制）。NodeId.0 越界静默跳过。
/// null 句柄 → no-op。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_set_node_disabled(
    h: *mut StageHandle,
    node_id: u32,
    disabled: bool,
) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    sh.stage.set_node_disabled(NodeId(node_id as usize), disabled);
}

/// 返 parent node_id（v1c.2：C# 事件路由沿链用，spec §4.2）。根/越界/无 scene → 0xFFFF_FFFF（sentinel）。
///
/// **常驻（不 gate）：**runtime 稳定入口，`--no-default-features` 构建的 .dll 仍有本函数（坑 21）。
#[no_mangle]
pub extern "C" fn loomgui_node_parent(h: *const StageHandle, node_id: u32) -> u32 {
    const ROOT_SENTINEL: u32 = 0xFFFF_FFFF;
    if h.is_null() {
        return ROOT_SENTINEL;
    }
    let sh = unsafe { &*h };
    match &sh.stage.scene {
        Some(scene) => {
            let idx = node_id as usize;
            if idx < scene.nodes.len() {
                scene.nodes[idx].parent.map(|p| p.0 as u32).unwrap_or(ROOT_SENTINEL)
            } else {
                ROOT_SENTINEL
            }
        }
        None => ROOT_SENTINEL,
    }
}

/// 按 CSS id 属性查节点（v1c.1+ enabler：业务用 id 定位节点替代硬编码 build 序 id）。
/// id = UTF-8 字节（指针+len）。返 node_id；null 句柄/非 UTF-8/无匹配 → 0xFFFF_FFFF（sentinel，同 node_parent）。
///
/// **常驻（不 gate）：**runtime 稳定入口，`--no-default-features` 构建的 .dll 仍有本函数（坑 21）。
#[no_mangle]
pub extern "C" fn loomgui_stage_find_node_by_id(
    h: *const StageHandle,
    id: *const u8,
    id_len: usize,
) -> u32 {
    const NOT_FOUND: u32 = 0xFFFF_FFFF;
    if h.is_null() || id.is_null() {
        return NOT_FOUND;
    }
    let sh = unsafe { &*h };
    let bytes = unsafe { std::slice::from_raw_parts(id, id_len) };
    let id_str = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return NOT_FOUND,
    };
    match sh.stage.find_node_by_id(id_str) {
        Some(nid) => nid.0 as u32,
        None => NOT_FOUND,
    }
}

/// 加 touch monitor（v1c.3：C# CaptureTouch 后调）。核心把 node 加进 touch_id 对应槽的 touch_monitors（去重）。
/// touch_id=-1 → 鼠标主指槽；找不到槽 → no-op。null 句柄 → no-op。
///
/// **常驻（不 gate）：**runtime 稳定入口。
#[no_mangle]
pub extern "C" fn loomgui_stage_add_touch_monitor(h: *mut StageHandle, touch_id: i32, node_id: u32) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    sh.stage.add_touch_monitor(touch_id, NodeId(node_id as usize));
}

/// 移除 touch monitor（v1c.3：C# 主动释放调）。从所有槽移除该 node。null 句柄 → no-op。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_remove_touch_monitor(h: *mut StageHandle, node_id: u32) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    sh.stage.remove_touch_monitor(NodeId(node_id as usize));
}

/// v1c.4：外部取消待 click（照 fgui Stage.CancelClick(touchId)）。置对应槽 click_cancelled。
/// null 句柄 → no-op。
#[no_mangle]
pub extern "C" fn loomgui_stage_cancel_click(h: *mut StageHandle, touch_id: i32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    sh.stage.cancel_click(touch_id);
}

/// v1d.2：注入本帧键盘事件（扁平 KeyEvent 数组）。tick 前调。null/len=0 = 无键盘输入。
///
/// **常驻（不 gate）：**输入是 runtime 稳定入口。
#[no_mangle]
pub extern "C" fn loomgui_stage_set_key_input(h: *mut StageHandle, keys: *const KeyEvent, len: usize) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    if keys.is_null() || len == 0 {
        sh.stage.set_key_input(&[]);
        return;
    }
    let ks = unsafe { std::slice::from_raw_parts(keys, len) };
    sh.stage.set_key_input(ks);
}

/// v1d.2：编程聚焦节点（照 fgui RequestFocus）。强制聚焦任意非 disabled 节点
/// （含 tabindex=None/-1）；disabled 拒；越界跳过。null 句柄 → no-op。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_request_focus(h: *mut StageHandle, node_id: u32) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    sh.stage.request_focus(NodeId(node_id as usize));
}

/// v1d.2：读当前焦点节点。无焦点/无 scene → 0xFFFF_FFFF（sentinel，同 node_parent）。null 句柄 → sentinel。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_focused_node(h: *const StageHandle) -> u32 {
    const NONE: u32 = 0xFFFF_FFFF;
    if h.is_null() { return NONE; }
    let sh = unsafe { &*h };
    match &sh.stage.scene {
        Some(scene) => scene.focused_node.map(|n| n.0 as u32).unwrap_or(NONE),
        None => NONE,
    }
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
    fn version_returns_c_string_v1d3() {
        unsafe {
            let s = CStr::from_ptr(loomgui_version() as *const i8);
            assert_eq!(s.to_str().unwrap(), "v1d.3");
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
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Text { content: "hi".into() }, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0), &loomgui_core::asset::AtlasSection::default(), &loomgui_core::style::dynamic::DynamicRuleTable::default());

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

    /// atlas_count/atlas_info：手搓含 atlas 的包 → load_package → 读 atlas 元数据。
    /// 契约（坑16）：atlas_info 返 String::as_ptr（无尾 NUL）+ *out_src_len=字节长，
    /// 故用 slice::from_raw_parts + from_utf8 读，不能用 CStr（String 无 trailing \0）。
    /// *out_tex_id = atlas index + 1（atlas[0]→tex_id 1，build_registry 同约定）。
    #[test]
    fn atlas_count_and_info_round_trip() {
        use loomgui_core::asset::{write_package, AtlasInfo, AtlasSection, AtlasSprite};
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let (fp, fplen) = font_path();
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "a.png".into() }, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "b.png".into() }, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let atlas = AtlasSection {
            atlases: vec![AtlasInfo { filename: "loom.atlas.png".into(), width: 512, height: 256 }],
            sprites: vec![
                AtlasSprite { src: "a.png".into(), x: 0, y: 0, w: 64, h: 32 },
                AtlasSprite { src: "b.png".into(), x: 64, y: 0, w: 100, h: 200 },
            ],
        };
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0), &atlas, &loomgui_core::style::dynamic::DynamicRuleTable::default());

        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        assert!(!h.is_null());
        assert_eq!(loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len()), 0);

        assert_eq!(loomgui_stage_atlas_count(h), 1);
        let mut tid = 0u32; let mut w = 0u32; let mut hh = 0u32; let mut slen = 0usize;
        let p = loomgui_stage_atlas_info(h, 0, &mut tid, &mut w, &mut hh, &mut slen);
        assert!(!p.is_null());
        assert_eq!(tid, 1, "atlas[0] → tex_id 1");
        assert_eq!((w, hh), (512, 256));
        let fname = unsafe { std::str::from_utf8(std::slice::from_raw_parts(p, slen)).unwrap() };
        assert_eq!(fname, "loom.atlas.png");
        // OOB → null
        assert!(loomgui_stage_atlas_info(h, 99, &mut tid, &mut w, &mut hh, &mut slen).is_null());

        loomgui_stage_free(h);
    }

    /// set_input → tick → borrow_events：装载按钮 + Move 到 (50,25) 应产 RollOver。
    /// 读 EventRecord[] POD slice，扫 event_type 字段（repr(C) 手解，避免 Marshal）。
    #[cfg(feature = "parse")]
    #[test]
    fn set_input_borrow_events_round_trip() {
        use loomgui_core::input::{PointerEvent, PointerKind, EVT_ROLL_OVER};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        // 装载一个按钮
        let html = std::ffi::CString::new(r#"<div class="root"><button class="btn">OK</button></div>"#).unwrap();
        let css = std::ffi::CString::new(r#".btn { width: 100px; height: 50px; }"#).unwrap();
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.as_bytes().len(), css.as_ptr() as *const u8, css.as_bytes().len());
        // set_input：Move 到按钮 (50,25)
        let ev = PointerEvent { kind: PointerKind::Move, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 };
        loomgui_stage_set_input(h, &ev, 1);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_events(h, &mut len);
        assert!(!ptr.is_null() && len > 0, "tick 后应有事件");
        // 读 EventRecord POD slice，扫 event_type 找 RollOver（event_type=4）
        let rec_size = std::mem::size_of::<loomgui_core::input::EventRecord>();
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len * rec_size) };
        let mut found_rollover = false;
        for i in 0..len {
            let off = i * rec_size;
            let event_type = bytes[off + 4]; // node_id u32 (4 字节) 后是 event_type u8
            if event_type == EVT_ROLL_OVER {
                found_rollover = true;
                break;
            }
        }
        assert!(found_rollover, "应产 RollOver 事件");
        loomgui_stage_free(h);
    }

    /// borrow_events 契约：未 tick / 空 last_events → null + len=0。
    #[test]
    fn borrow_events_null_before_tick() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let mut len = 1usize;
        let ptr = loomgui_stage_borrow_events(h, &mut len);
        assert!(ptr.is_null() && len == 0, "未 tick → null+len=0");
        loomgui_stage_free(h);
    }

    /// is_pointer_on_ui 契约：手搓空包（单根 Container）→ 命中根 → false（根不算 UI）。
    /// 覆盖 4 函数在无 parse feature 路径下也可用的契约（手搓包不走 parse）。
    #[test]
    fn is_pointer_on_ui_true_on_hit_false_on_miss() {
        use loomgui_core::input::{PointerEvent, PointerKind};
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::dynamic::DynamicRuleTable;
        use loomgui_core::style::resolved::ResolvedStyle;
        use loomgui_core::asset::{write_package, AtlasSection};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        // 手搓空 scene（单根 Container），不走 parse
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![(None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None)];
        let pkg = write_package(
            &Scene::build(&entries),
            (200.0, 100.0),
            &AtlasSection::default(),
            &DynamicRuleTable::default(),
        );
        loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len());
        // 命中根 (100,50)——根不算 UI → is_pointer_on_ui=false
        let ev = PointerEvent { kind: PointerKind::Move, x: 100.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 };
        loomgui_stage_set_input(h, &ev, 1);
        loomgui_stage_tick(h, 0.0);
        // 单根节点：命中根 → is_pointer_on_ui=false（根不算）
        assert!(!loomgui_stage_is_pointer_on_ui(h), "命中根 → false");
        loomgui_stage_free(h);
    }

    /// EventRecord/PointerEvent sizeof（v1c.3 契约）。
    /// PointerEvent 16B：PointerKind repr(u8) 1B + button 1B + pad 2B + touch_id@4 + x@8 + y@12。
    /// EventRecord 20B：node_id@0(4) + event_type@4(1) + pad@5(3) + touch_id@8(4) + x@12(4) + y@16(4)。
    #[test]
    fn pointer_event_event_record_sizeof() {
        use loomgui_core::input::{PointerEvent, EventRecord};
        assert_eq!(std::mem::size_of::<PointerEvent>(), 16, "PointerEvent 16B（PointerKind repr(u8)）");
        assert_eq!(std::mem::size_of::<EventRecord>(), 20, "EventRecord 20B（v1c.2 是 16，+touch_id@8）");
    }

    /// 借事件读 touch_id 字段（POD @8 偏移）。装载按钮 + 触摸 Down，验 touch_id 贯穿。
    #[cfg(feature = "parse")]
    #[test]
    fn event_record_has_touch_id() {
        use loomgui_core::input::{PointerEvent, PointerKind, EVT_DOWN};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = std::ffi::CString::new(r#"<div class="root"><button class="btn">OK</button></div>"#).unwrap();
        let css = std::ffi::CString::new(r#".btn { width: 100px; height: 50px; }"#).unwrap();
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.as_bytes().len(), css.as_ptr() as *const u8, css.as_bytes().len());
        // 触摸 touch_id=3 Down 在 btn (50,25)
        let ev = PointerEvent { kind: PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 3 };
        loomgui_stage_set_input(h, &ev, 1);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_events(h, &mut len);
        assert!(!ptr.is_null() && len > 0);
        let rec_size = std::mem::size_of::<loomgui_core::input::EventRecord>();
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len * rec_size) };
        // 找 Down 事件，验 touch_id @8 == 3（LE i32）
        let mut found = false;
        for i in 0..len {
            let off = i * rec_size;
            if bytes[off + 4] == EVT_DOWN {
                let touch_id = i32::from_le_bytes([bytes[off + 8], bytes[off + 9], bytes[off + 10], bytes[off + 11]]);
                assert_eq!(touch_id, 3, "Down 事件 touch_id=3");
                found = true;
                break;
            }
        }
        assert!(found, "应有 Down 事件");
        loomgui_stage_free(h);
    }

    /// add_touch_monitor round-trip：Down → add monitor → Move 移出 → 借事件验 monitor 收 Move。
    #[cfg(feature = "parse")]
    #[test]
    fn add_touch_monitor_round_trip() {
        use loomgui_core::input::{PointerEvent, PointerKind, EVT_MOVE};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = std::ffi::CString::new(r#"<div class="root"><button class="btn">OK</button></div>"#).unwrap();
        let css = std::ffi::CString::new(r#".btn { width: 100px; height: 50px; }"#).unwrap();
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.as_bytes().len(), css.as_ptr() as *const u8, css.as_bytes().len());
        // touch_id=1 Down 在 btn
        let down = PointerEvent { kind: PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 };
        loomgui_stage_set_input(h, &down, 1);
        loomgui_stage_tick(h, 0.0);
        // capture btn (node 1)——模拟 C# CaptureTouch 后调
        loomgui_stage_add_touch_monitor(h, 1, 1);
        // Move 移出 btn (150, 25 命中 root)——有 monitor 应产 Move@btn
        let mv = PointerEvent { kind: PointerKind::Move, x: 150.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 };
        loomgui_stage_set_input(h, &mv, 1);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_events(h, &mut len);
        assert!(!ptr.is_null() && len > 0);
        let rec_size = std::mem::size_of::<loomgui_core::input::EventRecord>();
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len * rec_size) };
        let mut found_move_at_btn = false;
        for i in 0..len {
            let off = i * rec_size;
            let event_type = bytes[off + 4];
            let node_id = u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
            if event_type == EVT_MOVE && node_id == 1 { found_move_at_btn = true; break; }
        }
        assert!(found_move_at_btn, "capture 后 Move 移出仍产 Move@btn(node 1)");
        loomgui_stage_free(h);
    }

    /// 5 函数常驻契约：无 parse feature 也能编译（§14.6 坑21）。
    /// 此测在 normal build 跑，验证 5 函数 + PointerEvent/EventRecord 常驻可调。
    /// 不 tick（tick_and_render 需先 load scene）——本测只验常驻编译/调用安全；
    /// 真正的 --no-default-features 验在 Step 5 `cargo build -p loomgui_ffi_c --no-default-features`。
    /// 行为验（含 set_input→tick→borrow_events/is_pointer_on_ui）在 parse-feature 测中覆盖。
    #[test]
    fn no_default_features_builds() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        loomgui_stage_set_input(h, std::ptr::null(), 0); // null/len=0 应安全（清空 pending_input）
        loomgui_stage_set_node_disabled(h, 0, true); // 无 scene → no-op，不 panic
        // 无 scene + 未 tick：is_pointer_on_ui 读 cur_hit=None → false，不 panic
        assert!(!loomgui_stage_is_pointer_on_ui(h));
        // borrow_events：未 tick → null + len=0
        let mut len = 1usize;
        let ptr = loomgui_stage_borrow_events(h, &mut len);
        assert!(ptr.is_null() && len == 0);
        assert_eq!(loomgui_node_parent(h, 0), 0xFFFF_FFFF, "无 scene → sentinel，不 panic");
        loomgui_stage_free(h);
    }

    /// node_parent 契约（v1c.2）：child.parent==root；root.parent==sentinel；OOB==sentinel。
    #[test]
    fn node_parent_returns_chain_and_sentinel() {
        use loomgui_core::asset::{write_package, AtlasSection};
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::{resolved::ResolvedStyle, dynamic::DynamicRuleTable};
        let (fp, fplen) = font_path();
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0), &AtlasSection::default(), &DynamicRuleTable::default());
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        assert!(!h.is_null());
        assert_eq!(loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len()), 0);
        assert_eq!(loomgui_node_parent(h, 1), 0, "child(1).parent == root(0)");
        assert_eq!(loomgui_node_parent(h, 0), 0xFFFF_FFFF, "root(0).parent == sentinel");
        assert_eq!(loomgui_node_parent(h, 99), 0xFFFF_FFFF, "OOB == sentinel");
        loomgui_stage_free(h);
    }

    /// find_node_by_id round-trip：手搓包（root + btn id="ok" + Text 子）→ find "ok" 返 btn(1)；
    /// 无匹配 → sentinel。照 node_parent 测用包路径（不走 parse）。
    #[test]
    fn find_node_by_id_round_trip() {
        use loomgui_core::asset::{write_package, AtlasSection};
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::{resolved::ResolvedStyle, dynamic::DynamicRuleTable};
        let (fp, fplen) = font_path();
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), Some("ok".to_string()), false, None),
            (Some(1), NodeKind::Text { content: "OK".into() }, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let pkg = write_package(&Scene::build(&entries), (100.0, 50.0), &AtlasSection::default(), &DynamicRuleTable::default());
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 100.0, 50.0);
        assert!(!h.is_null());
        assert_eq!(loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len()), 0);
        let id = std::ffi::CString::new("ok").unwrap();
        assert_eq!(
            loomgui_stage_find_node_by_id(h, id.as_ptr() as *const u8, id.as_bytes().len()),
            1,
            "find 'ok' → btn node 1"
        );
        let miss = std::ffi::CString::new("nope").unwrap();
        assert_eq!(
            loomgui_stage_find_node_by_id(h, miss.as_ptr() as *const u8, miss.as_bytes().len()),
            0xFFFF_FFFF,
            "无匹配 → sentinel"
        );
        loomgui_stage_free(h);
    }

    /// v1d.3：version 字符串 == "v1d.3"。
    #[test]
    fn version_is_v1d_3() {
        let p = loomgui_version();
        let len = (0..).take_while(|&i| unsafe { *p.add(i) != 0 }).count();
        let s = std::str::from_utf8(unsafe { std::slice::from_raw_parts(p, len) }).unwrap();
        assert_eq!(s, "v1d.3");
    }

    /// v1d.1：EventRecord 仍 20B（drag/longpress 复用 event_type 空位 6-9）、PointerEvent 16B、Canceled=3。
    #[test]
    fn event_record_and_pointer_event_sizes_unchanged() {
        use loomgui_core::input::{EventRecord, PointerEvent, PointerKind};
        use std::mem::size_of;
        assert_eq!(size_of::<EventRecord>(), 20, "EventRecord 20B（drag/longpress 复用 event_type）");
        assert_eq!(size_of::<PointerEvent>(), 16, "PointerEvent 16B 不变");
        assert_eq!(PointerKind::Canceled as u8, 3, "Canceled=3");
    }

    /// v1c.4：cancel_click FFI——Down → cancel_click → Up → 无 Click，Up 仍发。
    /// 2-frame flow：frame1 Down@btn + tick，cancel_click(-1)，frame2 Up@btn + tick，borrow_events 验。
    #[cfg(feature = "parse")]
    #[test]
    fn cancel_click_skips_click_event() {
        use loomgui_core::input::{PointerEvent, PointerKind, EVT_CLICK, EVT_UP};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = b"<button class=\"btn\">OK</button>";
        let css = b".btn{width:100px;height:50px;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        // frame1: Down@btn
        loomgui_stage_set_input(h, [PointerEvent { kind: PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }].as_ptr(), 1);
        loomgui_stage_tick(h, 0.0);
        // 取消（Down 后、Up 前）
        loomgui_stage_cancel_click(h, -1);
        // frame2: Up@btn → click_cancelled → 无 Click
        loomgui_stage_set_input(h, [PointerEvent { kind: PointerKind::Up, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }].as_ptr(), 1);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let p = loomgui_stage_borrow_events(h, &mut len);
        // borrow_events 的 out_len 是记录数（非字节；照 set_input_borrow_events_round_trip 契约 + FFI doc）
        let recs = unsafe { std::slice::from_raw_parts(p as *const loomgui_core::input::EventRecord, len) };
        assert!(!recs.iter().any(|e| e.event_type == EVT_CLICK), "cancel_click → 无 Click");
        assert!(recs.iter().any(|e| e.event_type == EVT_UP), "Up 仍发");
        loomgui_stage_free(h);
    }

    /// v1d.1：EVT 常量值锁（6/7/8/9）+ drag 端到端：draggable btn Down+Move>阈值 → borrow_events 含 DragStart。
    #[cfg(feature = "parse")]
    #[test]
    fn drag_start_round_trip() {
        use loomgui_core::input::{PointerEvent, PointerKind, EVT_DRAG_START};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let html = b"<button class=\"btn\" draggable=\"true\">OK</button>";
        let css = b".btn{width:100px;height:50px;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        // EVT 常量值
        assert_eq!(loomgui_core::input::EVT_DRAG_START, 6);
        assert_eq!(loomgui_core::input::EVT_DRAG_MOVE, 7);
        assert_eq!(loomgui_core::input::EVT_DRAG_END, 8);
        assert_eq!(loomgui_core::input::EVT_LONG_PRESS, 9);
        // Down@btn + Move dx=5>mouse阈值2 → DragStart
        loomgui_stage_set_input(h, [
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Move, x: 55.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 },
        ].as_ptr(), 2);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let p = loomgui_stage_borrow_events(h, &mut len);
        let recs = unsafe { std::slice::from_raw_parts(p as *const loomgui_core::input::EventRecord, len) };
        assert!(recs.iter().any(|e| e.event_type == EVT_DRAG_START), "draggable btn Down+Move → DragStart");
        loomgui_stage_free(h);
    }

    /// v1d.1：longpress 端到端——Down@btn + tick dt 累积 1.5s → LongPress。
    #[cfg(feature = "parse")]
    #[test]
    fn long_press_round_trip() {
        use loomgui_core::input::{PointerEvent, PointerKind, EVT_LONG_PRESS};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let html = b"<button class=\"btn\">OK</button>";
        let css = b".btn{width:100px;height:50px;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        // frame1: Down@btn（tick dt=0）
        loomgui_stage_set_input(h, [PointerEvent { kind: PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }].as_ptr(), 1);
        loomgui_stage_tick(h, 0.0);
        // frame2: 空输入 + tick dt=1.5 → time_s 累积 1.5 → LongPress
        loomgui_stage_set_input(h, std::ptr::null(), 0);
        loomgui_stage_tick(h, 1.5);
        let mut len = 0usize;
        let p = loomgui_stage_borrow_events(h, &mut len);
        let recs = unsafe { std::slice::from_raw_parts(p as *const loomgui_core::input::EventRecord, len) };
        assert!(recs.iter().any(|e| e.event_type == EVT_LONG_PRESS), "按住 1.5s → LongPress");
        loomgui_stage_free(h);
    }

    /// v1d.2：KeyEvent sizeof 8B + EventRecord 仍 20B / PointerEvent 16B。
    #[test]
    fn key_event_sizeof_and_unchanged() {
        use loomgui_core::input::{EventRecord, KeyEvent, PointerEvent};
        use std::mem::size_of;
        assert_eq!(size_of::<KeyEvent>(), 8, "KeyEvent 8B");
        assert_eq!(size_of::<EventRecord>(), 20, "EventRecord 20B 不变");
        assert_eq!(size_of::<PointerEvent>(), 16, "PointerEvent 16B 不变");
    }

    /// v1d.2：EVT 常量值锁（12/13/14/15）。
    #[test]
    fn evt_constants_v1d2() {
        assert_eq!(loomgui_core::input::EVT_KEY_DOWN, 12);
        assert_eq!(loomgui_core::input::EVT_KEY_UP, 13);
        assert_eq!(loomgui_core::input::EVT_FOCUS_IN, 14);
        assert_eq!(loomgui_core::input::EVT_FOCUS_OUT, 15);
    }

    /// v1d.2：key 事件 round-trip——click-to-focus btn + Enter keydown + tick → borrow_events 含 KeyDown@焦点。
    #[cfg(feature = "parse")]
    #[test]
    fn key_event_round_trip() {
        use loomgui_core::input::{KeyEvent, EVT_KEY_DOWN};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        // btn tabindex=0 可聚焦
        let html = b"<button class=\"btn\" tabindex=\"0\">OK</button>";
        let css = b".btn{width:100px;height:50px;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        // click-to-focus：Down@btn → tick → 焦点 btn
        use loomgui_core::input::{PointerEvent, PointerKind};
        loomgui_stage_set_input(h, [PointerEvent { kind: PointerKind::Down, x: 50.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }].as_ptr(), 1);
        loomgui_stage_tick(h, 0.0);
        // 现在焦点应 btn。再 Enter keydown + tick
        loomgui_stage_set_key_input(h, [KeyEvent { key_code: 13, modifiers: 0, is_down: true, pad: [0, 0] }].as_ptr(), 1);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let p = loomgui_stage_borrow_events(h, &mut len);
        let recs = unsafe { std::slice::from_raw_parts(p as *const loomgui_core::input::EventRecord, len) };
        assert!(recs.iter().any(|e| e.event_type == EVT_KEY_DOWN), "聚焦 btn + Enter down → KeyDown@btn");
        loomgui_stage_free(h);
    }

    /// v1d.2：Tab 导航 round-trip——两可聚焦 btn + Tab → borrow_events 含 FocusIn（无 KeyDown）。
    #[cfg(feature = "parse")]
    #[test]
    fn tab_navigation_round_trip() {
        use loomgui_core::input::{KeyEvent, EVT_FOCUS_IN, EVT_KEY_DOWN, KEY_TAB};
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = b"<button class=\"a\" tabindex=\"0\">A</button><button class=\"b\" tabindex=\"0\">B</button>";
        let css = b"button{width:50px;height:30px;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        // Tab → 焦点首个可聚焦（A，node 1）
        loomgui_stage_set_key_input(h, [KeyEvent { key_code: KEY_TAB, modifiers: 0, is_down: true, pad: [0, 0] }].as_ptr(), 1);
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let p = loomgui_stage_borrow_events(h, &mut len);
        let recs = unsafe { std::slice::from_raw_parts(p as *const loomgui_core::input::EventRecord, len) };
        assert!(recs.iter().any(|e| e.event_type == EVT_FOCUS_IN), "Tab → FocusIn");
        assert!(recs.iter().all(|e| e.event_type != EVT_KEY_DOWN), "Tab 被消费，无 KeyDown");
        // focused_node 读首个可聚焦（A=node 0：parse 无合成根，两 button 各为 root；
        // DFS 先序：button.a(0)→Text(1)→button.b(2)→Text(3)；tabindex=0 进 zero 桶 → chain=[0,2]，Tab→0）
        assert_eq!(loomgui_stage_focused_node(h), 0, "Tab → 焦点 button.a(node 0，首个可聚焦)");
        loomgui_stage_free(h);
    }

    /// v1d.2：request_focus + focused_node round-trip。验 R3 修正：request_focus 记 pending，
    /// 未 tick 时 focused_node 仍 sentinel；tick 后消费生效。
    #[cfg(feature = "parse")]
    #[test]
    fn request_focus_round_trip() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = b"<button id=\"ok\" tabindex=\"0\">OK</button>";
        let css = b"button{width:50px;height:30px;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        let id = std::ffi::CString::new("ok").unwrap();
        let ok_node = loomgui_stage_find_node_by_id(h, id.as_ptr() as *const u8, id.as_bytes().len());
        assert_ne!(ok_node, 0xFFFF_FFFF, "find ok");
        loomgui_stage_request_focus(h, ok_node);
        assert_eq!(loomgui_stage_focused_node(h), 0xFFFF_FFFF, "request_focus 后未 tick → focused_node 仍 sentinel");
        loomgui_stage_tick(h, 0.0);
        assert_eq!(loomgui_stage_focused_node(h), ok_node, "tick 后焦点 = ok");
        loomgui_stage_free(h);
    }

    /// v1d.3：dump_scene FFI round-trip——load_html → tick → dump_scene 返 JSON 数组（首字节 `[`）。
    #[cfg(feature = "parse")]
    #[test]
    fn dump_scene_returns_json_array() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let html = CString::new(r#"<div class="root"><button class="btn">OK</button></div>"#).unwrap();
        let css = CString::new(r#".btn { width: 100px; height: 50px; }"#).unwrap();
        loomgui_stage_load_html(
            h,
            html.as_ptr() as *const u8,
            html.as_bytes().len(),
            css.as_ptr() as *const u8,
            css.as_bytes().len(),
        );
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_dump_scene(h, &mut len);
        assert!(!ptr.is_null(), "dump_scene 应返非空指针");
        assert!(len > 0, "out_len > 0");
        unsafe {
            assert_eq!(*ptr, b'[', "首字节应为 '['（JSON 数组）");
        }
        loomgui_stage_free(h);
    }
}
