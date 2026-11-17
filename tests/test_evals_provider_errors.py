"""Provider stderr must not carry credentials into error messages."""

from __future__ import annotations

import shlex
import sys

import pytest

from evals import labelling, runner, twins

FAKE_KEY = "AKIAFAKEFAKEFAKE0000"
FAKE_SECRET = "FAKEsecretFAKEsecretFAKEsecretFAKEsecret00"


def leaky_command() -> str:
    script = (
        "import sys; sys.stderr.write("
        f"'AWS_ACCESS_KEY_ID={FAKE_KEY} AWS_SECRET_ACCESS_KEY={FAKE_SECRET}'); "
        "sys.exit(3)"
    )
    return shlex.join([sys.executable, "-c", script])


def test_labelling_provider_errors_do_not_echo_credentials_from_stderr():
    with pytest.raises(runner.EngineError) as caught:
        labelling.command_asker(leaky_command())("prompt")
    message = str(caught.value)
    assert "exited 3" in message
    assert FAKE_KEY not in message
    assert FAKE_SECRET not in message


def test_twin_provider_errors_do_not_echo_credentials_from_stderr():
    with pytest.raises(RuntimeError) as caught:
        twins.run_provider(leaky_command(), "prompt", 30)
    message = str(caught.value)
    assert "exited 3" in message
    assert FAKE_KEY not in message
    assert FAKE_SECRET not in message
