"""Model-family twins: a model writes from a human outline.

The generation run is separate from this repository's tests and needs a model
provider. This module never calls one itself. You give it a command that reads
a prompt on stdin and writes the document on stdout.

    python -m evals.twins outline                          # write outlines to the cache
    python -m evals.twins generate --family anthropic \\
        --provider-cmd 'my-llm --model some-model' --model some-model
    python -m evals.twins record                           # hash twins into the manifest

Procedure
1. `outline` derives a structural outline from each human parent (headings,
   word budgets, code spans and numbers). The outline hash is pinned in the
   manifest, so a change in the procedure shows as a mismatch.
2. `generate` sends the prompt plus the outline to the provider command, once per
   pending slot of the chosen family. The provider sees no human sentence.
3. `record` hashes each generated file into the manifest and marks the slot
   `generated`. Twins stay in the cache; model-output terms vary by provider.
"""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from . import corpuslib as lib
from .textops import outline_for

PROMPT_VERSION = "1"
PROMPT = """\
Write a complete {genre} document in {format} from the outline below.
Cover every section in order and mention each listed item.
Use the length the outline asks for. Write the way you normally would.
Output only the document.

Outline:
{outline}"""


def outline_text(parent: dict[str, Any], parent_text: str) -> str:
    return outline_for(parent_text, parent["genre"], parent["format"])


def parent_text(parent: dict[str, Any], cache: Path) -> str:
    path = (
        lib.committed_file(parent)
        if parent["committed"]
        else lib.cache_file(cache, parent["id"])
    )
    return path.read_text(encoding="utf-8")


def slots(manifest: dict[str, Any], family: str | None = None) -> list[dict[str, Any]]:
    return [
        d
        for d in manifest["documents"]
        if d["origin"] == "model-twin"
        and (family is None or d["source"]["family"] == family)
    ]


def run_provider(command: str, prompt: str, timeout: int) -> str:
    done = subprocess.run(
        shlex.split(command),
        input=prompt,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    if done.returncode != 0:
        raise RuntimeError(
            f"provider exited {done.returncode}: {done.stderr.strip()[:200]}"
        )
    return done.stdout


def cmd_outline(manifest: dict[str, Any], cache: Path) -> int:
    by_id = {d["id"]: d for d in manifest["documents"]}
    bad = 0
    for slot in slots(manifest):
        parent = by_id[slot["source"]["outline_of"]]
        text = outline_text(parent, parent_text(parent, cache))
        if lib.sha256_text(text) != slot["source"]["outline_sha256"]:
            print(
                f"FAIL {slot['id']}: outline hash differs from the manifest",
                file=sys.stderr,
            )
            bad += 1
            continue
        target = cache / "outlines" / f"{parent['id']}.txt"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    print(f"outlines written to {cache / 'outlines'}")
    return 1 if bad else 0


def cmd_generate(
    manifest: dict[str, Any],
    cache: Path,
    family: str,
    provider: str,
    limit: int | None,
    timeout: int,
) -> int:
    by_id = {d["id"]: d for d in manifest["documents"]}
    todo = [s for s in slots(manifest, family) if s["status"] == "pending"]
    done = 0
    for slot in todo[: limit or len(todo)]:
        target = cache / "twins" / f"{slot['id']}.txt"
        if target.exists():
            continue
        parent = by_id[slot["source"]["outline_of"]]
        outline = outline_text(parent, parent_text(parent, cache))
        if lib.sha256_text(outline) != slot["source"]["outline_sha256"]:
            print(
                f"FAIL {slot['id']}: outline hash differs from the manifest",
                file=sys.stderr,
            )
            return 1
        prompt = PROMPT.format(
            genre=parent["genre"], format=parent["format"], outline=outline
        )
        text = run_provider(provider, prompt, timeout).strip()
        if not text:
            print(f"FAIL {slot['id']}: provider returned no text", file=sys.stderr)
            return 1
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(lib.normalise(text) + "\n", encoding="utf-8")
        done += 1
    print(f"{done} twins generated for family {family}")
    return 0


def cmd_record(
    manifest: dict[str, Any],
    cache: Path,
    model: str | None,
    today: str | None = None,
    out: Path = lib.MANIFEST_PATH,
) -> int:
    stamp = today or datetime.now(UTC).date().isoformat()
    recorded = 0
    for slot in slots(manifest):
        path = cache / "twins" / f"{slot['id']}.txt"
        if slot["status"] != "pending" or not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        slot["sha256"] = lib.sha256_text(text)
        slot["bytes"] = len(lib.normalise(text).encode())
        slot["words"] = len(text.split())
        slot["status"] = "generated"
        slot["date"] = stamp
        slot["source"]["prompt_version"] = PROMPT_VERSION
        if model:
            slot["source"]["model"] = model
        recorded += 1
    lib.dump_manifest(manifest, out)
    print(f"{recorded} twins recorded")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("action", choices=["outline", "generate", "record"])
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path(os.environ.get("SLOPVAC_EVALS_CACHE", lib.DEFAULT_CACHE)),
    )
    parser.add_argument("--family")
    parser.add_argument("--provider-cmd", help="command that reads a prompt on stdin")
    parser.add_argument("--model", help="model name to record in the manifest")
    parser.add_argument("--limit", type=int)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args(argv)
    manifest = lib.load_manifest()
    if args.action == "outline":
        return cmd_outline(manifest, args.cache)
    if args.action == "generate":
        if not (args.family and args.provider_cmd):
            parser.error("generate needs --family and --provider-cmd")
        return cmd_generate(
            manifest,
            args.cache,
            args.family,
            args.provider_cmd,
            args.limit,
            args.timeout,
        )
    return cmd_record(manifest, args.cache, args.model)


if __name__ == "__main__":
    raise SystemExit(main())
