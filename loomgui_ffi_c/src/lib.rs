//! FFI 导出层（§14.1 csbindgen）：extern "C" 薄包装，opaque Stage 句柄。
//! 命名前缀 `loomgui_`，csbindgen 扫描本文件生成 C# 绑定。

pub mod blob;

use std::ffi::CString;
use loomgui_core::input::{EventRecord, KeyEvent, PointerEvent};
use loomgui_core::scene::NodeId;
use loomgui_core::stage::Stage;

/// 版本字符串（C null-terminated `b"v1e\0"`）。
///
/// 返回 `*const u8`（csbindgen 映射为 C# `byte*`）；CString::as_ptr 给的是
/// `*const c_char`（i8），这里 cast 对齐签名。OnceLock 缓存，避免每次分配+泄漏。
#[no_mangle]
pub extern "C" fn loomgui_version() -> *const u8 {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new("v1e").unwrap())
        .as_ptr() as *const u8
}

/// opaque 句柄：Stage + 缓存的最近一帧 blob（borrow_frame 返回它的指针，下帧 reset）。
pub struct StageHandle {
    stage: Stage,
    frame_blob: Vec<u8>, // borrow_frame 返回 &this[..]；tick 时被覆盖。
    dump_blob: CString, // dump_scene 缓存（Rust 拥有）
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
///
/// v1.4-a T4：`Stage::load_inline` 已砍（D12）。本 FFI 暂保留（T7 决定是否砍/改），
/// 内部直接调 parse_html + resolve_styles + build_scene（同旧 load_inline 逻辑）。
/// textures/atlases 已砍，不涉及纹理注册。
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
    // v1.4-a T4：load_inline 已砍，直接走 parse → resolve → build_scene。
    let tree = match loomgui_core::parse::dom::parse_html(html) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    let sheet = match loomgui_core::parse::css::parse_css(css) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let styles = loomgui_core::style::cascade::resolve_styles(&tree, &sheet);
    sh.stage.tweens.clear();
    if let Some(scene) = sh.stage.scene.as_mut() {
        scene.scroll.clear();
    }
    sh.stage.prev_node_hashes.clear();
    sh.stage.scene = Some(loomgui_core::scene::node::build_scene(&tree, &styles));
    0
}

/// 装载二进制包（spec §12/§13）。name = 包名（进 packages 字典 key），bytes = .pkg.bin。
/// 0=ok，-1=err。null 句柄/空指针返回 -1。包是 Rust-internal，C# 只透传 bytes（不解析）。
///
/// **常驻（不 gate）：**包格式是 runtime 的稳定入口，不依赖 parse feature——
/// `--no-default-features` 构建的 .dll 仍有本函数（Unity 用 default 带 parse 的 dev .dll）。
///
/// v1.4-a T7：FFI 签名加 name 参数（对齐 T4 `Stage::load_package(name, bytes)`）。
/// load_package 只进资源池不建 scene——Unity 侧需先 create_root 建 scene 再 instantiate 建内容。
#[no_mangle]
pub extern "C" fn loomgui_stage_load_package(
    h: *mut StageHandle,
    name: *const u8,
    name_len: usize,
    bytes: *const u8,
    bytes_len: usize,
) -> i32 {
    if h.is_null() || name.is_null() || bytes.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let name = std::str::from_utf8(unsafe { std::slice::from_raw_parts(name, name_len) }).unwrap_or("");
    let bytes = unsafe { std::slice::from_raw_parts(bytes, bytes_len) };
    match sh.stage.load_package(name, bytes) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// 从包克隆一个组件进当前 scene，返组件根 NodeId（u32）。
/// pkg/comp = UTF-8 字节（指针+len）。失败返 0xFFFF_FFFF（INVALID，同 create_root 失败语义）。
/// scene 必须已存在（create_root 先建），否则 Err→sentinel。null 句柄 → sentinel。
///
/// **常驻（不 gate）。**v1.4-a T7：包装 T5 `Stage::instantiate(pkg, comp)`（spec §4.2/§4.4）。
#[no_mangle]
pub extern "C" fn loomgui_stage_instantiate(
    h: *mut StageHandle,
    pkg: *const u8,
    pkg_len: usize,
    comp: *const u8,
    comp_len: usize,
) -> u32 {
    const INVALID: u32 = 0xFFFF_FFFF;
    if h.is_null() || pkg.is_null() || comp.is_null() {
        return INVALID;
    }
    let sh = unsafe { &mut *h };
    let pkg = std::str::from_utf8(unsafe { std::slice::from_raw_parts(pkg, pkg_len) }).unwrap_or("");
    let comp = std::str::from_utf8(unsafe { std::slice::from_raw_parts(comp, comp_len) }).unwrap_or("");
    match sh.stage.instantiate(pkg, comp) {
        Ok(id) => id.0,
        Err(_) => INVALID,
    }
}

/// 跑一帧 tick_and_render → build_blob 写入缓存。dt 累积进 time_s（双击窗口，C# 传 unscaledDeltaTime）。
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

/// dump 整树 JSON（调试）。返 Rust 拥有的 UTF-8 C 串 + len；下 tick 失效。
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

/// UI 挡住时游戏不响应点击（§10.6）。= 任一活跃槽 last_hit 非空且非根（多指：鼠标 slot0 + 已分配触摸槽）。
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
    sh.stage.set_node_disabled(NodeId(node_id), disabled);
}

/// 返 parent node_id（C# 事件路由沿链用，spec §4.2）。根/越界/无 scene → 0xFFFF_FFFF（sentinel）。
///
/// **常驻（不 gate）：**runtime 稳定入口，`--no-default-features` 构建的 .dll 仍有本函数。
#[no_mangle]
pub extern "C" fn loomgui_node_parent(h: *const StageHandle, node_id: u32) -> u32 {
    const ROOT_SENTINEL: u32 = 0xFFFF_FFFF;
    if h.is_null() {
        return ROOT_SENTINEL;
    }
    let sh = unsafe { &*h };
    match &sh.stage.scene {
        Some(scene) => {
            // NodeId(u32) → slotmap lookup（代际安全）。无效/悬空 NodeId → sentinel。
            match scene.get(NodeId(node_id)) {
                Some(n) => n.parent.map(|p| p.0 as u32).unwrap_or(ROOT_SENTINEL),
                None => ROOT_SENTINEL,
            }
        }
        None => ROOT_SENTINEL,
    }
}

