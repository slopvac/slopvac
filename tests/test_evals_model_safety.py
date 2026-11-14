"""Offline tests for the model-run safety rails: budget, journal, isolation, timeouts.

No model and no network. Fake commands are small Python scripts.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import threading

import pytest

from evals import __main__ as cli
from evals import models, opencode_model


def script(tmp_path, name, body):
    path = tmp_path / name
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return path


COUNTING = """\
import fcntl, os, sys
with open(os.environ["SPAWN_LOG"], "a") as fh:
    fcntl.flock(fh, fcntl.LOCK_EX)
    fh.write("spawn\\n")
print("answer " + sys.stdin.read()[:12])
"""


def spawns(tmp_path):
    log = tmp_path / "spawns.log"
    return len(log.read_text().splitlines()) if log.exists() else 0


@pytest.fixture
def spawn_log(tmp_path, monkeypatch):
    monkeypatch.setenv("SPAWN_LOG", str(tmp_path / "spawns.log"))


def jobs_roster(tmp_path, command):
    return [{"name": "m", "family": "f", "tier": "t", "command": command}]


def author_tasks(n):
    return [
        {
            "id": f"t{i}",
            "type": "update",
            "role": "author",
            "doc": "d",
            "genre": "g",
            "prompt": f"Update the passage {i}",
            "source": "Cache 40 items.",
            "expected": [],
        }
        for i in range(n)
    ]


NO_FINDINGS = lambda text: []


def test_the_ceiling_stops_spawning_before_it_is_passed(tmp_path, spawn_log):
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', COUNTING)}"
    budget = models.Budget(ceiling=5)
    journal = models.Journal(tmp_path / "j.jsonl")
    with pytest.raises(models.BudgetExceeded):
        models.execute(
            author_tasks(20),
            jobs_roster(tmp_path, cmd),
            3,
            NO_FINDINGS,
            workers=8,
            budget=budget,
            journal=journal,
        )
    assert spawns(tmp_path) == 5
    assert budget.spent == 5


def test_every_retry_counts_against_the_budget(tmp_path):
    cmd = f"{sys.executable} -c 'import sys; sys.exit(1)'"
    budget = models.Budget(ceiling=10)
    with pytest.raises(models.CallFailed) as caught:
        models.command_ask(cmd, attempts=3, budget=budget)("x")
    assert caught.value.attempts == 3 and budget.spent == 3


def test_a_stopped_run_resumes_from_the_journal_without_repeating_calls(
    tmp_path, spawn_log
):
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', COUNTING)}"
    tasks, roster = author_tasks(4), jobs_roster(tmp_path, cmd)
    path = tmp_path / "j.jsonl"
    journal = models.Journal(path)
    with pytest.raises(models.BudgetExceeded):
        models.execute(
            tasks, roster, 3, NO_FINDINGS, budget=models.Budget(7), journal=journal
        )
    assert spawns(tmp_path) == 7

    resumed = models.Journal(path)
    assert resumed.attempts == 7
    budget = models.Budget(ceiling=12, spent=resumed.attempts)
    out = models.execute(tasks, roster, 3, NO_FINDINGS, budget=budget, journal=resumed)
    assert spawns(tmp_path) == 12  # 4 tasks x 3 repeats, each asked once
    assert len(out["runs"]) == 12 and out["failed_calls"] == 0
    assert out["invocations"] == 12


def test_job_ids_are_stable_and_distinguish_model_prompt_and_repeat():
    base = models.job_id("task", "m", "prompt", 0)
    assert base == models.job_id("task", "m", "prompt", 0)
    assert (
        len(
            {
                base,
                models.job_id("task", "n", "prompt", 0),
                models.job_id("task", "m", "other", 0),
                models.job_id("task", "m", "prompt", 1),
                models.job_id("review", "m", "prompt", 0),
            }
        )
        == 5
    )


def test_failed_jobs_are_asked_again_on_resume_and_keep_their_attempts(tmp_path):
    journal = models.Journal(tmp_path / "j.jsonl")
    journal.record("a", "", "boom", 3)
    journal.record("b", "text", None, 1)
    again = models.Journal(tmp_path / "j.jsonl")
    assert again.done("a") is None and again.done("b")["answer"] == "text"
    assert again.attempts == 4


def test_journal_lines_survive_concurrent_writers(tmp_path):
    journal = models.Journal(tmp_path / "j.jsonl")
    threads = [
        threading.Thread(target=journal.record, args=(f"j{i}", "x" * 5000, None, 1))
        for i in range(40)
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert len(models.Journal(tmp_path / "j.jsonl")._entries) == 40


def test_a_hung_command_times_out_and_counts_its_attempts(tmp_path):
    hang = script(tmp_path, "hang.py", "import time\ntime.sleep(30)\n")
    budget = models.Budget(ceiling=5)
    ask = models.command_ask(
        f"{sys.executable} {hang}", attempts=2, timeout=1, budget=budget
    )
    with pytest.raises(models.CallFailed, match="timed out") as caught:
        ask("x")
    assert caught.value.attempts == 2 and budget.spent == 2


# --- opencode_model isolation ------------------------------------------------------------

FAKE_OPENCODE = """\
#!{python}
import json, os, sys, time
cfg = open("opencode.json").read() if os.path.exists("opencode.json") else None
if os.environ.get("FAKE_HANG"):
    time.sleep(30)
