#!/usr/bin/env python3
"""Regenerate the slopvac-pos regression fixtures from Vale 3.27 (stdlib only).

Vale tags with prose v3.3.1, the tagger this crate ports, so Vale's tags are
the expected values. The script writes a throwaway Vale config with one
`sequence` rule per Penn tag, runs Vale on the inputs, and records the tag of
every word Vale reports.

Outputs, under tests/fixtures/ (all committed):

  gold.tsv    one row per hand-labelled target in gold.txt:
              sentence, word, byte offset, gold class (N/V/J/R), Vale tag
  corpus.md   a slice of the evaluation corpus, paragraphs separated by blank lines
  corpus.tsv  one row per Vale-tagged word of corpus.md:
              line (1-based), byte offset in the line, word, Vale tag
  SOURCES.md  which corpus files the slice came from

Usage:
  gen_fixtures.py --corpus DIR   regenerate (needs `vale` >= 3.27 on PATH)
  gen_fixtures.py --check        regenerate in memory and diff the committed
                                 files (also needs --corpus and `vale`)

The corpus is the 92-file evaluation corpus of the 3.0 POS spike
(slopvac-8l4.3.10). Only files from permissively licensed projects feed the
slice; see SOURCES.md.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
FIX = HERE.parent / "tests" / "fixtures"

# Penn tags of words. Punctuation tags are left out: the crate tags them, but
# Vale's `tag` matching on them proves nothing about vocabulary rules.
PENN = (
    "CC CD DT EX FW IN JJ JJR JJS LS MD NN NNP NNPS NNS PDT POS PRP PRP$ RB RBR "
    "RBS RP SYM TO UH VB VBD VBG VBN VBP VBZ WDT WP WP$ WRB"
).split()

# Corpus files from projects whose documentation licence permits excerpts with
# attribution (MIT, BSD, Apache-2.0, CC BY 4.0). OWASP
# (CC BY-SA) and projects without a stated licence are excluded.
PERMISSIVE = {
    "astral-sh_ruff": "MIT",
    "astral-sh_uv": "MIT/Apache-2.0",
    "BurntSushi_ripgrep": "MIT/Unlicense",
    "denoland_deno": "MIT",
    "docker_docs": "Apache-2.0",
    "encode_httpx": "BSD-3-Clause",
    "fastapi_fastapi": "MIT",
    "tiangolo_fastapi": "MIT",
    "jqlang_jq": "MIT",
    "kubernetes_kubernetes": "Apache-2.0",
    "kubernetes_website": "CC-BY-4.0",
    "junegunn_fzf": "MIT",
    "nodejs_node": "MIT",
    "pallets_flask": "BSD-3-Clause",
    "psf_requests": "Apache-2.0",
    "pydantic_pydantic": "MIT",
    "redis_redis": "BSD-3-Clause",
    "rust-lang_book": "MIT/Apache-2.0",
    "rust-lang_rust": "MIT/Apache-2.0",
    "sharkdp_bat": "MIT/Apache-2.0",
    "tailwindlabs_tailwindcss": "MIT",
    "tokio-rs_tokio": "MIT",
    "vitejs_vite": "MIT",
}

SHIPPED_WORDS = {
    "deploy", "return", "execute", "branch", "request", "file", "key",
    "simple", "easy", "quickly", "correctly",
}  # fmt: skip
TARGET_WORDS = 14000
MAX_SHIPPED_PARAS = 300


def style_name(tag: str) -> str:
    return "t_" + tag.replace("$", "S")


def vale_tags(
    md: pathlib.Path, workdir: pathlib.Path
) -> list[tuple[int, int, str, str]]:
    """Returns (line, 1-based char column, word, tag) for every word Vale tags."""
    styles = workdir / "styles" / "P"
    styles.mkdir(parents=True, exist_ok=True)
    (workdir / ".vale.ini").write_text(
        "StylesPath = styles\nMinAlertLevel = suggestion\n[*.md]\nBasedOnStyles = P\n"
    )
    for tag in PENN:
        (styles / f"{style_name(tag)}.yml").write_text(
            "extends: sequence\nmessage: '%s'\nlevel: suggestion\ntokens:\n"
            f"  - tag: '^{re.escape(tag)}$'\n"
        )
    by_check = {f"P.{style_name(t)}": t for t in PENN}
    proc = subprocess.run(
        ["vale", "--config", str(workdir / ".vale.ini"), "--output", "JSON", str(md)],
        capture_output=True,
        text=True,
        cwd=workdir,
        check=False,
    )
    if proc.returncode not in (0, 1) or not proc.stdout.strip():
        sys.exit(f"vale failed ({proc.returncode}): {proc.stderr.strip()}")
    out = []
    for alerts in json.loads(proc.stdout).values():
        for a in alerts:
            out.append((a["Line"], a["Span"][0], a["Match"], by_check[a["Check"]]))
    out.sort()
    return out


def byte_offset(line: str, char_col: int) -> int:
    return len(line[: char_col - 1].encode())


def gold_rows(workdir: pathlib.Path) -> str:
    sentences, targets = [], []
    for raw in (FIX / "gold.txt").read_text().splitlines():
        if not raw or raw.startswith("#"):
            continue
        text, pos = "", 0
        for m in re.finditer(r"\[([^/\]]+)/([NVJR])\]", raw):
            text += raw[pos : m.start()]
            targets.append((len(sentences) + 1, len(text), m.group(1), m.group(2)))
            text += m.group(1)
            pos = m.end()
        text += raw[pos:]
        sentences.append(text)
    md = workdir / "gold.md"
    md.write_text("\n\n".join(sentences) + "\n")
    # Paragraph k sits on line 2k-1 of gold.md.
    tags = {(ln, col): (w, t) for ln, col, w, t in vale_tags(md, workdir)}
    rows = []
    for n, char_col, word, cls in targets:
        sentence = sentences[n - 1]
        got = tags.get((2 * n - 1, char_col + 1))
        if got is None or got[0] != word:
            sys.exit(f"vale reported no tag for {word!r} in {sentence!r}")
        rows.append(
            f"{sentence}\t{word}\t{byte_offset(sentence, char_col + 1)}\t{cls}\t{got[1]}"
        )
    return "\n".join(rows) + "\n"


def paragraphs(corpus: pathlib.Path) -> list[tuple[str, str]]:
    """Prose paragraphs: no markup other than inline code, no lists or headings."""
    out = []
    for f in sorted(corpus.glob("*.md")):
        project = f.name.split("__")[1]
        if project not in PERMISSIVE:
            continue
        fence = False
        for block in re.split(r"\n\s*\n", f.read_text()):
            if block.lstrip().startswith(("```", "~~~")) or block.count("```") % 2:
                fence = block.count("```") % 2 == 1 if "```" in block else fence
                continue
            if fence:
                continue
            para = block.strip()
            bare = re.sub(r"`[^`\n]*`", "", para)
            if (
                any(
                    re.match(r"\s|[-+#>]|\d+[.)]\s|[=-]{3}", ln)
                    for ln in para.split("\n")
                )
                or not re.match(r"[A-Z]", para)
                or re.search(r"[<>|\[\]*~_\\{}]|https?:|://", bare)
                or "`" in bare
                or not para.endswith((".", "?", "!"))
                or not 8 <= len(para.split()) <= 120
            ):
                continue
            out.append((f.name, para))
    return out


def corpus_slice(corpus: pathlib.Path) -> tuple[list[tuple[str, str]], list[str]]:
    paras = paragraphs(corpus)
    shipped = [
        i
        for i, (_, p) in enumerate(paras)
        if SHIPPED_WORDS & set(re.findall(r"[a-z]+", p.lower()))
    ][:MAX_SHIPPED_PARAS]
    total = sum(len(p.split()) for _, p in paras)
    step = max(1, round(total / TARGET_WORDS))
    chosen = sorted(set(shipped) | set(range(0, len(paras), step)))
    return [paras[i] for i in chosen], sorted({paras[i][0] for i in chosen})


def corpus_rows(
    slice_: list[tuple[str, str]], workdir: pathlib.Path
) -> tuple[str, str]:
    md_text = "\n\n".join(p for _, p in slice_) + "\n"
    lines = md_text.split("\n")
    md = workdir / "corpus.md"
    md.write_text(md_text)
    rows = []
    for ln, col, word, tag in vale_tags(md, workdir):
        rows.append(f"{ln}\t{byte_offset(lines[ln - 1], col)}\t{word}\t{tag}")
    return md_text, "\n".join(rows) + "\n"


def sources(files: list[str]) -> str:
    head = (
        "# Corpus slice sources\n\n"
        "`corpus.md` holds single-paragraph excerpts from the files below, kept as\n"
        "test input for the part-of-speech tagger. Each project's documentation is\n"
        "under the licence shown; this list is the attribution.\n\n"
        "| Project | Licence | Files |\n| --- | --- | --- |\n"
    )
    by_project: dict[str, list[str]] = {}
    for f in files:
        by_project.setdefault(f.split("__")[1], []).append(f.split("__", 2)[2])
    body = "".join(
        f"| {p} | {PERMISSIVE[p]} | {', '.join(sorted(fs))} |\n"
        for p, fs in sorted(by_project.items())
    )
    return head + body


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--corpus", type=pathlib.Path, required=True)
    ap.add_argument("--check", action="store_true", help="diff instead of writing")
    args = ap.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp)
        slice_, files = corpus_slice(args.corpus)
        corpus_md, corpus_tsv = corpus_rows(slice_, work)
        outputs = {
            "gold.tsv": gold_rows(work),
            "corpus.md": corpus_md,
            "corpus.tsv": corpus_tsv,
            "SOURCES.md": sources(files),
        }
    stale = []
    for name, text in outputs.items():
        path = FIX / name
        if args.check:
            if not path.exists() or path.read_text() != text:
                stale.append(name)
        else:
            path.write_text(text)
    if stale:
        print("stale fixtures:", ", ".join(stale), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
