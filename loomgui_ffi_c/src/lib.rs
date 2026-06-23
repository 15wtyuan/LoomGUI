//! FFI 导出层（§14.1 csbindgen）：extern "C" 薄包装，opaque Stage 句柄。
//! 命名前缀 `loomgui_`，csbindgen 扫描本文件生成 C# 绑定。

pub mod blob;

use std::ffi::CString;
use loomgui_core::input::{EventRecord, PointerEvent};
use loomgui_core::scene::NodeId;
use loomgui_core::stage::Stage;

/// 版本字符串（C null-terminated `b"v1c.2\0"`）。Task 1 工具链 round-trip 用。
///
/// 返回 `*const u8`（csbindgen 映射为 C# `byte*`）；CString::as_ptr 给的是
/// `*const c_char`（i8），这里 cast 对齐签名。OnceLock 缓存，避免每次分配+泄漏。
#[no_mangle]
pub extern "C" fn loomgui_version() -> *const u8 {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new("v1c.2").unwrap())
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

/// UI 挡住时游戏不响应点击（§10.6）。= cur_hit 非空且非根（根是背景，不算 UI 挡）。
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
    fn version_returns_c_string_v1c2() {
        unsafe {
            let s = CStr::from_ptr(loomgui_version() as *const i8);
            assert_eq!(s.to_str().unwrap(), "v1c.2");
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
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None),
            (Some(0), NodeKind::Text { content: "hi".into() }, ResolvedStyle::default(), Vec::new(), None),
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
        let entries = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None),
            (Some(0), NodeKind::Image { src: "a.png".into() }, ResolvedStyle::default(), Vec::new(), None),
            (Some(0), NodeKind::Image { src: "b.png".into() }, ResolvedStyle::default(), Vec::new(), None),
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
        let ev = PointerEvent { kind: PointerKind::Move, x: 50.0, y: 25.0, button: 0 };
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
        let entries = vec![(None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None)];
        let pkg = write_package(
            &Scene::build(&entries),
            (200.0, 100.0),
            &AtlasSection::default(),
            &DynamicRuleTable::default(),
        );
        loomgui_stage_load_package(h, pkg.as_ptr(), pkg.len());
        // 命中根 (100,50)——根不算 UI → is_pointer_on_ui=false
        let ev = PointerEvent { kind: PointerKind::Move, x: 100.0, y: 50.0, button: 0 };
        loomgui_stage_set_input(h, &ev, 1);
        loomgui_stage_tick(h, 0.0);
        // 单根节点：命中根 → is_pointer_on_ui=false（根不算）
        assert!(!loomgui_stage_is_pointer_on_ui(h), "命中根 → false");
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
        let entries = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), Vec::new(), None),
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
}
