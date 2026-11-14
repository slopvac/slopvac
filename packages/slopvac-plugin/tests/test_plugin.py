"""SDK behaviour, seen from the wire."""

import pytest
from conftest import script
from slopvac_plugin.conformance.cases import error_of, handshake, host_params, result_of
from slopvac_plugin.conformance.session import CaseFailure

DOC = {"id": "d1", "text": "héllo"}  # 6 bytes


def tag_params(hook: str, **extra) -> dict:
    return {"tagger": hook, "documents": [DOC], **extra}


def raw_frame(body: bytes) -> bytes:
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def test_reply_lists_registered_hooks_and_defaults_max_in_flight_to_one(start):
    s = start(script("all_hooks_plugin.py"))
    reply = handshake(s)
    assert reply["capabilities"]["hooks"] == [
        {"kind": "tagger", "id": "words"},
        {"kind": "extractor", "id": "lines"},
        {"kind": "reporter", "id": "plain"},
        {"kind": "fixer", "id": "upper"},
        {"kind": "scorer", "id": "count"},
        {"kind": "provider", "id": "echo"},
        {"kind": "recipe", "id": "files"},
    ]
    assert reply["capabilities"]["requiresNetwork"] is True
    assert reply["limits"]["maxInFlight"] == 1


@pytest.mark.parametrize(
    ("proposed", "expected"),
    [(65536, 65536), (16 * 1024 * 1024, 16 * 1024 * 1024), (10**9, 16 * 1024 * 1024)],
)
def test_message_limit_is_the_lower_of_host_and_plugin(start, proposed, expected):
    s = start(script("all_hooks_plugin.py"))
    reply = result_of(s.request("initialize", host_params(max_message_bytes=proposed)))
    assert reply["limits"]["maxMessageBytes"] == expected


def test_duplicate_key_is_a_framing_error_and_the_plugin_exits_1(start):
    s = start(script("all_hooks_plugin.py"))
    s.send_bytes(
        raw_frame(b'{"jsonrpc":"2.0","jsonrpc":"2.0","id":1,"method":"initialize"}')
    )
    assert s.wait(5) == 1


def test_invalid_json_gets_parse_error_with_null_id_and_the_plugin_goes_on(start):
    s = start(script("all_hooks_plugin.py"))
    s.send_bytes(raw_frame(b"{not json"))
    reply = s.next_frame()
    assert reply["id"] is None
    error_of(reply, -32700, "parse-error")
    handshake(s)


def test_top_level_array_gets_invalid_request(start):
    s = start(script("all_hooks_plugin.py"))
    s.send_bytes(raw_frame(b'[{"jsonrpc":"2.0","id":1,"method":"initialize"}]'))
    reply = s.next_frame()
    assert reply["id"] is None
    error_of(reply, -32600, "invalid-request")


@pytest.mark.parametrize(
    "bad_id", ["1.0", "1e2", "9007199254740992", "0", '"1"', "null", "-1"]
)
def test_request_id_must_be_an_integer_from_1_to_2_53_minus_1(start, bad_id):
    s = start(script("all_hooks_plugin.py"))
    body = b'{"jsonrpc":"2.0","id":' + bad_id.encode() + b',"method":"shutdown"}'
    s.send_bytes(raw_frame(body))
    reply = s.next_frame()
    assert reply["id"] is None
    error_of(reply, -32600, "invalid-request")


def test_integer_field_given_as_float_is_invalid_params_with_the_id_kept(start):
    s = start(script("all_hooks_plugin.py"))
    handshake(s)
    params = {
        "fixer": "upper",
        "documents": [
            {
                "id": "d",
                "text": "abc",
                "findings": [
                    {
                        "rule": "a:b",
                        "severity": "warning",
                        "span": {"start": 0.0, "end": 2},
                        "message": "m",
                    }
                ],
            }
        ],
    }
    reply = s.request("fixer/fix", params)
    assert reply["id"] == 2
    error_of(reply, -32602, "invalid-params")


