import io

import pytest
from slopvac_plugin.framing import (
    FramingError,
    ParseError,
    UnsafeInt,
    encode_frame,
    loads_strict,
    read_frame,
)


def read(data: bytes, limit: int = 1000) -> bytes | None:
    return read_frame(io.BufferedReader(io.BytesIO(data)), lambda: limit)


def test_encode_counts_body_bytes_not_characters():
    # "é" is 2 bytes in UTF-8, so the body {"a":"é"} is 10 bytes
    body = '{"a":"é"}'.encode()
    assert encode_frame(body) == b"Content-Length: 10\r\n\r\n" + body


@pytest.mark.parametrize(
    "frame",
    [
        b"Content-Length: 2\r\n\r\n{}",
        b"content-length:2\r\n\r\n{}",
        b"Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
        b"Content-Length: 2\r\nX-Other: 1\r\n\r\n{}",
        b"Content-Length: 2\r\n"
        + b"X-A: "
        + b"a" * 1019
        + b"\r\n\r\n{}",  # 1024-byte line
        b"Content-Length: 2\r\n" + b"X: 1\r\n" * 7 + b"\r\n{}",  # 8 header lines
    ],
)
def test_reader_accepts_valid_frames(frame):
    assert read(frame) == b"{}"


def test_clean_end_of_stream_is_none():
    assert read(b"") is None


@pytest.mark.parametrize(
    "frame",
    [
        b"Content-Length: +2\r\n\r\n{}",
        b"Content-Length: 0x2\r\n\r\n{}",
        b"Content-Length: 02\r\n\r\n{}",
        b"Content-Length: -2\r\n\r\n{}",
        b"Content-Length:\r\n\r\n{}",
        b"Content-Length: 2\n\n{}",
        b"Content-Length: 2\r\n\n{}",
        b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
        b"Content-Length: 2\r\nContent-Length: 3\r\n\r\n{}",
        b"Content-Type: text/plain\r\n\r\n{}",
        b"Content-Length 2\r\n\r\n{}",
        b"Content-Length: 18446744073709551616\r\n\r\n{}",  # 2^64
        b"Content-Length: 2\r\n"
        + b"X-A: "
        + b"a" * 1020
        + b"\r\n\r\n{}",  # 1025-byte line
        b"Content-Length: 2\r\n" + b"X: 1\r\n" * 8 + b"\r\n{}",  # 9 header lines
        b"Content-Length: 2\r\n\r\n{",  # body stops halfway
        b"Content-Length: 2\r\n",  # headers stop halfway
    ],
)
def test_reader_rejects_framing_errors(frame):
    with pytest.raises(FramingError):
        read(frame)


def test_oversized_length_is_rejected_before_the_body_is_read():
    # The body is absent. A reader that tried to read it would report a short body.
    with pytest.raises(FramingError, match="exceeds the limit of 99"):
        read(b"Content-Length: 100\r\n\r\n", limit=99)


def test_body_at_the_limit_is_accepted():
    assert read(b"Content-Length: 2\r\n\r\n{}", limit=2) == b"{}"


def test_limit_is_read_after_the_headers():
    # A limit negotiated while the reader waited for a frame applies to that frame.
    limits = iter([10])
    stream = io.BufferedReader(io.BytesIO(b"Content-Length: 2\r\n\r\n{}"))
    assert read_frame(stream, lambda: next(limits)) == b"{}"


@pytest.mark.parametrize(
    ("body", "expected"),
    [
        (b'{"a":[1,2,{"b":null}]}', {"a": [1, 2, {"b": None}]}),
        (b'{"n":9007199254740991}', {"n": 9007199254740991}),
        (b'{"n":-9007199254740991}', {"n": -9007199254740991}),
    ],
)
def test_strict_json_accepts_valid_bodies(body, expected):
    assert loads_strict(body) == expected


@pytest.mark.parametrize(
    "body", [b'{"a":1,"a":2}', b'{"o":{"a":1,"a":1}}', b'[{"a":1,"a":2}]']
)
def test_duplicate_keys_are_framing_errors(body):
    with pytest.raises(FramingError):
        loads_strict(body)


def test_depth_64_is_accepted_and_65_is_not():
    assert loads_strict(b"[" * 64 + b"]" * 64) is not None
    with pytest.raises(FramingError):
        loads_strict(b"[" * 65 + b"]" * 65)


def test_extreme_depth_is_a_framing_error_not_a_crash():
    with pytest.raises(FramingError):
        loads_strict(b"[" * 200000)


@pytest.mark.parametrize(
    "body",
    [b"{", b"", b"NaN", b'{"a":Infinity}', b"\xef\xbb\xbf{}", b'"\xff"', b"{'a':1}"],
)
def test_other_invalid_bodies_are_parse_errors(body):
    with pytest.raises(ParseError):
        loads_strict(body)


def test_integer_beyond_2_53_is_not_a_plain_int():
    value = loads_strict(b'{"n":9007199254740992}')["n"]  # 2^53
    assert isinstance(value, UnsafeInt) and type(value) is not int


@pytest.mark.parametrize("body", [b'{"n":1.0}', b'{"n":1e2}'])
def test_float_spellings_are_not_ints(body):
    assert type(loads_strict(body)["n"]) is float


def test_huge_integer_does_not_crash_the_parser():
    value = loads_strict(b'{"n":' + b"9" * 5000 + b"}")["n"]
    assert type(value) is not int
