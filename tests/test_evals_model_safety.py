"""Offline tests for the model-run safety rails: budget ledger, journal, process
groups, environment isolation and redaction.

No model and no network. Fake commands are small Python scripts.
"""

from __future__ import annotations

import datetime as dt
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
if plan.get("opencode_pidfile"):
    open(plan["opencode_pidfile"], "w").write(str(os.getpid()))
if plan.get("child_pidfile"):
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
    open(plan["child_pidfile"], "w").write(str(child.pid))
if plan.get("hang"):
    time.sleep(60)
open(plan["log"], "w").write(json.dumps({{
    "argv": sys.argv[1:], "cwd": os.getcwd(), "env": dict(os.environ),
    "config": config, "stdin": sys.stdin.read(),
    "home_listing": sorted(os.listdir(os.environ["HOME"])),
}}))
print(json.dumps({{"type": "text", "part": {{"text": "fine"}}}}))
"""

FAKE_AWS = """\
#!{python}
import json, os, sys
here = os.path.dirname(os.path.abspath(__file__))
open(json.load(open(os.path.join(here, "fake.json"))).get("aws_log", os.devnull), "w").write(json.dumps({{"argv": sys.argv[1:], "home": os.environ.get("HOME")}}))
plan = json.load(open(os.path.join(here, "fake.json")))
if plan.get("aws_count"):
    count_path = plan["aws_count"]
    n = int(open(count_path).read()) if os.path.exists(count_path) else 0
    open(count_path, "w").write(str(n + 1))
if plan.get("aws_pidfile"):
    open(plan["aws_pidfile"], "w").write(str(os.getpid()))
if plan.get("aws_child_pidfile"):
    import subprocess
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
    open(plan["aws_child_pidfile"], "w").write(str(child.pid))
if plan.get("aws_hang"):
    import time
    time.sleep(60)
if "--profile" in sys.argv and sys.argv[sys.argv.index("--profile") + 1] == "good":
    print(json.dumps({{"Version": 1, "AccessKeyId": "AKIAFAKEFAKEFAKEFAKE", "SecretAccessKey": "s3cret-value", "SessionToken": "tok-value"}}))
else:
    sys.exit(255)
