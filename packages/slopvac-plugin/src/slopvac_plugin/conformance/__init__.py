"""Conformance suite for slopvac plugins, in any language."""

from .runner import CaseResult, Report, load_declaration, run_conformance
from .session import SuiteError

__all__ = ["CaseResult", "Report", "SuiteError", "load_declaration", "run_conformance"]