def test_second_initialize_is_invalid_request(start):
    s = start(script("all_hooks_plugin.py"))
    handshake(s)
    error_of(s.request("initialize", host_params()), -32600, "invalid-request")


def test_required_feature_the_host_lacks_stops_the_load(start):
    s = start(script("behaviour_plugin.py", "needs-feature"))
    error = error_of(
        s.request("initialize", host_params()), -32002, "required-feature-missing"
    )
    assert error["data"]["feature"] == "x-needed"


def test_granted_features_reach_the_hook(start):
    s = start(script("behaviour_plugin.py"))
    handshake(s)
    result = result_of(s.request("tagger/tag", tag_params("features")))
    assert result["results"][0]["tags"][0]["evidence"] == "document-path"


def test_span_past_the_end_is_never_sent(start):
    s = start(script("behaviour_plugin.py"))
    handshake(s)
    reply = s.request("tagger/tag", tag_params("bad-span"))
    error_of(reply, -32603, "internal-error")


def test_crash_in_a_hook_is_internal_error_and_the_log_omits_the_message(start):
    s = start(script("behaviour_plugin.py"))
    handshake(s)
    error_of(s.request("tagger/tag", tag_params("crash")), -32603, "internal-error")
    result_of(s.request("tagger/tag", tag_params("features")))  # still alive
    assert "secret document text" not in s.stderr_tail()
    assert "RuntimeError" in s.stderr_tail()


def test_reply_over_the_limit_becomes_message_too_large(start):
    s = start(script("behaviour_plugin.py"))
    result_of(s.request("initialize", host_params(max_message_bytes=65536)))
    s.notify("initialized", {})
    error_of(s.request("tagger/tag", tag_params("huge")), -32004, "message-too-large")


def test_cancel_stops_a_running_hook_with_request_cancelled(start):
    s = start(script("behaviour_plugin.py"))
    handshake(s)
    request_id = s.start_request("tagger/tag", tag_params("slow"))
    s.notify("$/cancelRequest", {"id": request_id})
    error_of(s.response_to(request_id, 4), -32005, "request-cancelled")


def test_stdout_stays_clean_when_a_hook_prints_logs_and_spawns_children(start):
    s = start(script("behaviour_plugin.py"))
    handshake(s)
    result_of(s.request("tagger/tag", tag_params("chatty")))
    result_of(s.request("shutdown"))
    s.notify("exit")
    assert s.wait(5) == 0
    assert [kind for kind, _ in s.drain_events()] == ["eof"]
    assert "print to stdout" in s.stderr_tail()


def test_two_requests_run_at_once_when_max_in_flight_is_two(start):
    s = start(script("behaviour_plugin.py", "in-flight-2"))
    reply = handshake(s)
    assert reply["limits"]["maxInFlight"] == 2
    ids = [s.start_request("tagger/tag", tag_params("gate")) for _ in range(2)]
    answers = {m["id"]: m for m in (s.next_frame(8), s.next_frame(8))}
    assert sorted(answers) == ids
    assert all("result" in m for m in answers.values())


def test_a_request_while_not_initialized_is_not_a_hang(start):
    s = start(script("all_hooks_plugin.py"))
    error_of(s.request("tagger/tag", tag_params("words")), -32003, "not-initialized")
    with pytest.raises(CaseFailure):
        s.next_frame(0.3)  # nothing else arrives


@pytest.mark.parametrize("ending", ["end of input", "framing error"])
def test_a_plugin_with_a_busy_hook_still_ends_at_once(start, ending):
    s = start(script("behaviour_plugin.py"))
    handshake(s)
    s.start_request("tagger/tag", tag_params("stuck"))  # busy for 30 s, maxInFlight 1
    if ending == "end of input":
        s.close_stdin()
    else:
        s.send_bytes(b"Content-Length: nope\r\n\r\n")
    assert s.wait(10) == 1