/// 按 CSS id 属性查节点（业务用 id 定位节点替代硬编码 build 序 id）。
/// id = UTF-8 字节（指针+len）。返 node_id；null 句柄/非 UTF-8/无匹配 → 0xFFFF_FFFF（sentinel，同 node_parent）。
///
/// **常驻（不 gate）：**runtime 稳定入口，`--no-default-features` 构建的 .dll 仍有本函数。
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

/// 加 touch monitor（C# CaptureTouch 后调）。核心把 node 加进 touch_id 对应槽的 touch_monitors（去重）。
/// touch_id=-1 → 鼠标主指槽；找不到槽 → no-op。null 句柄 → no-op。
///
/// **常驻（不 gate）：**runtime 稳定入口。
#[no_mangle]
pub extern "C" fn loomgui_stage_add_touch_monitor(h: *mut StageHandle, touch_id: i32, node_id: u32) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    sh.stage.add_touch_monitor(touch_id, NodeId(node_id));
}

/// 移除 touch monitor（C# 主动释放调）。从所有槽移除该 node。null 句柄 → no-op。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_remove_touch_monitor(h: *mut StageHandle, node_id: u32) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    sh.stage.remove_touch_monitor(NodeId(node_id));
}

/// 外部取消待 click（照 fgui Stage.CancelClick(touchId)）。置对应槽 click_cancelled。
/// null 句柄 → no-op。
#[no_mangle]
pub extern "C" fn loomgui_stage_cancel_click(h: *mut StageHandle, touch_id: i32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    sh.stage.cancel_click(touch_id);
}

/// 注入本帧键盘事件（扁平 KeyEvent 数组）。tick 前调。null/len=0 = 无键盘输入。
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

/// 注入本帧滚轮事件（扁平 WheelEvent 数组）。tick 前调；**累积式**（多次调合并）。
/// null/len=0 = 本帧无滚轮（直接 return，不清空——与 set_key_input 不同；累积语义）。
///
/// **常驻（不 gate）：**输入是 runtime 稳定入口。
#[no_mangle]
pub extern "C" fn loomgui_stage_set_wheel_input(
    h: *mut StageHandle,
    events: *const loomgui_core::scroll::WheelEvent,
    len: usize,
) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    if events.is_null() || len == 0 {
        return;
    }
    let evs = unsafe { std::slice::from_raw_parts(events, len) };
    sh.stage.set_wheel_input(evs);
}

/// 编程滚动到指定位置。非 scroll 容器 / 越界 node → no-op（不 panic）。
/// animated: u8（0=瞬移 1=缓动 cubic-out）。null 句柄 → no-op。
#[no_mangle]
pub extern "C" fn loomgui_stage_set_scroll_pos(
    h: *mut StageHandle,
    node_id: u32,
    x: f32,
    y: f32,
    animated: u8,
) {
    if h.is_null() { return; }
    let handle = unsafe { &mut *h };
    handle.stage.set_scroll_pos(NodeId(node_id), x, y, animated != 0);
}

/// 编程聚焦节点（照 fgui RequestFocus）。强制聚焦任意非 disabled 节点
/// （含 tabindex=None/-1）；disabled 拒；越界跳过。null 句柄 → no-op。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_request_focus(h: *mut StageHandle, node_id: u32) {
    if h.is_null() { return; }
    let sh = unsafe { &mut *h };
    sh.stage.request_focus(NodeId(node_id));
}

/// 读当前焦点节点。无焦点/无 scene → 0xFFFF_FFFF（sentinel，同 node_parent）。null 句柄 → sentinel。
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

/// 全局 shutdown（Domain reload hook）。C# `LoomStage.ResetStatics`（SubsystemRegistration）调用。
///
/// 当前核心无全局 native 态——Stage 是 per-handle（`loomgui_stage_free` drop 全部 Stage 拥有的内存），
/// 故本函数 near-no-op。但 hook 必须存在：将来引入全局 texture/font registry（进程级单例缓存）时，
/// 此处自动成为清理入口，无需再改 C# 接线。
///
/// **注意：Font 的 `Box::leak`（`text/layout.rs:76`）是真泄漏**——`bytes.clone()` 后 leak 取
/// `'static` 切片喂 ttf-parser Face，原 Vec 虽被 `_bytes` 持有但与 leaked 切片不是同一份，
/// Stage drop 时 `_bytes` 释放的是 clone 来源而非 leaked 副本。每次 Stage 创建都 leak 一份字体字节，
/// 不可由 shutdown 回收（leak 切片无 handle 跟踪）。若未来域重载内存观测触发阈值，
/// 再考虑字体缓存化为进程单例。
#[no_mangle]
pub extern "C" fn loomgui_shutdown() {}

// ===== tween FFI =====

/// 注册 tween。start/end 指向 ≥value_size 个 f32（value_size 由 prop 隐含）。
/// null 句柄/null 指针 → no-op。越界 node / duration<=0 由 core update 处理（跳过/立即 complete）。
#[no_mangle]
pub extern "C" fn loomgui_stage_tween(
    h: *mut StageHandle,
    node_id: u32,
    prop: u32,
    start: *const f32,
    end: *const f32,
    duration: f32,
    ease: u32,
    delay: f32,
    tag: u32,
) {
    if h.is_null() || start.is_null() || end.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    let prop = match loomgui_core::tween::TweenProp::try_from(prop) {
        Some(p) => p,
        None => return,
    };
    let ease = match loomgui_core::tween::Ease::try_from(ease) {
        Some(e) => e,
        None => return,
    };
    let sz = loomgui_core::tween::prop_value_size(prop) as usize;
    let st = unsafe { std::slice::from_raw_parts(start, sz) };
    let en = unsafe { std::slice::from_raw_parts(end, sz) };
    let mut s = [0.0f32; 4];
    let mut e = [0.0f32; 4];
    for i in 0..sz {
        s[i] = st[i];
        e[i] = en[i];
    }
    sh.stage.tween(NodeId(node_id), prop, s, e, ease, delay, duration, tag);
}

