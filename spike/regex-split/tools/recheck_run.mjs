// SPIKE: ReDoS analysis of every pattern with recheck (npm `recheck`), and attack strings.
// Usage: node recheck_run.mjs <patterns.jsonl> <outdir>
// Writes <outdir>/recheck.jsonl (one line per pattern) and attack strings, pumped to about
// 64 KiB and 1 MiB, in <outdir>/attack-core-{native,64k,1m}/<id>.txt.
// PCRE -> JS translation is best effort and conservative (atomic and possessive become
// plain groups and greedy quantifiers), so recheck may call a safe PCRE2 pattern
// vulnerable; the measured runs decide.
import { createRequire } from "node:module";
const { checkSync } = createRequire(import.meta.url)("recheck");
import fs from "node:fs";
import path from "node:path";

const [, , patsFile, outdir] = process.argv;
const W = String.raw`[\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200C\u200D]`;
const NW = String.raw`[^\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200C\u200D]`;
const SYNTAX = new Set("^$\\.*+?()[]{}|/".split(""));

function toJs(src, caseless) {
  let flags = "u" + (caseless ? "i" : "");
  let s = src;
  const m = s.match(/^\(\?([a-z]+)\)/);
  if (m) {
    if (m[1].includes("i")) flags += flags.includes("i") ? "" : "i";
    if (m[1].includes("m")) flags += "m";
    if (m[1].includes("s")) flags += "s";
    s = s.slice(m[0].length);
  }
  let out = "";
  let inClass = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "\\") {
      const n = s[++i];
      if (n === undefined) break;
      if (n === "A") { out += "^"; continue; }
      if (n === "z") { out += "$"; continue; }
      if (n === "Z") { out += "(?=\\n?$)"; continue; }
      if (n === "h") { out += inClass ? " \\t\\u00A0" : "[ \\t\\u00A0]"; continue; }
      if (n === "K") { continue; }
      if (n === "w") { out += inClass ? W.slice(1, -1) : W; continue; }
      if (n === "W" && !inClass) { out += NW; continue; }
      if (n === "x" && s[i + 1] === "{") {
        const j = s.indexOf("}", i);
        out += "\\u{" + s.slice(i + 2, j) + "}";
        i = j;
        continue;
      }
      if (n === "Q") {
        const j = s.indexOf("\\E", i);
        const lit = s.slice(i + 1, j < 0 ? s.length : j);
        out += lit.replace(/[\\^$.*+?()[\]{}|/-]/g, (x) => "\\" + x);
        i = j < 0 ? s.length : j + 1;
        continue;
      }
      if (n === "E") continue;
      if (/[A-Za-z0-9]/.test(n) || SYNTAX.has(n) || (inClass && n === "-")) {
        out += "\\" + n;
      } else {
        out += n === "/" ? "\\/" : n;
      }
      continue;
    }
    if (inClass) {
      if (c === "]") inClass = false;
      out += c;
      continue;
    }
    if (c === "[") {
      inClass = true;
      out += c;
      if (s[i + 1] === "^") { out += "^"; i++; }
      if (s[i + 1] === "]") { out += "\\]"; i++; }
      continue;
    }
    if (c === "(" && s[i + 1] === "?") {
      if (s[i + 2] === ">") { out += "(?:"; i += 2; continue; }
      if (s[i + 2] === "P" && s[i + 3] === "<") { out += "(?<"; i += 3; continue; }
    }
    if ((c === "+" ) && /[*+?}]/.test(s[i - 1] ?? "") && s[i - 2] !== "\\") continue; // possessive
    out += c;
  }
  return { source: out, flags };
}

function pump(attack, size) {
  if (!attack || !attack.pumps || attack.pumps.length === 0) return attack?.string ?? null;
  const base = attack.pumps.reduce((a, p) => a + p.prefix.length, 0) + attack.suffix.length;
  const unit = attack.pumps.reduce((a, p) => a + p.pump.length, 0) || 1;
  const k = Math.max(1, Math.floor((size - base) / unit));
  return attack.pumps.map((p) => p.prefix + p.pump.repeat(k)).join("") + attack.suffix;
}

for (const d of ["native", "64k", "1m"]) fs.mkdirSync(path.join(outdir, `attack-core-${d}`), { recursive: true });
const out = fs.createWriteStream(path.join(outdir, "recheck.jsonl"));
for (const line of fs.readFileSync(patsFile, "utf8").split("\n")) {
  if (!line.trim()) continue;
  const p = JSON.parse(line);
  const js = toJs(p.source, p.caseless);
  let rec = { id: p.id, origin: p.origin, js: js.source, flags: js.flags };
  try {
    const t0 = Date.now();
    const r = checkSync(js.source, js.flags, { timeout: 10000 });
    rec.status = r.status;
    rec.checker = r.checker;
    rec.complexity = r.complexity ? { type: r.complexity.type, degree: r.complexity.degree ?? null } : null;
    rec.error = r.error ? r.error.kind + ": " + (r.error.message ?? "") : null;
    rec.ms = Date.now() - t0;
    if (r.status === "vulnerable" && r.attack) {
      rec.attack_native_len = r.attack.string.length;
      for (const [d, size] of [["native", 0], ["64k", 65536], ["1m", 1 << 20]]) {
        const s = size === 0 ? r.attack.string : pump(r.attack, size);
        fs.writeFileSync(path.join(outdir, `attack-core-${d}`, `${p.id}.txt`), s);
      }
    }
  } catch (e) {
    rec.status = "exception";
    rec.error = String(e).slice(0, 300);
  }
  out.write(JSON.stringify(rec) + "\n");
}
out.end();
