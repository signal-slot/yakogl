#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Finds memory regions claimed twice across devicetree files.

usage: python3 -I tools/mp2_dt_overlap_check.py [--range NAME=ADDR:SIZE]...
           FILE.dts|.dtsi|.overlay ...        (or one built zephyr.dts)

The files are merged into one tree in the order given (a quoted #include that
resolves next to its file is read first, once; `&label { }` and `&{/path} { }`
merge into the node they name (a label or path no file defines, a node of
the SoC dtsi, gets an empty node), later reg properties replace earlier ones), as
the devicetree compiler does for a board plus overlays. Collected, with reg
read by the parent's #address-cells / #size-cells (default 2 / 1, as dtc;
1 / 1 when the parent sets neither, as the SoC dtsi often does, and the
reg only fits that):

  reserved   a child of a reserved-memory node
  memory     a node with device_type = "memory"
  image      a child of a memory node (the image's own section)
  carveout   any other node whose name or label contains "carveout" or
             "carve-out" / "carve_out"
  extra      each --range (ADDR:SIZE, hex or decimal)

Pairs of an ancestor and its descendant are skipped. A reserved or carveout
region lying inside System RAM (a memory node) is how reserved memory is
declared, so memory is compared only with memory; extras are compared with
reserved, carveout and other extras (an extra against the image section or
System RAM would always match: the image is the window).

Prints `overlap: <a> <b> <first>-<last>` per overlapping pair (hex, inclusive)
and exits 1 on any, 0 on none, 2 on a usage or parse error. Names are the
node's first label, else its path.
"""
import ast
import os
import re
import sys

SIZE_MACROS = {"DT_SIZE_K": 1 << 10, "DT_SIZE_M": 1 << 20, "DT_SIZE_G": 1 << 30}
TOKEN = re.compile(r'"(?:\\.|[^"\\])*"|[{};]|[^{};"]+')


class DtError(Exception):
    pass


class Node:
    def __init__(self, name, parent):
        self.name = name
        self.parent = parent
        self.labels = []
        self.props = {}
        self.children = {}

    def path(self):
        if self.parent is None:
            return "/"
        p = self.parent.path()
        return (p if p != "/" else "") + "/" + self.name


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", " ", text)


def expand_macros(tok):
    """DT_SIZE_M(n) -> (1048576*(n)), nested parentheses allowed."""
    while True:
        m = re.search(r"DT_SIZE_[KMG]\(", tok)
        if not m:
            return tok
        mult = SIZE_MACROS[m.group(0)[:-1]]
        depth, j = 1, m.end()
        while j < len(tok) and depth:
            depth += {"(": 1, ")": -1}.get(tok[j], 0)
            j += 1
        if depth:
            return tok  # unbalanced: left as is, eval rejects it
        tok = "%s(%d*(%s))%s" % (tok[:m.start()], mult, tok[m.end():j - 1], tok[j:])


def _safe(n):
    if isinstance(n, ast.Constant) and isinstance(n.value, int):
        return n.value
    if isinstance(n, ast.BinOp):
        a, b = _safe(n.left), _safe(n.right)
        ops = {ast.Add: lambda: a + b, ast.Sub: lambda: a - b,
               ast.Mult: lambda: a * b, ast.LShift: lambda: a << b,
               ast.RShift: lambda: a >> b, ast.BitOr: lambda: a | b,
               ast.BitAnd: lambda: a & b}
        return ops[type(n.op)]()
    raise ValueError("expr")


def eval_cell(tok):
    """A cell token to an int, or None (a phandle or anything unknown)."""
    tok = expand_macros(tok.strip())
    if not re.fullmatch(r"[0-9a-fA-FxXuUlL+*()<>|& -]+", tok):
        return None
    tok = re.sub(r"(?<=[0-9a-fA-F])[uUlL]+\b", "", tok)
    try:
        return _safe(ast.parse(tok, mode="eval").body)
    except (SyntaxError, ValueError, KeyError, TypeError, RecursionError):
        return None


def split_cells(body):
    """Tokens of a <...> body; a parenthesised group stays one token."""
    toks, cur, depth = [], "", 0
    for c in body:
        depth += {"(": 1, ")": -1}.get(c, 0)
        if c.isspace() and depth == 0:
            if cur:
                toks.append(cur)
            cur = ""
        else:
            cur += c
    if cur:
        toks.append(cur)
    return toks


def parse_value(val):
    """A property value: a list of ints/None for <...>, else the string."""
    val = val.strip()
    cells, found = [], False
    for m in re.finditer(r"<([^>]*)>", val):
        found = True
        cells.extend(eval_cell(t) for t in split_cells(m.group(1)))
    if found:
        return cells
    m = re.fullmatch(r'"((?:\\.|[^"\\])*)"', val)
    return m.group(1) if m else val


class Tree:
    def __init__(self):
        self.root = Node("", None)
        self.labels = {}
        self.loaded = set()

    def resolve(self, ref):
        if ref.startswith("&{"):
            node = self.root
            for part in ref[2:-1].strip().strip("/").split("/"):
                if part:
                    if part not in node.children:
                        node.children[part] = Node(part, node)
                    node = node.children[part]
            return node
        if ref[1:] not in self.labels:
            # A node of the SoC dtsi, which is not among the files: nothing
            # of it is a memory region, so its properties go to a detached
            # node.
            self.labels[ref[1:]] = Node(ref, None)
        return self.labels[ref[1:]]

    def load(self, path):
        real = os.path.realpath(path)
        if real in self.loaded:
            return
        self.loaded.add(real)
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                text = strip_comments(f.read())
        except OSError as e:
            raise DtError("%s: %s" % (path, e))
        base = os.path.dirname(path)
        lines = []
        for line in text.split("\n"):
            s = line.strip()
            m = re.match(r'#\s*include\s+"([^"]+)"', s)
            if m:
                inc = os.path.join(base, m.group(1))
                if os.path.isfile(inc):
                    self.load(inc)
                continue
            if re.match(r"#\s*(include|define|if|else|endif|undef)", s):
                continue
            lines.append(line)
        try:
            self.parse("\n".join(lines))
        except DtError as e:
            raise DtError("%s: %s" % (path, e))

    def parse(self, text):
        stack = [self.root]
        cur = ""
        for tok in TOKEN.findall(text):
            if tok == "{":
                stack.append(self.open_node(stack[-1], cur.strip()))
                cur = ""
            elif tok == "}":
                if len(stack) < 2:
                    raise DtError("unbalanced }")
                stack.pop()
                cur = ""
            elif tok == ";":
                self.statement(stack[-1], cur.strip())
                cur = ""
            else:
                cur += tok
        if len(stack) != 1:
            raise DtError("unbalanced {")

    def open_node(self, parent, head):
        labels = []
        while True:
            m = re.match(r"([A-Za-z_]\w*)\s*:\s*(.*)$", head, re.S)
            if not m:
                break
            labels.append(m.group(1))
            head = m.group(2)
        head = head.strip()
        if head == "/":
            node = self.root
        elif head.startswith("&"):
            node = self.resolve(head)
        else:
            if not head or re.search(r"\s", head):
                raise DtError("bad node header %r" % head)
            node = parent.children.get(head)
            if node is None:
                node = Node(head, parent)
                parent.children[head] = node
        for label in labels:
            node.labels.append(label)
            self.labels[label] = node
        return node

    def statement(self, node, st):
        if not st or re.match(r"/(dts-v1|delete-node|delete-property|plugin|memreserve)/", st):
            return
        m = re.match(r"([\w,.+*#?@-]+)\s*=\s*(.*)$", st, re.S)
        if m:
            node.props[m.group(1)] = parse_value(m.group(2))
        elif re.fullmatch(r"[\w,.+*#?@-]+", st):
            node.props[st] = True


def cells_of(node, prop, default):
    v = node.props.get(prop)
    if isinstance(v, list) and len(v) == 1 and v[0] is not None:
        return v[0]
    return default


def walk(node):
    yield node
    for c in node.children.values():
        yield from walk(c)


def ancestors(node):
    n = node.parent
    while n is not None:
        yield n
        n = n.parent


def collect(tree):
    regions = []  # (name, kind, start, end_exclusive, node)
    for node in walk(tree.root):
        reg = node.props.get("reg")
        if node.parent is None or not isinstance(reg, list):
            continue
        ac = cells_of(node.parent, "#address-cells", 2)
        sc = cells_of(node.parent, "#size-cells", 1)
        if ac < 1 or sc < 0:
            continue
        # A parent that sets neither property (the SoC dtsi that does is not
        # among the files): dtc's 2/1 if the reg fits it, else 1/1.
        undefined = "#address-cells" not in node.parent.props and \
            "#size-cells" not in node.parent.props
        if undefined and len(reg) % (ac + sc) and not len(reg) % 2:
            ac, sc = 1, 1
        kind = None
        if node.parent.name == "reserved-memory":
            kind = "reserved"
        elif node.props.get("device_type") == "memory":
            kind = "memory"
        elif any(a.props.get("device_type") == "memory" for a in ancestors(node)):
            kind = "image"
        elif re.search(r"carve[-_]?out", " ".join([node.name] + node.labels), re.I):
            kind = "carveout"
        if kind is None:
            continue
        name = node.labels[0] if node.labels else node.path()
        step = ac + sc
        if len(reg) % step:
            raise DtError("%s: reg has %d cells, not a multiple of %d"
                          % (node.path(), len(reg), step))
        for i in range(0, len(reg), step):
            ent = reg[i:i + step]
            if any(c is None for c in ent):
                continue
            a = s = 0
            for c in ent[:ac]:
                a = (a << 32) | (c & 0xFFFFFFFF)
            for c in ent[ac:]:
                s = (s << 32) | (c & 0xFFFFFFFF)
            if s:
                regions.append((name, kind, a, a + s, node))
    return regions


def skip_pair(a, b):
    ka, kb = a[1], b[1]
    if a[4] is not None and b[4] is not None and \
            (a[4] in ancestors(b[4]) or b[4] in ancestors(a[4])):
        return True
    if "extra" in (ka, kb):
        other = kb if ka == "extra" else ka
        return other in ("memory", "image")
    if "memory" in (ka, kb):
        return not (ka == kb)
    return False


def main(argv):
    ranges, files, i = [], [], 0
    while i < len(argv):
        a = argv[i]
        if a in ("-h", "--help"):
            print(__doc__)
            return 0
        if a == "--range" and i + 1 < len(argv):
            i += 1
            m = re.fullmatch(r"([^=\s]+)=(\S+):(\S+)", argv[i])
            try:
                if not m:
                    raise ValueError
                start, size = int(m.group(2), 0), int(m.group(3), 0)
                if size <= 0 or start < 0:
                    raise ValueError
            except ValueError:
                print("error: bad --range %r (NAME=ADDR:SIZE)" % argv[i], file=sys.stderr)
                return 2
            ranges.append((m.group(1), "extra", start, start + size, None))
        elif a.startswith("-"):
            print("error: unknown option %s" % a, file=sys.stderr)
            return 2
        else:
            files.append(a)
        i += 1
    if not files and not ranges:
        print(__doc__, file=sys.stderr)
        return 2
    tree = Tree()
    try:
        for f in files:
            tree.load(f)
        regions = collect(tree) + ranges
    except DtError as e:
        print("error: %s" % e, file=sys.stderr)
        return 2
    n = 0
    for x in range(len(regions)):
        for y in range(x + 1, len(regions)):
            a, b = regions[x], regions[y]
            if skip_pair(a, b):
                continue
            lo, hi = max(a[2], b[2]), min(a[3], b[3])
            if lo < hi:
                n += 1
                print("overlap: %s %s 0x%x-0x%x" % (a[0], b[0], lo, hi - 1))
    print("%d region(s), %d overlap(s)" % (len(regions), n), file=sys.stderr)
    return 1 if n else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
