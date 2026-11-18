"""Offline tests for the model-run safety rails: budget ledger, journal, process
groups, environment isolation and redaction.

No model and no network. Fake commands are small Python scripts.
"""

from __future__ import annotations

import datetime as dt
import io
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
    release = tmp_path / "release"
    repo = Path(models.lib.EVALS_DIR).parent
    code = textwrap.dedent(
        """\
        import os, sys, time
        from evals import models
        try:
            budget = models.Budget(3, sys.argv[1])
        except models.ModelEvalError:
            print("refused"); sys.exit(0)
        for _ in range(3):
            budget.reserve()
        deadline = time.time() + 60
        while not os.path.exists(sys.argv[2]) and time.time() < deadline:
            time.sleep(0.02)
        print("ran")
        """
    )
    env = {**os.environ, "PYTHONPATH": str(repo)}
    procs_ = [
        subprocess.Popen(
            [sys.executable, "-c", code, str(ledger), str(release)],
            cwd=repo,
            env=env,
            stdout=subprocess.PIPE,
            text=True,
        )
        for _ in range(4)
    ]
    # The holder keeps the lock until every other launch has been refused.
    deadline = time.time() + 60
    while sum(p.poll() is not None for p in procs_) < 3 and time.time() < deadline:
        time.sleep(0.02)
    release.write_text("go")
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
open(sys.argv[1] + ".ready", "a").write("up\\n")
time.sleep(60)
"""


def lines_in(path):
    try:
        return len(Path(path).read_text().split())
    except FileNotFoundError:
        return 0


@pytest.fixture
def ready_first(monkeypatch):
    """Start a run's timeout clock only after the run's processes report ready.

    `arm(*markers)` names files whose word count must reach the number of runs
    started so far (a pidfile counts as one word, a ready log one word per
    start). A slow start under load then cannot eat the timeout, while the
    timeout itself stays as short as the test wants. The wait is bounded.
    """
    real = procs._wait_exit_noreap
    state = {"markers": [], "runs": 0}

    def arm(*markers):
        state["markers"] = list(markers)

    def wait(pid, timeout):
        state["runs"] += 1
        deadline = time.monotonic() + 60
        while any(lines_in(m) < state["runs"] for m in state["markers"]):
            assert time.monotonic() < deadline, "the run never reported ready"
            time.sleep(0.02)
        return real(pid, timeout)

    monkeypatch.setattr(procs, "_wait_exit_noreap", wait)
    return arm


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


def test_a_timeout_kills_the_wrapper_and_the_child_it_spawned(tmp_path, ready_first):
    pidfile = tmp_path / "child.pid"
    ready_first(Path(f"{pidfile}.ready"))
    wrapper = script(tmp_path, "wrap.py", SPAWNS_CHILD)
    with pytest.raises(subprocess.TimeoutExpired):
        procs.run_group([sys.executable, str(wrapper), str(pidfile)], timeout=1.5)
    assert wait_dead(int(pidfile.read_text()))


def test_the_harness_timeout_reaps_the_whole_tree_and_counts_the_attempts(
    tmp_path, ready_first
):
    pidfile = tmp_path / "child.pid"
    ready_first(Path(f"{pidfile}.ready"))
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
for event in plan.get("events", []):
    print(json.dumps(event))
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
    fake_opencode, tmp_path, ready_first
):
    _, set_plan, parent = fake_opencode
    pids = tree_plan(tmp_path, set_plan)
    ready_first(*pids.values())
    with pytest.raises(subprocess.TimeoutExpired):
        procs.run_group(
            adapter_command(), input="x", timeout=3, env=adapter_env(parent)
        )
    assert wait_dead(int(pids["opencode"].read_text()))
    assert wait_dead(int(pids["child"].read_text()))


def test_the_adapter_timeout_kills_opencode_and_everything_below_it(
    fake_opencode, tmp_path
):
    # The adapter's own timer runs in another process, so the test cannot gate it
    # on readiness. Give it a generous limit instead: opencode hangs for 60s, so
    # the adapter must still be the one that stops it, well before the harness.
    _, set_plan, parent = fake_opencode
    pids = tree_plan(tmp_path, set_plan)
    started = time.time()
    result = procs.run_group(
        [*adapter_command(), "--timeout", "15"],
        input="x",
        timeout=120,
        env=adapter_env(parent),
    )
    assert result.returncode == 1 and "timed out" in result.stderr
    assert time.time() - started < 50
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


@pytest.mark.parametrize(
    ("raw", "gone"),
    [
        ('{"token": "abc123"}', "abc123"),
        ('{"access_token":"s3cr3tv4lue"}', "s3cr3tv4lue"),
        ('{"Authorization": "Bearer x7"}', "x7"),
        ('{"aws_session_token": "FwoGZXIvYXdz"}', "FwoGZXIvYXdz"),
        ("aws_session_token = FwoGZXIvYXdz", "FwoGZXIvYXdz"),
        ("Authorization: Bearer shorttok", "shorttok"),
        ("{'password': 'pw1'}", "pw1"),
        ("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI", "wJalrXUtnFEMI"),
        ("id_token=eyJabc12", "eyJabc12"),
        ("refresh_token: r3fr3sh", "r3fr3sh"),
        ("api_key = k3y9", "k3y9"),
    ],
    ids=[
        "quoted-token",
        "no-space",
        "authorization-header-json",
        "session-token-json",
        "session-token-env",
        "short-bearer",
        "single-quotes",
        "secret-access-key",
        "id-token",
        "refresh-token",
        "api-key",
    ],
)
def test_redaction_removes_short_and_quoted_structured_credentials(raw, gone):
    assert gone not in procs.redact(raw)
    assert gone not in procs.redact_credentials(raw)


FIELD_NAMES = [
    "token",
    "access_token",
    "accessToken",
    "AccessToken",
    "refresh_token",
    "refreshToken",
    "id_token",
    "idToken",
    "session_token",
    "sessionToken",
    "SessionToken",
    "secret",
    "secret_access_key",
    "secretAccessKey",
    "SecretAccessKey",
    "password",
    "api_key",
    "apiKey",
    "API_KEY",
    "authorization",
    "Authorization",
]


@pytest.mark.parametrize("name", FIELD_NAMES)
@pytest.mark.parametrize(
    "shape",
    ['{{"{n}": "FAKEsess1234"}}', "{n}=FAKEsess1234", "{n}: FAKEsess1234"],
    ids=["json", "env", "colon"],
)
def test_every_listed_credential_field_loses_its_value(name, shape):
    raw = shape.format(n=name)
    assert "FAKEsess1234" not in procs.redact_credentials(raw)
    assert name in procs.redact_credentials(raw)  # the field name stays


def test_an_escaped_quote_inside_a_json_value_does_not_end_the_value_early():
    raw = '{"password": "ab\\"cd-FAKEtail1234\\"ef", "limit": 63}'
    cleaned = procs.redact_credentials(raw)
    assert "FAKEtail1234" not in cleaned
    assert '"limit": 63' in cleaned


@pytest.mark.parametrize(
    "text",
    [
        "max_tokens = 4096",
        '{"max_tokens": 4096}',
        "token_budget: 63",
        "tokens: 63",
        "{'tokens': 63, 'token_budget': 5}",
        "maxToken: 5 and my_token_limit=7",
    ],
)
def test_names_that_only_contain_token_are_not_credential_fields(text):
    assert procs.redact_credentials(text) == text


STANDALONE_SECRETS = [
    ("Sent Bearer abc123def to the server", "abc123def"),
    ("curl -H 'Bearer x7'", "x7"),
    ("Basic dXNlcjpwYXNz was rejected", "dXNlcjpwYXNz"),
    ("Authorization: token ghp_abcd1234", "ghp_abcd1234"),
    ("pushed with token ghp_ab12cd34ef today", "ghp_ab12cd34ef"),
    ('{"password": "ab\\"cd-SECRETTAIL\\"ef"}', "SECRETTAIL"),
    ("{'token': 'it\\'s-SECRETTAIL'}", "SECRETTAIL"),
    ('{"secret": "a\\\\\\"SECRETTAIL"}', "SECRETTAIL"),
]


@pytest.mark.parametrize(("raw", "gone"), STANDALONE_SECRETS)
def test_redaction_removes_standalone_schemes_and_escaped_quote_values(raw, gone):
    assert gone not in procs.redact_credentials(raw)
    assert gone not in procs.redact(raw)


@pytest.mark.parametrize(
    "text",
    [
        "A bearer of news arrived.",
        "Basic usage needs no token.",
        "Basic authentication is optional here.",
        "Check token 63 against the budget.",
        "The token budget is explicit.",
    ],
)
def test_prose_that_names_a_scheme_is_not_redacted(text):
    assert procs.redact_credentials(text) == text


@pytest.mark.parametrize(
    "text",
    [
        "max_tokens = 4096",
        '{"max_tokens": 4096, "temperature": 0}',
        "Set max_tokens: 4096 and the token budget to 63.",
        "token_budget: 63",
        "tokens: 63",
        "completion_tokens=1200 prompt_tokens=88",
        "Rewrite the paragraph so the token budget is explicit.",
        "The password policy requires 12 characters; see /Users/docs/policy.md.",
        "Use `max_retries` 3 times, then fail. Secret sauce is optional.",
        "A bearer of news arrived at the secret garden.",
        'Return {"name": "queue", "limit": 63} for each request.',
    ],
)
def test_credential_redaction_leaves_ordinary_text_unchanged(text):
    assert procs.redact_credentials(text) == text


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
    fake_opencode, tmp_path, ready_first
):
    _, set_plan, parent = fake_opencode
    pids = {"aws": tmp_path / "aws.pid", "child": tmp_path / "aws-child.pid"}
    ready_first(*pids.values())
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


class FakeClock:
    """A monotonic clock that only moves when the test sleeps."""

    def __init__(self):
        self.t = 0.0
        self.slept = []

    def sleep(self, seconds):
        self.slept.append(seconds)
        self.t += seconds

    def __call__(self):
        return self.t


def test_short_lived_credentials_wait_for_a_fresh_export_inside_one_attempt(
    monkeypatch,
):
    short = NOW + dt.timedelta(minutes=9, seconds=59)
    export, calls = fake_export([short, short, NOW + dt.timedelta(hours=1)])
    monkeypatch.setattr(opencode_model, "export_credentials", export)
    clock = FakeClock()
    creds = opencode_model.fresh_credentials(
        "p", {}, now=lambda: NOW, sleep=clock.sleep, clock=clock
    )
    assert creds == CREDS and len(calls) == 3
    assert clock.slept == [opencode_model.CREDENTIAL_POLL] * 2


def test_an_attempt_is_refused_after_a_bounded_wait_when_credentials_stay_short(
    monkeypatch,
):
    short = NOW + dt.timedelta(minutes=3)
    export, calls = fake_export([short] * 100)
    monkeypatch.setattr(opencode_model, "export_credentials", export)
    clock = FakeClock()
    with pytest.raises(opencode_model.ModelCallError, match="under 10 minutes"):
        opencode_model.fresh_credentials(
            "p", {}, now=lambda: NOW, sleep=clock.sleep, clock=clock
        )
    assert sum(clock.slept) == opencode_model.CREDENTIAL_WAIT  # 330s, then it gives up
    assert len(calls) == len(clock.slept) + 1


def test_the_credential_wait_and_the_model_call_fit_inside_the_harness_timeout():
    total = (
        opencode_model.CREDENTIAL_WAIT
        + opencode_model.CREDENTIAL_TIMEOUT
        + procs.ADAPTER_TIMEOUT
    )
    assert total < procs.OUTER_TIMEOUT


def test_each_attempt_resolves_credentials_again(fake_opencode, tmp_path):
    _, set_plan, parent = fake_opencode
    set_plan(aws_log=str(tmp_path / "aws.log"), aws_count=str(tmp_path / "aws.count"))
    for _ in range(2):
        opencode_model.call("m", "x", profile="good", region="r", parent_env=parent)
    assert (tmp_path / "aws.count").read_text() == "2"


# --- token usage ---------------------------------------------------------------------------------------


def step_finish(**tokens):
    return {
        "type": "step_finish",
        "part": {"type": "step-finish", "cost": 0, "tokens": tokens},
    }


def stream(*events):
    return "\n".join(json.dumps(e) for e in events)


def test_usage_sums_every_step_and_keeps_each_token_kind():
    events = stream(
        step_finish(input=100, output=20, reasoning=5, cache={"read": 50, "write": 7}),
        {"type": "text", "part": {"text": "hi"}},
        step_finish(input=30, output=4, reasoning=0, cache={"read": 0, "write": 0}),
    )
    assert opencode_model.parse_usage(events) == {
        "input": 130,
        "output": 24,
        "reasoning": 5,
        "cache_read": 50,
        "cache_write": 7,
    }


def test_usage_lists_only_the_fields_the_stream_reports():
    assert opencode_model.parse_usage(stream(step_finish(input=3, output=2))) == {
        "input": 3,
        "output": 2,
    }


def test_a_stream_without_step_tokens_has_no_usage():
    events = stream({"type": "text", "part": {"text": "hi"}}, step_finish())
    assert opencode_model.parse_usage(events) is None


def test_the_adapter_keeps_stdout_for_the_answer_and_ends_stderr_with_the_usage(
    fake_opencode, monkeypatch, capsys
):
    _, set_plan, parent = fake_opencode
    set_plan(events=[step_finish(input=9, output=4, cache={"read": 2, "write": 1})])
    monkeypatch.setenv("PATH", parent["PATH"])
    monkeypatch.setenv("HOME", parent["HOME"])
    monkeypatch.setattr(sys, "stdin", io.StringIO("question"))
    assert opencode_model.main(["--model", "m"]) == 0
    out, err = capsys.readouterr()
    assert out == "fine"
    assert err.splitlines()[-1] == (
        'slopvac-usage {"cache_read": 2, "cache_write": 1, "input": 9, "output": 4}'
    )


USAGE_COMMAND = """\
import sys
print("answer")
print('slopvac-usage {"input": 10, "output": 3}', file=sys.stderr)
"""


def test_a_reply_carries_the_usage_the_command_reports(tmp_path):
    cmd = f"{sys.executable} {script(tmp_path, 'usage.py', USAGE_COMMAND)}"
    reply = models.command_ask(cmd)("x")
    assert reply.usage == {"input": 10, "output": 3}


def test_a_command_that_reports_nothing_has_no_usage(tmp_path):
    cmd = f"{sys.executable} -c 'print(1)'"
    assert models.command_ask(cmd)("x").usage is None


def test_usage_adds_up_over_retries_and_a_silent_attempt_makes_it_unknown(tmp_path):
    counter = tmp_path / "n"
    counter.write_text("0")
    body = (
        "import sys, pathlib\n"
        f"p = pathlib.Path({str(counter)!r})\n"
        "n = int(p.read_text()) + 1\n"
        "p.write_text(str(n))\n"
        "if n == 1 and {silent}:\n    sys.exit(1)\n"
        "print('slopvac-usage {{\"input\": 5}}', file=sys.stderr)\n"
        "if n == 1:\n    sys.exit(1)\n"
        "print('ok')\n"
    )
    loud = script(tmp_path, "loud.py", body.format(silent=False))
    assert models.command_ask(f"{sys.executable} {loud}")("x").usage == {"input": 10}
    counter.write_text("0")
    quiet = script(tmp_path, "quiet.py", body.format(silent=True))
    assert models.command_ask(f"{sys.executable} {quiet}")("x").usage is None


def test_the_usage_line_stays_out_of_a_failure_message(tmp_path):
    body = (
        "import sys\n"
        "print('boom', file=sys.stderr)\n"
        "print('slopvac-usage {\"input\": 1}', file=sys.stderr)\n"
        "sys.exit(1)\n"
    )
    cmd = f"{sys.executable} {script(tmp_path, 'bad.py', body)}"
    with pytest.raises(models.CallFailed) as caught:
        models.command_ask(cmd, attempts=1)("x")
    assert str(caught.value) == "model command exited 1: boom"
    assert caught.value.usage == {"input": 1}


def test_the_ledger_names_the_model_on_every_reserve_and_finish(tmp_path):
    cmd = f"{sys.executable} -c 'print(1)'"
    with models.Budget(5, tmp_path / "l.jsonl") as budget:
        models.command_ask(cmd, budget=budget, model="claude-haiku-5-5")("x")
    events = [
        json.loads(line) for line in (tmp_path / "l.jsonl").read_text().splitlines()
    ]
    assert [(e["event"], e["model"]) for e in events] == [
        ("reserve", "claude-haiku-5-5"),
        ("finish", "claude-haiku-5-5"),
    ]


def test_a_ledger_from_before_models_were_named_still_resumes(tmp_path):
    ledger = tmp_path / "l.jsonl"
    ledger.write_text(
        '{"event": "reserve", "n": 1}\n'
        '{"event": "finish", "n": 1, "status": "ok"}\n'
        '{"event": "reserve", "n": 2}\n'
    )
    with models.Budget(3, ledger) as budget:
        assert (budget.spent, budget.unfinished_at_start) == (2, 1)
        assert budget.reserve() == 3


def test_a_journal_from_before_usage_was_recorded_still_loads(tmp_path):
    path = tmp_path / "j.jsonl"
    path.write_text('{"job": "a", "answer": "text", "error": null, "attempts": 2}\n')
    journal = models.Journal(path)
    assert journal.done("a")["answer"] == "text" and journal.attempts == 2


def test_a_run_journals_the_model_and_usage_and_a_resume_reuses_them(
    tmp_path, spawn_log
):
    body = (
        COUNTING
        + 'print(\'slopvac-usage {"input": 7, "output": 2}\', file=sys.stderr)\n'
    )
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', body)}"
    path = tmp_path / "j.jsonl"
    results = models.execute(
        author_tasks(1), roster(cmd), 3, no_findings, journal=models.Journal(path)
    )
    entry = json.loads(path.read_text().splitlines()[0])
    assert (entry["model"], entry["usage"]) == ("m", {"input": 7, "output": 2})
    assert results["runs"][0]["usage"] == {"input": 7, "output": 2}
    again = models.execute(
        author_tasks(1), roster(cmd), 3, no_findings, journal=models.Journal(path)
    )
    assert spawns(tmp_path) == 3
    assert again["runs"][0]["usage"] == {"input": 7, "output": 2}


def test_a_run_without_reported_usage_journals_null(tmp_path, spawn_log):
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', COUNTING)}"
    path = tmp_path / "j.jsonl"
    results = models.execute(
        author_tasks(1), roster(cmd), 3, no_findings, journal=models.Journal(path)
    )
    assert json.loads(path.read_text().splitlines()[0])["usage"] is None
    assert results["runs"][0]["usage"] is None


def adapter_stderr(fake_opencode, monkeypatch, capsys, events):
    _, set_plan, parent = fake_opencode
    set_plan(events=events)
    monkeypatch.setenv("PATH", parent["PATH"])
    monkeypatch.setenv("HOME", parent["HOME"])
    monkeypatch.setattr(sys, "stdin", io.StringIO("question"))
    code = opencode_model.main(["--model", "m"])
    out, err = capsys.readouterr()
    return code, out, err


def test_error_text_cannot_pass_for_a_usage_report(fake_opencode, monkeypatch, capsys):
    fake_line = 'slopvac-usage {"input": 999999999}'
    events = [
        step_finish(input=4, output=1),
        {"type": "error", "error": {"message": f"boom\n{fake_line}\nmore"}},
    ]
    code, _, err = adapter_stderr(fake_opencode, monkeypatch, capsys, events)
    assert code == 1
    assert procs.parse_usage_line(err) == {"input": 4, "output": 1}
    assert [l for l in err.splitlines() if l.startswith("slopvac-usage")] == [
        'slopvac-usage {"input": 4, "output": 1}'
    ]


def test_a_usage_line_that_is_not_last_is_ignored():
    text = 'slopvac-usage {"input": 5}\nsomething else\n'
    assert procs.parse_usage_line(text) is None


def test_a_failed_call_still_reports_the_tokens_it_saw(
    fake_opencode, monkeypatch, capsys
):
    events = [
        step_finish(input=12, output=3, cache={"read": 8, "write": 0}),
        {"type": "error", "error": {"message": "throttled"}},
    ]
    code, out, err = adapter_stderr(fake_opencode, monkeypatch, capsys, events)
    assert code == 1 and out == ""
    assert "throttled" in err
    assert procs.parse_usage_line(err) == {
        "input": 12,
        "output": 3,
        "cache_read": 8,
        "cache_write": 0,
    }


def test_the_harness_journals_the_usage_of_a_failed_call(tmp_path):
    body = (
        "import sys\n"
        "print('boom', file=sys.stderr)\n"
        "print('slopvac-usage {\"input\": 6}', file=sys.stderr)\n"
        "sys.exit(1)\n"
    )
    cmd = f"{sys.executable} {script(tmp_path, 'bad.py', body)}"
    path = tmp_path / "j.jsonl"
    results = models.execute(
        author_tasks(1),
        roster(cmd),
        3,
        no_findings,
        journal=models.Journal(path),
    )
    entries = [json.loads(line) for line in path.read_text().splitlines()]
    assert [e["usage"] for e in entries] == [{"input": 18}] * 3
    assert results["runs"][0]["usage"] == {"input": 18}


def test_a_resumed_job_keeps_the_spend_of_its_failed_executions(tmp_path):
    counter = tmp_path / "n"
    body = (
        "import sys, pathlib\n"
        f"p = pathlib.Path({str(counter)!r})\n"
        "n = int(p.read_text()) + 1 if p.exists() else 1\n"
        "p.write_text(str(n))\n"
        "print('slopvac-usage {\"input\": 5}', file=sys.stderr)\n"
        "if n <= 3:\n    sys.exit(1)\n"
        "print('ok')\n"
    )
    cmd = f"{sys.executable} {script(tmp_path, 'late.py', body)}"
    path = tmp_path / "j.jsonl"
    tasks = author_tasks(1)
    first = models.execute(
        tasks, roster(cmd), 3, no_findings, journal=models.Journal(path)
    )
    assert first["runs"][0]["error"]
    again = models.execute(
        tasks, roster(cmd), 3, no_findings, journal=models.Journal(path)
    )
    # Job 0 used three failing attempts of 5 tokens, then succeeded on the resume.
    job0 = models.job_id("task", "m", tasks[0]["prompt"], 0)
    mine = [e["usage"] for e in again["executions"] if e["job"] == job0]
    assert mine == [{"input": 15}, {"input": 5}]
    assert again["runs"][0]["usage"] == {"input": 5}


# --- exhaustion mid-retry, and cost per execution ---------------------------------------


def usage_command(tmp_path, name, fail_through, tokens, answer="[]"):
    """A command that reports `tokens` input every run and fails its first N runs."""
    counter = tmp_path / f"{name}.n"
    body = (
        "import sys, pathlib\n"
        f"p = pathlib.Path({str(counter)!r})\n"
        "n = int(p.read_text()) + 1 if p.exists() else 1\n"
        "p.write_text(str(n))\n"
        f"print('slopvac-usage {{\"input\": {tokens}}}', file=sys.stderr)\n"
        f"if n <= {fail_through}:\n    sys.exit(1)\n"
        f"print({answer!r})\n"
    )
    return f"{sys.executable} {script(tmp_path, name + '.py', body)}"


def cost_of(path):
    prices = {
        "models": {"a": {"input": 1, "output": 1}, "b": {"input": 1, "output": 1}}
    }
    return models.estimate_cost(models.load_usage_records(path), prices)


def test_spend_of_attempts_made_before_the_ceiling_is_journaled_and_kept(tmp_path):
    cmd = usage_command(tmp_path, "late", fail_through=2, tokens=5)
    journal_path, ledger = tmp_path / "j.jsonl", tmp_path / "l.jsonl"
    with models.Budget(2, ledger) as budget, pytest.raises(models.BudgetExceeded):
        models.execute(
            author_tasks(1),
            roster(cmd),
            3,
            no_findings,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    partial = [json.loads(line) for line in journal_path.read_text().splitlines()]
    assert [(e["attempts"], e["usage"]) for e in partial] == [(2, {"input": 10})]
    assert partial[0]["error"]  # so the job is asked again on resume
    with models.Budget(100, ledger) as budget:  # raise the ceiling and resume
        results = models.execute(
            author_tasks(1),
            roster(cmd),
            3,
            no_findings,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    # 2 failed attempts of 5, then 3 successful runs of 5: 25 tokens in all.
    assert cost_of(journal_path)["models"]["m"]["tokens"] == {"input": 25}
    assert [e["usage"] for e in results["executions"]][:2] == [
        {"input": 10},
        {"input": 5},
    ]


def test_a_partial_job_without_reported_usage_leaves_the_cost_incomplete(tmp_path):
    cmd = f"{sys.executable} -c 'import sys; sys.exit(1)'"
    journal_path = tmp_path / "j.jsonl"
    with (
        models.Budget(2, tmp_path / "l.jsonl") as budget,
        pytest.raises(models.BudgetExceeded),
    ):
        models.execute(
            author_tasks(1),
            roster(cmd),
            3,
            no_findings,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    report = cost_of(journal_path)
    assert report["models"]["m"]["missing_usage"] == 1 and report["complete"] is False


REWRITE_SOURCE = "Retry 3 times, use `max_retries`."


def rewrite_task():
    return {
        "id": "rw",
        "type": "rewrite",
        "role": "author",
        "doc": "d",
        "genre": "g",
        "prompt": "Rewrite the passage",
        "source": REWRITE_SOURCE,
    }


def test_a_superseded_review_is_still_charged_and_both_cost_views_agree(tmp_path):
    # Model a fails job 0 on its first three runs (100 tokens each), so its empty
    # candidate is reviewed by b. On the resume a answers, the new candidate gets a
    # new review, and the old review has no row any more.
    a = usage_command(tmp_path, "a", fail_through=3, tokens=100, answer=REWRITE_SOURCE)
    b = usage_command(tmp_path, "b", fail_through=0, tokens=7)
    pair = [
        {"name": "a", "family": "fa", "tier": "t", "command": a},
        {"name": "b", "family": "fb", "tier": "t", "command": b},
    ]
    journal_path, out = tmp_path / "j.jsonl", tmp_path / "r.json"
    for _ in range(2):
        results = models.execute(
            [rewrite_task()],
            pair,
            3,
            no_findings,
            journal=models.Journal(journal_path),
        )
    models.dump(results, out)
    # Run 1, a: author job 0 fails 3 times (300), jobs 1 and 2 (200), then reviews of
    # b's three rewrites (300): 800. b: three author jobs (21) and three reviews of
    # a's candidates, one of them empty (21): 42.
    # Resume: a answers job 0 (100) and b reviews the new candidate (7).
    expected = {"a": {"input": 900}, "b": {"input": 49}}
    for path in (journal_path, out):
        report = cost_of(path)
        assert {m: r["tokens"] for m, r in report["models"].items()} == expected
        assert report["complete"] is True
    rows = [r for r in results["runs"] if r["model"] == "b"]
    assert len(rows) == 6  # 7 executions: the superseded review has no row


def test_one_execution_behind_several_rows_is_charged_once(tmp_path, spawn_log):
    body = COUNTING + "print('slopvac-usage {\"input\": 7}', file=sys.stderr)\n"
    cmd = f"{sys.executable} {script(tmp_path, 'count.py', body)}"
    first = author_tasks(1)[0]
    twin = {**first, "id": "twin"}  # same prompt, so the same job per repeat
    journal_path, out = tmp_path / "j.jsonl", tmp_path / "r.json"
    results = models.execute(
        [first, twin],
        roster(cmd),
        3,
        no_findings,
        journal=models.Journal(journal_path),
    )
    models.dump(results, out)
    assert len(results["runs"]) == 6 and spawns(tmp_path) == 3
    for path in (journal_path, out):
        assert cost_of(path)["models"]["m"]["tokens"] == {"input": 21}


# --- an interrupted run's spend stays in the cost report ---------------------------------

INTERRUPTS_FIRST_RUN = """\
import os, pathlib, signal, sys, time
marker = pathlib.Path(os.environ["MARKER"])
sys.stdin.read()
if not marker.exists():
    marker.write_text("x")
    os.kill(os.getppid(), signal.SIGINT)
    time.sleep(60)
