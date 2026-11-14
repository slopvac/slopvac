"""Offline tests for the model-run safety rails: budget ledger, journal, process
groups, environment isolation and redaction.

No model and no network. Fake commands are small Python scripts.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import textwrap
import threading
import time
from pathlib import Path

import pytest

from evals import __main__ as cli
from evals import models, opencode_model, procs


def script(directory, name, body):
    path = Path(directory) / name
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


def roster(command):
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


def no_findings(text):
    return []


# --- budget ledger ---------------------------------------------------------------------


def test_the_ceiling_stops_spawning_before_it_is_passed(tmp_path, spawn_log):
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', COUNTING)}"
    with models.Budget(5, tmp_path / "l.jsonl") as budget:
        with pytest.raises(models.BudgetExceeded):
            models.execute(
                author_tasks(20),
                roster(cmd),
                3,
                no_findings,
                workers=8,
                budget=budget,
                journal=models.Journal(tmp_path / "j.jsonl"),
            )
        assert budget.spent == 5
    assert spawns(tmp_path) == 5


def test_every_retry_is_reserved_and_exhaustion_mid_retry_stops_the_call(tmp_path):
    cmd = f"{sys.executable} -c 'import sys; sys.exit(1)'"
    with models.Budget(2, tmp_path / "l.jsonl") as budget:
        with pytest.raises(models.BudgetExceeded):
            models.command_ask(cmd, attempts=3, budget=budget)("x")
        assert budget.spent == 2  # the third attempt never started
    events = [
        json.loads(line) for line in (tmp_path / "l.jsonl").read_text().splitlines()
    ]
    assert [e["event"] for e in events] == ["reserve", "finish", "reserve", "finish"]


def test_a_reservation_is_on_disk_before_the_process_starts(tmp_path):
    ledger = tmp_path / "l.jsonl"
    seen = {}
    marker = script(
        tmp_path,
        "peek.py",
        "import os\nprint(open(os.environ['LEDGER']).read().count('reserve'))\n",
    )
    os.environ["LEDGER"] = str(ledger)
    try:
        with models.Budget(5, ledger) as budget:
            reply = models.command_ask(f"{sys.executable} {marker}", budget=budget)("x")
            seen["reserved_when_spawned"] = int(reply.text)
    finally:
        del os.environ["LEDGER"]
    assert seen["reserved_when_spawned"] == 1


CRASH = """\
import os, sys
from evals import models
budget = models.Budget(10, sys.argv[1])
budget.reserve()
budget.reserve()
os._exit(9)  # killed between the reservation and the spawn
"""


def test_a_crash_between_reserve_and_finish_still_counts_as_spent(tmp_path):
    ledger = tmp_path / "l.jsonl"
    repo = Path(models.lib.EVALS_DIR).parent
    crashed = subprocess.run(
        [sys.executable, "-c", CRASH, str(ledger)],
        cwd=repo,
        env={**os.environ, "PYTHONPATH": str(repo)},
        check=False,
    )
    assert crashed.returncode == 9
    with models.Budget(10, ledger) as resumed:
        assert resumed.spent == 2 and resumed.unfinished_at_start == 2
        resumed.reserve()
        assert resumed.spent == 3


def test_a_second_live_run_on_the_same_ledger_refuses_to_start(tmp_path):
    ledger = tmp_path / "l.jsonl"
    with (
        models.Budget(5, ledger),
        pytest.raises(models.ModelEvalError, match="holds the ledger lock"),
    ):
        models.Budget(5, ledger)
    with models.Budget(5, ledger):  # the lock is released on close
        pass


def test_concurrent_launches_cannot_each_spend_the_ceiling(tmp_path, spawn_log):
    ledger = tmp_path / "l.jsonl"
    repo = Path(models.lib.EVALS_DIR).parent
    code = textwrap.dedent(
        """\
        import sys, time
        from evals import models
        try:
            budget = models.Budget(3, sys.argv[1])
        except models.ModelEvalError:
            print("refused"); sys.exit(0)
        for _ in range(3):
            budget.reserve()
        time.sleep(1.5)
        print("ran")
        """
    )
    env = {**os.environ, "PYTHONPATH": str(repo)}
    procs_ = [
        subprocess.Popen(
            [sys.executable, "-c", code, str(ledger)],
            cwd=repo,
            env=env,
            stdout=subprocess.PIPE,
            text=True,
        )
        for _ in range(4)
    ]
    outputs = sorted(p.communicate()[0].strip() for p in procs_)
    assert outputs == ["ran", "refused", "refused", "refused"]
    assert ledger.read_text().count("reserve") == 3


def test_resume_counts_earlier_reservations_against_the_ceiling(tmp_path, spawn_log):
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', COUNTING)}"
    tasks, models_ = author_tasks(4), roster(cmd)
    ledger, journal_path = tmp_path / "l.jsonl", tmp_path / "j.jsonl"
    with models.Budget(7, ledger) as budget, pytest.raises(models.BudgetExceeded):
        models.execute(
            tasks,
            models_,
            3,
            no_findings,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    assert spawns(tmp_path) == 7

    with models.Budget(12, ledger) as budget:
        assert budget.spent == 7
        out = models.execute(
            tasks,
            models_,
            3,
            no_findings,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    assert spawns(tmp_path) == 12  # 4 tasks x 3 repeats, each asked once
    assert len(out["runs"]) == 12 and out["failed_calls"] == 0
    assert out["invocations"] == 12


# --- journal ---------------------------------------------------------------------------------


def test_job_ids_are_stable_and_distinguish_model_prompt_and_repeat():
    base = models.job_id("task", "m", "prompt", 0)
    assert base == models.job_id("task", "m", "prompt", 0)
    others = {
        models.job_id("task", "n", "prompt", 0),
        models.job_id("task", "m", "other", 0),
        models.job_id("task", "m", "prompt", 1),
        models.job_id("review", "m", "prompt", 0),
    }
    assert base not in others and len(others) == 4


def test_failed_jobs_are_asked_again_on_resume(tmp_path):
    journal = models.Journal(tmp_path / "j.jsonl")
    journal.record("a", "", "boom", 3)
    journal.record("b", "text", None, 1)
    again = models.Journal(tmp_path / "j.jsonl")
    assert again.done("a") is None and again.done("b")["answer"] == "text"


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


# --- process groups and timeouts ------------------------------------------------------------------

SPAWNS_CHILD = """\
import os, subprocess, sys, time
child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
open(sys.argv[1], "w").write(str(child.pid))
time.sleep(60)
"""


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def wait_dead(pid, seconds=5):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return False


def test_a_timeout_kills_the_wrapper_and_the_child_it_spawned(tmp_path):
    pidfile = tmp_path / "child.pid"
    wrapper = script(tmp_path, "wrap.py", SPAWNS_CHILD)
    with pytest.raises(subprocess.TimeoutExpired):
        procs.run_group([sys.executable, str(wrapper), str(pidfile)], timeout=1.5)
    assert wait_dead(int(pidfile.read_text()))


def test_the_harness_timeout_reaps_the_whole_tree_and_counts_the_attempts(tmp_path):
    pidfile = tmp_path / "child.pid"
    wrapper = script(tmp_path, "wrap.py", SPAWNS_CHILD)
    with models.Budget(5, tmp_path / "l.jsonl") as budget:
        ask = models.command_ask(
            f"{sys.executable} {wrapper} {pidfile}",
            attempts=2,
            timeout=1.5,
            budget=budget,
        )
        with pytest.raises(models.CallFailed, match="timed out") as caught:
            ask("x")
        assert caught.value.attempts == 2 and budget.spent == 2
    assert wait_dead(int(pidfile.read_text()))


def test_a_child_left_running_after_the_command_exits_is_reaped(tmp_path):
    pidfile = tmp_path / "child.pid"
    body = (
        "import subprocess, sys\n"
        "c = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
        "open(sys.argv[1], 'w').write(str(c.pid))\n"
        "print('done')\n"
    )
    wrapper = script(tmp_path, "leave.py", body)
    result = procs.run_group([sys.executable, str(wrapper), str(pidfile)], timeout=10)
    assert result.stdout.strip() == "done"
    assert wait_dead(int(pidfile.read_text()))


def test_the_adapter_timeout_is_strictly_below_the_harness_timeout():
    assert procs.ADAPTER_TIMEOUT < procs.OUTER_TIMEOUT
    with pytest.raises(SystemExit):
        opencode_model.main(["--model", "m", "--timeout", str(procs.OUTER_TIMEOUT)])


# --- opencode_model isolation ------------------------------------------------------------------------

FAKE_OPENCODE = """\
#!{python}
import json, os, subprocess, sys, time
here = os.path.dirname(os.path.abspath(__file__))
plan = json.load(open(os.path.join(here, "fake.json")))
config = open("opencode.json").read() if os.path.exists("opencode.json") else None
if plan.get("child_pidfile"):
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
    open(plan["child_pidfile"], "w").write(str(child.pid))
