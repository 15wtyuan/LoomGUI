//! v1e 性能 bench：build_render_nodes + tick emit 耗时（静态/冷/换页帧）。
//! 不依赖 Unity（纯 Rust core）。验收线：冷帧/换页帧 ≤2ms（v1-scope §4）。

use criterion::{criterion_group, criterion_main, Criterion};
use loomgui_core::stage::Stage;

fn font_path() -> (String, usize) {
    let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
    (p.clone(), p.len())
}

/// 生成 500 节点 HTML（嵌套 div，各有 bg color）。
fn html_500() -> String {
    let mut s = String::new();
    for i in 0..500 {
        let r = (i % 256) as f32 / 255.0;
        s.push_str(&format!(
            r#"<div style="width:50px;height:10px;background-color:rgba({r},0,0,1)"></div>"#
        ));
    }
    s
}

/// 生成 500 节点 HTML，bg color 全不同（换页用：改 color 模拟全 dirty）。
fn html_500_colorized(seed: u32) -> String {
    let mut s = String::new();
    for i in 0..500u32 {
        let r = ((i + seed) % 256) as f32 / 255.0;
        s.push_str(&format!(
            r#"<div style="width:50px;height:10px;background-color:rgba({r},0,0,1)"></div>"#
        ));
    }
    s
}

fn bench_static(c: &mut Criterion) {
    let (fp, _fplen) = font_path();
    let mut group = c.benchmark_group("v1e_frame_emit");
    group.bench_function("static_frame", |b| {
        b.iter_batched(
            || {
                // 每次迭代 fresh Stage，先 tick 1 次建基线。
                let mut stage = Stage::new(&fp, (800.0, 600.0)).expect("stage");
                stage.load_inline(&html_500(), "").expect("load");
                stage.advance_time(0.016);
                let _ = stage.tick_and_render(); // 建基线
                stage.advance_time(0.016);
                stage
            },
            |mut stage| {
                // 测第 2 帧（全 Unchanged emit）。
                let _ = stage.tick_and_render();
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_cold(c: &mut Criterion) {
    let (fp, _fplen) = font_path();
    let mut group = c.benchmark_group("v1e_frame_emit");
    group.bench_function("cold_frame", |b| {
        b.iter_batched(
            || {
                let mut stage = Stage::new(&fp, (800.0, 600.0)).expect("stage");
                stage.load_inline(&html_500(), "").expect("load");
                stage.advance_time(0.016);
                stage
            },
            |mut stage| {
                let _ = stage.tick_and_render(); // 首帧全 dirty
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_page_turn(c: &mut Criterion) {
    let (fp, _fplen) = font_path();
    let mut group = c.benchmark_group("v1e_frame_emit");
    group.bench_function("page_turn_frame", |b| {
        b.iter_batched(
            || {
                let mut stage = Stage::new(&fp, (800.0, 600.0)).expect("stage");
                stage.load_inline(&html_500_colorized(0), "").expect("load");
                stage.advance_time(0.016);
                let _ = stage.tick_and_render(); // 建基线
                // 换页：reload 不同 color HTML（全节点 style 变）。
                stage.load_inline(&html_500_colorized(100), "").expect("reload");
                stage.advance_time(0.016);
                stage
            },
            |mut stage| {
                let _ = stage.tick_and_render(); // 全 dirty（reload 后首帧）
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_static, bench_cold, bench_page_turn);
criterion_main!(benches);