"""

CREDS = {
    "AWS_ACCESS_KEY_ID": "AKIAFAKEFAKEFAKEFAKE",
    "AWS_SECRET_ACCESS_KEY": "s3cret-value",
    "AWS_SESSION_TOKEN": "tok-value",
}


@pytest.fixture
def fake_opencode(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    script(bin_dir, "opencode", FAKE_OPENCODE.format(python=sys.executable))
    script(bin_dir, "aws", FAKE_AWS.format(python=sys.executable))
    log = tmp_path / "opencode.log"
    plan = {"log": str(log)}
    (bin_dir / "fake.json").write_text(json.dumps(plan))

    def set_plan(**more):
        (bin_dir / "fake.json").write_text(json.dumps({**plan, **more}))

    real_home = tmp_path / "real-home"
    (real_home / ".config" / "opencode").mkdir(parents=True)
    (real_home / ".opencode").mkdir()
    (real_home / ".aws").mkdir()
    (real_home / ".config" / "opencode" / "opencode.json").write_text("{}")
    parent = {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "HOME": str(real_home),
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
    "AWS_SECRET_ACCESS_KEY": "hostile",
    "GITHUB_TOKEN": "ghp_hostile",
    "AWS_CONFIG_FILE": "/hostile/aws-config",
    "PWD": "/hostile/repo",
}


def test_the_child_gets_an_empty_home_and_only_allow_listed_variables(fake_opencode):
    log, _, parent = fake_opencode
    parent = {**parent, **HOSTILE, "LANG": "C", "AWS_PROFILE": "from-parent"}
    answer = opencode_model.call(
        "amazon-bedrock/m",
        "hello",
        region="eu-west-1",
        parent_env=parent,
        credentials=CREDS,
    )
    assert answer == "fine"
    seen = json.loads(log.read_text())
    env = seen["env"]
    assert not any(k.startswith("OPENCODE") for k in env)
    assert (
        "GITHUB_TOKEN" not in env
        and "AWS_PROFILE" not in env
        and "AWS_CONFIG_FILE" not in env
    )
    assert (
        env["AWS_ACCESS_KEY_ID"] == CREDS["AWS_ACCESS_KEY_ID"]
    )  # the resolved ones, not the hostile ones
    assert env["AWS_SECRET_ACCESS_KEY"] == CREDS["AWS_SECRET_ACCESS_KEY"]
    allowed = (
        set(opencode_model.ALLOWED_ENV)
        | set(CREDS)
        | {
            "HOME",
            "PWD",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
            "__CF_USER_TEXT_ENCODING",
            "LC_CTYPE",
        }
    )
    assert set(env) <= allowed, set(env) - allowed
    tmp = os.path.realpath(seen["cwd"])
    assert os.path.realpath(env["PWD"]) == tmp
    for name in (
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "TMPDIR",
    ):
        assert os.path.realpath(env[name]).startswith(tmp), name
    assert os.path.realpath(env["HOME"]) != os.path.realpath(parent["HOME"])
    assert (
        seen["home_listing"] == []
    )  # no .config/opencode, .opencode or .aws in the child's HOME
    assert env["AWS_REGION"] == "eu-west-1"


def test_the_config_is_toolless_and_names_only_the_bare_agent(fake_opencode):
    log, _, parent = fake_opencode
    opencode_model.call(
        "amazon-bedrock/some-model",
        "hello",
        region="r",
        parent_env=parent,
        credentials={},
    )
    seen = json.loads(log.read_text())
    assert seen["argv"][:3] == ["run", "-m", "amazon-bedrock/some-model"]
    assert seen["argv"][seen["argv"].index("--agent") + 1] == opencode_model.AGENT
    config = json.loads(seen["config"])
    assert config["agent"][opencode_model.AGENT]["tools"] == {"*": False}
    assert config["provider"] == {"amazon-bedrock": {"options": {"region": "r"}}}
    assert set(config) == {"$schema", "agent", "provider"}
    assert os.getcwd() not in seen["config"] and seen["stdin"] == "hello"
    assert os.path.realpath(seen["cwd"]) != os.path.realpath(os.getcwd())


def test_credentials_are_resolved_with_the_real_home_and_never_reach_a_file(
    fake_opencode, tmp_path
):
    log, set_plan, parent = fake_opencode
    aws_log = tmp_path / "aws.log"
    set_plan(aws_log=str(aws_log))
    keep = tmp_path / "kept"
    opencode_model.call(
        "m", "x", profile="good", region="r", parent_env=parent, keep=keep
    )
    asked = json.loads(aws_log.read_text())
    assert asked["argv"] == [
        "configure",
        "export-credentials",
        "--profile",
        "good",
        "--format",
        "process",
    ]
    assert (
        asked["home"] == parent["HOME"]
    )  # only the credential helper sees the real HOME
    seen = json.loads(log.read_text())
    assert seen["env"]["AWS_ACCESS_KEY_ID"] == CREDS["AWS_ACCESS_KEY_ID"]
    assert "AWS_PROFILE" not in seen["env"]
    on_disk = "".join(
        p.read_text()
        for p in keep.rglob("*")
        if p.is_file() and p.stat().st_size < 1_000_000
    )
    assert "s3cret-value" not in on_disk and "tok-value" not in on_disk


def test_an_unresolvable_profile_fails_without_echoing_anything(fake_opencode):
    _, _, parent = fake_opencode
    with pytest.raises(
        opencode_model.ModelCallError, match="could not resolve credentials"
    ):
        opencode_model.call("m", "x", profile="bad", parent_env=parent)


def tree_plan(tmp_path, set_plan):
    pids = {"opencode": tmp_path / "opencode.pid", "child": tmp_path / "child.pid"}
    set_plan(
        hang=True,
        opencode_pidfile=str(pids["opencode"]),
        child_pidfile=str(pids["child"]),
    )
    return pids


def adapter_command():
    return [sys.executable, "-m", "evals.opencode_model", "--model", "amazon-bedrock/m"]


def adapter_env(parent):
    repo = str(Path(models.lib.EVALS_DIR).parent)
    return {**parent, "PYTHONPATH": repo}


def test_interrupting_the_real_adapter_kills_opencode_and_its_descendants(
    fake_opencode, tmp_path
):
    _, set_plan, parent = fake_opencode
    pids = tree_plan(tmp_path, set_plan)
    with pytest.raises(subprocess.TimeoutExpired):
        procs.run_group(
            adapter_command(), input="x", timeout=3, env=adapter_env(parent)
        )
    assert wait_dead(int(pids["opencode"].read_text()))
    assert wait_dead(int(pids["child"].read_text()))


def test_the_adapter_timeout_kills_opencode_and_everything_below_it(
    fake_opencode, tmp_path
):
    _, set_plan, parent = fake_opencode
    pids = tree_plan(tmp_path, set_plan)
    started = time.time()
    result = procs.run_group(
        [*adapter_command(), "--timeout", "2"],
        input="x",
        timeout=30,
        env=adapter_env(parent),
    )
    assert result.returncode == 1 and "timed out" in result.stderr
    assert time.time() - started < 20
    assert wait_dead(int(pids["opencode"].read_text()))
    assert wait_dead(int(pids["child"].read_text()))


def test_the_group_leader_is_not_reaped_before_its_group_is_signalled(tmp_path):
    proc = subprocess.Popen([sys.executable, "-c", "pass"], start_new_session=True)
    procs._wait_exit_noreap(proc.pid, 10)
    # Still a zombie that we own: waitid sees it again, and Popen has no return code yet.
    assert proc.returncode is None
    assert (
        os.waitid(os.P_PID, proc.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
    )
    assert proc.wait() == 0


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


def test_region_defaults_to_the_parent_environment(fake_opencode):
    log, _, parent = fake_opencode
    parent = {**parent, "AWS_REGION": "us-east-1"}
    opencode_model.call("m", "x", parent_env=parent, credentials={})
    seen = json.loads(log.read_text())
    options = json.loads(seen["config"])["provider"]["amazon-bedrock"]["options"]
    assert options == {"region": "us-east-1"}


def test_a_journal_without_a_ledger_is_refused(tmp_path, monkeypatch, capsys):
    (tmp_path / "r.journal.jsonl").write_text(
        json.dumps({"job": "j", "answer": "a", "error": None, "attempts": 1}) + "\n"
    )
    code = run_cli(
        tmp_path,
        monkeypatch,
        lambda *a, **k: {"date": "d", "runs": []},
        ["--max-invocations", "5"],
    )
    err = capsys.readouterr().err
    assert code == 2 and "without r.ledger.jsonl" in err and "fresh --out" in err


def test_an_empty_journal_without_a_ledger_is_fine(tmp_path, monkeypatch):
    (tmp_path / "r.journal.jsonl").write_text("")
    code = run_cli(
        tmp_path,
        monkeypatch,
        lambda *a, **k: {"date": "d", "runs": []},
        ["--max-invocations", "5"],
    )
    assert code == 0


def test_cancelling_while_the_credential_helper_hangs_kills_it_and_its_children(
    fake_opencode, tmp_path
):
    _, set_plan, parent = fake_opencode
    pids = {"aws": tmp_path / "aws.pid", "child": tmp_path / "aws-child.pid"}
    set_plan(
        aws_hang=True,
        aws_pidfile=str(pids["aws"]),
        aws_child_pidfile=str(pids["child"]),
    )
    cmd = [*adapter_command(), "--aws-profile", "good"]
    with pytest.raises(subprocess.TimeoutExpired):
        procs.run_group(cmd, input="x", timeout=3, env=adapter_env(parent))
    assert wait_dead(int(pids["aws"].read_text()))
    assert wait_dead(int(pids["child"].read_text()))


NOW = dt.datetime(2026, 11, 14, 12, 0, tzinfo=dt.UTC)


def fake_export(expiries):
    """An export_credentials stand-in that yields the given expiries in order."""
    calls = []
    queue = list(expiries)

    def export(profile, parent):
        calls.append(profile)
        return dict(CREDS), queue.pop(0)

    return export, calls


@pytest.mark.parametrize(
    ("minutes_left", "exports"),
    [(None, 1), (11, 1), (10, 1)],
)
def test_credentials_with_enough_lifetime_are_used_as_they_are(
    monkeypatch, minutes_left, exports
):
    expiry = None if minutes_left is None else NOW + dt.timedelta(minutes=minutes_left)
    export, calls = fake_export([expiry])
    monkeypatch.setattr(opencode_model, "export_credentials", export)
    assert opencode_model.fresh_credentials("p", {}, now=lambda: NOW) == CREDS
    assert len(calls) == exports


def test_short_lived_credentials_are_exported_again(monkeypatch):
    export, calls = fake_export(
        [NOW + dt.timedelta(minutes=9, seconds=59), NOW + dt.timedelta(hours=1)]
    )
    monkeypatch.setattr(opencode_model, "export_credentials", export)
    assert opencode_model.fresh_credentials("p", {}, now=lambda: NOW) == CREDS
    assert len(calls) == 2


def test_an_attempt_is_refused_when_the_credentials_stay_short_lived(monkeypatch):
    short = NOW + dt.timedelta(minutes=3)
    export, calls = fake_export([short, short])
    monkeypatch.setattr(opencode_model, "export_credentials", export)
    with pytest.raises(opencode_model.ModelCallError, match="under 10 minutes"):
        opencode_model.fresh_credentials("p", {}, now=lambda: NOW)
    assert len(calls) == 2


def test_each_attempt_resolves_credentials_again(fake_opencode, tmp_path):
    _, set_plan, parent = fake_opencode
    set_plan(aws_log=str(tmp_path / "aws.log"), aws_count=str(tmp_path / "aws.count"))
    for _ in range(2):
        opencode_model.call("m", "x", profile="good", region="r", parent_env=parent)
    assert (tmp_path / "aws.count").read_text() == "2"
