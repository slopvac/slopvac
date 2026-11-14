"""Thin Python SDK for slopvac plugin protocol v1."""

from .errors import DocumentInvalid, RequestCancelled, RpcError
from .plugin import Plugin
from .types import (
    Completion,
    Context,
    Document,
    Edit,
    Finding,
    Item,
    RecipeFile,
    Rendered,
    Segment,
    Span,
    Tag,
)

__all__ = [
    "Completion",
    "Context",
    "Document",
    "DocumentInvalid",
    "Edit",
    "Finding",
    "Item",
    "Plugin",
    "RecipeFile",
    "Rendered",
    "RequestCancelled",
    "RpcError",
    "Segment",
    "Span",
    "Tag",
]
