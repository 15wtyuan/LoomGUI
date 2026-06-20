//! 极简 CLI（不引 clap）：loomgui_pkg <html> <css> [-o out.pkg.bin] [-w designW] [-h designH]。
//! 默认 out = <html 去 .html>.pkg.bin，默认 root_size = 1080×1920。

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} <html> <css> [-o out.pkg.bin] [-w designW] [-h designH]", args.first().map(String::as_str).unwrap_or("loomgui_pkg"));
        return ExitCode::from(2);
    }
    let html_path = &args[1];
    let css_path = &args[2];
    let mut out_path = html_path.rsplit_once('.').map(|(stem, _)| format!("{stem}.pkg.bin")).unwrap_or_else(|| format!("{html_path}.pkg.bin"));
    let mut w = 1080.0f32;
    let mut h = 1920.0f32;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => { out_path = args.get(i + 1).cloned().unwrap_or(out_path); i += 2; }
            "-w" => { w = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(w); i += 2; }
            "-h" => { h = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(h); i += 2; }
            other => { eprintln!("unknown arg: {other}"); return ExitCode::from(2); }
        }
    }

    let html = match fs::read_to_string(html_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("read {html_path}: {e}"); return ExitCode::FAILURE; }
    };
    let css = match fs::read_to_string(css_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("read {css_path}: {e}"); return ExitCode::FAILURE; }
    };

    match loomgui_pkg::pack(&html, &css, (w, h)) {
        Ok(bytes) => match fs::write(&out_path, &bytes) {
            Ok(_) => { eprintln!("wrote {out_path} ({} bytes)", bytes.len()); ExitCode::SUCCESS }
            Err(e) => { eprintln!("write {out_path}: {e}"); ExitCode::FAILURE }
        },
        Err(e) => { eprintln!("pack: {e}"); ExitCode::FAILURE }
    }
}
