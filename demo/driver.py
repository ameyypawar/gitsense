#!/usr/bin/env python3
"""
gitsense MCP demo driver.
Spawns gitsense-mcp over stdio, performs the MCP handshake, then runs
blame_symbol + find_dead_code and pretty-prints the results.

Usage (from repo root):
  python3 demo/driver.py
"""
import json
import os
import subprocess
import sys
import time

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY    = os.path.join(REPO_ROOT, "target", "release", "gitsense-mcp")
TARGET    = "/tmp/anyhow-demo"   # dtolnay/anyhow — the hosted demo repo


# ── helpers ───────────────────────────────────────────────────────────────────

def _send(proc, obj):
    line = json.dumps(obj) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()


def _recv(proc):
    while True:
        line = proc.stdout.readline()
        if not line:
            raise EOFError("gitsense-mcp closed stdout unexpectedly")
        line = line.strip()
        if line:
            return json.loads(line)


def rpc(proc, method, params, id_):
    _send(proc, {"jsonrpc": "2.0", "id": id_, "method": method, "params": params})
    return _recv(proc)


def print_section(label, data):
    """Print a labelled, indented JSON block."""
    formatted = json.dumps(data, indent=2)
    # Indent every line by 2 spaces for visual nesting
    indented = "\n".join("  " + l for l in formatted.splitlines())
    print(indented)


# ── ANSI helpers ──────────────────────────────────────────────────────────────

BOLD  = "\033[1m"
DIM   = "\033[2m"
CYAN  = "\033[36m"
GREEN = "\033[32m"
YELLOW= "\033[33m"
RESET = "\033[0m"

def c(text, *codes):
    return "".join(codes) + text + RESET


# ── main ──────────────────────────────────────────────────────────────────────

def main():
    # Clone target if not already present
    if not os.path.isdir(TARGET):
        print(c("  cloning dtolnay/anyhow …", DIM))
        subprocess.run(
            ["git", "clone", "--quiet", "https://github.com/dtolnay/anyhow", TARGET],
            check=True,
        )

    # Spawn the MCP server
    proc = subprocess.Popen(
        [BINARY, "--repo-path", TARGET],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,   # suppress tracing logs in the recording
    )

    try:
        # ── MCP handshake ─────────────────────────────────────────────────────
        rpc(proc, "initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "gitsense-demo", "version": "0.1"},
        }, 1)
        _send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
        time.sleep(0.3)

        # ── Header ────────────────────────────────────────────────────────────
        print()
        print(c(" gitsense ", BOLD, CYAN) +
              c(" git-history-aware code intelligence (MCP) ", DIM))
        print(c(" target: ", DIM) + c("dtolnay/anyhow", BOLD) +
              c("  ·  ", DIM) + c("~500 commits of history", DIM))
        print()
        time.sleep(0.8)

        # ── 1. blame_symbol "context" ─────────────────────────────────────────
        print(c("▸ blame_symbol", GREEN, BOLD) +
              c('  name="context"', YELLOW))
        print(c("  who last touched the `context` method and in which commit?", DIM))
        time.sleep(0.5)

        resp = rpc(proc, "tools/call",
                   {"name": "blame_symbol", "arguments": {"name": "context"}}, 2)
        content = resp.get("result", {}).get("content", [])
        if content:
            data = json.loads(content[0]["text"])
            # Show a trimmed, readable subset
            display = {
                "last_author": data.get("last_author"),
                "last_commit": data.get("last_commit_short"),
                "last_date":   data.get("last_date"),
                "last_message": data.get("last_message"),
                "blame_hunks": len(data.get("lines", [])),
            }
            print_section("blame_symbol", display)
        print()
        time.sleep(1.2)

        # ── 2. blame_symbol "provide" ─────────────────────────────────────────
        print(c("▸ blame_symbol", GREEN, BOLD) +
              c('  name="provide"', YELLOW))
        print(c("  newer API — when did this land?", DIM))
        time.sleep(0.5)

        resp = rpc(proc, "tools/call",
                   {"name": "blame_symbol", "arguments": {"name": "provide"}}, 3)
        content = resp.get("result", {}).get("content", [])
        if content:
            data = json.loads(content[0]["text"])
            display = {
                "last_author": data.get("last_author"),
                "last_commit": data.get("last_commit_short"),
                "last_date":   data.get("last_date"),
                "last_message": data.get("last_message"),
                "blame_hunks": len(data.get("lines", [])),
            }
            print_section("blame_symbol", display)
        print()
        time.sleep(1.2)

        # ── 3. find_dead_code ─────────────────────────────────────────────────
        print(c("▸ find_dead_code", GREEN, BOLD) +
              c("  limit=5", YELLOW))
        print(c("  unreferenced symbols — oldest (safest to delete) first", DIM))
        time.sleep(0.5)

        resp = rpc(proc, "tools/call",
                   {"name": "find_dead_code", "arguments": {"limit": 5}}, 4)
        content = resp.get("result", {}).get("content", [])
        if content:
            data = json.loads(content[0]["text"])
            print_section("find_dead_code", data)
        print()
        time.sleep(0.8)

        # ── Footer ────────────────────────────────────────────────────────────
        print(c("  github.com/ameyypawar/gitsense", DIM))
        print()

    finally:
        proc.stdin.close()
        proc.wait(timeout=5)


if __name__ == "__main__":
    main()
