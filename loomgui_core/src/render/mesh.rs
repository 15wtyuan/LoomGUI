//! MeshFactory：quad 几何生成。
//!
//! 仅 quad（背景色块 / 图片占位）。UV 由 uv_min/uv_max 指定——纯色块/散图
//! 全图 = (0,0)-(1,1)；atlas sprite = 子区。返回 SOA 四表，
//! 与 `NodePayload::Mesh` 同形。

use crate::scene::node::Rect;

/// 生成一个 quad 的 verts/uvs/colors/indices。
///
/// - 4 顶点（左上 → 右上 → 右下 → 左下，CCW）。
/// - UV 由 uv_min/uv_max 指定：TL→(umin,vmin) TR→(umax,vmin) BR→(umax,vmax) BL→(umin,vmax)。
///   纯色块/散图全图 = [0,0],[1,1]；atlas sprite = 子区。
/// - 4 顶点同色（quad 单色）。
/// - 两个三角形：`(0,1,2)` + `(0,2,3)`。
///
/// 用于 Container/Button 背景色块、Image 占位贴图 quad。
///
/// 返回 SOA 四表（verts/uvs/colors/indices），与 `NodePayload::Mesh` 字段同形，
/// 调用方直接解构装 payload。不为 clippy 折叠成 type 别名（单一调用点，别名徒增间接）。
#[allow(clippy::type_complexity)]
pub fn quad(
    rect: &Rect,
    color: [f32; 4],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) {
    let verts = vec![
        [rect.x, rect.y],
        [rect.x + rect.w, rect.y],
        [rect.x + rect.w, rect.y + rect.h],
        [rect.x, rect.y + rect.h],
    ];
    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);
    let uvs = vec![[umin, vmin], [umax, vmin], [umax, vmax], [umin, vmax]];
    let colors = vec![color; 4];
    let indices = vec![0, 1, 2, 0, 2, 3];
    (verts, uvs, colors, indices)
}

/// 生成圆角矩形的 verts/uvs/colors/indices（SOA 四表，与 quad 同形）。
///
/// - 三角扇：中心点 + 4 角圆弧顶点，三角形 (0, i, i+1) 连接，末尾回 1 闭合。
/// - radii = [TL, TR, BR, BL]，每角 (h, v) 像素半径（本函数内钳制）。
/// - UV 线性映射：顶点归一化位置 (pos-rect.xy)/rect.size × (uv_max-uv_min) + uv_min。
///   与 fit_uv 共存：uv_min/uv_max 即 fit_uv 算出的子区。
/// - 产 design y-down 坐标；v 翻转由调用点交换 uv v 处理（同 quad）。
/// - 退化（w/h≤0）走 quad fallback。
#[allow(clippy::type_complexity)]
pub fn rounded_rect(
    rect: &Rect,
    color: [f32; 4],
    radii: &[(f32, f32); 4],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) {
    let (w, h) = (rect.w, rect.h);
    if w <= 0.0 || h <= 0.0 {
        return quad(rect, color, uv_min, uv_max);
    }
    // 改进 1：CSS 按边缩放钳制（vs fgui per-corner min）。两邻角半径和不超过边长，等比缩放；
    // 只缩不放（min(1.0) 兜底）；防负 max(0.0)。
    let (tl, tr, br, bl) = (radii[0], radii[1], radii[2], radii[3]);
    let scale = 1.0_f32
        .min(w / (tl.0 + tr.0).max(1e-6))
        .min(w / (bl.0 + br.0).max(1e-6))
        .min(h / (tl.1 + bl.1).max(1e-6))
        .min(h / (tr.1 + br.1).max(1e-6));
    let scale_r = |r: (f32, f32)| -> (f32, f32) {
        ((r.0 * scale).max(0.0), (r.1 * scale).max(0.0))
    };
    let tl = scale_r(tl);
    let tr = scale_r(tr);
    let br = scale_r(br);
    let bl = scale_r(bl);

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);

    let mut verts: Vec<[f32; 2]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    // 中心点（索引 0）
    let cx = rect.x + w / 2.0;
    let cy = rect.y + h / 2.0;
    verts.push([cx, cy]);
    uvs.push([lerp(umin, umax, 0.5), lerp(vmin, vmax, 0.5)]);
    colors.push(color);

    // 改进 2：角序 TL→TR→BR→BL（CSS 视觉序）。
    // 起始角 TL=π, TR=-π/2, BR=0, BL=π/2（逆时针 design y-down）。圆心 = 角顶点内缩 (rx,ry)。
    // 每角附矩形顶点 corner：直角分支（rx<=0||ry<=0）直接落矩形角，不靠圆心+方向
    // （否则 rx=0 ry>0 时 py=圆心.y+sin·ry 偏离角顶点，角附近镂空）。
    let corners: [(f32, f32, [f32; 2], f32, [f32; 2]); 4] = [
        (tl.0, tl.1, [rect.x + tl.0,         rect.y + tl.1],         std::f32::consts::PI,            [rect.x,     rect.y]),
        (tr.0, tr.1, [rect.x + w - tr.0,     rect.y + tr.1],         -std::f32::consts::FRAC_PI_2,    [rect.x + w, rect.y]),
        (br.0, br.1, [rect.x + w - br.0,     rect.y + h - br.1],     0.0,                             [rect.x + w, rect.y + h]),
        (bl.0, bl.1, [rect.x + bl.0,         rect.y + h - bl.1],     std::f32::consts::FRAC_PI_2,     [rect.x,     rect.y + h]),
    ];
    for (rx, ry, center, start, corner) in corners {
        if rx <= 0.0 || ry <= 0.0 {
            // 直角：单顶点 = 该角矩形顶点（ry>0 时圆心+方向会偏移，故直接用 corner）。
            verts.push(corner);
            uvs.push([lerp(umin, umax, (corner[0] - rect.x) / w), lerp(vmin, vmax, (corner[1] - rect.y) / h)]);
            colors.push(color);
            continue;
        }
        // 自适应分段：ceil(π·max(rx,ry)/4)+1，最小 2（每 ~4px 弧长一段，加密自 fgui /8 消圆角毛刺）
        let sides = ((std::f32::consts::PI * rx.max(ry) / 4.0).ceil() as i32 + 1).max(2);
        let delta = std::f32::consts::FRAC_PI_2 / sides as f32;
        for j in 0..=sides {
            let a = if j == sides {
                start + std::f32::consts::FRAC_PI_2  // 末段精度锁（照 fgui）
            } else {
                start + delta * j as f32
            };
            let px = center[0] + a.cos() * rx;
            let py = center[1] + a.sin() * ry;
            verts.push([px, py]);
            uvs.push([lerp(umin, umax, (px - rect.x) / w), lerp(vmin, vmax, (py - rect.y) / h)]);
            colors.push(color);
        }
    }
    // 三角扇：(0, i, i+1)，末尾回 1 闭合
    let n = verts.len() as u32;
    let mut indices: Vec<u32> = Vec::new();
    for i in 1..n {
        let next = if i + 1 < n { i + 1 } else { 1 };
        indices.extend_from_slice(&[0, i, next]);
    }
    (verts, uvs, colors, indices)
}

