#!/usr/bin/env node
// init.mjs — 把 LoomGUI 围栏规则 + skill 注入设计师工作区。
// 交互输入：工作区路径 / 输出路径 / harness（claude/opencode/codex）。
// 零第三方依赖：只用 node:fs / node:path / node:readline。

import { createInterface } from "node:readline/promises";
import { stdin as input, stdout as output } from "node:process";
import {
  existsSync, mkdirSync, readFileSync, writeFileSync, readdirSync, copyFileSync, statSync,
} from "node:fs";
import { join, resolve, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
// LOOMGUI_ROOT = editor/ 的上一层 = 仓库根。
const LOOMGUI_ROOT = resolve(__dirname, "..");

const BEGIN = "<!-- loomgui-editor-begin -->";
const END = "<!-- loomgui-editor-end -->";

// harness → (规则文件名, skill 目录)
const HARNESS = {
  claude: { ruleFile: "CLAUDE.md", ruleDir: join(__dirname, "rules", "claude") },
  opencode: { ruleFile: "AGENTS.md", ruleDir: join(__dirname, "rules", "opencode") },
  codex: { ruleFile: "AGENTS.md", ruleDir: join(__dirname, "rules", "codex") },
};

function ask(rl, q) { return rl.question(q); }

// 增量合并规则文件：无则新建，有则替换标签段（保留用户原有内容）。
function mergeRuleFile(targetPath, tmplContent) {
  const block = `${BEGIN}\n${tmplContent.replace(/^<!-- loomgui-editor-begin -->\n?/, "").replace(/\n?<!-- loomgui-editor-end -->\s*$/, "")}\n${END}\n`;
  // tmplContent 本身已含标签，直接用 tmplContent 作 block。
  const tagged = tmplContent.includes(BEGIN) ? tmplContent : `${BEGIN}\n${tmplContent}\n${END}\n`;
  if (!existsSync(targetPath)) {
    writeFileSync(targetPath, tagged, "utf8");
    return "created";
  }
  const existing = readFileSync(targetPath, "utf8");
  if (!existing.includes(BEGIN)) {
    // 无标签：追加。
    writeFileSync(targetPath, existing.replace(/\n*$/, "\n\n") + tagged, "utf8");
    return "appended";
  }
  // 有标签：替换标签段。
  const re = new RegExp(`${BEGIN}[\\s\\S]*?${END}`, "g");
  const updated = existing.replace(re, tagged.trimEnd());
  writeFileSync(targetPath, updated, "utf8");
  return "updated";
}

// 递归拷贝 skill 目录，pack.mjs 的 __LOOMGUI_ROOT__ 占位符替换成实际路径。
function copySkill(srcDir, destDir) {
  mkdirSync(destDir, { recursive: true });
  for (const entry of readdirSync(srcDir)) {
    const srcPath = join(srcDir, entry);
    const destPath = join(destDir, entry);
    if (statSync(srcPath).isDirectory()) {
      copySkill(srcPath, destPath);
    } else {
      let content = readFileSync(srcPath, "utf8");
      if (entry === "pack.mjs") {
        content = content.replaceAll("__LOOMGUI_ROOT__", LOOMGUI_ROOT.replaceAll("\\", "/"));
      }
      writeFileSync(destPath, content, "utf8");
    }
  }
}

async function main() {
  const rl = createInterface({ input, output });

  const workspace = resolve(await ask(rl, "目标工作区路径（绝对路径）: "));
  const outputDir = resolve(await ask(rl, "pkg.bin 输出目录（绝对路径，如 Unity StreamingAssets）: "));
  console.log("harness 选项: claude / opencode / codex");
  const harness = (await ask(rl, "选择 harness: ")).trim();
  rl.close();

  if (!HARNESS[harness]) {
    console.error(`未知 harness: ${harness}（支持 claude/opencode/codex）`);
    process.exit(2);
  }
  if (!existsSync(workspace)) {
    console.error(`工作区不存在: ${workspace}`);
    process.exit(2);
  }
  const { ruleFile, ruleDir } = HARNESS[harness];
  const tmplPath = join(ruleDir, `${ruleFile}.tmpl`);
  if (!existsSync(tmplPath)) {
    console.error(`规则模板不存在: ${tmplPath}`);
    process.exit(2);
  }

  // ① 注入围栏规则（增量合并）。
  const tmplContent = readFileSync(tmplPath, "utf8");
  const ruleTarget = join(workspace, ruleFile);
  const action = mergeRuleFile(ruleTarget, tmplContent);
  console.log(`[init] ${ruleFile}: ${action}（${ruleTarget}）`);

  // ② 拷贝 skill（pack.mjs 占位符替换）。
  const skillSrc = join(__dirname, "skill", "loomgui-editor");
  // harness 的 skill 发现路径：claude=.claude/skills/，opencode/codex 暂同（实现期查文档，先放 .claude/skills）。
  const skillDest = join(workspace, ".claude", "skills", "loomgui-editor");
  copySkill(skillSrc, skillDest);
  console.log(`[init] skill 注入: ${skillDest}`);

  // ③ 写 outputDir 提示到工作区（skill 需要知道 pkg.bin 落哪）。
  const cfgPath = join(workspace, ".claude", "skills", "loomgui-editor", "config.json");
  mkdirSync(dirname(cfgPath), { recursive: true });
  writeFileSync(cfgPath, JSON.stringify({ output_dir: outputDir, loomgui_root: LOOMGUI_ROOT }, null, 2), "utf8");
  console.log(`[init] config.json: ${cfgPath}（output_dir=${outputDir}）`);

  console.log("\n完成。接下来：");
  console.log(`  1. open-design import 工作区: ${workspace}`);
  console.log(`  2. 在 open-design 里用 AI 生成 UI，skill 会引导跑 pack.mjs 验证+打包`);
  console.log(`  3. pkg.bin 产出到: ${outputDir}`);
}

main().catch((e) => { console.error(e); process.exit(1); });
