#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The main thread's stack must hold the image's deepest direct call path.

Run after zephyr.elf links (lib/board/CMakeLists.txt says for which images).
Our arm64 Zephyr images have no guard under a thread's stack: z_main_stack
sits right above z_idle_stacks and z_interrupt_stacks, so a call path deeper
than CONFIG_MAIN_STACK_SIZE writes into the idle thread's stack and then into
the interrupt stack, and the image still runs until an interrupt lands there.

    check_main_stack.py <nm> <objdump> <zephyr.elf>
    check_main_stack.py --selftest

What is summed: the frames (sub sp and pre-indexed stores to sp) of the
functions on the deepest path of direct calls (bl) from bg_thread_main, the
main thread's entry. A direct jump to another function (b) is a tail call:
the caller's frame is gone by then. Functions that call each other in a
cycle count once, as the largest frame among them. On top comes one
exception frame (___esf_t_SIZEOF): an interrupt saves it on the running
thread's stack before it switches to the interrupt stack
(arch/arm64/core/vector_table.S, z_arm64_enter_exc).

Not counted, so the result is a lower bound: calls through a register (blr:
printk's output function, init hooks, callbacks), recursion beyond one
frame, and stack taken with a register operand. A path that passes may still
be too deep; a path that fails is too deep.
"""
import re
import subprocess
import sys

FUNC = re.compile(r"^[0-9a-f]+ <([^>]+)>:$")
SUB_SP = re.compile(r"\tsub\tsp, sp, #(0x[0-9a-f]+|\d+)(, lsl #12)?\s*(//.*)?$")
PRE_INDEX = re.compile(r"\tst[a-z0-9]*\t[^\[]*\[sp, #-(0x[0-9a-f]+|\d+)\]!")
BRANCH = re.compile(r"\t(bl|b)\t[0-9a-f]+ <([^>+]+)>$")


def parse(disassembly):
    """-> {function: (frame bytes, called functions, tail-called functions)}."""
    funcs = {}
    cur = None
    for line in disassembly.splitlines():
        m = FUNC.match(line)
        if m:
            cur = m.group(1)
            funcs.setdefault(cur, [0, set(), set()])
            continue
        if cur is None:
            continue
        m = SUB_SP.search(line)
        if m:
            funcs[cur][0] += int(m.group(1), 0) << (12 if m.group(2) else 0)
            continue
        m = PRE_INDEX.search(line)
        if m:
            funcs[cur][0] += int(m.group(1), 0)
            continue
        m = BRANCH.search(line)
        if m and m.group(2) != cur:
            funcs[cur][1 if m.group(1) == "bl" else 2].add(m.group(2))
    return {
        f: (frame, {c for c in calls if c in funcs}, {t for t in tails if t in funcs})
        for f, (frame, calls, tails) in funcs.items()
    }


def cycles(funcs, entry):
    """Tarjan's strongly connected components of what entry reaches, callees
    first, without recursion (a GL image's call graph is deep)."""
    index, low, on_stack, stack, out = {}, {}, set(), [], []
    work = [(entry, iter(sorted(funcs[entry][1] | funcs[entry][2])))]
    index[entry] = low[entry] = 0
    stack.append(entry)
    on_stack.add(entry)
    while work:
        f, it = work[-1]
        for c in it:
            if c not in index:
                index[c] = low[c] = len(index)
                stack.append(c)
                on_stack.add(c)
                work.append((c, iter(sorted(funcs[c][1] | funcs[c][2]))))
                break
            if c in on_stack:
                low[f] = min(low[f], index[c])
        else:
            work.pop()
            if work:
                low[work[-1][0]] = min(low[work[-1][0]], low[f])
            if low[f] == index[f]:
                group = []
                while True:
                    g = stack.pop()
                    on_stack.discard(g)
                    group.append(g)
                    if g == f:
                        break
                out.append(group)
    return out


def deepest(funcs, entry):
    """-> (bytes, [(function, frame bytes), ...]) of the deepest direct path."""
    best = {}
    for group in cycles(funcs, entry):
        members = set(group)
        top = (0, [])
        for f in sorted(group):
            frame, calls, tails = funcs[f]
            below = max((best[c] for c in calls - members), key=lambda b: b[0], default=(0, []))
            here = (frame + below[0], [(f, frame)] + below[1])
            after = max((best[t] for t in tails - members), key=lambda b: b[0], default=(0, []))
            top = max(top, here, after, key=lambda b: b[0])
        for f in group:
            best[f] = top
    return best[entry]


def verdict(disassembly, entry, stack_size, exception_frame):
    """-> (fits, the line to print)."""
    funcs = parse(disassembly)
    if entry not in funcs:
        return False, f"no function {entry} in the disassembly"
    depth, path = deepest(funcs, entry)
    need = depth + exception_frame
    chain = " > ".join(f"{f}:{frame}" for f, frame in path if frame)
    line = (
        f"deepest direct call path from {entry} {depth} bytes + exception frame "
        f"{exception_frame} = {need} of the main stack's {stack_size} (calls through "
        f"a register are not counted): {chain}"
    )
    return need <= stack_size, line


def selftest():
    def fn(name, *insns):
        return f"0000000010000000 <{name}>:\n" + "".join(f"    10000000:\t{i}\n" for i in insns)

    chain = (
        fn("bg_thread_main", "stp\tx29, x30, [sp, #-32]!", "bl\t10000100 <main>")
        + fn("main", "sub\tsp, sp, #0x60", "bl\t10000200 <big>", "bl\t10000300 <small>")
        + fn("big", "stp\tx29, x30, [sp, #-96]!", "sub\tsp, sp, #0x1, lsl #12", "sub\tsp, sp, #0x610")
        + fn("small", "sub\tsp, sp, #16")
    )
    tail = (
        fn("bg_thread_main", "sub\tsp, sp, #0x100", "bl\t10000100 <main>")
        + fn("main", "sub\tsp, sp, #0x80", "add\tsp, sp, #0x80", "b\t10000200 <next>")
        + fn("next", "sub\tsp, sp, #0x200", "b\t10000208 <next+0x8>")
    )
    cycle = (
        fn("bg_thread_main", "sub\tsp, sp, #0x10", "bl\t10000100 <a>")
        + fn("a", "sub\tsp, sp, #0x40", "bl\t10000200 <b>")
        + fn("b", "sub\tsp, sp, #0x100", "bl\t10000100 <a>", "bl\t10000300 <leaf>")
        + fn("leaf", "sub\tsp, sp, #0x20", "blr\tx1")
    )
    cases = [
        # 32 + 96 + (96 + 4096 + 1552) = 5872
        ("a chain, lsl #12, the larger callee", chain, 5872),
        # main's 0x80 is gone when next takes its 0x200: not 0x100 + 0x80 + 0x200
        ("a tail call leaves the caller's frame", tail, 0x100 + 0x200),
        # a and b call each other: the larger frame once, then leaf
        ("a cycle counts its largest frame once", cycle, 0x10 + 0x100 + 0x20),
    ]
    fails = 0
    for name, text, want in cases:
        got = deepest(parse(text), "bg_thread_main")[0]
        if got != want:
            print(f"check_main_stack selftest: FAIL {name}: {got}, want {want}")
            fails += 1
    for size, want in ((5872 + 176, True), (5872 + 175, False)):
        fits, line = verdict(chain, "bg_thread_main", size, 176)
        if fits != want or "big:5744" not in line:
            print(f"check_main_stack selftest: FAIL a {size}-byte stack: fits {fits}, want {want}: {line}")
            fails += 1
    if verdict(chain, "absent", 1 << 20, 176)[0]:
        print("check_main_stack selftest: FAIL an image without the entry passes")
        fails += 1
    print(f"check_main_stack selftest: {len(cases) + 3} checks, {fails} failed")
    return 1 if fails else 0


def run(cmd):
    return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout


def main(argv):
    if argv == ["--selftest"]:
        return selftest()
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    nm, objdump, elf = argv
    syms = {}
    for line in run([nm, "-S", elf]).splitlines():
        parts = line.split()
        if parts[-1] in ("z_main_stack", "___esf_t_SIZEOF"):
            syms[parts[-1]] = parts
    if "z_main_stack" not in syms or len(syms["z_main_stack"]) != 4:
        print(f"check_main_stack: {elf} has no sized z_main_stack", file=sys.stderr)
        return 1
    if "___esf_t_SIZEOF" not in syms:
        print(f"check_main_stack: {elf} has no ___esf_t_SIZEOF", file=sys.stderr)
        return 1
    stack_size = int(syms["z_main_stack"][1], 16)
    exception_frame = int(syms["___esf_t_SIZEOF"][0], 16)
    fits, line = verdict(
        run([objdump, "-d", "--no-show-raw-insn", elf]), "bg_thread_main", stack_size, exception_frame
    )
    if not fits:
        print(f"check_main_stack: {elf}: the main stack is too small: {line}", file=sys.stderr)
        return 1
    print(f"-- check_main_stack: {line}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
