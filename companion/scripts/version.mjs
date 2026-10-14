#!/usr/bin/env node
/**
 * 版本一致性脚本（零依赖，复刻 easyop scripts/version.mjs 模式）。
 *
 * 铁律（钦友 2026-09-18 立规）：
 *   - 版本号只有一个真源：package.json 的 version；
 *   - Cargo.toml / tauri.conf.json 一律由本脚本同步，禁止手改；
 *   - 收工必须跑 `npm run version:check`，exit 0 才算完。
 *
 * 用法：
 *   node scripts/version.mjs check          校验一致（CI / 收工）
 *   node scripts/version.mjs sync           把 package.json 版本写进各文件
 *   node scripts/version.mjs bump <patch|minor|major>
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pkgPath = join(root, "package.json");
const cargoPath = join(root, "src-tauri", "Cargo.toml");
const confPath = join(root, "src-tauri", "tauri.conf.json");

function readJson(p) {
  return JSON.parse(readFileSync(p, "utf8"));
}

function replaceOnce(text, pattern, replacement, what) {
  if (!pattern.test(text)) {
    console.error(`✗ ${what}: 未找到匹配 ${pattern}`);
    process.exit(1);
  }
  return text.replace(pattern, replacement);
}

function syncTargets(version) {
  // Cargo.toml: [package] 下的 version 行（第一个）。
  let cargo = readFileSync(cargoPath, "utf8");
  cargo = replaceOnce(
    cargo,
    /^version = "[^"]*"$/m,
    `version = "${version}"`,
    "Cargo.toml version"
  );
  writeFileSync(cargoPath, cargo);

  // tauri.conf.json: 顶层 version 字段。
  let conf = readFileSync(confPath, "utf8");
  conf = replaceOnce(
    conf,
    /"version": "[^"]*"/,
    `"version": "${version}"`,
    "tauri.conf.json version"
  );
  writeFileSync(confPath, conf);
}

function currentVersions() {
  const pkg = readJson(pkgPath).version;
  const cargo = readFileSync(cargoPath, "utf8").match(/^version = "([^"]*)"/m)?.[1];
  const conf = readJson(confPath).version;
  return { pkg, cargo, conf };
}

function check() {
  const v = currentVersions();
  const ok = v.pkg === v.cargo && v.pkg === v.conf;
  if (ok) {
    console.log(`✓ 版本一致: ${v.pkg}`);
    process.exit(0);
  }
  console.error(`✗ 版本不一致: package.json=${v.pkg} Cargo.toml=${v.cargo} tauri.conf.json=${v.conf}`);
  console.error("  修复: npm run version:sync");
  process.exit(1);
}

const cmd = process.argv[2];
if (cmd === "check") {
  check();
} else if (cmd === "sync") {
  const v = readJson(pkgPath).version;
  syncTargets(v);
  const after = currentVersions();
  console.log(`✓ 已同步到 ${v}（Cargo.toml=${after.cargo}, tauri.conf.json=${after.conf}）`);
} else if (cmd === "bump") {
  const part = process.argv[3];
  if (!["patch", "minor", "major"].includes(part)) {
    console.error("用法: node scripts/version.mjs bump <patch|minor|major>");
    process.exit(1);
  }
  const pkg = readJson(pkgPath);
  const [maj, min, pat] = pkg.version.split(".").map(Number);
  const next =
    part === "major"
      ? `${maj + 1}.0.0`
      : part === "minor"
        ? `${maj}.${min + 1}.0`
        : `${maj}.${min}.${pat + 1}`;
  pkg.version = next;
  writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
  syncTargets(next);
  console.log(`✓ 版本已升级 ${maj}.${min}.${pat} → ${next}`);
} else {
  console.error("用法: node scripts/version.mjs <check|sync|bump>");
  process.exit(1);
}
