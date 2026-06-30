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
/// - 有 radius → 四角圆角扇形（不拉伸，UV 取源图角区）+ 四边/中心切片拉伸。
/// - 不变量：四角不拉伸，四边单轴拉伸，中心双轴拉伸。
///
/// 顶点布局（实现期定）：
/// - 四角各生成一个三角扇（中心=圆心 + 外角顶点 + 圆弧顶点），扇形贴在角区 slice×slice
///   子矩形内，圆心 = 角区子矩形内缩 (rx,ry) 点，半径 = slice 钳制后的 radius。外角顶点连
///   两直边（覆盖角像素含源角透明角），弧顶点连成圆角；UV 按角子矩形内位置比例映射源角区像素。
/// - 四边（上中/下中/左中/右中）+ 中心 = 5 个 quad，位置取 nine_slice 的 grid_x/grid_y
///   中段，UV 取 nine_slice 的 tex_x/tex_y 中段（单轴/双轴拉伸语义）。
/// - 索引：四角各为独立三角扇 + 5 quad 各 2 三角形，全部按运行偏移拼接到一个索引表。
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

    // grid_x/grid_y + tex_x/tex_y（同 nine_slice：rect 坐标 4 值 + UV 4 值，clamp 防四角越界）
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
    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);
    let sx = (umax - umin) / src_w.max(1e-6);
    let sy = (vmax - vmin) / src_h.max(1e-6);
    let tex_x = [umin, umin + slice.left * sx, umin + (src_w - slice.right) * sx, umax];
    let tex_y = [vmin, vmin + slice.top * sy, vmin + (src_h - slice.bottom) * sy, vmax];

    // 角区子矩形尺寸（slice clamped by rect half）。角区 UV 仍按源 slice 像素映射（tex_x/tex_y），
    // 因角区不拉伸不变量要求 UV 取源图角区像素。
    let sl_l = (grid_x[1] - grid_x[0]).max(0.0);
    let sl_r = (grid_x[3] - grid_x[2]).max(0.0);
    let sl_t = (grid_y[1] - grid_y[0]).max(0.0);
    let sl_b = (grid_y[3] - grid_y[2]).max(0.0);

    // radius 钳制：按角区子矩形尺寸钳（radius ≤ slice，角弧须落角子矩形内）。
    // 全角 0 已在入口 fallback；此处逐角独立钳（角区子矩形尺寸各异）。
    let clamp_r = |r: (f32, f32), sz_x: f32, sz_y: f32| -> (f32, f32) {
        (r.0.min(sz_x).max(0.0), r.1.min(sz_y).max(0.0))
    };
    let tl_r = clamp_r(radii[0], sl_l, sl_t);
    let tr_r = clamp_r(radii[1], sl_r, sl_t);
    let br_r = clamp_r(radii[2], sl_r, sl_b);
    let bl_r = clamp_r(radii[3], sl_l, sl_b);

    let mut verts: Vec<[f32; 2]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // 局部 helper：push 一个顶点（pos + UV 映射）。UV 把角子矩形内位置 (px,py) 按比例
    // 映射到源图角区像素 [u0..u1, v0..v1]（角区不拉伸）。
    let push_v = |vs: &mut Vec<[f32; 2]>,
                      us: &mut Vec<[f32; 2]>,
                      cs: &mut Vec<[f32; 4]>,
                      px: f32, py: f32,
                      sub_x0: f32, sub_y0: f32, sub_w: f32, sub_h: f32,
                      u0: f32, u1: f32, v0: f32, v1: f32| {
        vs.push([px, py]);
        us.push([
            u0 + ((px - sub_x0) / sub_w.max(1e-6)) * (u1 - u0),
            v0 + ((py - sub_y0) / sub_h.max(1e-6)) * (v1 - v0),
        ]);
        cs.push(color);
    };

    // ---- 四角 ----
    // 每角：角区子矩形 [sub_x0,sub_y0]-[sub_x0+sub_w, sub_y0+sub_h]；
    //   矩形外角顶点（rect 角）= sub 矩形的某角；圆心 = 外角内缩 (rx,ry)；弧起始角同 rounded_rect。
    // 圆角（rx>0 && ry>0）：三角扇（中心=圆心 + 外角顶点 + 弧顶点）——外角顶点连两直边，
    //   弧顶点连成圆角；UV 把角子矩形内位置按比例映射源角区像素（含外角→(u0,v0) 角像素）。
    // 直角：4 顶点 quad 覆盖角子矩形。
    // corners: (rx, ry, center, start_angle, sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1, outer_x, outer_y)
    let corners: [(f32, f32, [f32; 2], f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32); 4] = [
        // TL: 子矩形 [grid_x0, grid_y0]-[grid_x1, grid_y1]，外角 (x0,y0)，圆心 (x0+rx, y0+ry)，start=π
        (tl_r.0, tl_r.1, [grid_x[0] + tl_r.0, grid_y[0] + tl_r.1], std::f32::consts::PI,
         grid_x[0], grid_y[0], sl_l, sl_t, tex_x[0], tex_x[1], tex_y[0], tex_y[1], grid_x[0], grid_y[0]),
        // TR: 子矩形 [grid_x2, grid_y0]-[grid_x3, grid_y1]，外角 (x3,y0)，圆心 (x3-rx, y0+ry)，start=-π/2
        (tr_r.0, tr_r.1, [grid_x[3] - tr_r.0, grid_y[0] + tr_r.1], -std::f32::consts::FRAC_PI_2,
         grid_x[2], grid_y[0], sl_r, sl_t, tex_x[2], tex_x[3], tex_y[0], tex_y[1], grid_x[3], grid_y[0]),
        // BR: 子矩形 [grid_x2, grid_y2]-[grid_x3, grid_y3]，外角 (x3,y3)，圆心 (x3-rx, y3-ry)，start=0
        (br_r.0, br_r.1, [grid_x[3] - br_r.0, grid_y[3] - br_r.1], 0.0,
         grid_x[2], grid_y[2], sl_r, sl_b, tex_x[2], tex_x[3], tex_y[2], tex_y[3], grid_x[3], grid_y[3]),
        // BL: 子矩形 [grid_x0, grid_y2]-[grid_x1, grid_y3]，外角 (x0,y3)，圆心 (x0+rx, y3-ry)，start=π/2
        (bl_r.0, bl_r.1, [grid_x[0] + bl_r.0, grid_y[3] - bl_r.1], std::f32::consts::FRAC_PI_2,
         grid_x[0], grid_y[2], sl_l, sl_b, tex_x[0], tex_x[1], tex_y[2], tex_y[3], grid_x[0], grid_y[3]),
    ];

    for (rx, ry, center, start, sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1, outer_x, outer_y) in corners {
        if rx <= 0.0 || ry <= 0.0 {
            // 直角：角子矩形为方角，用 4 顶点 quad 覆盖（TL→TR→BR→BL CCW）。
            // 子矩形 4 角 UV 按位置映射源角区（(0,0)→(u0,v0), (1,1)→(u1,v1)）。
            let base = verts.len() as u32;
            push_v(&mut verts, &mut uvs, &mut colors, sub_x0,            sub_y0,            sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            push_v(&mut verts, &mut uvs, &mut colors, sub_x0 + sub_w,    sub_y0,            sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            push_v(&mut verts, &mut uvs, &mut colors, sub_x0 + sub_w,    sub_y0 + sub_h,    sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            push_v(&mut verts, &mut uvs, &mut colors, sub_x0,            sub_y0 + sub_h,    sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        } else {
            // 圆角三角扇：顶点序 = [center, outer, arc_start, arc_1, ..., arc_end]。
            // 索引：(center, outer, arc_start) 边三角 + (center, arc_j, arc_{j+1}) 弧扇形
            //       + (center, arc_end, outer) 边三角。外角覆盖角像素（含源角透明角，UV=(u0,v0)）。
            let fan_base = verts.len() as u32;
            // center（扇枢轴）
            push_v(&mut verts, &mut uvs, &mut colors, center[0], center[1],
                    sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            // outer（外角顶点，连两直边）
            push_v(&mut verts, &mut uvs, &mut colors, outer_x, outer_y,
                    sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            // 弧顶点（同 rounded_rect）：sides = ceil(π·max(rx,ry)/4)+1, min 2
            let sides = ((std::f32::consts::PI * rx.max(ry) / 4.0).ceil() as i32 + 1).max(2);
            let delta = std::f32::consts::FRAC_PI_2 / sides as f32;
            for j in 0..=sides {
                let a = if j == sides {
                    start + std::f32::consts::FRAC_PI_2 // 末段精度锁（照 rounded_rect）
                } else {
                    start + delta * j as f32
                };
                let px = center[0] + a.cos() * rx;
                let py = center[1] + a.sin() * ry;
                push_v(&mut verts, &mut uvs, &mut colors, px, py,
                        sub_x0, sub_y0, sub_w, sub_h, u0, u1, v0, v1);
            }
            // 索引
            let outer = fan_base + 1;          // 外角顶点索引
            let arc_start = fan_base + 2;      // 首弧顶点
            let arc_count = (sides + 1) as u32; // 弧顶点数（0..=sides）
            let arc_end = arc_start + arc_count - 1;
            // 边三角：center → outer → arc_start（连外角到弧首）
            indices.extend_from_slice(&[fan_base, outer, arc_start]);
            // 弧扇形：(center, arc_j, arc_{j+1}) for j in 0..arc_count-1
            for j in 0..arc_count - 1 {
                indices.extend_from_slice(&[fan_base, arc_start + j, arc_start + j + 1]);
            }
            // 边三角：center → arc_end → outer（连弧尾到外角，闭合两直边）
            indices.extend_from_slice(&[fan_base, arc_end, outer]);
        }
    }

    // ---- 五条带 quad（上中/下中/左中/右中/中心）----
    // 位置取 grid_x/grid_y 中段，UV 取 tex_x/tex_y 中段（nine_slice 拉伸语义）。
    // 每 quad 4 顶点（TL→TR→BR→BL CCW）+ 2 三角形。
    let push_quad = |vs: &mut Vec<[f32; 2]>,
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
    // 上中：x grid_x[1..2], y grid_y[0..1]（水平拉伸）
    push_quad(&mut verts, &mut uvs, &mut colors, &mut indices,
              grid_x[1], grid_x[2], grid_y[0], grid_y[1],
              tex_x[1], tex_x[2], tex_y[0], tex_y[1]);
    // 下中：x grid_x[1..2], y grid_y[2..3]（水平拉伸）
    push_quad(&mut verts, &mut uvs, &mut colors, &mut indices,
              grid_x[1], grid_x[2], grid_y[2], grid_y[3],
              tex_x[1], tex_x[2], tex_y[2], tex_y[3]);
    // 左中：x grid_x[0..1], y grid_y[1..2]（垂直拉伸）
    push_quad(&mut verts, &mut uvs, &mut colors, &mut indices,
              grid_x[0], grid_x[1], grid_y[1], grid_y[2],
              tex_x[0], tex_x[1], tex_y[1], tex_y[2]);
    // 右中：x grid_x[2..3], y grid_y[1..2]（垂直拉伸）
    push_quad(&mut verts, &mut uvs, &mut colors, &mut indices,
              grid_x[2], grid_x[3], grid_y[1], grid_y[2],
              tex_x[2], tex_x[3], tex_y[1], tex_y[2]);
    // 中心：x grid_x[1..2], y grid_y[1..2]（双轴拉伸）
    push_quad(&mut verts, &mut uvs, &mut colors, &mut indices,
              grid_x[1], grid_x[2], grid_y[1], grid_y[2],
              tex_x[1], tex_x[2], tex_y[1], tex_y[2]);

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
        let (_v, uvs, _col, _idx) = nine_slice_rounded(
            &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            [1.0; 4],
            &SliceInsets { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 },
            &[(10.0, 10.0); 4],
            100.0, 100.0,
            [0.0, 0.0], [1.0, 1.0],
        );
        // 左上角区顶点 UV 应在 [0..0.2, 0..0.2]（slice 20 / src 100 = 0.2）
        // 取前几个顶点（左上角扇形）验
        let tl_uvs: Vec<[f32;2]> = uvs.iter().cloned().filter(|uv| uv[0] <= 0.21 && uv[1] <= 0.21).collect();
        assert!(!tl_uvs.is_empty(), "左上角区有顶点");
        // 最左上顶点 UV ≈ (0,0)
        let has_origin = tl_uvs.iter().any(|uv| uv[0].abs() < 1e-3 && uv[1].abs() < 1e-3);
        assert!(has_origin, "左上角含 (0,0) UV 顶点");
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