print('slopvac-usage {"input": 5}', file=sys.stderr)
print("[]")
"""


def test_cost_after_ctrl_c_and_resume_counts_the_reservation_with_no_journal_line(
    tmp_path, monkeypatch
):
    monkeypatch.setenv("MARKER", str(tmp_path / "marker"))
    cmd = f"{sys.executable} {script(tmp_path, 'once.py', INTERRUPTS_FIRST_RUN)}"
    journal_path, ledger = tmp_path / "r.journal.jsonl", tmp_path / "r.ledger.jsonl"
    with models.Budget(10, ledger) as budget, pytest.raises(KeyboardInterrupt):
        models.execute(
            author_tasks(1),
            roster(cmd),
            3,
            no_findings,
            workers=1,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    with models.Budget(10, ledger) as budget:
        results = models.execute(
            author_tasks(1),
            roster(cmd),
            3,
            no_findings,
            workers=1,
            budget=budget,
            journal=models.Journal(journal_path),
        )
    # The ledger holds four reservations of "m": one interrupted, then three on
    # the resume. The journal holds three lines, so one reservation is spend
    # with no usage on record.
    from_journal = cost_of(journal_path)
    assert from_journal["complete"] is False
    assert from_journal["models"]["m"]["invocations"] == 4
    assert from_journal["models"]["m"]["missing_usage"] == 1
    assert sorted((e["model"], e["usage"] is None) for e in results["executions"]) == [
        ("m", False),
        ("m", False),
        ("m", False),
        ("m", True),
    ]
    scratch = tmp_path / "results.json"
    models.dump(results, scratch)
    assert cost_of(scratch) == from_journal


# --- a Python without os.waitid is refused before anything spawns ------------------------


def test_without_os_waitid_the_harness_refuses_before_spawning_or_reserving(
    tmp_path, monkeypatch
):
    monkeypatch.delattr(os, "waitid")
    marker = tmp_path / "spawned"
    body = f"import pathlib; pathlib.Path({str(marker)!r}).write_text('x')\n"
    cmd = f"{sys.executable} {script(tmp_path, 'touch.py', body)}"
    with pytest.raises(procs.UnsupportedPlatform, match="Python 3.13"):
        procs.run_group(cmd.split(), timeout=5)
    ledger = tmp_path / "l.jsonl"
    with (
        models.Budget(5, ledger) as budget,
        pytest.raises(procs.UnsupportedPlatform, match="Python 3.13"),
    ):
        models.command_ask(cmd, budget=budget)
    assert not marker.exists()
    assert ledger.read_text() == ""


def test_models_run_without_os_waitid_exits_2_and_spawns_and_reserves_nothing(
    tmp_path, monkeypatch, capsys
):
    monkeypatch.delattr(os, "waitid")
    marker = tmp_path / "spawned"
    body = f"import pathlib; pathlib.Path({str(marker)!r}).write_text('x')\n"
    command = f"{sys.executable} {script(tmp_path, 'touch.py', body)}"
    tasks, roster_ = author_tasks(2), roster(command)
    monkeypatch.setattr(cli, "_model_tasks", lambda args: (tasks, roster_))
    out = tmp_path / "r.json"
    code = cli.main(
        [
            "models",
            "run",
            "--confirm-calls",
            str(models.plan(tasks, roster_, 3)["total_calls"]),
            "--out",
            str(out),
            "--models",
            "unused.json",
            "--max-invocations",
            "100",
        ]
    )
    assert code == 2
    assert "Python 3.13" in capsys.readouterr().err
    assert not marker.exists()
    ledger = out.with_suffix(".ledger.jsonl")
    reserved = [x for x in ledger.read_text().splitlines() if '"reserve"' in x]
    assert reserved == [] and not out.exists()