open(os.environ["FAKE_OPENCODE_LOG"], "w").write(json.dumps({{
    "argv": sys.argv[1:], "cwd": os.getcwd(), "pwd": os.environ.get("PWD"), "config": cfg,
    "stdin": sys.stdin.read(),
}}))
print(json.dumps({{"type": "text", "part": {{"text": "fine"}}}}))
"""


@pytest.fixture
def fake_opencode(tmp_path, monkeypatch):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    script(bin_dir, "opencode", FAKE_OPENCODE.format(python=sys.executable))
    log = tmp_path / "opencode.json.log"
    monkeypatch.setenv("PATH", f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setenv("FAKE_OPENCODE_LOG", str(log))
    return log


def test_the_model_runs_in_a_toolless_temp_project_with_a_matching_pwd(
    fake_opencode, monkeypatch
):
    monkeypatch.setenv(
        "PWD", os.getcwd()
    )  # the caller's own directory, as in a real run
    assert opencode_model.call("amazon-bedrock/some-model", "hello") == "fine"
    seen = json.loads(fake_opencode.read_text())
    assert os.path.realpath(seen["cwd"]) == os.path.realpath(seen["pwd"])
    assert os.path.realpath(seen["cwd"]) != os.path.realpath(os.getcwd())
    assert seen["argv"][:3] == ["run", "-m", "amazon-bedrock/some-model"]
    assert seen["argv"][seen["argv"].index("--agent") + 1] == opencode_model.AGENT
    config = json.loads(seen["config"])
    assert config["agent"][opencode_model.AGENT]["tools"] == {"*": False}
    assert os.getcwd() not in seen["config"]
    assert seen["stdin"] == "hello"


def test_the_caller_directory_is_not_inherited_through_the_environment(
    fake_opencode, monkeypatch
):
    monkeypatch.setenv("PWD", "/definitely/the/repo")
    opencode_model.call("m", "x")
    assert json.loads(fake_opencode.read_text())["pwd"] != "/definitely/the/repo"


def test_a_hung_opencode_is_killed_at_the_timeout(fake_opencode, monkeypatch):
    monkeypatch.setenv("FAKE_HANG", "1")
    with pytest.raises(subprocess.TimeoutExpired):
        opencode_model.call("m", "x", timeout=1)


# --- token boundaries ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("fact", "text", "expected"),
    [
        ("3", "retries 3 is gone", True),
        ("3", "lost: 3.", True),
        ("3", "the 13 limit", False),
        ("3", "limit 3.5", False),
        ("3", "limit 3,500", False),
        ("3", "range 1,3", False),
        ("--offline", "the --offline flag", True),
        ("--offline", "the --offline-mode flag", False),
        ("--offline", "the ---offline flag", False),
        ("parse_config", "call parse_config now", True),
        ("parse_config", "call parse_config2 now", False),
        ("a.b", "see a.b.c", False),
    ],
)
def test_facts_match_only_as_complete_tokens(fact, text, expected):
    assert models._mentions(fact, text) is expected


# --- the command line --------------------------------------------------------------------------------


def test_run_forwards_workers_and_the_ceiling_and_journals_next_to_the_output(
    tmp_path, monkeypatch
):
    models_file = tmp_path / "models.json"
    models_file.write_text(json.dumps({"models": jobs_roster(tmp_path, "true")}))
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: {"documents": []})
    seen = {}

    def fake_execute(tasks, roster, repeats, lint, **kwargs):
        seen.update(kwargs)
        return {"date": "d", "runs": []}

    monkeypatch.setattr(cli.models, "execute", fake_execute)
    out = tmp_path / "r.json"
    code = cli.main(
        [
            "models",
            "run",
            "--models",
            str(models_file),
            "--cache",
            str(tmp_path),
            "--confirm-calls",
            "0",
            "--workers",
            "7",
            "--max-invocations",
            "9",
            "--out",
            str(out),
        ]
    )
    assert code == 0
    assert seen["workers"] == 7 and seen["budget"].ceiling == 9
    assert seen["journal"].path == tmp_path / "r.journal.jsonl"


def test_run_requires_an_explicit_ceiling(tmp_path):
    with pytest.raises(SystemExit):
        cli.main(["models", "run", "--models", "m.json", "--confirm-calls", "0"])


def test_a_ceiling_hit_exits_two_and_keeps_the_journal(tmp_path, monkeypatch, capsys):
    models_file = tmp_path / "models.json"
    models_file.write_text(json.dumps({"models": jobs_roster(tmp_path, "true")}))
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: {"documents": []})

    def stop(*args, **kwargs):
        raise models.BudgetExceeded(
            "invocation ceiling of 1 reached; nothing more was started"
        )

    monkeypatch.setattr(cli.models, "execute", stop)
    code = cli.main(
        [
            "models",
            "run",
            "--models",
            str(models_file),
            "--cache",
            str(tmp_path),
            "--confirm-calls",
            "0",
            "--max-invocations",
            "1",
            "--out",
            str(tmp_path / "r.json"),
        ]
    )
    assert code == 2 and "keeps the finished calls" in capsys.readouterr().err
