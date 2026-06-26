//! 极简 CLI（不引 clap）：loomgui_pkg <html> <css> [-o out.pkg.bin] [-w designW] [-h designH]。
//! 默认 out = <html 去 .html>.pkg.bin，默认 root_size = 1080×1920。
//! res_dir = html_path.parent()（解析 `<img src>` 的基准目录）。
//! 产物：out.pkg.bin + out.atlas.png（无图时不写 atlas）。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: {} <html> <css> [-o out.pkg.bin] [-w designW] [-h designH] [-a atlas.png]",
            args.first().map(String::as_str).unwrap_or("loomgui_pkg")
        );
        return ExitCode::from(2);
    }
    let html_path = PathBuf::from(&args[1]);
    let css_path = &args[2];
    let html_str = args[1].as_str();
    let mut out_path = html_str
        .rsplit_once('.')
        .map(|(stem, _)| format!("{stem}.pkg.bin"))
        .unwrap_or_else(|| format!("{html_str}.pkg.bin"));
    let mut w = 1080.0f32;
    let mut h = 1920.0f32;
    let mut atlas_name = String::from("loom.atlas.png");
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                out_path = args.get(i + 1).cloned().unwrap_or(out_path);
                i += 2;
            }
            "-w" => {
                w = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(w);
                i += 2;
            }
            "-h" => {
                h = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(h);
                i += 2;
            }
            "-a" | "--atlas-name" => {
                atlas_name = args.get(i + 1).cloned().filter(|s| !s.is_empty()).unwrap_or(atlas_name);
                i += 2;
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let res_dir: &Path = html_path.parent().unwrap_or_else(|| Path::new("."));

    let html = match fs::read_to_string(&html_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", html_path.display());
            return ExitCode::FAILURE;
        }
    };
    let css = match fs::read_to_string(css_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {css_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    match loomgui_pkg::pack_named(&html, &css, (w, h), res_dir, &atlas_name) {
        Ok(p) => {
            if let Err(e) = fs::write(&out_path, &p.pkg_bytes) {
                eprintln!("write {out_path}: {e}");
                return ExitCode::FAILURE;
            }
            let out_parent = Path::new(&out_path)
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let atlas_path = out_parent.join(&p.atlas_filename);
            if !p.atlas_png.is_empty() {
                if let Err(e) = fs::write(&atlas_path, &p.atlas_png) {
                    eprintln!("write {}: {e}", atlas_path.display());
                    return ExitCode::FAILURE;
                }
            }
            eprintln!(
                "wrote {out_path} ({} bytes) + atlas {} ({} bytes)",
                p.pkg_bytes.len(),
                p.atlas_filename,
                p.atlas_png.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pack: {e}");
            ExitCode::FAILURE
        }
    }
}
