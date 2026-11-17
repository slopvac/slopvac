"""Runner, corpus-digest and label tests for the eval harness."""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

import pytest

from evals import __main__ as cli
from evals import ab, runner, scoring

FAKE_ENGINE = """\
import json, os, sys
argv = sys.argv[1:]
paths = [a for a in argv if a.endswith((".md", ".txt", ".rs", ".rst"))]
log = os.environ["FAKE_ENGINE_LOG"]
with open(log, "a") as fh:
    fh.write(json.dumps({"argv": argv, "cwd": os.getcwd()}) + "\\n")
docs = [{"path": p, "words": 7, "findings": [
    {"rule_id": "cat.one", "line": 3, "severity": "warning", "matched_text": "t"}]}
    for p in paths]
print(json.dumps({"slopvac_version": "9.9.9", "documents": docs}))
sys.exit(1)
"""


def make_doc(doc_id, fmt, *, origin="human", committed=False):
    return {
        "id": doc_id,
        "origin": origin,
        "format": fmt,
        "committed": committed,
        "source": {},
    }


def write_cache(cache: Path, doc, folder="docs"):
    target = cache / folder / f"{doc['id']}.txt"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("text\n")


@pytest.fixture
def fake(tmp_path, monkeypatch):
    script = tmp_path / "fake.py"
    script.write_text(FAKE_ENGINE)
    log = tmp_path / "log.jsonl"
    monkeypatch.setenv("FAKE_ENGINE_LOG", str(log))

    def calls():
        return [json.loads(line) for line in log.read_text().splitlines()]

    return runner.Engine("fake", (sys.executable, str(script))), calls


def test_documents_are_staged_with_extensions_and_engine_sees_a_neutral_cwd(
    tmp_path, fake
):
    engine, calls = fake
    cache = tmp_path / "cache"
    manifest = {
        "documents": [
            make_doc("a", "markdown"),
            make_doc("b", "commit-message"),
            make_doc("c", "rust"),
        ]
    }
    for d in manifest["documents"]:
        write_cache(cache, d)
    out = runner.run_engine(engine, manifest, cache)
    assert sorted(out["documents"]) == ["a", "b", "c"]
    argv = [c["argv"] for c in calls()]
    suffixes = sorted(
        Path(p).suffix for a in argv for p in a if p.endswith((".md", ".txt", ".rs"))
    )
    assert suffixes == [".md", ".rs", ".txt"]
    code_calls = [a for a in argv if "code-comments" in a]
    assert len(code_calls) == 1
    assert code_calls[0][code_calls[0].index("--mode") + 1] == "code-comments"
    assert code_calls[0][1].endswith(".rs")
    repo = Path(runner.REPO_ROOT).resolve()
    assert all(repo not in Path(c["cwd"]).resolve().parents for c in calls())


def test_a_committed_corpus_file_is_staged_under_its_real_extension(
    tmp_path, fake, monkeypatch
):
    engine, calls = fake
    source = tmp_path / "x.corpus"
    source.write_text("text\n")
    monkeypatch.setattr(runner.lib, "committed_file", lambda doc: source)
    manifest = {"documents": [make_doc("a", "markdown", committed=True)]}
    runner.run_engine(engine, manifest, tmp_path)
    assert calls()[0]["argv"][1].endswith(".md")


def test_formats_the_engine_cannot_read_are_recorded_as_skipped(tmp_path, fake):
    engine = runner.Engine("fake", fake[0].command, unsupported=frozenset({"drawio"}))
    cache = tmp_path / "cache"
    manifest = {"documents": [make_doc("a", "markdown"), make_doc("d", "drawio")]}
    write_cache(cache, manifest["documents"][0])
    out = runner.run_engine(engine, manifest, cache)
    assert list(out["documents"]) == ["a"]
    assert "drawio" in out["skipped"]["d"]


def test_generated_twins_are_read_from_the_twins_folder(tmp_path, fake):
    engine, _ = fake
    twin = make_doc("t", "markdown", origin="model-twin")
    twin["status"] = "generated"
    write_cache(tmp_path, twin, folder="twins")
    out = runner.run_engine(engine, {"documents": [twin]}, tmp_path)
    assert list(out["documents"]) == ["t"]


@pytest.mark.parametrize(
    ("doc", "hint"),
    [
        ({"id": "d", "origin": "human", "committed": False}, "evals.fetch"),
        ({"id": "t", "origin": "model-twin", "committed": False}, "evals.twins"),
    ],
)
def test_missing_text_names_the_remedy(tmp_path, doc, hint):
    with pytest.raises(runner.MissingText, match=hint):
        runner.text_path(doc, tmp_path)


def test_pending_twins_are_not_linted():
    assert not runner.default_selector({"origin": "model-twin", "status": "pending"})
    assert runner.default_selector({"origin": "model-twin", "status": "generated"})


