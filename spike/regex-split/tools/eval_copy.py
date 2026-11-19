"""SPIKE: copy the eval corpus with real extensions.

The eval files are named `<name>-<ext>.<hash>.corpus`, which `slopvac lint` skips. This copies
each to `<name>-<ext>.<hash>.<ext>` (unknown ext -> md) and prints the extension counts.
Usage: python3 eval_copy.py <evals/corpus> <dst>
"""
import collections
import pathlib
import shutil
import sys

src, dst = map(pathlib.Path, sys.argv[1:])
known = {"md", "mdx", "rst", "txt", "py", "js", "ts", "tsx", "jsx", "rs", "go", "html", "htm",
         "adoc", "yaml", "yml", "toml", "sh", "java", "c", "h", "cpp", "rb", "mermaid", "mmd",
         "puml", "dot", "d2", "json", "svg", "kt", "swift", "cs", "php", "tex"}
c = collections.Counter()
for f in sorted(src.rglob("*.corpus")):
    stem = f.name[: -len(".corpus")]
    base = stem.rsplit(".", 1)[0]
    ext = base.rsplit("-", 1)[-1].lower()
    if ext not in known:
        ext = "md"
    c[ext] += 1
    out = dst / f.relative_to(src).parent / f"{stem}.{ext}"
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(f, out)
for k, v in c.most_common():
    print(k, v)
