"""Write `evals/NOTICE`, the attribution for every committed upstream document.

python -m evals.notice
"""

from __future__ import annotations

from collections import defaultdict
from typing import Any

from . import corpuslib as lib

HEADER = """\
Third-party text in evals/corpus
================================

The files under evals/corpus are excerpts of public documents, kept as test
data for the Slopvac eval corpus. Each is released by its authors under the
licence named below. The only changes are line-ending normalisation and, where
the manifest lists a `lines` range, cutting the file to that range. The
manifest (manifest.json) gives the exact upstream URL, pinned commit and
SHA-256 for every file.

The cached documents that evals.fetch downloads are not redistributed here.

"""


def render(manifest: dict[str, Any]) -> str:
    by_repo: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for doc in manifest["documents"]:
        if doc["committed"]:
            by_repo[doc["source"]["repo"]].append(doc)
    blocks = []
    for repo in sorted(by_repo):
        docs = by_repo[repo]
        info = manifest["repos"].get(repo, {})
        licences = sorted({d["license"] for d in docs})
        commits = sorted(
            {
                d["source"].get("commit", "")[:12]
                for d in docs
                if d["source"].get("commit")
            }
        )
        lines = [
            f"{repo}",
            f"  Source:    https://github.com/{repo}",
            f"  Licence:   {', '.join(licences)}",
            f"  Copyright: {info.get('copyright') or 'The ' + repo + ' authors'}",
            f"  Files:     {len(docs)} ({', '.join(sorted({d['genre'] for d in docs}))})",
        ]
        if commits:
            lines.append(f"  Commits:   {', '.join(commits)}")
        blocks.append("\n".join(lines))
    return HEADER + "\n\n".join(blocks) + "\n"


def main() -> int:
    manifest = lib.load_manifest()
    (lib.EVALS_DIR / "NOTICE").write_text(render(manifest), encoding="utf-8")
    print("NOTICE written")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