if plan.get("hang"):
    time.sleep(60)
open(plan["log"], "w").write(json.dumps({{
    "argv": sys.argv[1:], "cwd": os.getcwd(), "env": dict(os.environ),
    "config": config, "stdin": sys.stdin.read(),
}}))
print(json.dumps({{"type": "text", "part": {{"text": "fine"}}}}))
"""


@pytest.fixture
def fake_opencode(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    script(bin_dir, "opencode", FAKE_OPENCODE.format(python=sys.executable))
    log = tmp_path / "opencode.log"
    plan = {"log": str(log)}
    (bin_dir / "fake.json").write_text(json.dumps(plan))

    def set_plan(**more):
        (bin_dir / "fake.json").write_text(json.dumps({**plan, **more}))

    parent = {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "HOME": str(tmp_path / "home"),
    }
    return log, set_plan, parent


HOSTILE = {
    "OPENCODE_CONFIG_DIR": "/hostile/opencode",
    "OPENCODE_CONFIG": "/hostile/config.json",
    "OPENCODE_SESSION_ID": "ses_hostile",
    "OPENCODE": "1",
    "XDG_CONFIG_HOME": "/hostile/xdg",
    "XDG_DATA_HOME": "/hostile/data",
    "AWS_ACCESS_KEY_ID": "AKIAHOSTILE00000000",
    "GITHUB_TOKEN": "ghp_hostile",
    "PWD": "/hostile/repo",
}


def test_the_child_sees_only_allow_listed_variables_and_its_own_directories(
    fake_opencode,
):
    log, _, parent = fake_opencode
    parent = {**parent, **HOSTILE, "AWS_PROFILE": "from-parent", "LANG": "C"}
    answer = opencode_model.call(
        "amazon-bedrock/m", "hello", region="eu-west-1", profile="p", parent_env=parent
    )
    assert answer == "fine"
    seen = json.loads(log.read_text())
    env = seen["env"]
    assert not any(k.startswith("OPENCODE") for k in env)
    assert "GITHUB_TOKEN" not in env and "AWS_ACCESS_KEY_ID" not in env
    allowed = set(opencode_model.ALLOWED_ENV) | {
        "PWD",
        "TMPDIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "__CF_USER_TEXT_ENCODING",
        "LC_CTYPE",  # macOS interpreter additions
    }
    assert set(env) <= allowed, set(env) - allowed
    tmp = os.path.realpath(seen["cwd"])
    assert os.path.realpath(env["PWD"]) == tmp
    for name in (
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "TMPDIR",
    ):
        assert os.path.realpath(env[name]).startswith(tmp)
    assert env["AWS_PROFILE"] == "p" and env["AWS_REGION"] == "eu-west-1"


def test_the_config_is_toolless_and_names_only_the_bare_agent(fake_opencode):
    log, _, parent = fake_opencode
    opencode_model.call(
        "amazon-bedrock/some-model", "hello", region="r", profile="p", parent_env=parent
    )
    seen = json.loads(log.read_text())
    assert seen["argv"][:3] == ["run", "-m", "amazon-bedrock/some-model"]
    assert seen["argv"][seen["argv"].index("--agent") + 1] == opencode_model.AGENT
    config = json.loads(seen["config"])
    assert config["agent"][opencode_model.AGENT]["tools"] == {"*": False}
    assert config["provider"] == {
        "amazon-bedrock": {"options": {"region": "r", "profile": "p"}}
    }
    assert set(config) == {"$schema", "agent", "provider"}
    assert os.getcwd() not in seen["config"] and seen["stdin"] == "hello"
    assert os.path.realpath(seen["cwd"]) != os.path.realpath(os.getcwd())


def test_a_hung_opencode_and_its_child_are_killed_at_the_timeout(
    fake_opencode, tmp_path
):
    _, set_plan, parent = fake_opencode
    pidfile = tmp_path / "child.pid"
    set_plan(hang=True, child_pidfile=str(pidfile))
    with pytest.raises(subprocess.TimeoutExpired):
        opencode_model.call("m", "x", timeout=2, parent_env=parent)
    assert wait_dead(int(pidfile.read_text()))


# --- redaction ----------------------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("raw", "gone"),
    [
        ("failed at /Users/alice/work/repo/file.py line 3", "alice"),
        (
            "open /private/var/folders/ab/cd/T/slopvac-model-eval-x/opencode.json",
            "folders",
        ),
        ("key AKIAABCDEFGHIJKLMNOP rejected", "AKIAABCDEFGHIJKLMNOP"),
        ("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9abcdefghijklmnop", "eyJhbGci"),
        ("token=ghp_0123456789abcdef0123456789abcdef0123", "ghp_0123"),
        ("secret: hunter2hunter2", "hunter2"),
    ],
)
def test_redaction_removes_paths_and_tokens(raw, gone):
    assert gone not in procs.redact(raw)


def test_failure_messages_are_redacted_before_they_are_stored(tmp_path):
    script_path = script(
        tmp_path,
        "leak.py",
        "import sys\nsys.stderr.write('crash in /Users/alice/x token=abcdefghijklmnop123456')\nsys.exit(3)\n",
    )
    with pytest.raises(models.CallFailed) as caught:
        models.command_ask(f"{sys.executable} {script_path}", attempts=1)("x")
    assert "alice" not in str(caught.value) and "abcdefghijklmnop" not in str(
        caught.value
    )


# --- token boundaries ---------------------------------------------------------------------------------


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


# --- the command line -----------------------------------------------------------------------------------


def run_cli(tmp_path, monkeypatch, execute, extra=()):
    models_file = tmp_path / "models.json"
    models_file.write_text(json.dumps({"models": roster("true")}))
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: {"documents": []})
    monkeypatch.setattr(cli.models, "execute", execute)
    return cli.main(
        [
            "models",
            "run",
            "--models",
            str(models_file),
            "--cache",
            str(tmp_path),
            "--confirm-calls",
            "0",
            "--out",
            str(tmp_path / "r.json"),
            *extra,
        ]
    )


def test_run_forwards_workers_and_the_ceiling_and_keeps_files_next_to_the_output(
    tmp_path, monkeypatch
):
    seen = {}

    def fake_execute(tasks, roster_, repeats, lint, **kwargs):
        seen.update(kwargs)
        seen["ledger"] = kwargs["budget"].ledger
        return {"date": "d", "runs": []}

    code = run_cli(
        tmp_path,
        monkeypatch,
        fake_execute,
        ["--workers", "7", "--max-invocations", "9"],
    )
    assert code == 0
    assert seen["workers"] == 7 and seen["budget"].ceiling == 9
    assert seen["journal"].path == tmp_path / "r.journal.jsonl"
    assert seen["ledger"] == tmp_path / "r.ledger.jsonl"


def test_run_requires_an_explicit_ceiling(tmp_path):
    with pytest.raises(SystemExit):
        cli.main(["models", "run", "--models", "m.json", "--confirm-calls", "0"])


def test_a_ceiling_hit_exits_two_and_keeps_the_journal(tmp_path, monkeypatch, capsys):
    def stop(*args, **kwargs):
        raise models.BudgetExceeded(
            "invocation ceiling of 1 reached; nothing more was started"
        )

    code = run_cli(tmp_path, monkeypatch, stop, ["--max-invocations", "1"])
    assert code == 2 and "keeps the finished calls" in capsys.readouterr().err


def test_a_second_launch_on_a_held_ledger_exits_two(tmp_path, monkeypatch, capsys):
    with models.Budget(5, tmp_path / "r.ledger.jsonl"):
        code = run_cli(
            tmp_path,
            monkeypatch,
            lambda *a, **k: {"date": "d", "runs": []},
            ["--max-invocations", "5"],
        )
    assert code == 2 and "holds the ledger lock" in capsys.readouterr().err


def test_region_and_profile_default_to_the_parent_environment(fake_opencode):
    log, _, parent = fake_opencode
    parent = {**parent, "AWS_PROFILE": "from-env", "AWS_REGION": "us-east-1"}
    opencode_model.call("m", "x", parent_env=parent)
    seen = json.loads(log.read_text())
    options = json.loads(seen["config"])["provider"]["amazon-bedrock"]["options"]
    assert options == {"region": "us-east-1", "profile": "from-env"}
