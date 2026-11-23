"""Runs a command and reports its wall time and peak working set (Windows; fix_export item 9, the 4096 export).

Usage: python -E -P peak_mem.py COMMAND [ARGS ...]
Prints the command's output, then `peak working set: N MB, wall: S s, exit: C`.
The peak is the process's own PeakWorkingSetSize (GetProcessMemoryInfo), read while it runs and once more at exit
(the handle stays open, so the counter survives the exit).
"""

import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time


class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("cb", wt.DWORD),
        ("PageFaultCount", wt.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def peak(handle):
    c = PROCESS_MEMORY_COUNTERS()
    c.cb = ctypes.sizeof(c)
    if ctypes.windll.psapi.GetProcessMemoryInfo(handle, ctypes.byref(c), c.cb):
        return c.PeakWorkingSetSize, c.PeakPagefileUsage
    return 0, 0


def main(argv):
    start = time.perf_counter()
    proc = subprocess.Popen(argv)
    handle = int(proc._handle)
    best = (0, 0)
    while proc.poll() is None:
        p = peak(handle)
        best = (max(best[0], p[0]), max(best[1], p[1]))
        time.sleep(0.05)
    p = peak(handle)
    best = (max(best[0], p[0]), max(best[1], p[1]))
    wall = time.perf_counter() - start
    print("peak working set: %.0f MB, peak commit: %.0f MB, wall: %.1f s, exit: %d"
          % (best[0] / 2**20, best[1] / 2**20, wall, proc.returncode))
    return proc.returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