/// 生成九宫格切片矩形的 verts/uvs/colors/indices（照搬 fgui Image.cs SliceFill）。
///
/// - 4×4 顶点（gridX/gridY 各 4 值 = 3 段切分）+ 9 quad（TRIANGLES_9_GRID 固定索引）。
/// - slice = 源图像素切片量（已 resolve，% 已乘源图边）。
/// - src_w/src_h = 源图像素尺寸（算 UV 切片比例）。
/// - 四角不缩放、四边单轴拉伸、中心双轴拉伸。
/// - clamp：rect 比源图小时四角重叠；`slice.left.min(w*0.5)` 防左切片过半，`.max(grid_x[1])` 防右切片线越过左切片线（clamp 策略等效 fgui SliceFill 防四角越界，实现用 `min(w*0.5)` + `.max()`）。
/// - UV 由 uv_min/uv_max 指定（atlas 子区），按切片像素比例切。v 翻转由调用点交换 uv v 处理。
#[allow(clippy::type_complexity)]
pub fn nine_slice(
    rect: &Rect,
    color: [f32; 4],
    slice: &crate::style::resolved::SliceInsets,
    src_w: f32,
    src_h: f32,
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) {
    let (w, h) = (rect.w, rect.h);
    if w <= 0.0 || h <= 0.0 {
        return quad(rect, color, uv_min, uv_max);
    }
    // gridX/Y：rect 坐标 4 值（左、左+sliceL、右-sliceR、右），clamp 防四角越界
    let grid_x = [
        rect.x,
        rect.x + slice.left.min(w * 0.5),
        (rect.x + w - slice.right).max(rect.x + slice.left.min(w * 0.5)),
        rect.x + w,
    ];
    let grid_y = [
        rect.y,
        rect.y + slice.top.min(h * 0.5),
        (rect.y + h - slice.bottom).max(rect.y + slice.top.min(h * 0.5)),
        rect.y + h,
    ];
    // UV 切片线：按 slice 像素 / src 尺寸 比例
    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);
    let sx = (umax - umin) / src_w.max(1e-6);
    let sy = (vmax - vmin) / src_h.max(1e-6);
    let tex_x = [umin, umin + slice.left * sx, umin + (src_w - slice.right) * sx, umax];
    let tex_y = [vmin, vmin + slice.top * sy, vmin + (src_h - slice.bottom) * sy, vmax];

    let mut verts: Vec<[f32; 2]> = Vec::with_capacity(16);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(16);
    // 行主序 4×4：行 r (0..4) × 列 c (0..4)
    for r in 0..4 {
        for c in 0..4 {
            verts.push([grid_x[c], grid_y[r]]);
            uvs.push([tex_x[c], tex_y[r]]);
        }
    }
    let colors = vec![color; 16];
    // TRIANGLES_9_GRID（照搬 fgui Image.cs:267）：9 quad，每 quad (BL,TL,TR)+(TR,BR,BL) = (a,b,c)+(c,d,a)
    let indices: Vec<u32> = vec![
        4,0,1, 1,5,4,    // 行 0 列 0/1
        5,1,2, 2,6,5,    // 行 0 列 1/2
        6,2,3, 3,7,6,    // 行 0 列 2/3
        8,4,5, 5,9,8,    // 行 1
        9,5,6, 6,10,9,
        10,6,7, 7,11,10,
        12,8,9, 9,13,12, // 行 2
        13,9,10, 10,14,13,
        14,10,11, 11,15,14, // 行 3
    ];
    (verts, uvs, colors, indices)
}

