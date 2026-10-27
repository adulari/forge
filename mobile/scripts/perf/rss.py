#!/usr/bin/env python3
"""Resident memory of a desktop instance (app + WebKit helper processes) and its DOM size.

    python3 rss.py INSTANCE_NAME PORT

INSTANCE_NAME is the run-app.sh name (its XDG_DATA_HOME identifies the process). Prints total RSS
over the app process tree in MiB and the number of DOM nodes in the page.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from inspector import Inspector  # noqa: E402


def children(pid):
    out = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            stat = open(f"/proc/{entry}/stat").read()
            ppid = int(stat.rsplit(")", 1)[1].split()[1])
        except OSError:
            continue
        if ppid == pid:
            out.append(int(entry))
    return out


def tree(pid):
    pids = [pid]
    for child in children(pid):
        pids += tree(child)
    return pids


def rss_mib(pid):
    try:
        for line in open(f"/proc/{pid}/status"):
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024
    except OSError:
        pass
    return 0.0


def find_app(name):
    marker = f"XDG_DATA_HOME=/home/floris/.cache/forge-apptest/inst/{name}/data"
    for entry in os.listdir("/proc"):
        if entry.isdigit():
            try:
                env = open(f"/proc/{entry}/environ").read().split("\0")
                exe = os.path.basename(os.readlink(f"/proc/{entry}/exe"))
            except OSError:
                continue
            if marker in env and "forge-desktop" in exe:
                return int(entry)
    raise SystemExit(f"no app for instance {name}")


if __name__ == "__main__":
    name, port = sys.argv[1], sys.argv[2]
    pid = find_app(name)
    total = sum(rss_mib(p) for p in tree(pid))
    nodes = Inspector(port).eval("document.querySelectorAll('*').length")
    print(f"rss_mib={total:.0f} dom_nodes={nodes}")
