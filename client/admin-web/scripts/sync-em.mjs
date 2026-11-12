#!/usr/bin/env node
/**
 * W15-1 / em 白名单同步（构建链 C publicDir 方案）。
 *
 * 把 `client/apps/h5App/em/` 的**运行时最小集 4 文件**同步到本工程的 `public/em/`：
 *   index.html + static/{adapter.js,dashboard.js,host.js}
 * 官方页只引用这三个相对脚本（em/index.html:587-591），第三方库走 CDN + SRI；
 * 源目录里的 static/dashboard.jsx（648KB 源码）、tests/ 5 个 .cjs、
 * README.md / README.sha256 / LICENSE 一律不带入下发面。
 *
 * 不直接改 vite.config.ts 的 publicDir：Vite publicDir 没有 exclude 能力，指向源目录
 * 会把上述非运行时文件全量下发；而 build.emptyOutDir:true 会清空 dist，build 之后
 * 再手工拷进 dist/em 也必被下一次 build 清掉。这里写进 public/，Vite 会在同一次
 * build 内（清空 outDir 之后）自动把 public/em 拷进 dist/em，天然兼容。
 *
 * 幂等 & 清理口径：`public/em/` 是本脚本**独占的生成目录**（.gitignore 已忽略），
 * 每次运行整体重建；绝不触碰 public/ 下其它文件（如 gen-icons.py 生成的 logo.png）。
 * em 源目录 13 文件原位保留、不迁不删（旧端软下线第一波）。
 */
import { constants } from "node:fs";
import { access, copyFile, mkdir, readdir, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = path.resolve(SCRIPT_DIR, "..");
const EM_SOURCE_DIR = path.resolve(PROJECT_ROOT, "../apps/h5App/em");
const EM_TARGET_DIR = path.resolve(PROJECT_ROOT, "public/em");

/** @type {{ from: string, to: string }[]} 运行时最小集，顺序即拷贝顺序。 */
const WHITELIST = [
  { from: "index.html", to: "index.html" },
  { from: "static/adapter.js", to: "static/adapter.js" },
  { from: "static/dashboard.js", to: "static/dashboard.js" },
  { from: "static/host.js", to: "static/host.js" },
];

/**
 * 断言源文件可读；缺失时立刻失败，不允许「静默少拷一个文件」。
 * @param {string} filePath 待检查的绝对路径。
 * @returns {Promise<void>}
 */
async function assertReadable(filePath) {
  try {
    await access(filePath, constants.R_OK);
  } catch {
    throw new Error(`缺少 em 白名单源文件：${filePath}（em/ 源目录 13 文件应原位保留）`);
  }
}

/**
 * 递归列出目录下所有文件，返回以 POSIX 分隔符表示的相对路径。
 * @param {string} dir 待遍历的绝对路径。
 * @returns {Promise<string[]>} 相对 EM_TARGET_DIR 的文件路径列表。
 */
async function listFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFiles(absolute)));
    } else if (entry.isFile()) {
      files.push(path.relative(EM_TARGET_DIR, absolute).split(path.sep).join("/"));
    }
  }
  return files;
}

/**
 * 重建 public/em/、拷贝白名单文件并自检产物集合恰好等于白名单。
 * @returns {Promise<void>}
 */
async function main() {
  for (const entry of WHITELIST) {
    await assertReadable(path.join(EM_SOURCE_DIR, entry.from));
  }

  await rm(EM_TARGET_DIR, { recursive: true, force: true });
  await mkdir(path.join(EM_TARGET_DIR, "static"), { recursive: true });

  for (const entry of WHITELIST) {
    await copyFile(path.join(EM_SOURCE_DIR, entry.from), path.join(EM_TARGET_DIR, entry.to));
  }

  const actual = (await listFiles(EM_TARGET_DIR)).sort();
  const expected = WHITELIST.map((entry) => entry.to).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `public/em/ 产物集合不符：期望 [${expected.join(", ")}]；实际 [${actual.join(", ")}]`,
    );
  }
  console.log(
    `[sync-em] 已同步 ${actual.length} 个白名单文件到 ${EM_TARGET_DIR}：${actual.join(", ")}`,
  );
}

main().catch((error) => {
  console.error(`[sync-em] FAIL: ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
});