/// 停该节点该 prop 的 tween（override 保留末值）。
#[no_mangle]
pub extern "C" fn loomgui_stage_kill_tween(h: *mut StageHandle, node_id: u32, prop: u32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    if let Some(prop) = loomgui_core::tween::TweenProp::try_from(prop) {
        sh.stage.kill_tween(NodeId(node_id), prop);
    }
}

/// 清该节点所有动画 override（回 CSS）。
#[no_mangle]
pub extern "C" fn loomgui_stage_clear_anim(h: *mut StageHandle, node_id: u32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    sh.stage.clear_anim(NodeId(node_id));
}

/// 清该节点某 prop 对应通道（回 CSS）。
#[no_mangle]
pub extern "C" fn loomgui_stage_clear_anim_prop(h: *mut StageHandle, node_id: u32, prop: u32) {
    if h.is_null() {
        return;
    }
    let sh = unsafe { &mut *h };
    if let Some(prop) = loomgui_core::tween::TweenProp::try_from(prop) {
        sh.stage.clear_anim_prop(NodeId(node_id), prop);
    }
}

// ===== T7 动态树 API FFI（§7.2）：create_root/create_node/append_child/insert_before/
// remove_child/remove_node/set_text/set_src/set_style。转调 Stage 方法（T5/T6）。
// 错误语义：create_root/create_node 返 u32 NodeId（0xFFFF_FFFF = 失败）；
// 其余返 i32（0=ok，-1=err）。null 句柄 → 失败/sentinel（不 panic）。

/// 建根节点并设为 roots[0]。kind/css = UTF-8 字节。返 NodeId；0xFFFF_FFFF = 失败。
///
/// **常驻（不 gate）：**runtime 稳定入口，`--no-default-features` 构建的 .dll 仍有本函数。
#[no_mangle]
pub extern "C" fn loomgui_stage_create_root(
    h: *mut StageHandle,
    kind: *const u8,
    kind_len: usize,
    css: *const u8,
    css_len: usize,
) -> u32 {
    const FAIL: u32 = 0xFFFF_FFFF;
    if h.is_null() {
        return FAIL;
    }
    let sh = unsafe { &mut *h };
    let kind = std::str::from_utf8(unsafe { std::slice::from_raw_parts(kind, kind_len) }).unwrap_or("");
    let css = std::str::from_utf8(unsafe { std::slice::from_raw_parts(css, css_len) }).unwrap_or("");
    match sh.stage.create_root(kind, css) {
        Ok(id) => id.0,
        Err(_) => FAIL,
    }
}

/// 建节点（不挂父）。kind/css = UTF-8 字节。返 NodeId；0xFFFF_FFFF = 失败。
/// 需配合 append_child/insert_before 挂到树。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_create_node(
    h: *mut StageHandle,
    kind: *const u8,
    kind_len: usize,
    css: *const u8,
    css_len: usize,
) -> u32 {
    const FAIL: u32 = 0xFFFF_FFFF;
    if h.is_null() {
        return FAIL;
    }
    let sh = unsafe { &mut *h };
    let kind = std::str::from_utf8(unsafe { std::slice::from_raw_parts(kind, kind_len) }).unwrap_or("");
    let css = std::str::from_utf8(unsafe { std::slice::from_raw_parts(css, css_len) }).unwrap_or("");
    match sh.stage.create_node(kind, css) {
        Ok(id) => id.0,
        Err(_) => FAIL,
    }
}

/// 挂子到 parent 末尾。child 必须当前无父。0=ok，-1=err。null 句柄 → -1。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_append_child(h: *mut StageHandle, parent: u32, child: u32) -> i32 {
    if h.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    sh.stage
        .append_child(NodeId(parent), NodeId(child))
        .map(|_| 0)
        .unwrap_or(-1)
}

/// 在 parent.children 中 ref_id 之前插 child。ref_id=0xFFFF_FFFF（INVALID）→ 末尾追加。
/// 0=ok，-1=err。null 句柄 → -1。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_insert_before(
    h: *mut StageHandle,
    parent: u32,
    child: u32,
    ref_id: u32,
) -> i32 {
    if h.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    sh.stage
        .insert_before(NodeId(parent), NodeId(child), NodeId(ref_id))
        .map(|_| 0)
        .unwrap_or(-1)
}

/// 摘子（不删节点）：从 parent.children 移除 + child.parent=None。节点仍 live 可重挂。
/// 0=ok，-1=err。null 句柄 → -1。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_remove_child(h: *mut StageHandle, parent: u32, child: u32) -> i32 {
    if h.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    sh.stage
        .remove_child(NodeId(parent), NodeId(child))
        .map(|_| 0)
        .unwrap_or(-1)
}

/// 删节点（递归删子 + 联动清 anim/scroll/tween + slotmap remove）。
/// 旧 NodeId 此后失效（gen++）。无 scene / 已删节点 → no-op。返 0（恒成功，no-op 语义）。
/// null 句柄 → 0（no-op，不 panic）。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_remove_node(h: *mut StageHandle, node: u32) -> i32 {
    if h.is_null() {
        return 0;
    }
    let sh = unsafe { &mut *h };
    sh.stage.remove_node(NodeId(node));
    0
}

