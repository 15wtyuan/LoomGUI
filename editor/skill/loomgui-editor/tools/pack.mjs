#!/usr/bin/env node
// pack.mjs — 调 loomgui_pkg 验证+打包。封装层，设计师/AI 只见"成功产出 pkg.bin / 失败报围栏错"。
// LOOMGUI_ROOT 由 init.mjs 注入时替换 __LOOMGUI_ROOT__ 占位符。
// 用法：node pack.mjs <html> <css> -o <out.pkg.bin> [-w 1080 -h 1920] [-a atlas.png]

import { execFileSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { join } from "node:path";

const LOOMGUI_ROOT = "__LOOMGUI_ROOT__"; // init.mjs 替换

// 解析命令行参数（与 loomgui_pkg CLI 对齐）。
const args = process.argv.slice(2);
if (args.length < 2) {
  console.error("usage: node pack.mjs <html> <css> -o <out.pkg.bin> [-w 1080] [-h 1920] [-a atlas.png]");
  process.exit(2);
}
const html = args[0];
const css = args[1];
const pkgArgs = [html, css];
for (let i = 2; i < args.length; i++) {
  if (args[i] === "-o" || args[i] === "-w" || args[i] === "-h" || args[i] === "-a" || args[i] === "--atlas-name") {
    if (args[i + 1] === undefined || args[i + 1] === "") {
      console.error(`flag ${args[i]} 缺值`);
      process.exit(2);
    }
    pkgArgs.push(args[i], args[i + 1]);
    i++;
  } else {
    console.error(`unknown arg: ${args[i]}`);
    process.exit(2);
  }
}

// 定位 loomgui_pkg 二进制：优先 target/release，不存在或源码更新则 cargo build。
const binPath = join(LOOMGUI_ROOT, "target", "release", "loomgui_pkg" + (process.platform === "win32" ? ".exe" : ""));
const cargoToml = join(LOOMGUI_ROOT, "loomgui_pkg", "Cargo.toml");

function needBuild() {
  if (!existsSync(binPath)) return true;
  // 源码 mtime 比 二进制新 → 重新 build。
  const binMtime = statSync(binPath).mtimeMs;
  for (const src of [join(LOOMGUI_ROOT, "loomgui_pkg", "src", "main.rs"), join(LOOMGUI_ROOT, "loomgui_pkg", "src", "lib.rs"), cargoToml]) {
    if (existsSync(src) && statSync(src).mtimeMs > binMtime) return true;
  }
  return false;
}

if (needBuild()) {
  process.stderr.write("[pack] building loomgui_pkg (release)...\n");
  try {
    execFileSync("cargo", ["build", "-p", "loomgui_pkg", "--release"], { cwd: LOOMGUI_ROOT, stdio: "inherit" });
  } catch (e) {
    console.error("[pack] cargo build failed");
    process.exit(1);
  }
}

// 调 loomgui_pkg CLI，透传 stdout/stderr/exit code。
// 违规 → loomgui_pkg 非零退出 + stderr 报围栏错，AI 据此自纠。
try {
  execFileSync(binPath, pkgArgs, { stdio: "inherit" });
} catch (e) {
  // 非零退出：围栏违规或打包失败。透传已由 stdio:inherit 完成。
  process.exit(e.status ?? 1);
}
