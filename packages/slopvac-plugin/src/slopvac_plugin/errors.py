"""Error codes of plugin protocol v1 (docs/contract/plugin-protocol.md, section 10)."""

from __future__ import annotations

from typing import Any

PARSE_ERROR = (-32700, "parse-error")
INVALID_REQUEST = (-32600, "invalid-request")
METHOD_NOT_FOUND = (-32601, "method-not-found")
INVALID_PARAMS = (-32602, "invalid-params")
INTERNAL_ERROR = (-32603, "internal-error")
PROTOCOL_VERSION_UNSUPPORTED = (-32001, "protocol-version-unsupported")
REQUIRED_FEATURE_MISSING = (-32002, "required-feature-missing")
NOT_INITIALIZED = (-32003, "not-initialized")
MESSAGE_TOO_LARGE = (-32004, "message-too-large")
REQUEST_CANCELLED = (-32005, "request-cancelled")
DOCUMENT_INVALID = (-32006, "document-invalid")
OFFLINE = (-32007, "offline")


class RpcError(Exception):
    """An error the plugin answers with. Handlers may raise it."""

    def __init__(
        self,
        kind: tuple[int, str],
        message: str,
        data: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(message)
        self.code, self.slopvac_code = kind
        self.message = message
        self.data = data or {}

    def to_wire(self) -> dict[str, Any]:
        return {
            "code": self.code,
            "message": self.message,
            "data": {**self.data, "slopvac_code": self.slopvac_code},
        }


class DocumentInvalid(RpcError):
    """Raise from a per-document handler to fail one document and keep the batch."""

    def __init__(self, message: str = "the document cannot be processed") -> None:
        super().__init__(DOCUMENT_INVALID, message)


class RequestCancelled(RpcError):
    """Raised by ``Context.raise_if_cancelled`` after the host cancelled the request."""

    def __init__(self) -> None:
        super().__init__(REQUEST_CANCELLED, "request cancelled")


class OutputInvalid(Exception):
    """A handler returned a value the contract forbids. The SDK never sends it."""
