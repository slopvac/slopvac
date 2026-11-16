"""A Windows Job Object that holds the plugin and every process it starts (TM-4, P5).

``taskkill /T`` walks the parent links from a live process. Once the plugin has
exited, its children are orphans and the walk finds nothing. A job keeps the
membership, so ``TerminateJobObject`` ends the descendants after their parent is gone.

Only the standard library is used. Nothing here touches a Windows API unless
``os.name == "nt"``, so other platforms import the module and get ``None`` back.

The job is assigned right after ``Popen`` returns. Python's ``subprocess`` cannot
start a process suspended, so a child that the plugin starts in the first
microseconds could escape the job. ``kill_tree`` keeps ``taskkill`` as a fallback.
"""

from __future__ import annotations

import ctypes
import os
import subprocess
from typing import Any

JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS = 9
PROCESS_SET_QUOTA = 0x0100
PROCESS_TERMINATE = 0x0001


def _api() -> Any:
    """Kernel32 with the argument types set, so 64-bit handles are not truncated."""
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)  # type: ignore[attr-defined]
    kernel32.CreateJobObjectW.argtypes = [wintypes.LPVOID, wintypes.LPCWSTR]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        wintypes.LPVOID,
        wintypes.DWORD,
    ]
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateJobObject.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    return kernel32


def _limit_information() -> Any:
    """``JOBOBJECT_EXTENDED_LIMIT_INFORMATION`` with only KILL_ON_JOB_CLOSE set."""
    from ctypes import wintypes

    class BasicLimits(ctypes.Structure):
        _fields_ = (
            ("PerProcessUserTimeLimit", ctypes.c_int64),
            ("PerJobUserTimeLimit", ctypes.c_int64),
            ("LimitFlags", wintypes.DWORD),
            ("MinimumWorkingSetSize", ctypes.c_size_t),
            ("MaximumWorkingSetSize", ctypes.c_size_t),
            ("ActiveProcessLimit", wintypes.DWORD),
            ("Affinity", ctypes.c_size_t),
            ("PriorityClass", wintypes.DWORD),
            ("SchedulingClass", wintypes.DWORD),
        )

    class IoCounters(ctypes.Structure):
        _fields_ = tuple((name, ctypes.c_uint64) for name in _IO_COUNTER_NAMES)

    class ExtendedLimits(ctypes.Structure):
        _fields_ = (
            ("BasicLimitInformation", BasicLimits),
            ("IoInfo", IoCounters),
            ("ProcessMemoryLimit", ctypes.c_size_t),
            ("JobMemoryLimit", ctypes.c_size_t),
            ("PeakProcessMemoryUsed", ctypes.c_size_t),
            ("PeakJobMemoryUsed", ctypes.c_size_t),
        )

    info = ExtendedLimits()
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    return info


_IO_COUNTER_NAMES = (
    "ReadOperationCount",
    "WriteOperationCount",
    "OtherOperationCount",
    "ReadTransferCount",
    "WriteTransferCount",
    "OtherTransferCount",
)


class JobObject:
    """A job whose members die when ``terminate`` runs or the last handle closes."""

    def __init__(self, handle: int) -> None:
        self._handle: int | None = handle

    @classmethod
    def around(cls, proc: subprocess.Popen[bytes]) -> JobObject | None:
        """Put ``proc`` in a new job. ``None`` off Windows or when any step fails."""
        if os.name != "nt":
            return None
        try:
            kernel32 = _api()
            handle = kernel32.CreateJobObjectW(None, None)
            if not handle:
                return None
            info = _limit_information()
            ok = kernel32.SetInformationJobObject(
                handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                ctypes.byref(info),
                ctypes.sizeof(info),
            )
            process = kernel32.OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE, False, proc.pid
            )
            assigned = bool(process) and kernel32.AssignProcessToJobObject(
                handle, process
            )
            if process:
                kernel32.CloseHandle(process)
            if not (ok and assigned):
                kernel32.CloseHandle(handle)
                return None
            return cls(handle)
        except (OSError, AttributeError, ImportError):
            return None

    def terminate(self) -> None:
        """End every process in the job, then release the handle. Safe to repeat."""
        if self._handle is None:
            return
        handle, self._handle = self._handle, None
        kernel32 = _api()
        kernel32.TerminateJobObject(handle, 1)
        kernel32.CloseHandle(handle)
