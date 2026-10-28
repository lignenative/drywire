// Checks the license of every JavaScript package Bun installed, against
// the list in CLAUDE.md: the JavaScript side of `cargo deny` and
// `go-licenses`. Run with Bun from the repository root:
//
//   bun scripts/js-licenses.ts desktop
//
// It reads what is installed in each directory's node_modules, so run it
// after `bun install`. Platform packages (one binary per system) are
// only installed for the system at hand: CI runs this on each of them.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/** CLAUDE.md, "Dependencies and licenses", as SPDX identifiers. */
export const ALLOWED = new Set([
  "MIT",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "MPL-2.0",
]);

/**
 * Whether an SPDX expression is acceptable: with OR, one choice must be
 * allowed; with AND, every part. A license WITH an exception is only
 * allowed when the whole phrase is on the list.
 */
export function allowed(expression: string, list: Set<string> = ALLOWED): boolean {
  const tokens = expression.replace(/[()]/g, " $& ").trim().split(/\s+/).filter(Boolean);
  let i = 0;
  const peek = () => tokens[i];
  const next = () => tokens[i++];
  const or = (): boolean => {
    let ok = and();
    while (peek()?.toUpperCase() === "OR") {
      next();
      ok = and() || ok;
    }
    return ok;
  };
  const and = (): boolean => {
    let ok = atom();
    while (peek()?.toUpperCase() === "AND") {
      next();
      ok = atom() && ok;
    }
    return ok;
  };
  const atom = (): boolean => {
    const token = next();
    if (token === undefined) throw new Error(`incomplete license ${JSON.stringify(expression)}`);
    if (token === "(") {
      const ok = or();
      if (next() !== ")") throw new Error(`unbalanced license ${JSON.stringify(expression)}`);
      return ok;
    }
    if (peek()?.toUpperCase() === "WITH") {
      next();
      return list.has(`${token} WITH ${next()}`);
    }
    return list.has(token.replace(/\+$/, ""));
  };
  try {
    const ok = or();
    return ok && i === tokens.length;
  } catch {
    return false;
  }
}

interface Manifest {
  name?: string;
  version?: string;
  license?: string | { type?: string };
  licenses?: { type?: string }[];
}

/** The license a package declares, as one SPDX expression, if any. */
export function declared(manifest: Manifest): string | undefined {
  const { license, licenses } = manifest;
  if (typeof license === "string") return license;
  if (license?.type) return license.type;
  // The old form: a list of licenses to choose from.
  const types = (licenses ?? []).map((l) => l.type).filter((t): t is string => !!t);
  return types.length > 0 ? `(${types.join(" OR ")})` : undefined;
}

/** Every package under a node_modules directory, nested ones included. */
function* packages(nodeModules: string): Generator<string> {
  let entries: string[];
  try {
    entries = readdirSync(nodeModules);
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry === ".bin" || entry === ".cache") continue;
    const path = join(nodeModules, entry);
    if (!statSync(path).isDirectory()) continue;
    if (entry === ".bun") {
      // Bun's isolated installs: .bun/<name>@<version>/node_modules/<name>.
      for (const store of readdirSync(path)) yield* packages(join(path, store, "node_modules"));
    } else if (entry.startsWith("@")) {
      for (const scoped of readdirSync(path)) yield* packageAndNested(join(path, scoped));
    } else {
      yield* packageAndNested(path);
    }
  }
}

function* packageAndNested(dir: string): Generator<string> {
  try {
    statSync(join(dir, "package.json"));
  } catch {
    return;
  }
  yield dir;
  yield* packages(join(dir, "node_modules"));
}

function main(dirs: string[]): number {
  if (dirs.length === 0) {
    console.error("usage: bun scripts/js-licenses.ts DIR...");
    return 2;
  }
  const seen = new Map<string, string | undefined>();
  for (const dir of dirs) {
    for (const pkg of packages(join(dir, "node_modules"))) {
      const manifest: Manifest = JSON.parse(readFileSync(join(pkg, "package.json"), "utf8"));
      if (!manifest.name || !manifest.version) continue;
      seen.set(`${manifest.name}@${manifest.version}`, declared(manifest));
    }
  }
  if (seen.size === 0) {
    console.error(`no package installed in ${dirs.join(", ")}: run bun install first`);
    return 1;
  }
  const refused = [...seen].filter(([, license]) => !license || !allowed(license));
  for (const [pkg, license] of refused) {
    console.error(`${pkg}: license ${license ?? "(none)"} is not allowed (CLAUDE.md)`);
  }
  if (refused.length > 0) return 1;
  console.log(`licenses ok: ${seen.size} packages`);
  return 0;
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
