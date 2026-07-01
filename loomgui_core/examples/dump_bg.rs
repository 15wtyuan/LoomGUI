//! 诊断 §3.6 bg-image / §3.8 filter / §3.9 nineslice：
//! dump rect / program / vcol / uv 区间 / color_matrix / verts，定位偏移、滤镜色差、slice 失真。
//! inline 路径（直读 showcase HTML+CSS，绕过 pkg 缓存）。
use loomgui_core::render::node::NodePayload;
use loomgui_core::stage::Stage;

fn main() {
    let font = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/DejaVuSans.ttf");
    let pkg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../loomgui_unity/Assets/StreamingAssets/loom_showcase.pkg.bin");
    let pkg = std::fs::read(pkg_path).expect("read pkg");
    let mut s = Stage::new(font, (1080.0, 1920.0)).expect("Stage::new");
    s.load_package(&pkg).expect("load_package");
    let frame = s.tick_and_render();
    let scene = s.scene.as_ref().unwrap();

    let want = ["bg-demo", "cf-demo", "ns-demo", "br-demo"];
    for rn in &frame.nodes {
        let nid = rn.node_id as usize;
        if nid >= scene.nodes.len() { continue; }
        let n = &scene.nodes[nid];
        let is_img = matches!(n.kind, loomgui_core::scene::node::NodeKind::Image { .. });
        if !is_img && !want.iter().any(|c| n.classes.iter().any(|cl| cl == c)) { continue; }
        let r = n.layout_rect;
        let bg_img = n.style.background_image.as_deref().unwrap_or("-");
        let bg_size = match n.style.background_size {
            loomgui_core::style::resolved::BackgroundSize::Stretch => "Stretch",
            loomgui_core::style::resolved::BackgroundSize::Cover => "Cover",
            loomgui_core::style::resolved::BackgroundSize::Contain => "Contain",
        };
        let has_filter = n.style.color_filter.is_some();
        let has_slice = n.style.border_image_slice.is_some();
        println!(
            "\n=== n{} [{}] rect=({:.0},{:.0},{:.0},{:.0}) bg_img={} bg_size={} filter={} slice={}",
            nid, n.classes.join(","), r.x, r.y, r.w, r.h, bg_img, bg_size, has_filter, has_slice
        );
        match &rn.payload {
            NodePayload::Mesh { verts, uvs, colors, texture, program, color_matrix, .. } => {
                let c0 = colors.first().copied().unwrap_or([0.0; 4]);
                let v0 = verts.first().copied().unwrap_or([0.0; 2]);
                let (umin, umax, vmin, vmax) = uvs.iter().fold(
                    (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
                    |(a, b, c, d), uv| (a.min(uv[0]), b.max(uv[0]), c.min(uv[1]), d.max(uv[1])),
                );
                let cu = (umin + umax) / 2.0;
                let cv = (vmin + vmax) / 2.0;
                println!(
                    "  program={} tex={} vcol=({:.0},{:.0},{:.0},{:.0}) uv u[{:.3},{:.3}] v[{:.3},{:.3}] center=({:.3},{:.3}) {} verts={} v0=({:.0},{:.0}) uv0=({:.3},{:.3})",
                    program, texture,
                    c0[0] * 255., c0[1] * 255., c0[2] * 255., c0[3],
                    umin, umax, vmin, vmax, cu, cv,
                    if (cv - 0.5).abs() > 0.02 { "<<v非居中>>" } else { "v居中" },
                    verts.len(), v0[0], v0[1], uvs[0][0], uvs[0][1],
                );
                if *program == 3 || *program == 4 {
                    println!("  color_matrix: {:?}", color_matrix);
                }
                if verts.len() > 4 {
                    print!("  verts:");
                    for v in verts.iter().take(24) { print!(" ({:.0},{:.0})", v[0], v[1]); }
                    println!();
                }
            }
            _ => println!("  (non-mesh)"),
        }
    }
}
