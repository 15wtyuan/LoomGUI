//! 一次性 PNG 生成器（spec §5/T7 sample 资产）：写 3 张纯色 PNG 进 samples/atlas/。
//! 再跑：`cargo run -p loomgui_pkg --example gen_atlas_samples`。
//!
//! 产物：
//! - red.png   64×64  #ff0000
//! - green.png 64×64  #00ff00
//! - blue.png  48×48  #0000ff
//!
//! 尺寸故意混合（64×64 ×2 + 48×48 ×1）—— shelf 打包会产生 NPOT atlas + 两行（64+48 高），
//! 给 PlayMode/FrameDebugger 验 UV 方向（R1）留可辨的横向布局。

use image::{ImageBuffer, Rgba};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 写到 crate-root/samples/atlas/（CARGO_MANIFEST_DIR = loomgui_pkg/）。
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples").join("atlas");
    std::fs::create_dir_all(&dir)?;

    write_solid(&dir.join("red.png"), 64, 64, [255, 0, 0, 255])?;
    write_solid(&dir.join("green.png"), 64, 64, [0, 255, 0, 255])?;
    write_solid(&dir.join("blue.png"), 48, 48, [0, 0, 255, 255])?;

    println!("wrote 3 PNGs into {}", dir.display());
    Ok(())
}

fn write_solid(
    path: &std::path::Path,
    w: u32,
    h: u32,
    rgba: [u8; 4],
) -> Result<(), Box<dyn std::error::Error>> {
    // ImageBuffer::from_pixel 建 w×h 全 pixel=rgba 的 RgbaImage；write_to 编码 PNG。
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(w, h, Rgba(rgba));
    img.save(path)?;
    Ok(())
}
