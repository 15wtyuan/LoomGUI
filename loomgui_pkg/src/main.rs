//! 极简 CLI（不引 clap）：loomgui_pkg <sourceDir> <pkgName> [--html <h1,h2,...>] [--res <name>] [-o <out.pkg.bin>]。
//! 不传 --html → 扫 sourceDir 顶层所有 .html（不递归，排除 res 目录）。
//! --res 默认 res。-o 默认 <sourceDir>/<pkgName>.pkg.bin。
//! 产物只写 pkg.bin（不写 atlas.png——图集归 Unity，D8）。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: {} <sourceDir> <pkgName> [--html <h1,h2,...>] [--res <name>] [-o <out.pkg.bin>]",
            args.first().map(String::as_str).unwrap_or("loomgui_pkg")
        );
        return ExitCode::from(2);
    }
    let source_dir = PathBuf::from(&args[1]);
    let pkg_name = &args[2];
    let mut html_list: Option<Vec<String>> = None;
    let mut res_dir = String::from("res");
    let mut out_path: Option<String> = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--html" => {
                let v = args.get(i + 1).cloned().unwrap_or_default();
                html_list = Some(
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
                i += 2;
            }
            "--res" => {
                res_dir = args.get(i + 1).cloned().filter(|s| !s.is_empty()).unwrap_or(res_dir);
                i += 2;
            }
            "-o" => {
                out_path = args.get(i + 1).cloned();
                i += 2;
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::from(2);
            }
        }
    }

    // 不传 --html → 扫 sourceDir 顶层所有 .html（不递归，排除 res 目录下的）。
    let html_files: Vec<String> = match html_list {
        Some(list) => list,
        None => match scan_top_level_html(&source_dir, &res_dir) {
            Ok(list) if !list.is_empty() => list,
            Ok(_) => {
                eprintln!("no .html files found in {}", source_dir.display());
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("scan {}: {e}", source_dir.display());
                return ExitCode::FAILURE;
            }
        },
    };

    let out = out_path.unwrap_or_else(|| {
        source_dir
            .join(format!("{pkg_name}.pkg.bin"))
            .to_string_lossy()
            .into_owned()
    });

    match loomgui_pkg::pack(&source_dir, pkg_name, &html_files, &res_dir) {
        Ok(p) => {
            if let Err(e) = fs::write(&out, &p.pkg_bytes) {
                eprintln!("write {out}: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!(
                "wrote {out} ({} bytes, {} components, {} manifest paths)",
                p.pkg_bytes.len(),
                html_files.len(),
                p.asset_manifest.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pack: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 扫 sourceDir 顶层 .html 文件（不递归子目录），排除 res 目录下的。
/// 返回相对 sourceDir 的文件名列表（如 ["a.html", "b.html"]），按字母序。
fn scan_top_level_html(source_dir: &Path, res_dir: &str) -> std::io::Result<Vec<String>> {
    let mut list: Vec<String> = Vec::new();
    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let path = entry.path();
        // 只收文件（跳过子目录，含 res/），不递归。
        if !path.is_file() {
            continue;
        }
        // 排除 res 目录下的文件（顶层 res 是目录，已被 is_file 跳过；此处防御同名文件）。
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == res_dir {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("html") {
            list.push(name);
        }
    }
    list.sort();
    Ok(list)
}