/// 九宫格 + 圆角共存 mesh（spec §3.7，fgui 无现成，LoomGUI 自设计）。
///
/// - radius 全 0 → 退化 nine_slice（方角）。
/// - 有 radius → 四角圆弧扇（**纯几何圆角，无外角顶点 / 无边三角**，照 rounded_rect 语义：
///   弧外 fragment 不绘制）+ 角区 L 形补齐 + 四边/中心切片拉伸。
/// - 不变量：四角不拉伸（1:1 映射源角像素），四边单轴拉伸，中心双轴拉伸。
/// - 圆角剪裁是**几何**的——弧外区域（rect 角的 `[0,r]² \ quarter-disc`）无 fragment，
///   不靠源图 alpha 伪造。即便源图角像素不透明，圆角仍正确呈现。
///
/// ## 区域分解（修复 review Critical 1 覆盖间隙 + Important 2 几何圆角）
///
/// X 边界（去重排序后）`{0, r_l, slice_l, w-slice_r, w-r_r, w}`，Y 边界同理
/// `{0, r_t, slice_t, h-slice_b, h-r_b, h}`——把 rect 切成至多 6×6 网格。每格分类：
///
/// 1. **四分之一圆弧格**（角区 `[0,slice]²` 内的 `[0,r]²` 子格）：仅发三角扇覆盖
///    `[0,r]² ∩ quarter-disc`（弧内），弧外 `[0,r]² \ disc` **不发顶点**（圆角镂空）。
///    无外角顶点、无边三角——修复 Important 2。
/// 2. **角区 L 形格**（角区 `[0,slice]²` 内四分之一圆弧格之外的部分，即
///    `[r,slice]×[0,r]` / `[0,r]×[r,slice]` / `[r,slice]×[r,slice]`）：发 quad，UV 1:1
///    映射源角像素（不拉伸）——填满弧与切片线之间的角区，修复 Critical 1 间隙。
/// 3. **边带格**（角区与中心之间 `[slice, w-slice]` 或 `[slice, h-slice]` 段）：发 quad，
///    单轴拉伸 UV。
/// 4. **中心格** `[slice,w-slice]×[slice,h-slice]`：发 quad，双轴拉伸 UV。
///
/// 网格边界本身处理 UV 不连续（角区 vs 边带 vs 中心 UV 分段落在不同格），无需在单 quad
/// 内硬接不连续 UV——每格独立 quad / 扇形。
#[allow(clippy::type_complexity)]
pub fn nine_slice_rounded(
    rect: &Rect,
    color: [f32; 4],
    slice: &crate::style::resolved::SliceInsets,
    radii: &[(f32, f32); 4],
    src_w: f32,
    src_h: f32,
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) {
    let (w, h) = (rect.w, rect.h);
    if w <= 0.0 || h <= 0.0 {
        return quad(rect, color, uv_min, uv_max);
    }
    let all_zero = radii.iter().all(|&(rx, ry)| rx <= 0.0 || ry <= 0.0);
    if all_zero {
        return nine_slice(rect, color, slice, src_w, src_h, uv_min, uv_max);
    }

    // nine_slice 同款切片线（含 clamp 防四角越界）
    let sl_l = slice.left.min(w * 0.5);
    let sl_r = slice.right.min(w * 0.5);
    let sl_t = slice.top.min(h * 0.5);
    let sl_b = slice.bottom.min(h * 0.5);
    let cx_l = rect.x + sl_l;
    let cx_r = (rect.x + w - sl_r).max(cx_l);
    let cy_t = rect.y + sl_t;
    let cy_b = (rect.y + h - sl_b).max(cy_t);

    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);
    let sxf = (umax - umin) / src_w.max(1e-6);
    let syf = (vmax - vmin) / src_h.max(1e-6);
    // UV 切片线（源图像素坐标）
    let tx_l = umin + slice.left * sxf;
    let tx_r = umin + (src_w - slice.right) * sxf;
    let ty_t = vmin + slice.top * syf;
    let ty_b = vmin + (src_h - slice.bottom) * syf;

    // radius 钳制：≤ 角区子矩形尺寸（弧须落角区内）。逐角独立钳。
    let tl_r = (radii[0].0.min(sl_l).max(0.0), radii[0].1.min(sl_t).max(0.0));
    let tr_r = (radii[1].0.min(sl_r).max(0.0), radii[1].1.min(sl_t).max(0.0));
    let br_r = (radii[2].0.min(sl_r).max(0.0), radii[2].1.min(sl_b).max(0.0));
    let bl_r = (radii[3].0.min(sl_l).max(0.0), radii[3].1.min(sl_b).max(0.0));

    let mut verts: Vec<[f32; 2]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // rect 局部 px → UV，按 slice 分段（左角区 1:1 从左、中拉伸、右角区 1:1 从右钉 umax）。
    // 修复：旧全局线性 umin+(px-rect.x)*sxf 在 rect.w>src_w 时右角区 UV 超 umax（采相邻图失真）。
    let mid_span_x = (w - sl_l - sl_r).max(1e-6);
    let mid_span_y = (h - sl_t - sl_b).max(1e-6);
    let u_of = |px: f32| -> f32 {
        let lx = px - rect.x;
        if lx <= sl_l { umin + lx * sxf }
        else if lx >= w - sl_r { umax - (w - lx) * sxf }
        else { tx_l + (lx - sl_l) / mid_span_x * (tx_r - tx_l) }
    };
    let v_of = |py: f32| -> f32 {
        let ly = py - rect.y;
        if ly <= sl_t { vmin + ly * syf }
        else if ly >= h - sl_b { vmax - (h - ly) * syf }
        else { ty_t + (ly - sl_t) / mid_span_y * (ty_b - ty_t) }
    };
    // push 一个 rect-space (px,py) 顶点，UV 按 slice 分段映射（角区 1:1，中间拉伸）。
    let push_rect_uv = |vs: &mut Vec<[f32; 2]>,
                        us: &mut Vec<[f32; 2]>,
                        cs: &mut Vec<[f32; 4]>,
                        px: f32, py: f32| {
        vs.push([px, py]);
        us.push([u_of(px), v_of(py)]);
        cs.push(color);
    };
    // push 一个 quad（4 顶点 + 2 三角形）。UV 由调用方算好传。
    let push_quad_uv = |vs: &mut Vec<[f32; 2]>,
                        us: &mut Vec<[f32; 2]>,
                        cs: &mut Vec<[f32; 4]>,
                        ix: &mut Vec<u32>,
                        x0: f32, x1: f32, y0: f32, y1: f32,
                        u0: f32, u1: f32, v0: f32, v1: f32| {
        let base = vs.len() as u32;
        vs.push([x0, y0]); us.push([u0, v0]); cs.push(color);
        vs.push([x1, y0]); us.push([u1, v0]); cs.push(color);
        vs.push([x1, y1]); us.push([u1, v1]); cs.push(color);
        vs.push([x0, y1]); us.push([u0, v1]); cs.push(color);
        ix.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    let corner_quad = |vs: &mut Vec<[f32; 2]>,
                       us: &mut Vec<[f32; 2]>,
                       cs: &mut Vec<[f32; 4]>,
                       ix: &mut Vec<u32>,
                       x0: f32, x1: f32, y0: f32, y1: f32| {
        push_quad_uv(vs, us, cs, ix, x0, x1, y0, y1, u_of(x0), u_of(x1), v_of(y0), v_of(y1));
    };

    // ---- 四角：四分之一圆弧扇形（纯几何圆角）----
    // 每角三角扇 = center（弧圆心）+ 弧顶点列（start..start+π/2）。**无外角顶点、无边三角**。
    // 弧外 `[0,r]² \ disc` 不发顶点（圆角镂空）。UV 1:1 映射源角像素。
    // corners: (rx, ry, center, start_angle)
    let arc_corners: [(f32, f32, [f32; 2], f32); 4] = [
        // TL: 圆心 (x + rx, y + ry)，start = π
        (tl_r.0, tl_r.1, [rect.x + tl_r.0, rect.y + tl_r.1], std::f32::consts::PI),
        // TR: 圆心 (x+w - rx, y + ry)，start = -π/2
        (tr_r.0, tr_r.1, [rect.x + w - tr_r.0, rect.y + tr_r.1], -std::f32::consts::FRAC_PI_2),
        // BR: 圆心 (x+w - rx, y+h - ry)，start = 0
        (br_r.0, br_r.1, [rect.x + w - br_r.0, rect.y + h - br_r.1], 0.0),
        // BL: 圆心 (x + rx, y+h - ry)，start = π/2
        (bl_r.0, bl_r.1, [rect.x + bl_r.0, rect.y + h - bl_r.1], std::f32::consts::FRAC_PI_2),
    ];
    for (rx, ry, center, start) in arc_corners {
        if rx <= 0.0 || ry <= 0.0 {
            // 该角无圆角：角区子矩形整体当角区 L 形 quad 处理（下方网格覆盖），这里跳过扇形。
            continue;
        }
        // 三角扇：center + 弧顶点列 (0..=sides)。
        let fan_base = verts.len() as u32;
        push_rect_uv(&mut verts, &mut uvs, &mut colors, center[0], center[1]);
        let sides = ((std::f32::consts::PI * rx.max(ry) / 4.0).ceil() as i32 + 1).max(2);
        let delta = std::f32::consts::FRAC_PI_2 / sides as f32;
        for j in 0..=sides {
            let a = if j == sides {
                start + std::f32::consts::FRAC_PI_2
            } else {
                start + delta * j as f32
            };
            let px = center[0] + a.cos() * rx;
            let py = center[1] + a.sin() * ry;
            push_rect_uv(&mut verts, &mut uvs, &mut colors, px, py);
        }
        // 扇形索引：(center, arc_j, arc_{j+1}) for j in 0..sides
        for j in 0..sides as u32 {
            indices.extend_from_slice(&[fan_base, fan_base + 1 + j, fan_base + 1 + j + 1]);
        }
    }

    // ---- 角区 L 形 quad（填角区 [0,slice]² 减 [0,r]² 镂空）----
    // 逐角发角区 L 形 quad：角区 [0,slice]² 内，[0,r]² 子块由弧扇覆盖弧内、弧外镂空；
    // L 形剩余（[r,slice]×[0,slice] ∪ [0,r]×[r,slice]）发 quad，UV 1:1 映射源角像素（不拉伸）。
    // r=0 时无镂空，整个角区 [0,slice]² 当 L 形（2 quad 合并覆盖）。每角用各自 r 精确处理。
    // 4 角各自的角区子矩形：
    let tl_x0 = rect.x;          let tl_y0 = rect.y;          let tl_x1 = cx_l;     let tl_y1 = cy_t;
    let tr_x0 = cx_r;            let tr_y0 = rect.y;          let tr_x1 = rect.x+w; let tr_y1 = cy_t;
    let br_x0 = cx_r;            let br_y0 = cy_b;            let br_x1 = rect.x+w; let br_y1 = rect.y+h;
    let bl_x0 = rect.x;          let bl_y0 = cy_b;            let bl_x1 = cx_l;     let bl_y1 = rect.y+h;

    // 单角 L 形分解：角区 [ax0,ax1]×[ay0,ay1]，圆角 r=(rx,ry) 在角顶（角顶方向由 corner 决定）。
    // L 形 = 3 个 quad：
    //   q1 = [ax0, ax1]×[ay0+ry, ay1]（远端条，跨角区宽）—— 但角顶方向不同，远端定义不同。
    // 简化：L 形 = 角区减 [0,r]²（角顶 [0,r]² 镂空）。按角顶方向分 4 种。
    // 统一用"角顶内缩 r"模型：角顶 = (cx, cy)，圆心 = (cx±rx, cy±ry)。
    // 镂空方块 = 角顶起的 [0,r]×[0,r]（方向朝角顶）。
    // L 形 3 quad（以 TL 为例，角顶 (ax0,ay0)，镂空 [ax0,ax0+rx]×[ay0,ay0+ry]）：
    //   q_right = [ax0+rx, ax1]×[ay0, ay1]        （角区右部，含 [ay0,ay0+ry] 和 [ay0+ry,ay1]）
    //   q_top   = [ax0, ax0+rx]×[ay0+ry, ay1]     （角区左部上段，镂空上方）
    //   （[ax0,ax0+rx]×[ay0,ay0+ry] = 镂空，跳过）
    // 但 q_right 跨整个 [ay0,ay1]——其 [ay0,ay0+ry] 部分是角区右部近角段（[r,slice]×[0,r]），
    // [ay0+ry,ay1] 部分是角区右部远段（[r,slice]×[r,slice]）。两者 UV 都 1:1 源角像素，可合一 quad。
    // 故 TL L 形 = 2 quad：
    //   q_right = [ax0+rx, ax1]×[ay0, ay1]
    //   q_top   = [ax0, ax0+rx]×[ay0+ry, ay1]
    // 其他角对称。
    let emit_corner_l = |vs: &mut Vec<[f32; 2]>,
                         us: &mut Vec<[f32; 2]>,
                         cs: &mut Vec<[f32; 4]>,
                         ix: &mut Vec<u32>,
                         ax0: f32, ay0: f32, ax1: f32, ay1: f32,
                         rx: f32, ry: f32,
                         // 角顶方向：true = 朝 (ax0,ay0)（TL/BR 内缩 +rx,+ry / -rx,-ry 取决于角）
                         // 用 corner 标识：0=TL 1=TR 2=BR 3=BL
                         corner: usize| {
        if ax1 - ax0 <= 1e-6 || ay1 - ay0 <= 1e-6 { return; }
        // 镂空方块（角顶 [0,r]²）：按 corner 定方向
        // TL: 镂空 [ax0, ax0+rx]×[ay0, ay0+ry]
        // TR: 镂空 [ax1-rx, ax1]×[ay0, ay0+ry]
        // BR: 镂空 [ax1-rx, ax1]×[ay1-ry, ay1]
        // BL: 镂空 [ax0, ax0+rx]×[ay1-ry, ay1]
        let (hx0, hx1, hy0, hy1) = match corner {
            0 => (ax0, ax0 + rx, ay0, ay0 + ry),
            1 => (ax1 - rx, ax1, ay0, ay0 + ry),
            2 => (ax1 - rx, ax1, ay1 - ry, ay1),
            3 => (ax0, ax0 + rx, ay1 - ry, ay1),
            _ => unreachable!(),
        };
        // L 形 = 角区 - 镂空方块。分解为 2 quad（镂空方块把角区分成 L 形）：
        // 通用分解（按镂空方块在角区的位置）：
        //   q_strip_far = 角区"远端"条（不含镂空的那一整条）
        //   q_strip_near = 角区"近端"条（镂空那一侧的非镂空部分）
        // 以 TL（镂空在左上）为例：
        //   q_right = [hx1, ax1]×[ay0, ay1]   （右条，整高）
        //   q_bottom = [ax0, hx1]×[hy1, ay1]  （左下，镂空下方）
        // 其他角对称。
        match corner {
            0 => { // TL 镂空左上
                if ax1 - hx1 > 1e-6 { corner_quad(vs, us, cs, ix, hx1, ax1, ay0, ay1); }
                if hx1 - ax0 > 1e-6 && ay1 - hy1 > 1e-6 { corner_quad(vs, us, cs, ix, ax0, hx1, hy1, ay1); }
            }
            1 => { // TR 镂空右上
                if hx0 - ax0 > 1e-6 { corner_quad(vs, us, cs, ix, ax0, hx0, ay0, ay1); }
                if ax1 - hx0 > 1e-6 && ay1 - hy1 > 1e-6 { corner_quad(vs, us, cs, ix, hx0, ax1, hy1, ay1); }
            }
            2 => { // BR 镂空右下
                if hx0 - ax0 > 1e-6 { corner_quad(vs, us, cs, ix, ax0, hx0, ay0, ay1); }
                if ax1 - hx0 > 1e-6 && hy0 - ay0 > 1e-6 { corner_quad(vs, us, cs, ix, hx0, ax1, ay0, hy0); }
            }
            3 => { // BL 镂空左下
                if ax1 - hx1 > 1e-6 { corner_quad(vs, us, cs, ix, hx1, ax1, ay0, ay1); }
                if hx1 - ax0 > 1e-6 && hy0 - ay0 > 1e-6 { corner_quad(vs, us, cs, ix, ax0, hx1, ay0, hy0); }
            }
            _ => unreachable!(),
        }
    };

    emit_corner_l(&mut verts, &mut uvs, &mut colors, &mut indices, tl_x0, tl_y0, tl_x1, tl_y1, tl_r.0, tl_r.1, 0);
    emit_corner_l(&mut verts, &mut uvs, &mut colors, &mut indices, tr_x0, tr_y0, tr_x1, tr_y1, tr_r.0, tr_r.1, 1);
    emit_corner_l(&mut verts, &mut uvs, &mut colors, &mut indices, br_x0, br_y0, br_x1, br_y1, br_r.0, br_r.1, 2);
    emit_corner_l(&mut verts, &mut uvs, &mut colors, &mut indices, bl_x0, bl_y0, bl_x1, bl_y1, bl_r.0, bl_r.1, 3);

    // ---- 边带 + 中心 quad ----
    // 上边带：x [cx_l, cx_r], y [rect.y, cy_t] —— 水平拉伸（UV x: tx_l..tx_r, v: ty_t 段 1:1）
    //   但上边带 y 跨 [0, slice_t]，含角弧段 [0,r] 与 L 段 [r,slice]。边带 x 仅 [slice, w-slice]，
    //   不含角区——故 y 全段 [0, slice_t] 都是边带（无角弧，因 x 在中心段）。
    //   UV：x 拉伸 (tx_l..tx_r)，y 1:1 源像素 (vmin..ty_t)。
    if cx_r - cx_l > 1e-6 {
        // 上边带
        if cy_t - rect.y > 1e-6 {
            push_quad_uv(&mut verts, &mut uvs, &mut colors, &mut indices,
                cx_l, cx_r, rect.y, cy_t, tx_l, tx_r, vmin, ty_t);
        }
        // 下边带
        if (rect.y + h) - cy_b > 1e-6 {
            push_quad_uv(&mut verts, &mut uvs, &mut colors, &mut indices,
                cx_l, cx_r, cy_b, rect.y + h, tx_l, tx_r, ty_b, vmax);
        }
        // 中心
        if cy_b - cy_t > 1e-6 {
            push_quad_uv(&mut verts, &mut uvs, &mut colors, &mut indices,
                cx_l, cx_r, cy_t, cy_b, tx_l, tx_r, ty_t, ty_b);
        }
    }
    // 左右边带：y [cy_t, cy_b], x 角区段（含角弧段 [0,r] 与 L 段 [r,slice]）
    //   边带 y 仅 [slice, h-slice]（中心段），不含角区——x 全段 [0, slice] / [w-slice, w] 都是边带。
    //   UV：x 1:1 源像素，y 拉伸 (ty_t..ty_b)。
    if cy_b - cy_t > 1e-6 {
        // 左边带
        if cx_l - rect.x > 1e-6 {
            push_quad_uv(&mut verts, &mut uvs, &mut colors, &mut indices,
                rect.x, cx_l, cy_t, cy_b, umin, tx_l, ty_t, ty_b);
        }
        // 右边带
        if (rect.x + w) - cx_r > 1e-6 {
            push_quad_uv(&mut verts, &mut uvs, &mut colors, &mut indices,
                cx_r, rect.x + w, cy_t, cy_b, tx_r, umax, ty_t, ty_b);
        }
    }

    (verts, uvs, colors, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::resolved::SliceInsets;

    #[test]
    fn quad_four_verts_two_tris() {
        let (v, _uvs, _col, i) = quad(&Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        }, [1.0; 4], [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(v.len(), 4);
        assert_eq!(i.len(), 6);
    }

    #[test]
    fn quad_verts_at_corners() {
        // 左上 → 右上 → 右下 → 左下，CCW。
        let (v, _, _, _) = quad(
            &Rect {
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0,
            },
            [0.0; 4],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert_eq!(v[0], [1.0, 2.0]); // 左上 = (x, y)
        assert_eq!(v[1], [4.0, 2.0]); // 右上 = (x+w, y)
        assert_eq!(v[2], [4.0, 6.0]); // 右下 = (x+w, y+h)
        assert_eq!(v[3], [1.0, 6.0]); // 左下 = (x, y+h)
    }

    #[test]
    fn quad_uv_uses_min_max() {
        // 传 [0,0],[1,1] → 全图占位（TL=0,0 / BR=1,1）。
        let (_, uvs, _, _) = quad(&Rect::default(), [0.0; 4], [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(uvs[0], [0.0, 0.0]);
        assert_eq!(uvs[2], [1.0, 1.0]);
    }

    #[test]
    fn quad_uv_subregion() {
        // atlas sprite 子区：TL→(umin,vmin) TR→(umax,vmin) BR→(umax,vmax) BL→(umin,vmax)。
        let (_, uvs, _, _) = quad(&Rect::default(), [0.0; 4], [0.25, 0.5], [0.75, 1.0]);
        assert_eq!(uvs[0], [0.25, 0.5]); // TL
        assert_eq!(uvs[1], [0.75, 0.5]); // TR
        assert_eq!(uvs[2], [0.75, 1.0]); // BR
        assert_eq!(uvs[3], [0.25, 1.0]); // BL
    }

    #[test]
    fn quad_colors_uniform() {
        let (_, _, colors, _) = quad(&Rect::default(), [0.5; 4], [0.0, 0.0], [1.0, 1.0]);
        for c in &colors {
            assert_eq!(*c, [0.5; 4]);
        }
    }

    #[test]
    fn rounded_rect_zero_radius_acts_as_rect() {
        // 全 0 直角：仍是三角扇，但所有弧顶点退化为角顶点 → 中心+4 角 = 5 顶点
        let (v, _uvs, _col, idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(0.0, 0.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 5, "全 0：中心 + 4 角顶点");
        assert_eq!(idx.len(), 12, "4 三角形 × 3 索引");
    }

    #[test]
    fn rounded_rect_vertex_count_scales_with_radius() {
        // r=8, 80×80：sides = ceil(π·8/4)+1 = 8 → 每角 9 顶点(0..=8) × 4 角 + 1 中心 = 37
        let (v, _uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(8.0, 8.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 37, "1 中心 + 4 角 × (8+1) 顶点");
    }

    #[test]
    fn rounded_rect_clamps_oversized_radius() {
        // 改进 1：r=40, 60×40 rect——四边约束取最紧：h/(tl.1+bl.1)=40/80=0.5 → scale=0.5 → r=20
        // TL 圆心 = (20, 20)，最左弧点 x = 20-20 = 0（贴 rect 左边）
        let (v, _uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 60.0, h: 40.0 },
            [1.0; 4],
            &[(40.0, 40.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        let xs: Vec<f32> = v.iter().map(|p| p[0]).collect();
        let min_x = xs.iter().cloned().fold(f32::INFINITY, f32::min);
        assert!((min_x - 0.0).abs() < 1e-3, "钳制后最左顶点贴 rect.x=0，得 {}", min_x);
        let max_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!((max_x - 60.0).abs() < 1e-3, "最右顶点贴 rect.x+w=60，得 {}", max_x);
    }

    #[test]
    fn rounded_rect_ellipse_radii() {
        // 椭圆角 (20,10)：TL 圆心 (20,10)，弧顶点 = center + (cos·20, sin·10)
        // 起始角 TL=π：cos(π)=-1, sin(π)=0 → 首弧点 = (20-20, 10+0) = (0, 10)
        let (v, _uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(20.0, 10.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        // v[1] = 第一个弧顶点 = TL 角起始 (0, 10)
        assert!((v[1][0] - 0.0).abs() < 1e-3, "TL 首弧点 x=0，得 {}", v[1][0]);
        assert!((v[1][1] - 10.0).abs() < 1e-3, "TL 首弧点 y=10，得 {}", v[1][1]);
    }

    #[test]
    fn rounded_rect_uv_linear_mapping() {
        // rect=(0,0,80,80), uv=[0,0]-[1,1] → 中心顶点 UV=(0.5,0.5)
        let (_v, uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(8.0, 8.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert!((uvs[0][0] - 0.5).abs() < 1e-3, "中心 UV.x=0.5");
        assert!((uvs[0][1] - 0.5).abs() < 1e-3, "中心 UV.y=0.5");
    }

    #[test]
    fn rounded_rect_degenerate_rect_falls_back_to_quad() {
        // w=0 退化 → 走 quad（4 顶点）
        let (v, _uvs, _col, idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 0.0, h: 80.0 },
            [1.0; 4],
            &[(8.0, 8.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 4, "退化走 quad");
        assert_eq!(idx.len(), 6);
    }

    #[test]
    fn rounded_rect_zero_h_radius_corner_at_rect_vertex() {
        // 混合椭圆角：TL/BR 水平半径 0（rx=0, ry=8）→ 直角，TR/BL 真弧（8,8）。
        // 直角分支须落在矩形顶点（TL=[0,0] / BR=[80,80]），
        // 而非圆心+方向算出的 [0,8]/[80,72]（ry>0 让 py 偏移，原 bug）。
        let (v, _, _, _) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(0.0, 8.0), (8.0, 8.0), (0.0, 8.0), (8.0, 8.0)],
            [0.0, 0.0], [1.0, 1.0],
        );
        let has = |x: f32, y: f32| v.iter().any(|p| (p[0] - x).abs() < 1e-4 && (p[1] - y).abs() < 1e-4);
        assert!(has(0.0, 0.0), "TL 直角顶点须落矩形角 [0,0]，verts={:?}", v);
        assert!(has(80.0, 80.0), "BR 直角顶点须落矩形角 [80,80]，verts={:?}", v);
    }

    #[test]
    fn nine_slice_16_verts_9_quads() {
        // 100×100 rect，slice 10 各边，源图 100×100 全图 uv 0..1
        let (v, _uvs, _col, idx) = nine_slice(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 },
            100.0, 100.0,  // 源图尺寸
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 16, "4×4 顶点");
        assert_eq!(idx.len(), 9 * 6, "9 quad × 6 索引 = 54");
    }

    #[test]
    fn nine_slice_corner_verts_at_slice_lines() {
        // rect 100×100，slice 10：gridX = [0, 10, 90, 100]，gridY = [0, 10, 90, 100]
        let (v, _, _, _) = nine_slice(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 },
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // 顶点行主序 4×4：v[0]=[0,0] v[3]=[100,0] v[12]=[0,100] v[15]=[100,100]
        // v[1]=[10,0]（左切片线） v[2]=[90,0]（右切片线）
        assert_eq!(v[0], [0.0, 0.0]);
        assert_eq!(v[1], [10.0, 0.0]);
        assert_eq!(v[2], [90.0, 0.0]);
        assert_eq!(v[3], [100.0, 0.0]);
        // v[4]=[0,10]（顶切片线）
        assert_eq!(v[4], [0.0, 10.0]);
    }

    #[test]
    fn nine_slice_clamps_when_rect_smaller_than_source() {
        // rect 10×10（比源图 100×100 - 切片 20 = 80 小）→ 四角重叠 clamp
        // fgui：contentRect.width < sourceW - gridRect.width 时 max(0,...) clamp
        let (v, _, _, _) = nine_slice(
            &Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            [1.0; 4],
            &SliceInsets { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 },
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // clamp 后 grid_x = [0, 0+10.min(5)=5, (0+10-10).max(5)=5, 10] → 左切片线==右切片线==5，中心段折叠
        // 关键：左切片线不超 rect 右边（gridX[1] <= gridX[2]）
        let xs: Vec<f32> = (0..4).map(|c| v[c][0]).collect();
        assert!(xs[1] <= xs[2] + 1e-3, "左切片线 <= 右切片线（clamp 防越界），xs={:?}", xs);
    }

    #[test]
    fn nine_slice_uv_proportional_to_slice() {
        // 源图 100×100，slice 10 → UV 切片线 = 0.1 / 0.9
        let (_, uvs, _, _) = nine_slice(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 },
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // uvs[1].x = 0.1（左切片线 UV）
        assert!((uvs[1][0] - 0.1).abs() < 1e-4, "左切片 UV=0.1");
        assert!((uvs[2][0] - 0.9).abs() < 1e-4, "右切片 UV=0.9");
    }

    #[test]
    fn nine_slice_rounded_produces_mesh() {
        // 100×100 rect，slice 20，radius 10，源图 100×100
        let (v, _uvs, _col, idx) = nine_slice_rounded(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 },
            &[(10.0, 10.0); 4],
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // 顶点数 > 16（四角圆角扇形比方角多顶点）
        assert!(v.len() > 16, "圆角四角扇形顶点数 > 16 方角，得 {}", v.len());
        // 索引数 > 54（9 quad 基础 + 四角扇形三角扇）
        assert!(idx.len() > 54, "圆角共存索引数 > 54，得 {}", idx.len());
    }

    #[test]
    fn nine_slice_rounded_corner_uv_in_source_corner_region() {
        // 不变量：四角顶点 UV 落在源图角区（左上角区 = uv [0..0.2, 0..0.2]）
        // 修复 Important 2 后：角区无外角顶点（(0,0) UV 顶点不再存在），弧扇顶点 UV
        // 落在源角区内（最接近角顶的弧点 = (r,0)/(0,r) → UV (0.1,0)/(0,0.1)）。
        let (_v, uvs, _col, _idx) = nine_slice_rounded(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 },
            &[(10.0, 10.0); 4],
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // 左上角区顶点 UV 应在 [0..0.2, 0..0.2]（slice 20 / src 100 = 0.2）
        let tl_uvs: Vec<[f32;2]> = uvs.iter().cloned().filter(|uv| uv[0] <= 0.21 && uv[1] <= 0.21).collect();
        assert!(!tl_uvs.is_empty(), "左上角区有顶点");
        // 弧扇顶点 UV 落在角区内（含接近角顶的弧点 (0.1,0)/(0,0.1)）。
        // 不应有 (0,0) UV 外角顶点（Important 2 修复后该顶点已删）。
        let has_origin = tl_uvs.iter().any(|uv| uv[0].abs() < 1e-3 && uv[1].abs() < 1e-3);
        assert!(!has_origin, "Important 2 修复：左上角不应有 (0,0) UV 外角顶点");
        // 弧扇中心 (r,r)=(10,10) → UV (0.1, 0.1)，应在角区内
        let has_arc_center = tl_uvs.iter().any(|uv| (uv[0] - 0.1).abs() < 1e-3 && (uv[1] - 0.1).abs() < 1e-3);
        assert!(has_arc_center, "左上角弧扇中心 UV=(0.1,0.1) 存在");
    }

    #[test]
    fn nine_slice_rounded_arc_cutout_geometric() {
        // 修复 Critical 1 + Important 2 的覆盖测试：point-in-triangle 扫描网格采样点。
        // rect 100×100，slice 20，radius 10，src 100×100。
        // 期望：圆弧外角点 (2,2)（在 [0,r]² 内但弧外，距圆心 (10,10)=√128≈11.3>10）**不被覆盖**；
        //      中心 (50,50) / 上边中点 (50,5) / 角区 L 形点 (15,5)（[r,slice]×[0,r]）被覆盖。
        let (verts, _uvs, _col, idx) = nine_slice_rounded(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 },
            &[(10.0, 10.0); 4],
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // 收集所有三角形（每 3 索引一个三角形）
        let tris: Vec<([f32; 2], [f32; 2], [f32; 2])> = idx
            .chunks(3)
            .filter_map(|c| {
                if c.len() == 3 {
                    Some((verts[c[0] as usize], verts[c[1] as usize], verts[c[2] as usize]))
                } else {
                    None
                }
            })
            .collect();
        assert!(!tris.is_empty(), "mesh 有三角形");

        let point_in_tri = |p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]| -> bool {
            // 重心坐标法（含边界，容差 1e-4）
            let d1 = (p[0] - b[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (p[1] - b[1]);
            let d2 = (p[0] - c[0]) * (b[1] - c[1]) - (b[0] - c[0]) * (p[1] - c[1]);
            let d3 = (p[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (p[1] - a[1]);
            let neg = (d1 < -1e-4) || (d2 < -1e-4) || (d3 < -1e-4);
            let pos = (d1 > 1e-4) || (d2 > 1e-4) || (d3 > 1e-4);
            !(neg && pos)
        };
        let covered = |px: f32, py: f32| -> bool {
            tris.iter().any(|&(a, b, c)| point_in_tri([px, py], a, b, c))
        };

        // 中心点：必覆盖
        assert!(covered(50.0, 50.0), "中心 (50,50) 被覆盖");
        // 上边中点 (50,5)：上边带（x 在 [slice,w-slice]，y 在 [0,slice]）必覆盖
        assert!(covered(50.0, 5.0), "上边中点 (50,5) 被覆盖");
        // 角区 L 形点 (15,5)：在 [r,slice]×[0,r]（TL 角区 L 形右条），必覆盖（修复 Critical 1 间隙）
        assert!(covered(15.0, 5.0), "角区 L 形点 (15,5) 被覆盖（修复 Critical 1 间隙）");
        // 圆弧外角点 (2,2)：在 [0,r]² 内但弧外（距圆心 (10,10)=√128>10），**不应覆盖**（修复 Important 2 几何圆角）
        assert!(!covered(2.0, 2.0), "圆弧外角点 (2,2) 不被覆盖（几何圆角镂空，修复 Important 2）");
        // 圆弧内点 (5,5)：距圆心 (10,10)=√50≈7.07<10，应覆盖（弧扇内）
        assert!(covered(5.0, 5.0), "圆弧内点 (5,5) 被覆盖（弧扇内）");
    }

    #[test]
    fn nine_slice_rounded_zero_radius_falls_back_to_nine_slice() {
        let (v, _, _, idx) = nine_slice_rounded(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 },
            &[(0.0, 0.0); 4],
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 16, "radius 0 退化为 nine_slice 16 顶点");
        assert_eq!(idx.len(), 54, "9 quad 索引");
    }
}