def test_engine_exit_two_is_an_error(tmp_path):
    script = tmp_path / "bad.py"
    script.write_text("import sys; sys.exit(2)")
    engine = runner.Engine("bad", (sys.executable, str(script)))
    with pytest.raises(runner.EngineError, match="exited 2"):
        engine.lint([tmp_path / "a.md"])


def test_a_report_with_paths_relative_to_the_engine_directory_is_matched(
    tmp_path, monkeypatch
):
    script = tmp_path / "rel.py"
    script.write_text(
        "import json, os, sys\n"
        "docs = [{'path': os.path.relpath(p), 'words': 4, 'findings': []}\n"
        "        for p in sys.argv[1:] if p.endswith('.md')]\n"
        "print(json.dumps({'documents': docs}))\n"
    )
    engine = runner.Engine("rel", (sys.executable, str(script)))
    cache = tmp_path / "cache"
    d = make_doc("a", "markdown")
    write_cache(cache, d)
    out = runner.run_engine(engine, {"documents": [d]}, cache)
    assert out["documents"]["a"]["words"] == 4


def test_run_skip_format_skips_that_format_and_still_runs_the_other_documents(
    tmp_path, fake, monkeypatch
):
    engine, calls = fake
    cache = tmp_path / "cache"
    manifest = {
        "documents": [make_doc("a", "markdown"), make_doc("b", "commit-message")]
    }
    for d in manifest["documents"]:
        write_cache(cache, d)
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    out = tmp_path / "out.json"
    code = cli.main(
        [
            "run",
            "--engine",
            " ".join(engine.command),
            "--cache",
            str(cache),
            "--skip-format",
            "markdown",
            "--out",
            str(out),
        ]
    )
    assert code == 0
    results = json.loads(out.read_text())
    assert sorted(results["documents"]) == ["b"]
    assert list(results["skipped"]) == ["a"]
    linted = [p for c in calls() for p in c["argv"] if p.endswith((".md", ".txt"))]
    assert [Path(p).suffix for p in linted] == [".txt"]


def test_an_incomplete_run_with_a_report_keeps_its_findings(tmp_path):
    script = tmp_path / "inc.py"
    script.write_text(
        "import json, sys\n"
        "print(json.dumps({'documents': [{'path': sys.argv[2], 'words': 3,"
        " 'findings': [], 'unchecked': ['a', 'b']}]}))\n"
        "sys.exit(2)\n"
    )
    engine = runner.Engine("inc", (sys.executable, str(script)))
    cache = tmp_path / "cache"
    d = make_doc("a", "markdown")
    write_cache(cache, d)
    out = runner.run_engine(engine, {"documents": [d]}, cache)
    assert out["documents"]["a"]["unchecked"] == 2
    manifest = {"documents": [d]}
    assert scoring.score(manifest, out, resamples=5)["incomplete_documents"] == 1


@pytest.mark.skipif(shutil.which("vale") is None, reason="needs vale on PATH")
def test_the_python_reference_lints_staged_corpus_files(tmp_path, monkeypatch):
    committed = tmp_path / "x.corpus"
    committed.write_text("We delve into the topic.\n")
    monkeypatch.setattr(runner.lib, "committed_file", lambda doc: committed)
    comment = tmp_path / "c.txt"
    comment.write_text("// We delve into the topic.\nfn main() {}\n")
    manifest = {
        "documents": [
            make_doc("prose", "markdown", committed=True),
            {**make_doc("code", "rust"), "committed": False},
        ]
    }
    cache = tmp_path / "cache"
    (cache / "docs").mkdir(parents=True)
    (cache / "docs" / "code.txt").write_text(comment.read_text())
    out = runner.run_engine(runner.Engine.parse("python"), manifest, cache)
    assert {f["rule"] for f in out["documents"]["prose"]["findings"]} >= {
        "prose-inflation.slop-lexicon"
    }
    assert "code" in out["documents"]


def labelled_manifest():
    return {
        "documents": [
            {**make_doc("h1", "markdown"), "source": {}},
            {**make_doc("h2", "markdown"), "source": {}},
        ]
    }


def results_for(manifest, documents):
    return {
        "schema": 1,
        "engine": {"name": "t"},
        "corpus": {"manifest_sha256": runner.manifest_digest(manifest)},
        "documents": documents,
    }


R = "cat.rule"


def f(line):
    return {"rule": R, "line": line}


