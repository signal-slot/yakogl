#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The implementation of tools/cts/deps-env.sh; read its header for the interface.

The entries come from tools/cts/deps.toml (DEPS_TOML names another file, for
the self-test). Run through deps-env.sh, which supplies python3 -I.
"""
import hashlib
import os
import shutil
import socket
import subprocess
import sys
import urllib.request
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    sys.exit("deps-env.sh: python3 >= 3.11 (tomllib) is required")

BOARDS = ("am62p", "imx8mp", "rpi_4b")
HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TOML = Path(os.environ.get("DEPS_TOML") or HERE / "deps.toml")
KEYS = {"name", "aliases", "path", "command", "marker", "board", "create", "requires",
        "build", "note", "inputs", "packages", "rustup_target"}
INPUT_KEYS = {"url", "sha256", "sha256_from"}
FAMILIES = ("apt", "portage", "brew")


class FetchError(Exception):
    """An input that cannot be had or does not match its digest."""


def die(msg, rc=2):
    print(f"deps-env.sh: {msg}", file=sys.stderr)
    sys.exit(rc)


def load():
    try:
        with open(TOML, "rb") as f:
            deps = tomllib.load(f).get("dep", [])
    except (OSError, tomllib.TOMLDecodeError) as e:
        die(f"{TOML}: {e}")
    seen = set()
    for d in deps:
        name = d.get("name") or die(f"{TOML}: an entry has no name")
        if bad := set(d) - KEYS:
            die(f"{TOML}: {name}: unknown key(s) {sorted(bad)}")
        for n in [name, *d.get("aliases", [])]:
            if n in seen:
                die(f"{TOML}: {n} is named twice")
            seen.add(n)
        if sum(k in d for k in ("path", "command", "rustup_target")) != 1:
            die(f"{TOML}: {name}: exactly one of path, command and rustup_target")
        if "path" in d and "marker" not in d:
            die(f"{TOML}: {name}: a path needs a marker")
        if d.get("board", "*") not in (*BOARDS, "*"):
            die(f"{TOML}: {name}: unknown board {d['board']!r}")
        for i in d.get("inputs", []):
            if bad := set(i) - INPUT_KEYS:
                die(f"{TOML}: {name}: unknown input key(s) {sorted(bad)}")
            if "url" not in i or sum(k in i for k in INPUT_KEYS - {"url"}) != 1:
                die(f"{TOML}: {name}: an input needs a url and one of sha256, sha256_from")
        if bad := set(d.get("packages", {})) - set(FAMILIES):
            die(f"{TOML}: {name}: unknown package family {sorted(bad)}")
    by_name = {d["name"]: d for d in deps}
    for d in deps:
        for r in d.get("requires", []):
            if r not in by_name:
                die(f"{TOML}: {d['name']}: requires unknown entry {r}")
    return deps, by_name


# connected's dependency layout for this project (`<data_dir>/deps/yakogl`).
HOME_LAYOUT = ".local/share/connected/deps/yakogl"
# The name the layout had before ccfarm became connected. Gate images built
# before 2026-10-26 hold their trees there; read only on a machine without
# HOME_LAYOUT, and dropped once those images are rebuilt.
OLD_HOME_LAYOUT = ".local/share/ccfarm/deps/yakogl"


def layout():
    """The layout directory: the one connected gives a turn (CONNECTED_DEPS_DIR,
    or CCFARM_DEPS_DIR, which it sets too), else ~/HOME_LAYOUT."""
    for name in ("CONNECTED_DEPS_DIR", "CCFARM_DEPS_DIR"):
        if os.environ.get(name):
            return Path(os.environ[name])
    home = Path.home() / HOME_LAYOUT
    old = Path.home() / OLD_HOME_LAYOUT
    return old if not home.is_dir() and old.is_dir() else home


def in_set(d, board):
    b = d.get("board")
    return b is None or (board != "" and b in ("*", board))


def resolve(d, deps, board):
    """The (value, marker) of an entry; board-less use of a {board} path is an error."""
    if "command" in d:
        p = shutil.which(d["command"]) or f"/usr/bin/{d['command']}"
        return p, p
    if "rustup_target" in d:
        # Installed per toolchain: the sysroot of the rustc in effect
        # (RUSTUP_TOOLCHAIN, else the default) holds lib/rustlib/<target>/lib
        # only when `rustup target add` has run for it.
        # The gate builds with rustup's toolchains, so the rustup shim
        # ~/.cargo/bin/rustc is asked first (it honours RUSTUP_TOOLCHAIN): the
        # CI shell may not have ~/.cargo/bin on PATH at this step (runner C's
        # login shell never has it), and a distro rustc on PATH would answer
        # for the wrong toolchain.
        shim = os.path.expanduser("~/.cargo/bin/rustc")
        rustc = shim if os.access(shim, os.X_OK) else (shutil.which("rustc") or "rustc")
        try:
            root = subprocess.run([rustc, "--print", "sysroot"], capture_output=True,
                                  text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            root = "/nonexistent-rustc-sysroot"
        p = f"{root}/lib/rustlib/{d['rustup_target']}/lib"
        return p, p
    if "{board}" in d["path"] + d["marker"]:
        if not board:
            die(f"{d['name']} is per board: name one (--board)")
    sub = lambda s: s.replace("{board}", board)
    marker = deps if d["marker"] == "." else deps / sub(d["marker"])
    return str(deps / sub(d["path"])), str(marker)


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def expected_sha(i):
    if "sha256" in i:
        return i["sha256"].lower()
    name = i["url"].rsplit("/", 1)[1]
    if "sha256_from" in i:
        rel, key = i["sha256_from"].rsplit(":", 1)
        for line in (ROOT / rel).read_text().splitlines():
            if line.startswith(key + ":"):
                return line.split(":", 1)[1].strip().lower()
        raise FetchError(f"no '{key}:' line in {rel}")
    raise FetchError(f"{name}: no pinned digest")  # load() rules this out


def fetch(i, deps):
    dest = deps / "downloads" / i["url"].rsplit("/", 1)[1]
    want = expected_sha(i)
    if dest.is_file() and sha256_of(dest) == want:
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    print(f"fetch    {i['url']}")
    with urllib.request.urlopen(i["url"]) as r, open(part, "wb") as f:
        shutil.copyfileobj(r, f)
    got = sha256_of(part)
    if got != want:
        part.unlink()
        raise FetchError(f"sha256 {got}, expected {want}")
    part.rename(dest)


def build(d, by_name, deps, board, done):
    """Make an entry; True when it is usable afterwards. Never touches a present one."""
    name = d["name"]
    _, marker = resolve(d, deps, board)
    if name in done:
        return done[name]
    done[name] = False
    if os.path.exists(marker):
        print(f"present  {name}  {marker}")
        done[name] = True
        return True
    for r in d.get("requires", []):
        if not build(by_name[r], by_name, deps, board, done):
            print(f"failed   {name}: needs {r}")
            return False
    if "build" not in d:
        print(f"nobuild  {name}: {d.get('note', 'no build recipe')}")
        return False
    print(f"build    {name}")
    for i in d.get("inputs", []):
        try:
            fetch(i, deps)
        except (OSError, FetchError) as e:
            print(f"failed   {name}: {i['url']}: {e}")
            return False
    env = {**os.environ, "DEPS": str(deps), "CCFARM_DEPS_DIR": str(deps), "BOARD_NAME": board}
    rc = subprocess.run(["bash", "-eu", "-o", "pipefail", "-c", d["build"]], cwd=ROOT, env=env).returncode
    if rc != 0:
        print(f"failed   {name}: the build exited {rc}")
        return False
    if not os.path.exists(marker):
        print(f"failed   {name}: the build finished but {marker} is not there")
        return False
    print(f"built    {name}  {marker}")
    done[name] = True
    return True


def main(argv):
    mode, board, names = "env", "", []
    args = list(argv)
    while args:
        a = args.pop(0)
        if a == "--check":
            mode = "check"
        elif a == "--layout":
            mode = "layout"
        elif a == "--build":
            mode = "build"
        elif a == "--build-missing":
            mode = "build-missing"
        elif a == "--packages":
            mode = "packages"
            if not args:
                die("--packages needs a family (" + " | ".join(FAMILIES) + ")")
            names = [args.pop(0)]
        elif a == "--board":
            if not args:
                die("--board needs a board name")
            board = args.pop(0)
        elif a.startswith("-"):
            die(f"unknown option {a}")
        elif mode == "build":
            names.append(a)
        elif not board:
            board = a
        else:
            die(f"unexpected argument {a}")
    if board not in ("", *BOARDS):
        die(f"unknown board '{board}' (" + " | ".join(BOARDS) + ")")
    if mode == "build" and not names:
        die("--build needs at least one entry name")

    all_deps, by_name = load()
    for d in all_deps:
        for a in d.get("aliases", []):
            by_name.setdefault(a, d)

    if mode == "packages":
        fam = names[0]
        if fam not in FAMILIES:
            die(f"unknown package family '{fam}' (" + " | ".join(FAMILIES) + ")")
        seen = {}
        for d in all_deps:
            for p in d.get("packages", {}).get(fam, []):
                seen.setdefault(p)
        print("\n".join(seen))
        return 0

    deps = layout()
    if mode == "layout":
        print(deps)
        return 0
    if mode in ("build", "build-missing"):
        deps.mkdir(parents=True, exist_ok=True)
    elif not deps.is_dir():
        die(f"no dependency layout at {deps} on {socket.gethostname()}", 1)
    sel = [d for d in all_deps if in_set(d, board)]

    if mode == "build":
        try:
            sel = [by_name[n] for n in names]
        except KeyError as e:
            die(f"unknown entry {e.args[0]}")
    if mode in ("build", "build-missing"):
        done = {}
        bad = [d["name"] for d in sel if not build(d, by_name, deps, board, done)]
        if bad:
            print(f"deps-env.sh: not made: {' '.join(bad)}")
            return 1
        return 0

    missing = 0
    if mode == "check":
        print(f"layout   {deps}")
    for d in sel:
        value, marker = resolve(d, deps, board)
        if mode == "check":
            ok = os.path.exists(marker)
            print(f"{'ok' if ok else 'missing':<9}{d['name']}  {marker}")
            missing += not ok
            continue
        if d.get("create"):
            os.makedirs(value, exist_ok=True)
        for n in (d["name"], *d.get("aliases", [])):
            print(f"{n}={value}")
    if missing:
        print(f"deps-env.sh: {missing} entr{'y' if missing == 1 else 'ies'} missing in {deps}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
