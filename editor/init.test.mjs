// init.test.mjs — mergeRuleFile 三态单元测试 + updated 分支真替换验证
// 跑法: node --test editor/init.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { mergeRuleFile } from "./init.mjs";
import { writeFileSync, readFileSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const BEGIN = "<!-- loomgui-editor-begin -->";
const END = "<!-- loomgui-editor-end -->";
const tmplV1 = `${BEGIN}\n# v1 rules\n${END}\n`;
const tmplV2 = `${BEGIN}\n# v2 rules\n${END}\n`;

test("created: no file → creates with tagged content", () => {
  const dir = mkdtempSync(join(tmpdir(), "init-test-"));
  try {
    const f = join(dir, "CLAUDE.md");
    const action = mergeRuleFile(f, tmplV1);
    assert.strictEqual(action, "created");
    const content = readFileSync(f, "utf8");
    assert.ok(content.includes("# v1 rules"), "v1 content written");
    assert.ok(content.includes(BEGIN), "BEGIN tag present");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("appended: file without tags → appends tagged block", () => {
  const dir = mkdtempSync(join(tmpdir(), "init-test-"));
  try {
    const f = join(dir, "CLAUDE.md");
    writeFileSync(f, "# user content\n", "utf8");
    const action = mergeRuleFile(f, tmplV1);
    assert.strictEqual(action, "appended");
    const content = readFileSync(f, "utf8");
    assert.ok(content.includes("# user content"), "user content preserved");
    assert.ok(content.includes("# v1 rules"), "rules appended");
    const beginCount = (content.match(new RegExp(BEGIN, "g")) || []).length;
    assert.strictEqual(beginCount, 1, "single BEGIN tag");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("updated: tagged block replaced — old content MUST NOT remain", () => {
  const dir = mkdtempSync(join(tmpdir(), "init-test-"));
  try {
    const f = join(dir, "CLAUDE.md");
    // Write v1 first, then update to v2
    mergeRuleFile(f, tmplV1);
    const action = mergeRuleFile(f, tmplV2);
    assert.strictEqual(action, "updated");
    const content = readFileSync(f, "utf8");
    // Key assertions: new content present, old content absent
    assert.ok(content.includes("# v2 rules"), "new rules written");
    assert.ok(!content.includes("# v1 rules"), "old rules replaced (no residue)");
    const beginCount = (content.match(new RegExp(BEGIN, "g")) || []).length;
    assert.strictEqual(beginCount, 1, "tags not duplicated");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