def test_adjudicated_positives_the_engine_misses_lower_recall_and_precision_uses_labels():
    manifest = labelled_manifest()
    results = results_for(
        manifest,
        {
            "h1": {"words": 10, "findings": [f(3), f(8)]},
            "h2": {"words": 10, "findings": [f(2)]},
        },
    )
    # Labelled true positives: h1 line 3 (found), h1 line 20 (missed), h2 line 2 (found).
    # Labelled false positive: h1 line 8 (found). Hand counts:
    #   recall = 2 of 3; precision = 2 of 3 (tp lines 3 and 2, fp line 8).
    labels = {
        ("h1", R, 3): "tp",
        ("h1", R, 20): "tp",
        ("h2", R, 2): "tp",
        ("h1", R, 8): "fp",
    }
    card = scoring.score(manifest, results, labels=labels, resamples=20)["rules"][R]
    assert (card["adjudicated_recall"]["hits"], card["adjudicated_recall"]["n"]) == (
        2,
        3,
    )
    assert (card["recall"]["hits"], card["recall"]["n"]) == (2, 3)
    assert (card["precision"]["tp"], card["precision"]["n"]) == (2, 3)


def test_score_command_reads_labels_from_a_file(tmp_path, monkeypatch):
    manifest = labelled_manifest()
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    results = tmp_path / "r.json"
    results.write_text(
        json.dumps(
            results_for(
                manifest,
                {
                    "h1": {"words": 5, "findings": [f(3), f(8)]},
                    "h2": {"words": 5, "findings": []},
                },
            )
        )
    )
    labels = tmp_path / "labels.jsonl"
    labels.write_text(
        "\n".join(
            json.dumps({"doc": "h1", "rule": R, "line": line, "label": label})
            for line, label in ((3, "tp"), (8, "fp"))
        )
        + "\n"
    )
    out = tmp_path / "card.json"
    code = cli.main(
        [
            "score",
            str(results),
            "--labels",
            str(labels),
            "--resamples",
            "10",
            "--out",
            str(out),
        ]
    )
    assert code == 0
    assert json.loads(out.read_text())["rules"][R]["precision"]["rate"] == 0.5


def test_score_rejects_results_from_another_manifest(tmp_path, monkeypatch):
    manifest = labelled_manifest()
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    stale = results_for({"documents": []}, {})
    path = tmp_path / "r.json"
    path.write_text(json.dumps(stale))
    assert cli.main(["score", str(path)]) == 2


def test_ab_rejects_arms_that_cover_different_documents():
    manifest = labelled_manifest()
    a = results_for(
        manifest,
        {"h1": {"words": 1, "findings": []}, "h2": {"words": 1, "findings": []}},
    )
    b = results_for(manifest, {"h1": {"words": 1, "findings": []}})
    with pytest.raises(runner.EngineError, match="different documents"):
        ab.ab(manifest, a, b, resamples=10)


def test_ab_lists_rules_that_only_appear_in_findings():
    manifest = labelled_manifest()
    quiet = {"h1": {"words": 100, "findings": []}, "h2": {"words": 100, "findings": []}}
    noisy = {
        "h1": {"words": 100, "findings": [f(1)] * 6},
        "h2": {"words": 100, "findings": [f(1)] * 6},
    }
    report = ab.ab(
        manifest,
        results_for(manifest, quiet),
        results_for(manifest, noisy),
        resamples=50,
    )
    assert R in report["rules"]
    assert report["rules"][R]["human_per_100_words"]["point"] == 6.0
    assert f"{R}:human_per_100_words" in ab.regressions(report)


def test_a_relative_engine_path_is_resolved_before_the_engine_changes_directory(
    tmp_path, monkeypatch
):
    script = tmp_path / "bin" / "engine.py"
    script.parent.mkdir()
    script.write_text(FAKE_ENGINE)
    script.chmod(0o755)
    monkeypatch.chdir(tmp_path)
    engine = runner.Engine.parse("./bin/engine.py")
    assert engine.command[0] == str(script.resolve())
    assert runner.Engine.parse("slopvac").command[0] == "slopvac"


def test_precision_interval_resamples_documents_not_findings():
    manifest = labelled_manifest()
    findings = [f(n) for n in range(1, 11)]
    results = results_for(
        manifest,
        {
            "h1": {"words": 10, "findings": findings},
            "h2": {"words": 10, "findings": findings},
        },
    )
    # h1: all 10 findings true, h2: all 10 false. Pooled precision is 10/20, but the
    # two documents are perfectly opposed, so resampling documents gives 0, 0.5 or 1.
    labels = {("h1", R, n): "tp" for n in range(1, 11)} | {
        ("h2", R, n): "fp" for n in range(1, 11)
    }
    card = scoring.score(manifest, results, labels=labels, resamples=400)["rules"][R]
    precision = card["precision"]
    assert (precision["tp"], precision["n"], precision["documents"]) == (10, 20, 2)
    assert precision["bootstrap"] == [0.0, 1.0]
    assert (
        precision["wilson"][0] > 0.27
    )  # a pooled interval alone would claim far more certainty
