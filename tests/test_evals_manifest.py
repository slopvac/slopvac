"""Offline checks on the eval corpus manifest and its committed files.

No network: upstream downloads are verified by `python -m evals.fetch`.
"""

from __future__ import annotations

import copy
import json
from collections import Counter

import pytest

from evals import corpuslib as lib
from evals import build_derived, twins

MANIFEST = lib.load_manifest()
DOCS = MANIFEST["documents"]
COMMITTED = [d for d in DOCS if d["committed"]]
REQUIRED_GENRES = {
    "product", "reference", "tutorials", "changelogs", "readmes", "adrs",
    "prs", "commits", "ui-strings", "code-comments", "diagrams",
}  # fmt: skip


def test_manifest_matches_schema():
    assert lib.validate_manifest(MANIFEST) == []


def test_every_genre_and_origin_class_is_present():
    human_genres = {d["genre"] for d in DOCS if d["origin"] == "human"}
    assert REQUIRED_GENRES <= human_genres
    assert {d["origin"] for d in DOCS} == set(lib.ORIGINS)


def test_committed_files_match_their_hashes():
    bad = []
    for doc in COMMITTED:
        path = lib.committed_file(doc)
        if (
            not path.is_file()
            or lib.sha256_text(path.read_text("utf-8")) != doc["sha256"]
        ):
            bad.append(doc["id"])
    assert bad == []


def test_no_unlisted_files_under_corpus():
    listed = {lib.committed_file(d) for d in COMMITTED}
    on_disk = set((lib.EVALS_DIR / "corpus").rglob("*.txt"))
    assert on_disk == listed


def test_only_permissive_licences_are_committed():
    assert {d["license"] for d in COMMITTED} <= lib.PERMISSIVE
    # Non-permissive upstream documents exist and stay in the cache.
    cache_only = [d for d in DOCS if d["origin"] == "human" and not d["committed"]]
    assert cache_only


def test_every_committed_repo_is_credited_in_notice():
    notice = (lib.EVALS_DIR / "NOTICE").read_text("utf-8")
    repos = {d["source"]["repo"] for d in COMMITTED}
    assert [r for r in sorted(repos) if f"https://github.com/{r}" not in notice] == []


def test_human_documents_predate_the_cutoff():
    late = [
        d["id"]
        for d in DOCS
        if d["origin"] == "human" and d["date"] >= lib.HUMAN_CUTOFF
    ]
    assert late == []


def _mutated(**changes):
    doc = copy.deepcopy(next(d for d in DOCS if d["origin"] == "human"))
    doc.update(changes)
    manifest = {"schema": 1, "repos": MANIFEST["repos"], "documents": [doc]}
    return lib.validate_manifest(manifest)


@pytest.mark.parametrize(
    ("changes", "fragment"),
    [
        ({"date": "2023-01-02"}, "predate"),
        ({"sha256": "abc"}, "sha256"),
        ({"genre": "poetry"}, "unknown genre"),
        ({"origin": "oracle"}, "unknown origin"),
        (
            {"committed": True, "license": "GPL-3.0-only", "path": "corpus/x.txt"},
            "may not be committed",
        ),
    ],
)
def test_schema_rejects_bad_records(changes, fragment):
    assert any(fragment in e for e in _mutated(**changes))


def test_seeded_defects_have_minimal_pair_controls():
    seeded = {d["id"]: d for d in DOCS if d["origin"] == "seeded-defect"}
    controls = [d for d in DOCS if d["origin"] == "clean-control"]
    assert seeded and len(controls) == len(seeded)
    for control in controls:
        defect = seeded[control["source"]["pair"]]
        assert [c["rule"] for c in control["source"]["controls"]] == [
            d["rule"] for d in defect["source"]["defects"]
        ]


def test_derived_documents_regenerate_identically_from_committed_parents():
    bank = json.loads(build_derived.BANK_PATH.read_text("utf-8"))
    assert MANIFEST["derived"]["bank_sha256"] == build_derived.bank_digest(bank)
    outputs, slots = build_derived.generate_all(
        MANIFEST, bank, build_derived.committed_reader
    )
    expected = {d["id"]: (d["sha256"], d["source"]) for d, _ in outputs}
    actual = {
        d["id"]: (d["sha256"], d["source"])
        for d in DOCS
        if d["origin"] in ("seeded-defect", "clean-control", "rewrite-pair")
    }
    assert actual == expected
    assert {s["id"] for s in slots} == {
        d["id"] for d in DOCS if d["origin"] == "model-twin"
    }


def test_seeded_defect_lines_hold_the_inserted_text():
    outputs, _ = build_derived.generate_all(
        MANIFEST,
        json.loads(build_derived.BANK_PATH.read_text("utf-8")),
        build_derived.committed_reader,
    )
    for doc, text in outputs:
        if doc["origin"] in ("seeded-defect", "clean-control"):
            key = "defects" if doc["origin"] == "seeded-defect" else "controls"
            lines = text.split("\n")
            for item in doc["source"][key]:
                assert lines[item["line"] - 1] == item["text"]


def test_rewrite_pairs_differ_from_their_parent_and_record_edits():
    pairs = [d for d in DOCS if d["origin"] == "rewrite-pair"]
    assert pairs
    by_id = {d["id"]: d for d in DOCS}
    for pair in pairs:
        parent = by_id[pair["source"]["parents"][0]]
        assert pair["sha256"] != parent["sha256"]
        assert pair["source"]["original_sha256"] == parent["sha256"]
        assert len(pair["source"]["edits"]) >= 2


def test_twin_outlines_match_pinned_hashes_and_are_pending():
    by_id = {d["id"]: d for d in DOCS}
    slots = twins.slots(MANIFEST)
    assert slots
    assert Counter(s["source"]["family"] for s in slots).keys() == set(
        MANIFEST["derived"]["twin_families"]
    )
    for slot in slots:
        parent = by_id[slot["source"]["outline_of"]]
        text = lib.committed_file(parent).read_text("utf-8")
        assert (
            lib.sha256_text(twins.outline_text(parent, text))
            == slot["source"]["outline_sha256"]
        )
        assert slot["status"] == "pending" and slot["sha256"] is None


def test_outline_leaks_no_sentence_from_the_parent():
    parent = next(
        d
        for d in DOCS
        if d["id"] in {s["source"]["outline_of"] for s in twins.slots(MANIFEST)}
    )
    text = lib.committed_file(parent).read_text("utf-8")
    outline = twins.outline_text(parent, text)
    from evals.textops import prose_paragraphs, split_sentences

    lines = text.split("\n")
    for first, last in prose_paragraphs(text):
        for sentence in split_sentences("\n".join(lines[first : last + 1])):
            if len(sentence.split()) >= 8:
                assert sentence not in outline