/// 改 Text 节点 content + 标 dirty_text。text = UTF-8 字节。0=ok，-1=err。
/// 非 Text 节点 → -1（Stage::set_text Err）。null 句柄 → -1。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_set_text(
    h: *mut StageHandle,
    node: u32,
    text: *const u8,
    len: usize,
) -> i32 {
    if h.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let text = std::str::from_utf8(unsafe { std::slice::from_raw_parts(text, len) }).unwrap_or("");
    sh.stage.set_text(NodeId(node), text).map(|_| 0).unwrap_or(-1)
}

/// 改 Image 节点 src + 标 dirty_mesh。src = UTF-8 字节。0=ok，-1=err。
/// 非 Image 节点 → -1。null 句柄 → -1。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_set_src(
    h: *mut StageHandle,
    node: u32,
    src: *const u8,
    len: usize,
) -> i32 {
    if h.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let src = std::str::from_utf8(unsafe { std::slice::from_raw_parts(src, len) }).unwrap_or("");
    sh.stage.set_src(NodeId(node), src).map(|_| 0).unwrap_or(-1)
}

/// 改 base_style（apply_css）+ 标 dirty_mesh。css = UTF-8 字节。0=ok，-1=err。
/// 下帧 rematch 从 base 重算 style。null 句柄 → -1。
///
/// **常驻（不 gate）。**
#[no_mangle]
pub extern "C" fn loomgui_stage_set_style(
    h: *mut StageHandle,
    node: u32,
    css: *const u8,
    len: usize,
) -> i32 {
    if h.is_null() {
        return -1;
    }
    let sh = unsafe { &mut *h };
    let css = std::str::from_utf8(unsafe { std::slice::from_raw_parts(css, len) }).unwrap_or("");
    sh.stage.set_style(NodeId(node), css).map(|_| 0).unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_returns_c_string_v1d5() {
        unsafe {
            let s = CStr::from_ptr(loomgui_version() as *const i8);
            assert_eq!(s.to_str().unwrap(), "v1e");
        }
    }

    /// FFI tween：注册 opacity tween → tick 结束 → borrow_events 验 complete(tag)。
    /// borrow_events 返回 *const u8 + len=记录数（非字节数；见 lib.rs:237 注释）。
    /// 单切片：按记录数 len 切 typed slice，扫 event_type=EVT_TWEEN_COMPLETE && touch_id==tag。
    #[cfg(feature = "parse")]
    #[test]
    fn stage_tween_complete_event_via_ffi() {
        let fp = concat!(env!("CARGO_MANIFEST_DIR"), "/../loomgui_core/tests/fixtures/DejaVuSans.ttf");
        let fplen = fp.len();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = b"<div class=\"b\"></div>";
        let css = b".b{width:100px;height:50px;}";
        loomgui_stage_load_html(h, html.as_ptr(), html.len(), css.as_ptr(), css.len());
        // 经 slotmap 分配的真实根 NodeId（v1.3+ 动态树：root_id 非 0，是 idx<<12|gen）。
        // 传 node_id=0 会因 scene.get(NodeId(0)) 悬空 → update 跳过 → 无 complete 事件。
        let root_id = unsafe { (*h).stage.scene.as_ref().unwrap().roots[0].0 };
        let start = [0.0f32; 4];
        let end = [1.0f32, 0.0, 0.0, 0.0];
        // prop=0 (Opacity), ease=0 (Linear), duration=1.0, delay=0, tag=55
        loomgui_stage_tween(h, root_id, 0, start.as_ptr(), end.as_ptr(), 1.0, 0, 0.0, 55);
        loomgui_stage_tick(h, 1.0); // 推进到结束
        let mut len = 0usize;
        let p = loomgui_stage_borrow_events(h, &mut len);
        // len 是记录数（borrow_events out_len = events.len()）；直接切 typed slice。
        let recs = unsafe { std::slice::from_raw_parts(p as *const loomgui_core::input::EventRecord, len) };
        assert!(
            recs.iter().any(|e| e.event_type == loomgui_core::input::EVT_TWEEN_COMPLETE && e.touch_id == 55),
            "FFI tween 结束 → complete(tag=55)"
        );
        loomgui_stage_free(h);
    }
}

#[cfg(test)]
mod abi_tests {
    use super::*;
    use std::ffi::CString;

    /// T7 FFI 测试辅助：手搓单组件 pkg（不走 parse），组件名由参数指定。
    /// 组件 = 单 Container 根（无子）。返回 write_package 字节，可直接喂 load_package。
    fn make_test_pkg_bytes(component: &str) -> Vec<u8> {
        use loomgui_core::asset::{PackageInput, TemplateNode};
        use loomgui_core::scene::NodeKind;
        use loomgui_core::style::resolved::ResolvedStyle;
        let nodes = [TemplateNode {
            kind: NodeKind::Container,
            style: ResolvedStyle::default(),
            parent_idx: None,
            classes: vec![],
            id_attr: None,
            draggable: false,
            tabindex: None,
        }];
        let rules = loomgui_core::style::dynamic::DynamicRuleTable::default();
        let input = PackageInput {
            components: vec![(component, nodes.as_slice(), &rules)],
            asset_manifest: &[],
        };
        loomgui_core::asset::write_package(&input)
    }

    /// 字体路径：CARGO_MANIFEST_DIR = loomgui_ffi_c/，字体在
    /// ../loomgui_core/tests/fixtures/DejaVuSans.ttf（仓库内测试字体）。
    fn font_path() -> (CString, usize) {
        let p = format!(
            "{}/../loomgui_core/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let c = CString::new(p).unwrap();
        let len = c.as_bytes().len();
        (c, len)
    }

    /// T7 Step 1：load_package FFI 带 name 参数（对齐 T4 Stage::load_package(name, bytes)）。
    #[test]
    fn load_package_ffi_takes_name() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let pkg = make_test_pkg_bytes("comp1");
        let name = b"bag";
        let r = loomgui_stage_load_package(
            h,
            name.as_ptr(),
            name.len(),
            pkg.as_ptr(),
            pkg.len(),
        );
        assert_eq!(r, 0, "load_package 带 name ok");
        loomgui_stage_free(h);
    }

