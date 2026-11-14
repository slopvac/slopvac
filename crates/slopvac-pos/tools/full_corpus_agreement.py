#!/usr/bin/env python3
"""Full-corpus blocklist agreement between slopvac-pos and Vale (the spike's method).

For every file of the evaluation corpus this compares, per part of speech, the
findings of the shipped vocabulary blocklist (`examples/blocklist.toml`):

  * Vale: the real generated `sequence` rules (the ones `compile_vale.py`
    writes), run by Vale 3.27;
  * slopvac-pos: a blocklist word (case-insensitive) whose Penn tag matches
    the rule's tag regex, over the tags `examples/tag_dump.rs` prints.

Agreement is |both| / |union| of findings keyed by (file, line, column). Two
views, as in the 3.0 POS spike (slopvac-8l4.3.10):

  all      every finding. It includes differences that come from the scope
           layer, not the tagger: Vale reads Markdown, this crate reads prose.
  aligned  only positions where both Vale's tag dump and this crate have a
           token with the same text, which blames the tagger alone.

`--mask` first blanks Markdown markup (headings, list markers, emphasis, link
targets, tables, fences, HTML tags, front matter) with spaces, character for
character, so positions stay put, and tags each heading, list item, table row
and paragraph as its own block (`Tagger::tag_blocks`), as Vale does. Without it
the crate sees raw Markdown as one block. The masking is a stand-in for the
scope layer, nothing more.

Needs: `vale` >= 3.27 and `cargo` on PATH. Run from the repository root:

  python3 crates/slopvac-pos/tools/full_corpus_agreement.py --corpus DIR [--mask]
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib
from collections import defaultdict

import gen_fixtures as g

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DEFAULT_BLOCKLIST = ROOT / "packages/slopvac-lint/examples/blocklist.toml"

# compile_vale.py PENN_TAGS: Vale matches these unanchored.
PENN_TAGS = {
    "noun": "NN|NNS|NNP|NNPS",
    "verb": "VB|VBD|VBG|VBN|VBP|VBZ",
    "adjective": "JJ|JJR|JJS",
    "adverb": "RB|RBR|RBS",
}
POSES = list(PENN_TAGS)


def mask_markdown(text: str) -> str:
    """Blank Markdown markup with spaces; every character keeps its column."""

    def blank(m: re.Match[str]) -> str:
        return re.sub(r"[^\n]", " ", m.group(0))

    text = re.sub(r"\A---\n.*?\n---\n", blank, text, flags=re.S)
    text = re.sub(
        r"^[ \t]*(```|~~~).*?^[ \t]*\1[^\n]*$", blank, text, flags=re.S | re.M
    )
    text = re.sub(r"<!--.*?-->", blank, text, flags=re.S)
    text = re.sub(r"</?[A-Za-z][^<>\n]*>", blank, text)
    text = re.sub(r"^\s*\|?[\s:|-]*-{3,}[\s:|-]*\|?\s*$", blank, text, flags=re.M)
    text = re.sub(r"^[ \t]*(?:>[ \t]*)+", blank, text, flags=re.M)
    text = re.sub(r"^[ \t]*#{1,6}[ \t]+", blank, text, flags=re.M)
    text = re.sub(r"^[ \t]*(?:[-*+]|\d+[.)])[ \t]+", blank, text, flags=re.M)
    text = re.sub(r"\]\([^)\n]*\)", blank, text)
    text = re.sub(r"\]\[[^\]\n]*\]", blank, text)
    text = re.sub(r"[\[\]|]", " ", text)
    text = re.sub(r"\*{1,3}", blank, text)
    text = re.sub(r"(?<!\w)_+|_+(?!\w)", blank, text)
    return text


BLOCK_STARTERS = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s|^\s*#{1,6}\s|^\s*\||^\s*>")


def blocks(original: str, masked: str) -> list[tuple[int, int]]:
    """Byte ranges of the blocks Vale tags separately: paragraphs, list items,
    headings and table rows. Blank (or masked-out) lines separate paragraphs."""
    out: list[tuple[int, int]] = []
    start = end = None
    offset = 0
    previous_single = False
    for orig, mask in zip(original.split("\n"), masked.split("\n")):
        width = len(mask.encode()) + 1
        if not mask.strip():
            if start is not None:
                out.append((start, end))
            start = end = None
            previous_single = False
        else:
            single = bool(re.match(r"^\s*(?:#{1,6}\s|\|)", orig))
            if start is not None and (BLOCK_STARTERS.match(orig) or previous_single):
                out.append((start, end))
                start = None
            if start is None:
                start = offset
            end = offset + len(mask.rstrip().encode())
            previous_single = single
        offset += width
    if start is not None:
        out.append((start, end))
    return out


def write_vale_config(work: pathlib.Path, blocklist: dict[str, list[str]]) -> None:
    styles = work / "styles" / "P"
    styles.mkdir(parents=True)
    (work / ".vale.ini").write_text(
        "StylesPath = styles\nMinAlertLevel = suggestion\n[*.md]\nBasedOnStyles = P\n"
    )
    for tag in g.PENN:
        (styles / f"{g.style_name(tag)}.yml").write_text(
            "extends: sequence\nmessage: '%s'\nlevel: suggestion\ntokens:\n"
            f"  - tag: '^{re.escape(tag)}$'\n"
        )
    for pos, words in blocklist.items():
        (styles / f"shipped_{pos}.yml").write_text(
            "extends: sequence\nmessage: '%s'\nlevel: error\nignorecase: true\n"
            f"tokens:\n- pattern: (?:{'|'.join(sorted(set(words)))})\n  tag: {PENN_TAGS[pos]}\n"
        )


def run_vale(work: pathlib.Path, files: list[pathlib.Path]):
    proc = subprocess.run(
        ["vale", "--config", str(work / ".vale.ini"), "--output", "JSON", *map(str, files)],
        capture_output=True, text=True, cwd=work, check=False,
    )  # fmt: skip
    if proc.returncode not in (0, 1) or not proc.stdout.strip():
        sys.exit(f"vale failed ({proc.returncode}): {proc.stderr.strip()}")
    tags: dict[tuple, tuple[str, str]] = {}
    findings: dict[str, set] = defaultdict(set)
    for path, alerts in json.loads(proc.stdout).items():
        name = pathlib.Path(path).name
        for a in alerts:
            key = (name, a["Line"], a["Span"][0])
            check = a["Check"].split(".", 1)[1]
            if check.startswith("shipped_"):
                findings[check[8:]].add(key)
            else:
                tags[key] = (a["Match"], check)
    return tags, findings


def run_tagger(files: list[pathlib.Path]) -> dict[tuple, tuple[str, str]]:
    proc = subprocess.run(
        ["cargo", "run", "-q", "-p", "slopvac-pos", "--example", "tag_dump", "--", *map(str, files)],
        capture_output=True, text=True, cwd=ROOT, check=False,
    )  # fmt: skip
    if proc.returncode != 0:
        sys.exit(f"tag_dump failed ({proc.returncode}): {proc.stderr[-2000:]}")
    out = {}
    for row in proc.stdout.split("\n"):
        if not row:
            continue
        path, line, col, word, tag = row.split("\t")
        out[(pathlib.Path(path).name, int(line), int(col))] = (word, tag)
    return out


def report(label, vale_tags, vale_findings, ours, blocklist, aligned) -> bool:
    ok = True
    if aligned:
        keep = {
            k
            for k, (w, _) in vale_tags.items()
            if k in ours and ours[k][0].lower() == w.lower()
        }
    print(f"\n{label}")
    print(
        f"{'pos':<10}{'vale':>6}{'ours':>6}{'both':>6}{'lost':>6}{'added':>6}  agreement"
    )
    for pos in POSES:
        words = {w.lower() for w in blocklist.get(pos, [])}
        family = PENN_TAGS[pos].split("|")[0][:2]
        mine = {k for k, (w, t) in ours.items() if w.lower() in words and family in t}
        theirs = set(vale_findings.get(pos, set()))
        if aligned:
            mine &= keep
            theirs &= keep
        both, union = mine & theirs, mine | theirs
        rate = len(both) / len(union) if union else 1.0
        ok &= rate >= 0.95
        print(
            f"{pos:<10}{len(theirs):>6}{len(mine):>6}{len(both):>6}{len(theirs - mine):>6}{len(mine - theirs):>6}  {rate:.1%}"
        )
        for kind, keys in (("lost", theirs - mine), ("added", mine - theirs)):
            for k in sorted(keys):
                word, tag = ours.get(k, ("-", "-"))
                vword, vtag = vale_tags.get(k, ("-", "-"))
                print(
                    f"    {pos} {kind}: {k[0]}:{k[1]}:{k[2]} crate={word}/{tag} vale={vword}/{vtag}"
                )
    return ok


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--corpus", type=pathlib.Path, required=True)
    ap.add_argument("--blocklist", type=pathlib.Path, default=DEFAULT_BLOCKLIST)
    ap.add_argument("--mask", action="store_true", help="blank Markdown markup first")
    args = ap.parse_args()

    entries = tomllib.loads(args.blocklist.read_text())["entries"]
    blocklist: dict[str, list[str]] = defaultdict(list)
    for e in entries:
        # compile_vale.vocabulary_sequence_rules: alphabetic, three letters or more.
        if e["pos"] in PENN_TAGS and e["word"].isalpha() and len(e["word"]) >= 3:
            blocklist[e["pos"]].append(e["word"].lower())

    files = sorted(args.corpus.glob("*.md"))
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp)
        write_vale_config(work / "vale", blocklist)
        vale_tags, vale_findings = run_vale(work / "vale", files)
        tagged = files
        if args.mask:
            masked_dir = work / "masked"
            masked_dir.mkdir()
            tagged = []
            for f in files:
                original = f.read_text()
                masked = mask_markdown(original)
                (masked_dir / f.name).write_text(masked)
                (masked_dir / f"{f.name}.blocks").write_text(
                    "".join(f"{a} {b}\n" for a, b in blocks(original, masked))
                )
                tagged.append(masked_dir / f.name)
        ours = run_tagger(tagged)
    mode = "masked Markdown" if args.mask else "raw Markdown"
    print(
        f"corpus: {len(files)} files; Vale tokens: {len(vale_tags)}; crate tokens: {len(ours)}; input: {mode}"
    )
    exact = sum(
        1
        for k, (w, t) in vale_tags.items()
        if k in ours
        and ours[k][0].lower() == w.lower()
        and ours[k][1]
        == t.removeprefix("t_").replace("PRPS", "PRP$").replace("WPS", "WP$")
    )
    aligned = sum(
        1
        for k, (w, _) in vale_tags.items()
        if k in ours and ours[k][0].lower() == w.lower()
    )
    print(
        f"exact Penn tag match on aligned tokens: {exact}/{aligned} = {exact / aligned:.2%}"
    )
    ok_all = report(
        "all findings (includes scope differences)",
        vale_tags,
        vale_findings,
        ours,
        blocklist,
        False,
    )
    ok_al = report(
        "aligned tokens (tagger only)", vale_tags, vale_findings, ours, blocklist, True
    )
    return 0 if ok_all and ok_al else 1


if __name__ == "__main__":
    sys.exit(main())