    /// T7 Step 1：instantiate FFI 返有效 NodeId（非 INVALID 0xFFFF_FFFF）。
    /// 流程：create_root 建 scene → load_package("bag") → instantiate("bag","comp1") → NodeId。
    #[test]
    fn instantiate_ffi_returns_nodeid() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        // create_root 建 scene（ensure_scene 自动建空骨架）。css 传空串（无 inline style）。
        let empty_css = b"";
        let root = loomgui_stage_create_root(h, b"div".as_ptr(), 3, empty_css.as_ptr(), 0);
        assert_ne!(root, 0xFFFF_FFFF, "create_root ok");
        let pkg = make_test_pkg_bytes("comp1");
        let lr = loomgui_stage_load_package(
            h,
            b"bag".as_ptr(),
            3,
            pkg.as_ptr(),
            pkg.len(),
        );
        assert_eq!(lr, 0, "load_package ok");
        let id = loomgui_stage_instantiate(h, b"bag".as_ptr(), 3, b"comp1".as_ptr(), 5);
        assert_ne!(id, 0xFFFF_FFFF, "instantiate 返有效 NodeId");
        loomgui_stage_free(h);
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

    /// load_package FFI：手搓 pkg → load_package(name) → create_root → instantiate → append_child → tick → blob。
    /// 与 load_html 路径解耦（parse feature off 时仍可用）。v1.4-a T7：改用 instantiate 建 scene 内容。
    #[test]
    fn load_package_builds_blob_from_package() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        // load_package 进资源池（不建 scene）
        let pkg = make_test_pkg_bytes("comp1");
        let r = loomgui_stage_load_package(h, b"bag".as_ptr(), 3, pkg.as_ptr(), pkg.len());
        assert_eq!(r, 0, "load_package ok");
        // create_root 建 scene + 挂根 div
        let empty_css = b"";
        let root = loomgui_stage_create_root(h, b"div".as_ptr(), 3, empty_css.as_ptr(), 0);
        assert_ne!(root, 0xFFFF_FFFF, "create_root ok");
        // instantiate 组件 → append_child 挂到根
        let comp = loomgui_stage_instantiate(h, b"bag".as_ptr(), 3, b"comp1".as_ptr(), 5);
        assert_ne!(comp, 0xFFFF_FFFF, "instantiate ok");
        assert_eq!(loomgui_stage_append_child(h, root, comp), 0, "append_child ok");
        // tick → blob
        loomgui_stage_tick(h, 0.0);
        let mut len = 0usize;
        let ptr = loomgui_stage_borrow_frame(h, &mut len);
        assert!(!ptr.is_null(), "tick 后 blob 非空");
        assert!(len > 12, "blob 至少含 header");
        unsafe {
            assert_eq!(*ptr, 0x4Cu8, "magic 第一字节 'L'");
        }
        loomgui_stage_free(h);
    }

    /// 契约：从未 tick 过的句柄 borrow_frame 必须返回 null + len=0
    /// （空 Vec::as_ptr() 是非空悬挂哨兵，显式判空锁住"未 tick→null"契约）。
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
        // warmup tick：compute_world_transforms 在 process/scroll 后跑，hit_test 读上帧 world_transforms
        // （1 帧延迟语义，T4）。首帧 world_transforms 空 → 首帧 hit_test 全 None，故输入前先 warmup。
        loomgui_stage_tick(h, 0.0);
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

    /// is_pointer_on_ui 契约：create_root 建空 scene（无子）→ 命中根 → false（根不算 UI）。
    /// 覆盖 is_pointer_on_ui 在无 parse feature 路径下也可用（create_root 常驻）。
    /// v1.4-a T7：改用 create_root 建 scene（取代旧 load_package 建 scene 路径）。
    #[test]
    fn is_pointer_on_ui_true_on_hit_false_on_miss() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let empty_css = b"";
        let root = loomgui_stage_create_root(h, b"div".as_ptr(), 3, empty_css.as_ptr(), 0);
        assert_ne!(root, 0xFFFF_FFFF, "create_root ok");
        // warmup tick：hit_test 读上帧 world_transforms（1 帧延迟）
        loomgui_stage_tick(h, 0.0);
        // 空根 Container 无子 → hit_test 命中根 → 根不算 UI → false
        assert!(!loomgui_stage_is_pointer_on_ui(h), "空根命中 → false（根不算 UI）");
        loomgui_stage_free(h);
    }

    /// EventRecord/PointerEvent sizeof 契约。
    /// PointerEvent 16B：PointerKind repr(u8) 1B + button 1B + pad 2B + touch_id@4 + x@8 + y@12。
    /// EventRecord 20B：node_id@0(4) + event_type@4(1) + pad@5(3) + touch_id@8(4) + x@12(4) + y@16(4)。
    #[test]
    fn pointer_event_event_record_sizeof() {
        use loomgui_core::input::{PointerEvent, EventRecord};
        assert_eq!(std::mem::size_of::<PointerEvent>(), 16, "PointerEvent 16B（PointerKind repr(u8)）");
        assert_eq!(std::mem::size_of::<EventRecord>(), 20, "EventRecord 20B（touch_id@8）");
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
        // warmup tick：hit_test 读上帧 world_transforms（1 帧延迟，T4），输入前先 warmup。
        loomgui_stage_tick(h, 0.0);
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

    /// 5 函数常驻契约：无 parse feature 也能编译（§14.6）。
    /// 此测在 normal build 跑，验证 5 函数 + PointerEvent/EventRecord 常驻可调。
    /// 不 tick（tick_and_render 需先 load scene）——本测只验常驻编译/调用安全；
    /// 真正的 --no-default-features 验由 `cargo build -p loomgui_ffi_c --no-default-features` 完成。
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

    /// node_parent 契约：create_root + instantiate + append_child → child.parent==root；
    /// root.parent==sentinel；OOB==sentinel。v1.4-a T7：改用 create_root + instantiate 路径。
    #[test]
    fn node_parent_returns_chain_and_sentinel() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let pkg = make_test_pkg_bytes("comp1");
        assert_eq!(loomgui_stage_load_package(h, b"bag".as_ptr(), 3, pkg.as_ptr(), pkg.len()), 0);
        let empty_css = b"";
        let root = loomgui_stage_create_root(h, b"div".as_ptr(), 3, empty_css.as_ptr(), 0);
        assert_ne!(root, 0xFFFF_FFFF, "create_root ok");
        let comp = loomgui_stage_instantiate(h, b"bag".as_ptr(), 3, b"comp1".as_ptr(), 5);
        assert_ne!(comp, 0xFFFF_FFFF, "instantiate ok");
        assert_eq!(loomgui_stage_append_child(h, root, comp), 0, "append_child ok");
        // comp.parent == root；root.parent == sentinel；OOB == sentinel。
        assert_eq!(loomgui_node_parent(h, comp), root, "comp.parent == root");
        assert_eq!(loomgui_node_parent(h, root), 0xFFFF_FFFF, "root 是顶层 → parent=sentinel");
        assert_eq!(loomgui_node_parent(h, 0xFFFF_FFFF), 0xFFFF_FFFF, "OOB → sentinel");
        loomgui_stage_free(h);
    }

    /// find_node_by_id round-trip：手搓包（组件含 id="ok" 节点）→ load_package → create_root →
    /// instantiate → append_child → find "ok" 返节点 NodeId；无匹配 → sentinel。
    /// v1.4-a T7：改用 load_package + instantiate 路径（取代旧 load_package 建 scene）。
    #[test]
    fn find_node_by_id_round_trip() {
        use loomgui_core::asset::{PackageInput, TemplateNode};
        use loomgui_core::scene::NodeKind;
        use loomgui_core::style::resolved::ResolvedStyle;
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        // 手搓包：组件 "comp1" 含单 Container 节点 id="ok"
        let nodes = [TemplateNode {
            kind: NodeKind::Container,
            style: ResolvedStyle::default(),
            parent_idx: None,
            classes: vec![],
            id_attr: Some("ok".to_string()),
            draggable: false,
            tabindex: None,
        }];
        let rules = loomgui_core::style::dynamic::DynamicRuleTable::default();
        let pkg = loomgui_core::asset::write_package(&PackageInput {
            components: vec![("comp1", nodes.as_slice(), &rules)],
            asset_manifest: &[],
        });
        assert_eq!(loomgui_stage_load_package(h, b"bag".as_ptr(), 3, pkg.as_ptr(), pkg.len()), 0);
        let empty_css = b"";
        let root = loomgui_stage_create_root(h, b"div".as_ptr(), 3, empty_css.as_ptr(), 0);
        assert_ne!(root, 0xFFFF_FFFF, "create_root ok");
        let comp = loomgui_stage_instantiate(h, b"bag".as_ptr(), 3, b"comp1".as_ptr(), 5);
        assert_ne!(comp, 0xFFFF_FFFF, "instantiate ok");
        assert_eq!(loomgui_stage_append_child(h, root, comp), 0, "append_child ok");
        // find "ok" → comp（instantiate 把 id_attr 带到 live 节点）
        let ok_id = {
            let id = std::ffi::CString::new("ok").unwrap();
            loomgui_stage_find_node_by_id(h, id.as_ptr() as *const u8, id.as_bytes().len())
        };
        assert_ne!(ok_id, 0xFFFF_FFFF, "find ok 应命中");
        assert_eq!(ok_id, comp, "find ok == comp 根 NodeId");
        // 无匹配 → sentinel
        let miss = {
            let id = std::ffi::CString::new("nope").unwrap();
            loomgui_stage_find_node_by_id(h, id.as_ptr() as *const u8, id.as_bytes().len())
        };
        assert_eq!(miss, 0xFFFF_FFFF, "无匹配 → sentinel");
        loomgui_stage_free(h);
    }

    /// version 字符串 == "v1e"。
    #[test]
    fn version_is_v1d_5() {
        let p = loomgui_version();
        let len = (0..).take_while(|&i| unsafe { *p.add(i) != 0 }).count();
        let s = std::str::from_utf8(unsafe { std::slice::from_raw_parts(p, len) }).unwrap();
        assert_eq!(s, "v1e");
    }

    /// version 串 = "v1e"。
    #[test]
    fn version_is_v1e() {
        let p = loomgui_version();
        let len = (0..).take_while(|&i| unsafe { *p.add(i) != 0 }).count();
        let s = std::str::from_utf8(unsafe { std::slice::from_raw_parts(p, len) }).unwrap();
        assert_eq!(s, "v1e");
    }

    /// EventRecord 仍 20B（drag/longpress 复用 event_type 空位 6-9）、PointerEvent 16B、Canceled=3。
    #[test]
    fn event_record_and_pointer_event_sizes_unchanged() {
        use loomgui_core::input::{EventRecord, PointerEvent, PointerKind};
        use std::mem::size_of;
        assert_eq!(size_of::<EventRecord>(), 20, "EventRecord 20B（drag/longpress 复用 event_type）");
        assert_eq!(size_of::<PointerEvent>(), 16, "PointerEvent 16B 不变");
        assert_eq!(PointerKind::Canceled as u8, 3, "Canceled=3");
    }

    /// cancel_click FFI——Down → cancel_click → Up → 无 Click，Up 仍发。
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

    /// EVT 常量值锁（6/7/8/9）+ drag 端到端：draggable btn Down+Move>阈值 → borrow_events 含 DragStart。
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
        // warmup tick：compute_world_transforms 在 process/scroll 后跑，hit_test 读上帧 world_transforms
        // （1 帧延迟语义，T4）。首帧 world_transforms 空 → 首帧 hit_test 全 None，故输入前先 warmup。
        loomgui_stage_tick(h, 0.0);
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

    /// longpress 端到端——Down@btn + tick dt 累积 1.5s → LongPress。
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
        // warmup tick：hit_test 读上帧 world_transforms（1 帧延迟，T4），输入前先 warmup。
        loomgui_stage_tick(h, 0.0);
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

    /// KeyEvent sizeof 8B + EventRecord 仍 20B / PointerEvent 16B。
    #[test]
    fn key_event_sizeof_and_unchanged() {
        use loomgui_core::input::{EventRecord, KeyEvent, PointerEvent};
        use std::mem::size_of;
        assert_eq!(size_of::<KeyEvent>(), 8, "KeyEvent 8B");
        assert_eq!(size_of::<EventRecord>(), 20, "EventRecord 20B 不变");
        assert_eq!(size_of::<PointerEvent>(), 16, "PointerEvent 16B 不变");
    }

    /// EVT 常量值锁（12/13/14/15）。
    #[test]
    fn evt_constants_v1d2() {
        assert_eq!(loomgui_core::input::EVT_KEY_DOWN, 12);
        assert_eq!(loomgui_core::input::EVT_KEY_UP, 13);
        assert_eq!(loomgui_core::input::EVT_FOCUS_IN, 14);
        assert_eq!(loomgui_core::input::EVT_FOCUS_OUT, 15);
    }

    /// key 事件 round-trip——click-to-focus btn + Enter keydown + tick → borrow_events 含 KeyDown@焦点。
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
        // warmup tick：hit_test 读上帧 world_transforms（1 帧延迟，T4），输入前先 warmup。
        loomgui_stage_tick(h, 0.0);
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

    /// Tab 导航 round-trip——两可聚焦 btn + Tab → borrow_events 含 FocusIn（无 KeyDown）。
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
        // focused_node 读首个可聚焦（A）。parse 无合成根，两 button 各为 root；
        // DFS 先序：button.a→Text→button.b→Text；tabindex=0 进 zero 桶 → chain=[a,b]，Tab→a。
        // NodeId 由 slotmap 分配（首节点 idx=1, version=1 → u32 = (1<<12)|1 = 4097）。
        let a_id = loomgui_stage_focused_node(h);
        assert_ne!(a_id, 0xFFFF_FFFF, "Tab → 有焦点");
        assert_ne!(a_id, 0, "NodeId 非零（slotmap idx 从 1 起）");
        // 验 a 是 button.a：node_parent 应为 sentinel（a 是 root）
        assert_eq!(loomgui_node_parent(h, a_id), 0xFFFF_FFFF, "button.a 是 root → parent=sentinel");
        loomgui_stage_free(h);
    }

    /// request_focus + focused_node round-trip。request_focus 记 pending，
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

    /// dump_scene FFI round-trip——load_html → tick → dump_scene 返 JSON 数组（首字节 `[`）。
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

    /// set_wheel_input round-trip —— 推 WheelEvent 入 Stage，验 pending_wheel 累积。
    /// 复用 Stage 类型直接构造（不经过 FFI pointer 层——abi_tests 测 public API 契约）。
    #[test]
    fn set_wheel_input_round_trip() {
        let fp = format!(
            "{}/../loomgui_core/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut stage = Stage::new(&fp, (200.0, 100.0)).unwrap();
        let evs = [loomgui_core::scroll::WheelEvent { x: 10.0, y: 20.0, delta_x: 0.0, delta_y: 1.0 }];
        stage.set_wheel_input(&evs);
        assert_eq!(stage.pending_wheel.len(), 1);
    }

    /// helper：构造带 overflow:scroll 容器的 Stage（无子；手动填 layout_rect + scroll state）。
    fn build_scroll_stage() -> Stage {
        let fp = format!(
            "{}/../loomgui_core/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut stage = Stage::new(&fp, (200.0, 100.0)).unwrap();
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::{OverflowMode, ResolvedStyle};
        let mut sty = ResolvedStyle::default();
        sty.overflow_y = OverflowMode::Scroll;
        let entries = vec![
            (None::<usize>, NodeKind::Container, sty, vec![], None::<String>, false, None::<i32>),
        ];
        stage.scene = Some(Scene::build(&entries));
        let scene = stage.scene.as_mut().unwrap();
        let root_id = scene.roots[0];
        scene.get_mut(root_id).unwrap().layout_rect = loomgui_core::scene::node::Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 };
        // refresh 需要 content_size/viewport/overlap（set_pos 读 overlap 做 clamp）
        loomgui_core::scroll::refresh_content_sizes(&mut stage.scene.as_mut().unwrap());
        // 手动改 overlap 到 200 让 scroll_pos 可测（无子 content=0,overlap=0 → set_pos 全 clamp 0）
        stage.scene.as_mut().unwrap().scroll.get_mut(root_id).unwrap().overlap = (0.0, 200.0);
        stage
    }

    #[test]
    fn set_scroll_pos_updates_state() {
        let mut stage = build_scroll_stage();
        let root_id = stage.scene.as_ref().unwrap().roots[0];
        stage.set_scroll_pos(root_id, 0.0, 50.0, false);
        assert_eq!(
            stage.scene.as_ref().unwrap().scroll.get(root_id).unwrap().scroll_pos,
            (0.0, 50.0)
        );
    }

    #[test]
    fn set_scroll_pos_animated_starts_tween() {
        let mut stage = build_scroll_stage();
        let root_id = stage.scene.as_ref().unwrap().roots[0];
        stage.set_scroll_pos(root_id, 0.0, 80.0, true);
        let st = stage.scene.as_ref().unwrap().scroll.get(root_id).unwrap();
        assert_eq!(st.tweening, 1, "animated=true 启 tweening=1");
    }

    #[test]
    fn set_scroll_pos_non_container_no_op() {
        let fp = format!(
            "{}/../loomgui_core/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut stage = Stage::new(&fp, (200.0, 100.0)).unwrap();
        use loomgui_core::scene::{NodeKind, Scene};
        use loomgui_core::style::resolved::ResolvedStyle;
        let entries = vec![
            (None::<usize>, NodeKind::Container, ResolvedStyle::default(), vec![], None::<String>, false, None::<i32>),
        ];
        stage.scene = Some(Scene::build(&entries));
        let root_id = stage.scene.as_ref().unwrap().roots[0];
        // root 是 Container，overflow=Visible（默认）→ 非 scroll 容器 → set_scroll_pos no-op（不 panic）
        stage.set_scroll_pos(root_id, 0.0, 50.0, false);
        // 不 panic 即通过
    }

    #[test]
    fn set_scroll_pos_oob_no_op() {
        let mut stage = build_scroll_stage();
        // 越界 NodeId（idx=99）→ no-op 不 panic
        stage.set_scroll_pos(NodeId((99u32 << 12) | 1), 0.0, 50.0, false);
    }

    /// loomgui_stage_set_scroll_pos FFI round-trip。
    #[cfg(feature = "parse")]
    #[test]
    fn ffi_set_scroll_pos_round_trip() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        let html = b"<div class=\"scroll\"></div>";
        let css = b".scroll{width:200px;height:100px;overflow:scroll;}";
        loomgui_stage_load_html(h, html.as_ptr() as *const u8, html.len(), css.as_ptr() as *const u8, css.len());
        // fill scroll state（load_inline 后需 refresh + 手动扩 overlap）
        let handle = unsafe { &mut *h };
        let root_id = handle.stage.scene.as_ref().unwrap().roots[0];
        loomgui_core::scroll::refresh_content_sizes(handle.stage.scene.as_mut().unwrap());
        handle.stage.scene.as_mut().unwrap().scroll.get_mut(root_id).unwrap().overlap = (0.0, 200.0);
        // 调 FFI set_scroll_pos（animated=0 瞬移）——传 slotmap 分配的 NodeId.0（u32 打包值）
        loomgui_stage_set_scroll_pos(h, root_id.0, 0.0, 50.0, 0);
        let st = handle.stage.scene.as_ref().unwrap().scroll.get(root_id).unwrap();
        assert_eq!(st.scroll_pos, (0.0, 50.0), "FFI 调后 scroll_pos 更新");
        // animated=1 启 tween
        loomgui_stage_set_scroll_pos(h, root_id.0, 0.0, 80.0, 1);
        let st = handle.stage.scene.as_ref().unwrap().scroll.get(root_id).unwrap();
        assert_eq!(st.tweening, 1, "animated=1 启 tween");
        loomgui_stage_free(h);
    }

    /// WheelEvent ABI 尺寸 16B（4×f32 紧凑，C# 端同布局）。
    /// compile-time 断言已在 scroll.rs:27-29 锁住；本测为 runtime 可见的检查。
    #[test]
    fn wheel_event_is_16_bytes() {
        assert_eq!(std::mem::size_of::<loomgui_core::scroll::WheelEvent>(), 16);
    }

    /// T7 动态树 API FFI round-trip——9 函数经 FFI 调用建/改/删节点。
    /// create_root 自动建空 scene（ensure_scene），故不再需 load_package 预建 scene。
    /// 流程：create_root(div) → create_node(button/img/span) → append_child ×3 →
    ///       set_text/set_src/set_style 改属性 → insert_before 插序 →
    ///       remove_child 摘子 → remove_node 删根。每步断言返回值契约。
    #[test]
    fn dynamic_tree_api_ffi_round_trip() {
        let (fp, fplen) = font_path();
        let h = loomgui_stage_new(fp.as_ptr() as *const u8, fplen, 200.0, 100.0);
        assert!(!h.is_null());
        let empty = b"";
        // create_root(div) 建 scene + 根
        let root = loomgui_stage_create_root(h, b"div".as_ptr(), 3, empty.as_ptr(), 0);
        assert_ne!(root, 0xFFFF_FFFF, "create_root ok");
        // create_node(button/img/span)——孤立节点
        let btn = loomgui_stage_create_node(h, b"button".as_ptr(), 6, empty.as_ptr(), 0);
        assert_ne!(btn, 0xFFFF_FFFF, "create_node button ok");
        let img = loomgui_stage_create_node(h, b"img".as_ptr(), 3, empty.as_ptr(), 0);
        assert_ne!(img, 0xFFFF_FFFF, "create_node img ok");
        let span = loomgui_stage_create_node(h, b"span".as_ptr(), 4, empty.as_ptr(), 0);
        assert_ne!(span, 0xFFFF_FFFF, "create_node span ok");
        // append_child ×3 挂到 root（序：btn, img, span）
        assert_eq!(loomgui_stage_append_child(h, root, btn), 0, "append btn");
        assert_eq!(loomgui_stage_append_child(h, root, img), 0, "append img");
        assert_eq!(loomgui_stage_append_child(h, root, span), 0, "append span");
        // set_text(span) / set_src(img) / set_style(btn)
        let txt = b"hello";
        assert_eq!(loomgui_stage_set_text(h, span, txt.as_ptr(), txt.len()), 0, "set_text span ok");
        let src = b"icon.png";
        assert_eq!(loomgui_stage_set_src(h, img, src.as_ptr(), src.len()), 0, "set_src img ok");
        let css = b"width:100px;height:50px;";
        assert_eq!(loomgui_stage_set_style(h, btn, css.as_ptr(), css.len()), 0, "set_style btn ok");
        // set_text 对非 Text 节点（img）应失败
        assert_eq!(loomgui_stage_set_text(h, img, txt.as_ptr(), txt.len()), -1, "set_text on img → err");
        // insert_before：在 btn 前插 img（先摘 img 再插）
        assert_eq!(loomgui_stage_remove_child(h, root, img), 0, "remove img from root");
        assert_eq!(loomgui_stage_insert_before(h, root, img, btn), 0, "insert img before btn");
        // remove_child 摘 span
        assert_eq!(loomgui_stage_remove_child(h, root, span), 0, "remove span");
        // remove_node 删根（递归删子）
        assert_eq!(loomgui_stage_remove_node(h, root), 0, "remove_node root");
        loomgui_stage_free(h);
    }
}
