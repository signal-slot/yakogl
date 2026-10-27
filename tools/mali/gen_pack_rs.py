#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate the Mali-G610 (Valhall, arch 10) descriptor packers in Rust.

    tools/mali/gen_pack_rs.py rust   <v10.xml> > src/mali/genxml/v10.rs
    tools/mali/gen_pack_rs.py cases  <v10.xml> > src/mali/genxml/v10_cases.rs
    tools/mali/gen_pack_rs.py c-cases <v10.xml> > <build>/v10_cases.inc

The input is Mesa's own description of the descriptors and command-stream
instructions, src/panfrost/genxml/v10.xml, vendored unmodified under
third_party/mesa-26.2.1/ (MIT; Collabora Ltd., Arm Ltd.). It is read the
way Mesa's generator (gen_pack.py, vendored beside it) reads it: the same
bit positions, lengths, defaults and modifiers. What differs is what a value
that does not fit becomes. Mesa ORs every field into its word and checks the
ranges only in debug builds (gen_pack.py:289-336, util/bitpack_helpers.h:
53-66). The Rust packer refuses instead, naming the structure and the field:

  - a value wider than its field;
  - `shr(n)` of a value whose low n bits are set;
  - `log2` of zero or of a value that is not a power of two;
  - `minus(n)` of a value below n;
  - `align(n)` of a value that is not a multiple of n (Mesa rounds up);
  - a non-zero enum value that is not one of the enum's values;
  - an enum field left at zero when zero is not one of the enum's values
    (Mesa's header leaves such a field at zero and packs it; the zero is no
    state, so the packer refuses it, unless a non-zero field that shares
    its bits is the descriptor's active union arm);
  - a ulod/slod value that is NaN or outside the fixed-point range (Mesa
    clamps);
  - two fields that share bits (the members of one union: the render-target
    variants, the blend shader and fixed-function halves, ...) both non-zero.

The three outputs:

  rust     src/mali/genxml/v10.rs: a type per enum and per struct, its
           Default (Mesa's `_header`), pack() and unpack() (unpack_as(arm)
           where fields share bits: the caller names the union members).
  cases    src/mali/genxml/v10_cases.rs: the cross-check cases as Rust,
           for the test in src/mali/genxml/mesa_tests.rs.
  c-cases  the same cases as C over Mesa's generated v10_pack.h, written
           by that test into its build directory and compiled with
           tools/mali/pack_ref.c.

The cases are made here from the XML alone, with a fixed seed: for every
struct Mesa can pack directly, one case of the defaults (Mesa's header, with
each field whose default Mesa itself would assert on given its smallest
valid value) and cases with a non-zero encoding in every field. Fields that
share bits cannot all be non-zero at once, so those cases pick a set of
fields that share none, as many cases as it takes until every field has
been encoded non-zero in one. The generator fails if a field never is.
"""

import hashlib
import struct
import sys
import xml.etree.ElementTree as ET

RUST_KEYWORDS = {
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
    "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof",
    "unsized", "virtual", "yield", "try",
}

# The types gen_pack.py packs with util_bitpack_uint (gen_pack.py:336).
UINT_TYPES = {"uint", "hex", "uint/float", "address", "Pixel Format", "Component Swizzle"}


class GenError(Exception):
    pass


# ---- Mesa's names (gen_pack.py:37-78), for the C side ------------------------


def to_alphanum(name):
    for i, j in {
        " ": "_", "/": "_", "[": "", "]": "", "(": "", ")": "", "-": "_", ":": "", ".": "", ",": "",
        "=": "", ">": "", "#": "", "&": "", "%": "", "*": "", '"': "", "+": "", "'": "",
    }.items():
        name = name.replace(i, j)
    return name


def safe_name(name):
    name = to_alphanum(name)
    if not name[0].isalpha():
        name = "_" + name
    return name


def mesa_struct(name):
    return "MALI_" + safe_name(name.upper())


def mesa_enum_value(enum_name, value_name):
    e = "mali_" + safe_name(enum_name).lower()
    prefix = e if enum_name != "Format" else "mali"
    return safe_name("{}_{}".format(prefix, value_name)).upper()


# ---- our names -------------------------------------------------------------


def camel(name):
    out, word = [], ""
    for ch in name:
        if ch.isalnum():
            word += ch
        else:
            if word:
                out.append(word)
            word = ""
    if word:
        out.append(word)
    s = "".join(w[0].upper() + w[1:].lower() for w in out)
    if not s or not s[0].isalpha():
        s = "T" + s
    return s


def snake(name):
    s = safe_name(name).lower()
    if s in RUST_KEYWORDS:
        s += "_"
    return s


def upper_const(name):
    s = safe_name(name).upper()
    if s in RUST_KEYWORDS:
        s += "_"
    return s


# ---- the model -------------------------------------------------------------


def bit_of(start):
    if ":" in start:
        w, b = start.split(":")
        return int(w) * 32 + int(b)
    return int(start)


def parse_modifier(m):
    if m is None:
        return None
    if m == "log2":
        return ("log2", 0)
    for mod in ("shr", "minus", "align"):
        if m.startswith(mod + "(") and m.endswith(")"):
            n = int(m[len(mod) + 1:-1])
            if mod == "align" and (n <= 0 or n & (n - 1)):
                raise GenError("align({}) is not a power of two".format(n))
            return (mod, n)
    raise GenError("unknown modifier " + m)


class Enum:
    def __init__(self, el):
        self.name = el.get("name")
        self.rust = camel(self.name)
        self.values = []  # (xml name, value)
        for v in el.findall("value"):
            self.values.append((v.get("name"), int(v.get("value"), 0)))
        consts = [upper_const(n) for n, _ in self.values]
        if len(set(consts)) != len(consts):
            raise GenError("enum {}: two values with one Rust name".format(self.name))

    def value(self, name):
        for n, v in self.values:
            if n == name:
                return v
        raise GenError("enum {} has no value {}".format(self.name, name))


class Field:
    def __init__(self, gen, struct, el):
        self.struct = struct
        self.name = el.get("name")
        self.c = safe_name(self.name).lower()
        self.rust = snake(self.name)
        self.start = bit_of(el.get("start"))
        self.size = int(el.get("size"))
        self.end = self.start + self.size - 1
        self.type = el.get("type")
        self.modifier = parse_modifier(el.get("modifier"))
        self.prefix = el.get("prefix")
        self.values = [(v.get("name"), int(v.get("value"), 0)) for v in el.findall("value")]
        self.default_text = el.get("default")
        if self.type == "bool" and self.size != 1:
            raise GenError("{}.{}: a bool of {} bits".format(struct.name, self.name, self.size))
        if self.type == "padded":
            raise GenError("{}.{}: 'padded' is not handled (no v10 field has it)".format(struct.name, self.name))
        if self.modifier and self.type not in UINT_TYPES:
            raise GenError("{}.{}: a modifier on type {}".format(struct.name, self.name, self.type))

    # The kind decides the Rust type and the pack call.
    def kind(self, gen):
        t = self.type
        if t in gen.structs:
            return "struct"
        if t in gen.enums:
            return "enum"
        if t == "bool":
            return "bool"
        if t in UINT_TYPES:
            return "uint"
        if t == "int":
            return "int"
        if t == "float":
            if self.size != 32:
                raise GenError("{}.{}: a float of {} bits".format(self.struct.name, self.name, self.size))
            return "float"
        if t in ("ulod", "slod"):
            return t
        raise GenError("{}.{}: unhandled type {}".format(self.struct.name, self.name, t))

    def value_bits(self):
        """Bits of the value (before the modifier) the field can carry."""
        m = self.modifier
        if m is None or m[0] == "align":
            return self.size
        if m[0] == "shr":
            return self.size + m[1]
        if m[0] == "minus":
            return ((1 << self.size) - 1 + m[1]).bit_length()
        if m[0] == "log2":
            return (1 << self.size)
        raise GenError("modifier")

    def rust_type(self, gen):
        k = self.kind(gen)
        if k == "struct":
            return gen.structs[self.type].rust
        if k == "enum":
            return gen.enums[self.type].rust
        if k == "bool":
            return "bool"
        if k == "uint":
            if self.type == "address":
                return "u64"
            return "u32" if self.value_bits() <= 32 else "u64"
        if k == "int":
            return "i32" if self.size <= 32 else "i64"
        return "f32"

    def default_value(self, gen):
        """The default as a Python value (None: zero), as Mesa's header sets it."""
        d = self.default_text
        if d is None:
            return None
        k = self.kind(gen)
        if k == "enum":
            return gen.enums[self.type].value(d)
        if k == "bool":
            if d not in ("true", "false"):
                raise GenError("{}.{}: bool default {}".format(self.struct.name, self.name, d))
            return d == "true"
        if k in ("float", "ulod", "slod"):
            return float(d)
        if k == "uint" or k == "int":
            for vn, vv in self.values:
                if self.prefix and d == safe_name("{}_{}".format(self.prefix, vn)).upper():
                    return vv
            try:
                return int(d, 0)
            except ValueError:
                f = float(d)
                if f != int(f):
                    raise GenError("{}.{}: default {}".format(self.struct.name, self.name, d))
                return int(f)
        raise GenError("{}.{}: a default on a {}".format(self.struct.name, self.name, k))


class Struct:
    def __init__(self, gen, el):
        self.name = el.get("name")
        self.rust = camel(self.name)
        self.c = mesa_struct(self.name)
        self.no_direct = el.get("no-direct-packing") is not None
        self.fields = [Field(gen, self, f) for f in el.findall("field")]
        self.explicit_len = int(el.get("size")) * 4 if el.get("size") else 0
        self.align = int(el.get("align")) if el.get("align") else None
        names = [f.rust for f in self.fields]
        if len(set(names)) != len(names):
            raise GenError("struct {}: two fields with one Rust name".format(self.name))

    def length(self):
        calc = max(f.end // 8 for f in self.fields) + 1 if self.fields else 0
        if self.explicit_len:
            if self.explicit_len < calc:
                raise GenError("struct {}: size {} < {}".format(self.name, self.explicit_len, calc))
            return self.explicit_len
        return calc

    def packable(self):
        return not self.no_direct and self.length() % 4 == 0


class Leaf:
    """A field of a primitive kind, at its absolute bit in the outer struct."""

    def __init__(self, field, start, rust_path, c_path, name_path):
        self.field = field
        self.start = start
        self.end = start + field.size - 1
        self.rust_path = rust_path
        self.c_path = c_path
        self.name_path = name_path

    def overlaps(self, o):
        return max(self.start, o.start) <= min(self.end, o.end)


class Gen:
    def __init__(self, path):
        root = ET.parse(path).getroot()
        if root.tag != "panxml" or root.get("arch") != "10":
            raise GenError("{} is not the arch 10 panxml".format(path))
        self.enums = {}
        self.structs = {}
        self.aggregates = []
        self.order = []
        for el in root:
            if el.tag == "enum":
                e = Enum(el)
                self.enums[e.name] = e
                self.order.append(("enum", e))
            elif el.tag == "struct":
                s = Struct(self, el)
                self.structs[s.name] = s
                self.order.append(("struct", s))
            elif el.tag == "aggregate":
                self.aggregates.append(el)
        # An enum and a struct of one name (`CS Heap Operation` and
        # `CS HEAP_OPERATION`, which C tells apart by case): the enum's Rust
        # name takes the suffix `Enum`.
        struct_names = {s.rust for s in self.structs.values()}
        for e in self.enums.values():
            if e.rust in struct_names:
                e.rust += "Enum"
        rust_names = [e.rust for e in self.enums.values()] + [s.rust for s in self.structs.values()]
        if len(set(rust_names)) != len(rust_names):
            dup = sorted({n for n in rust_names if rust_names.count(n) > 1})
            raise GenError("two types with one Rust name: {}".format(dup))
        for s in self.structs.values():
            for f in s.fields:
                f.kind(self)
                f.default_value(self)
                if f.kind(self) == "enum":
                    e = self.enums[f.type]
                    if f.default_value(self) not in (None, 0) and f.default_value(self) >= (1 << f.size):
                        raise GenError("{}.{}: the default does not fit".format(s.name, f.name))
                    del e
            if s.packable():
                for leaf in self.leaves(s):
                    if leaf.end >= s.length() * 8:
                        raise GenError("{}: {} ends past the struct".format(s.name, leaf.name_path))

    def leaves(self, s, base=0, rust="", c="", names=""):
        out = []
        for f in s.fields:
            rp = rust + f.rust
            cp = c + f.c
            np = names + f.name
            if f.kind(self) == "struct":
                out += self.leaves(self.structs[f.type], base + f.start, rp + ".", cp + ".", np + ".")
            else:
                out.append(Leaf(f, base + f.start, rp, cp, np))
        return out

    def has_shared_bits(self, s):
        ls = self.leaves(s)
        return any(a.overlaps(b) for i, a in enumerate(ls) for b in ls[i + 1:])


# ---- Rust literals -----------------------------------------------------------


def rust_int(v, ty):
    if ty in ("i32", "i64"):
        return str(v)
    if v < 10:
        return str(v)
    h = "{:x}".format(v)
    groups = []
    while h:
        groups.insert(0, h[-4:])
        h = h[:-4]
    return "0x" + "_".join(groups)


def f32_bits(v):
    return struct.unpack("<I", struct.pack("<f", v))[0]


def rust_float(v):
    """The shortest decimal that reads back as the same binary32 (what Rust
    prints for an f32); every value here is exact in binary32."""
    v = float(v)
    if f32_bits(v) != f32_bits(struct.unpack("<f", struct.pack("<f", v))[0]) or \
            struct.unpack("<f", struct.pack("<f", v))[0] != v:
        raise GenError("{} is not exact in binary32".format(v))
    for p in range(1, 18):
        r = "{:.{}g}".format(v, p)
        if f32_bits(float(r)) == f32_bits(v):
            break
    if "e" in r or "inf" in r or "nan" in r:
        raise GenError("float literal " + r)
    if "." not in r:
        r += ".0"
    return r


# ---- the packer --------------------------------------------------------------

RUST_HEADER = """\
// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2020 Collabora Ltd.
// SPDX-FileCopyrightText: 2026 Arm Ltd.
//
// v10.rs - GENERATED by tools/mali/gen_pack_rs.py from Mesa 26.2.1's
// src/panfrost/genxml/v10.xml (third_party/mesa-26.2.1/, unmodified).
// DO NOT EDIT: tools/check.sh regenerates it and requires these bytes.
//
//! The Mali-G610's descriptors and command-stream instructions (Valhall,
//! arch 10), one Rust type per `<struct>` and `<enum>` of Mesa's v10.xml.
//!
//! `Default` is Mesa's `<NAME>_header`. `pack()` gives the descriptor's
//! bytes or refuses, naming the structure and the field, where Mesa's
//! packer would mask, round or OR the value in (tools/mali/gen_pack_rs.py
//! lists the refusals). `unpack()` exists for every struct in which no two
//! fields share a bit, `unpack_as(bytes, arm)` for the others (the caller
//! names the union members the bytes hold); both refuse a reserved bit that
//! is set. An enum is a
//! newtype over its raw value. `unpack()` refuses a non-zero value the XML
//! does not name and gives a zero back as zero; `pack()` refuses the same
//! non-zero value, and refuses a zero that the enum does not name (Mesa's
//! header leaves every enum without a default at zero, also where zero is
//! not one of its values; `UNSET_ENUM_LEAVES` lists those fields).
// The layout is the generator's (as src/abi_layout.rs's): regenerated, not
// formatted.
#![cfg_attr(rustfmt, rustfmt_skip)]

use super::pack::{Arm, EnumDef, PackError, Packer, Unpacker};
"""


def emit_rust(gen):
    o = [RUST_HEADER]
    for kind, item in gen.order:
        if kind == "enum":
            o += emit_enum(item)
        else:
            o += emit_struct(gen, item)
    for el in gen.aggregates:
        o += emit_aggregate(gen, el)
    o += emit_unset_enum_leaves(gen)
    return "\n".join(o) + "\n"


def unset_enum_leaves(gen, s):
    """The leaves of struct `s` that are enums with no default whose zero is
    not a named value: Mesa's header leaves them at zero, and the packer
    refuses that."""
    out = []
    for lf in gen.leaves(s):
        f = lf.field
        if f.kind(gen) == "enum" and f.default_value(gen) is None and not enum_names_zero(gen.enums[f.type]):
            out.append(lf)
    return out


def enum_names_zero(e):
    return any(v == 0 for _, v in e.values)


def emit_unset_enum_leaves(gen):
    o = ["", "/// The packable fields that are an enum without a default whose zero is no named value, as"]
    o.append("/// `<struct name>.<field path>` (v10.xml's names): `pack()` refuses each while it is zero.")
    o.append("pub const UNSET_ENUM_LEAVES: &[&str] = &[")
    for _, s in [x for x in gen.order if x[0] == "struct"]:
        if not s.packable():
            continue
        for lf in unset_enum_leaves(gen, s):
            o.append("    {},".format(rust_str("{}.{}".format(s.name, lf.name_path))))
    o.append("];")
    return o


def emit_enum(e):
    o = ["/// `<enum name=\"{}\">`.".format(e.name)]
    o.append("#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]")
    o.append("pub struct {}(pub u32);".format(e.rust))
    o.append("")
    o.append("impl {} {{".format(e.rust))
    o.append("    /// The name v10.xml gives this enum.")
    o.append("    pub const XML_NAME: &'static str = {};".format(rust_str(e.name)))
    for n, v in e.values:
        o.append("    pub const {}: Self = Self({});".format(upper_const(n), rust_int(v, "u32")))
    o.append("")
    o.append("    /// The name and whether zero is one of the values, for the packer.")
    o.append("    pub const DEF: EnumDef = EnumDef {{ name: {}, zero_named: {} }};".format(
        rust_str(e.name), "true" if any(v == 0 for _, v in e.values) else "false"))
    o.append("")
    o.append("    /// One of the values v10.xml names.")
    o.append("    pub const fn is_known(self) -> bool {")
    o.append("        matches!(self.0, {})".format(value_pattern(sorted({v for _, v in e.values}))))
    o.append("    }")
    o.append("}")
    o.append("")
    return o


def value_pattern(vals):
    """`1 | 3..=5`: runs of consecutive values as ranges."""
    runs = []
    for v in vals:
        if runs and runs[-1][1] + 1 == v:
            runs[-1][1] = v
        else:
            runs.append([v, v])
    return " | ".join(
        rust_int(a, "u32") if a == b else "{}..={}".format(rust_int(a, "u32"), rust_int(b, "u32")) for a, b in runs)


def rust_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def emit_struct(gen, s):
    o = []
    o.append("/// `<struct name=\"{}\">`{}.".format(
        s.name, ", {} bytes".format(s.length()) if s.length() else ""))
    defaults = [(f, f.default_value(gen)) for f in s.fields]
    nonzero = [(f, d) for f, d in defaults if d not in (None, 0, False, 0.0)]
    derive = "Clone, Copy, Debug, PartialEq"
    if not nonzero:
        derive += ", Default"
    o.append("#[derive({})]".format(derive))
    if not s.fields:
        o.append("pub struct {};".format(s.rust))
    else:
        o.append("pub struct {} {{".format(s.rust))
        for f in s.fields:
            mod = ""
            if f.modifier:
                mod = ", {}{}".format(f.modifier[0], "" if f.modifier[0] == "log2" else "({})".format(f.modifier[1]))
            o.append("    /// `{}`: bits {}..={}{}.".format(f.name, f.start, f.end, mod))
            o.append("    pub {}: {},".format(f.rust, f.rust_type(gen)))
        o.append("}")
    o.append("")
    if nonzero:
        o.append("impl Default for {} {{".format(s.rust))
        o.append("    fn default() -> Self {")
        o.append("        Self {")
        for f, d in defaults:
            if d in (None, 0, False, 0.0):
                o.append("            {}: Default::default(),".format(f.rust))
            else:
                o.append("            {}: {},".format(f.rust, rust_value(gen, f, d)))
        o.append("        }")
        o.append("    }")
        o.append("}")
        o.append("")
    o.append("impl {} {{".format(s.rust))
    o.append("    /// The name v10.xml gives this struct.")
    o.append("    pub const XML_NAME: &'static str = {};".format(rust_str(s.name)))
    if s.length():
        o.append("    /// Bytes, as Mesa's `{}_LENGTH`.".format(s.c))
        o.append("    pub const LENGTH: usize = {};".format(s.length()))
    if s.align is not None:
        o.append("    /// Alignment in bytes, as Mesa's `{}_ALIGN`.".format(s.c))
        o.append("    pub const ALIGN: usize = {};".format(s.align))
    for f in s.fields:
        for vn, vv in f.values:
            o.append("    /// A value of `{}` named in v10.xml.".format(f.name))
            o.append("    pub const {}: {} = {};".format(upper_const(vn), f.rust_type(gen), rust_int(vv, f.rust_type(gen))))
    if s.packable():
        o += emit_pack(gen, s)
        if not gen.has_shared_bits(s):
            o += emit_unpack(gen, s)
        else:
            o += emit_unpack_as(gen, s)
    o.append("}")
    o.append("")
    return o


def rust_value(gen, f, v):
    k = f.kind(gen)
    if k == "enum":
        e = gen.enums[f.type]
        for n, ev in e.values:
            if ev == v:
                return "{}::{}".format(e.rust, upper_const(n))
        return "{}({})".format(e.rust, v)
    if k == "bool":
        return "true" if v else "false"
    if k in ("float", "ulod", "slod"):
        return rust_float(v)
    return rust_int(v, f.rust_type(gen))


def pos(start):
    return str(start)


def emit_pack(gen, s):
    leaves = gen.leaves(s)
    words = s.length() // 4
    o = []
    o.append("")
    o.append("    /// The descriptor's bytes, or the first field that does not fit.")
    o.append("    pub fn pack(&self) -> Result<[u8; {}], PackError> {{".format(s.length()))
    if not leaves:
        o.append("        Ok([0; {}])".format(s.length()))
        o.append("    }")
        return o
    o.append("        let mut p = Packer::<{}, {}>::new(Self::XML_NAME);".format(words, len(leaves)))
    for lf in leaves:
        f = lf.field
        k = f.kind(gen)
        name = rust_str(lf.name_path)
        v = "self." + lf.rust_path
        st, sz = pos(lf.start), f.size
        if k == "bool":
            o.append("        p.put({}, {}, 1, u64::from({}))?;".format(name, st, v))
        elif k == "enum":
            o.append("        p.enum_value({}, {}, {}, {}.0, {}.is_known(), &{}::DEF)?;".format(
                name, st, sz, v, v, gen.enums[f.type].rust))
        elif k == "uint":
            arg = v if f.rust_type(gen) == "u64" else "u64::from({})".format(v)
            m = f.modifier
            if m is None:
                o.append("        p.uint({}, {}, {}, {})?;".format(name, st, sz, arg))
            elif m[0] == "shr":
                o.append("        p.uint_shr({}, {}, {}, {}, {})?;".format(name, st, sz, arg, m[1]))
            elif m[0] == "minus":
                o.append("        p.uint_minus({}, {}, {}, {}, {})?;".format(name, st, sz, arg, m[1]))
            elif m[0] == "align":
                o.append("        p.uint_align({}, {}, {}, {}, {})?;".format(name, st, sz, arg, m[1]))
            elif m[0] == "log2":
                o.append("        p.uint_log2({}, {}, {}, {})?;".format(name, st, sz, arg))
        elif k == "int":
            arg = v if f.rust_type(gen) == "i64" else "i64::from({})".format(v)
            o.append("        p.sint({}, {}, {}, {})?;".format(name, st, sz, arg))
        elif k == "float":
            o.append("        p.put({}, {}, 32, u64::from({}.to_bits()))?;".format(name, st, v))
        elif k == "ulod":
            o.append("        p.ulod({}, {}, {}, {})?;".format(name, st, sz, v))
        elif k == "slod":
            o.append("        p.slod({}, {}, {}, {})?;".format(name, st, sz, v))
    o.append("        p.finish()")
    o.append("    }")
    return o


def word_masks(s, leaves):
    words = s.length() // 4
    masks = [0] * words
    for lf in leaves:
        for b in range(lf.start, lf.end + 1):
            masks[b // 32] |= 1 << (b % 32)
    return masks


def emit_unpack(gen, s):
    leaves = gen.leaves(s)
    words = s.length() // 4
    o = []
    o.append("")
    o.append("    /// The fields of a packed descriptor; refuses a set bit no field owns.")
    o.append("    pub fn unpack(bytes: &[u8; {}]) -> Result<Self, PackError> {{".format(s.length()))
    masks = word_masks(s, leaves)
    o.append("        const OWNED: [u32; {}] = [{}];".format(words, ", ".join(rust_int(m, "u32") for m in masks)))
    o.append("        let {}u = Unpacker::<{}>::new(Self::XML_NAME, bytes, &OWNED)?;".format(
        "" if leaves else "_", words))
    if not s.fields:
        o.append("        Ok(Self)")
    else:
        o.append("        Ok(" + unpack_literal(gen, s, 0, "", 8) + ")")
    o.append("    }")
    return o


def conflicting_leaves(gen, s):
    """The leaves that share bits with another leaf: the members of unions."""
    ls = gen.leaves(s)
    return [a for a in ls if any(a is not b and a.overlaps(b) for b in ls)]


def emit_unpack_as(gen, s):
    leaves = gen.leaves(s)
    words = s.length() // 4
    members = conflicting_leaves(gen, s)
    member_names = {m.name_path for m in members}
    fixed = [lf for lf in leaves if lf.name_path not in member_names]
    o = []
    o.append("")
    o.append("    /// The fields of a packed descriptor in which some fields share bits")
    o.append("    /// (the members of a union): `arm` names the members (v10.xml's field")
    o.append("    /// paths) the bytes hold; every other member is left at zero. Refuses a")
    o.append("    /// set bit that no fixed field or chosen member owns, a name that is no")
    o.append("    /// member, and two chosen members that share a bit.")
    o.append("    pub fn unpack_as(bytes: &[u8; {}], arm: &[&str]) -> Result<Self, PackError> {{".format(s.length()))
    o.append("        const MEMBERS: [&str; {}] = [{}];".format(len(members), ", ".join(rust_str(m.name_path) for m in members)))
    o.append("        let arm = Arm::new(Self::XML_NAME, arm, &MEMBERS)?;")
    o.append("        let mut owned: [u32; {}] = [{}];".format(words, ", ".join(rust_int(m, "u32") for m in word_masks(s, fixed))))
    o.append("        let mut taken = [0u32; {}];".format(words))
    var = {}
    for i, m in enumerate(members):
        var[m.name_path] = "m{}".format(i)
        o.append("        let m{} = arm.claim(&mut owned, &mut taken, {}, &[{}])?;".format(
            i, rust_str(m.name_path), ", ".join(rust_int(x, "u32") for x in word_masks(s, [m]))))
    o.append("        let u = Unpacker::<{}>::new(Self::XML_NAME, bytes, &owned)?;".format(words))
    o.append("        Ok(" + unpack_literal(gen, s, 0, "", 8, var) + ")")
    o.append("    }")
    return o


def unpack_literal(gen, s, base, names, indent, arm=None):
    pad = " " * indent
    lines = ["{} {{".format(s.rust)]
    for f in s.fields:
        name = names + f.name
        k = f.kind(gen)
        st, sz = base + f.start, f.size
        if k == "struct":
            val = unpack_literal(gen, gen.structs[f.type], st, name + ".", indent + 4, arm)
        else:
            val = unpack_expr(gen, f, rust_str(name), st, sz)
            if arm and name in arm:
                val = "if {} {{ {} }} else {{ {} }}".format(arm[name], val, rust_value(gen, f, zero_value(gen, f)))
        lines.append("{}    {}: {},".format(pad, f.rust, val))
    lines.append(pad + "}")
    return "\n".join(lines)


def unpack_expr(gen, f, name, st, sz):
    k = f.kind(gen)
    if k == "bool":
        return "u.bit({})".format(st)
    if k == "enum":
        e = gen.enums[f.type].rust
        return "{}(u.enum_value({}, {}, {}, |v| {}(v).is_known(), {}::XML_NAME)?)".format(e, name, st, sz, e, e)
    if k == "uint":
        m = f.modifier
        ty = f.rust_type(gen)
        if m is None:
            inner = "u.uint({}, {})".format(st, sz)
        elif m[0] == "shr":
            inner = "u.uint_shr({}, {}, {}, {})?".format(name, st, sz, m[1])
        elif m[0] == "minus":
            inner = "u.uint_minus({}, {}, {}, {})?".format(name, st, sz, m[1])
        elif m[0] == "align":
            inner = "u.uint_align({}, {}, {}, {})?".format(name, st, sz, m[1])
        else:
            inner = "u.uint_log2({}, {}, {})?".format(name, st, sz)
        if ty == "u64":
            return inner
        return "u.narrow({}, {})?".format(name, inner)
    if k == "int":
        if f.rust_type(gen) == "i64":
            return "u.sint({}, {})".format(st, sz)
        return "u.narrow_signed({}, u.sint({}, {}))?".format(name, st, sz)
    if k == "float":
        return "f32::from_bits(u.narrow({}, u.uint({}, 32))?)".format(name, st)
    if k == "ulod":
        return "u.ulod({}, {})".format(st, sz)
    if k == "slod":
        return "u.slod({}, {})".format(st, sz)
    raise GenError(k)


def emit_aggregate(gen, el):
    name = el.get("name")
    rust = camel(name)
    o = ["/// `<aggregate name=\"{}\">`: sections packed one after another.".format(name)]
    o.append("pub struct {};".format(rust))
    o.append("")
    o.append("impl {} {{".format(rust))
    size = 0
    secs = []
    for sec in el.findall("section"):
        st = gen.structs[sec.get("type")]
        off = int(sec.get("offset"))
        if off % 4:
            raise GenError("aggregate {}: section at {}".format(name, off))
        size = max(size, off + st.length())
        secs.append((sec.get("name"), st, off))
    if el.get("size"):
        size = max(size, int(el.get("size")))
    o.append("    /// Bytes, as Mesa's `{}_LENGTH`.".format(mesa_struct(name)))
    o.append("    pub const LENGTH: usize = {};".format(size))
    if el.get("align"):
        o.append("    /// Alignment in bytes.")
        o.append("    pub const ALIGN: usize = {};".format(int(el.get("align"))))
    for sn, st, off in secs:
        o.append("    /// Where the `{}` section ([`{}`]) starts.".format(sn, st.rust))
        o.append("    pub const {}_OFFSET: usize = {};".format(upper_const(sn), off))
    o.append("}")
    o.append("")
    return o


# ---- the cases ---------------------------------------------------------------


class Rng:
    """A fixed sequence, the same on every host and Python version."""

    def __init__(self, seed):
        self.state = hashlib.sha256(seed.encode()).digest()
        self.pool = b""

    def bits(self, n):
        need = (n + 7) // 8
        while len(self.pool) < need:
            self.state = hashlib.sha256(self.state).digest()
            self.pool += self.state
        v = int.from_bytes(self.pool[:need], "little")
        self.pool = self.pool[need:]
        return v & ((1 << n) - 1)

    def below(self, n):
        """0 <= v < n."""
        if n <= 1:
            return 0
        b = (n - 1).bit_length()
        while True:
            v = self.bits(b)
            if v < n:
                return v


def zero_value(gen, f):
    """The value whose encoding is all zero bits."""
    k = f.kind(gen)
    if k == "bool":
        return False
    if k in ("float", "ulod", "slod"):
        return 0.0
    if k == "uint" and f.modifier:
        m = f.modifier
        if m[0] == "minus":
            return m[1]
        if m[0] == "log2":
            return 1
    return 0


def default_or_zero(gen, f):
    """What Mesa's header sets: the default, or zero (the value, not the
    encoding: a minus(1) field without a default is 0, which its packer
    asserts on)."""
    d = f.default_value(gen)
    if d is not None:
        return d
    k = f.kind(gen)
    if k == "bool":
        return False
    if k in ("float", "ulod", "slod"):
        return 0.0
    return 0


def valid_default(gen, f):
    """The default when Mesa's own packer accepts it, else the smallest valid
    value (a minus(1) field's default 0 trips Mesa's assert)."""
    d = default_or_zero(gen, f)
    k = f.kind(gen)
    if k == "uint" and f.modifier:
        m = f.modifier
        if m[0] == "minus" and d < m[1]:
            return m[1]
        if m[0] == "log2" and (d == 0 or d & (d - 1)):
            return 1
    return d


def random_value(gen, f, rng):
    """A valid value whose encoding is non-zero (None when the field has none)."""
    for _ in range(64):
        v = random_once(gen, f, rng)
        if v is None:
            return None
        if encoded_nonzero(gen, f, v):
            return v
    return None


def random_once(gen, f, rng):
    k = f.kind(gen)
    sz = f.size
    if k == "bool":
        return True
    if k == "enum":
        e = gen.enums[f.type]
        # zero is a candidate only to keep the draw order stable: the caller
        # keeps a value only when its encoding is non-zero, so zero never
        # survives (and the packer refuses it unless it is a named value)
        cands = sorted(({0} if enum_names_zero(e) else set()) | {v for _, v in e.values if v < (1 << sz)})
        if not cands:
            return None
        return cands[rng.below(len(cands))]
    if k == "int":
        v = rng.bits(sz)
        if v >= 1 << (sz - 1):
            v -= 1 << sz
        return v if v != 0 else 1
    if k == "float":
        # a value with few significant bits: exact in binary32 and in a literal
        return (rng.below(2001) - 1000) / 8.0
    if k == "ulod":
        return (1 + rng.below((1 << sz) - 1)) / 256.0
    if k == "slod":
        v = rng.bits(sz)
        if v >= 1 << (sz - 1):
            v -= 1 << sz
        return (v if v != 0 else 1) / 256.0
    # uint
    m = f.modifier
    e = 1 + rng.below((1 << sz) - 1)
    if m is None:
        return e
    if m[0] == "shr":
        return e << m[1]
    if m[0] == "minus":
        return e + m[1]
    if m[0] == "align":
        n = ((1 << sz) - 1) // m[1]
        if n < 1:
            return None
        return (1 + rng.below(n)) * m[1]
    if m[0] == "log2":
        limit = 31 if f.rust_type(gen) == "u32" else 63
        e = 1 + rng.below(min((1 << sz) - 1, limit))
        return 1 << e
    raise GenError(m)


def encoded_nonzero(gen, f, v):
    """Is the field's encoding of `v` non-zero (what the packer claims bits for)?"""
    k = f.kind(gen)
    if k == "bool":
        return bool(v)
    if k in ("float", "ulod", "slod"):
        return v != 0.0
    if k == "uint" and f.modifier:
        m = f.modifier
        if m[0] == "minus":
            return v != m[1]
        if m[0] == "log2":
            return v != 1
    return v != 0


def shared_bits_refusal(gen, s, values):
    """For the struct's defaults (`values` holds the fixed fields), the first
    field whose non-zero default shares bits with an earlier non-zero one, as
    the packer meets them: (field path, the other's path), or None."""
    claimed = []
    for lf in gen.leaves(s):
        v = values.get(lf.rust_path, default_or_zero(gen, lf.field))
        if not encoded_nonzero(gen, lf.field, v):
            continue
        for o in claimed:
            if lf.overlaps(o):
                return (lf.name_path, o.name_path)
        claimed.append(lf)
    return None


def first_named_nonzero(gen, f):
    """The smallest named value of the field's enum that is not zero."""
    e = gen.enums[f.type]
    return min(v for _, v in e.values if v != 0 and v < (1 << f.size))


def unset_enum_refusal(gen, s, values):
    """For the struct's defaults (`values` holds the fixed fields): the first
    enum leaf left at an unnamed zero whose bits no non-zero field takes, as
    the packer's finish() meets them: its field path, or None."""
    leaves = gen.leaves(s)
    taken = [lf for lf in leaves if encoded_nonzero(gen, lf.field, values.get(lf.rust_path, default_or_zero(gen, lf.field)))]
    for lf in leaves:
        f = lf.field
        if f.kind(gen) != "enum" or enum_names_zero(gen.enums[f.type]):
            continue
        if values.get(lf.rust_path, default_or_zero(gen, f)) == 0 and not any(lf.overlaps(t) for t in taken):
            return lf.name_path
    return None


def fill_unset_enums(gen, s, values):
    """Give every enum leaf at an unnamed zero that no non-zero field covers
    its smallest named value (in leaf order, so a later leaf that shares bits
    with a filled one is covered by it). `values` is changed in place."""
    leaves = gen.leaves(s)
    for lf in leaves:
        f = lf.field
        if f.kind(gen) != "enum" or enum_names_zero(gen.enums[f.type]):
            continue
        if values.get(lf.rust_path, default_or_zero(gen, f)) != 0:
            continue
        taken = [o for o in leaves if encoded_nonzero(gen, o.field, values.get(o.rust_path, default_or_zero(gen, o.field)))]
        if not any(lf.overlaps(t) for t in taken):
            values[lf.rust_path] = first_named_nonzero(gen, f)


def make_cases(gen):
    """[(struct, case id, {leaf rust path: value} or None for 'defaults')]."""
    cases = []
    for _, s in [x for x in gen.order if x[0] == "struct"]:
        if not s.packable():
            continue
        leaves = gen.leaves(s)
        # the defaults case: only the fields whose default Mesa refuses
        fix = {}
        for lf in leaves:
            if valid_default(gen, lf.field) != default_or_zero(gen, lf.field):
                fix[lf.rust_path] = valid_default(gen, lf.field)
        cases.append((s, "{}/defaults".format(s.rust), ("defaults", fix)))
        if unset_enum_refusal(gen, s, fix) is not None:
            # the defaults are refused (a planned refusal); the same with a
            # named value in each unset enum keeps Mesa's header compared
            named = dict(fix)
            fill_unset_enums(gen, s, named)
            cases.append((s, "{}/defaults_named".format(s.rust), ("defaults", named)))
        covered = set()
        possible = set()
        rng = Rng("v10:" + s.name)
        n = 0
        while True:
            picked = []
            order = list(range(len(leaves)))
            # the first uncovered field first, then the others in a random order
            first = [i for i in order if i not in covered and i not in possible]
            rest = [i for i in order if i not in first[:1]]
            for i in range(len(rest) - 1, 0, -1):
                j = rng.below(i + 1)
                rest[i], rest[j] = rest[j], rest[i]
            values = {}
            for i in first[:1] + rest:
                lf = leaves[i]
                if any(lf.overlaps(leaves[j]) for j in picked):
                    continue
                v = random_value(gen, lf.field, rng)
                if v is None:
                    possible.add(i)
                    continue
                picked.append(i)
                values[lf.rust_path] = v
            for i, lf in enumerate(leaves):
                if i not in picked:
                    values[lf.rust_path] = zero_value(gen, lf.field)
            fill_unset_enums(gen, s, values)
            covered |= set(picked)
            n += 1
            cases.append((s, "{}/{}".format(s.rust, n), ("all", values)))
            left = [i for i in range(len(leaves)) if i not in covered and i not in possible]
            if not left and n >= 3:
                break
            if n > 64:
                raise GenError("{}: fields never covered: {}".format(s.name, [leaves[i].name_path for i in left]))
        never = [leaves[i].name_path for i in possible if i not in covered]
        if never:
            raise GenError("{}: no case encodes a non-zero value for {}".format(s.name, never))
    return cases


def leaf_value_c(gen, f, v):
    k = f.kind(gen)
    if k == "bool":
        return "true" if v else "false"
    if k in ("float", "ulod", "slod"):
        return float(v).hex() + "f"
    if k == "int":
        return "{}ll".format(v) if f.size > 32 else str(v)
    return "0x{:x}ull".format(v)


CASES_HEADER = """\
// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2020 Collabora Ltd.
// SPDX-FileCopyrightText: 2026 Arm Ltd.
//
// v10_cases.rs - GENERATED by tools/mali/gen_pack_rs.py from Mesa 26.2.1's
// src/panfrost/genxml/v10.xml. DO NOT EDIT: tools/check.sh regenerates it.
//
//! The cross-check cases of [`super::v10`], {count} of them: the same values
//! go through Mesa's own packers in C (`tools/mali/gen_pack_rs.py c-cases`,
//! compiled with tools/mali/pack_ref.c) and the test compares the bytes
//! (`super::mesa_tests`).
// The layout is the generator's: regenerated, not formatted.
#![cfg_attr(rustfmt, rustfmt_skip)]

use super::pack::PackError;
use super::v10::*;

/// One case: the bytes `pack()` gave, and, for a struct with `unpack()` or `unpack_as()`,
/// whether unpacking them gave the same value back. `refused` names the
/// refusal the case must meet instead of Mesa's bytes: Mesa's header of a
/// union that sets two of its members (it ORs them; the packer refuses).
pub struct CaseOut {{
    pub bytes: Result<Vec<u8>, PackError>,
    pub round_trip: Option<Result<bool, PackError>>,
    pub refused: Option<&'static str>,
}}

/// A case: its id (`<struct>/<n>`) and the function that packs it.
pub type Case = (&'static str, fn() -> CaseOut);

/// The cases, in the order the C harness prints them.
pub fn cases() -> Vec<Case> {{
    vec![
"""


def rust_case_literal(gen, s, values, mode, indent, prefix=""):
    pad = " " * indent
    out = ["{} {{".format(s.rust)]
    all_set = True
    for f in s.fields:
        rp = prefix + f.rust
        if f.kind(gen) == "struct":
            sub = gen.structs[f.type]
            if mode == "defaults" and not any(k.startswith(rp + ".") for k in values):
                all_set = False
                continue
            out.append("{}    {}: {},".format(pad, f.rust, rust_case_literal(gen, sub, values, mode, indent + 4, rp + ".")))
        elif rp in values:
            out.append("{}    {}: {},".format(pad, f.rust, rust_value(gen, f, values[rp])))
        else:
            all_set = False
    if not all_set:
        out.append("{}    ..Default::default()".format(pad))
    out.append(pad + "}")
    if len(out) == 2 and not all_set:
        return "{}::default()".format(s.rust)
    return "\n".join(out)


def emit_cases_rust(gen):
    cases = make_cases(gen)
    o = [CASES_HEADER.format(count=len(cases))]
    for s, cid, (mode, values) in cases:
        o.append("        ({}, case_{} as fn() -> CaseOut),".format(rust_str(cid), case_fn(cid)))
    o.append("    ]")
    o.append("}")
    o.append("")
    for s, cid, (mode, values) in cases:
        refused = "None"
        if mode == "defaults":
            r = shared_bits_refusal(gen, s, values)
            u = unset_enum_refusal(gen, s, values)
            if r:
                refused = "Some({})".format(rust_str("mali {}.{}: shares bits with {}, and both are non-zero".format(
                    s.name, r[0], r[1])))
            elif u:
                refused = "Some({})".format(rust_str("mali {}.{}: enum value 0 is not a named value".format(s.name, u)))
        o.append("fn case_{}() -> CaseOut {{".format(case_fn(cid)))
        o.append("    let v = " + rust_case_literal(gen, s, values, mode, 4) + ";")
        if s.packable():
            o.append("    let bytes = v.pack();")
            o.append("    let round_trip = Some(match &bytes {")
            if gen.has_shared_bits(s):
                arm = [m.name_path for m in conflicting_leaves(gen, s)
                       if encoded_nonzero(gen, m.field, values.get(m.rust_path, default_or_zero(gen, m.field)))]
                o.append("        Ok(b) => {}::unpack_as(b, &[{}]).map(|u| u == v),".format(
                    s.rust, ", ".join(rust_str(a) for a in arm)))
            else:
                o.append("        Ok(b) => {}::unpack(b).map(|u| u == v),".format(s.rust))
            o.append("        Err(e) => Err(*e),")
            o.append("    });")
            o.append("    CaseOut {{ bytes: bytes.map(|b| b.to_vec()), round_trip, refused: {} }}".format(refused))
        else:
            o.append("    CaseOut {{ bytes: v.pack().map(|b| b.to_vec()), round_trip: None, refused: {} }}".format(refused))
        o.append("}")
        o.append("")
    return "\n".join(o)


def case_fn(cid):
    return cid.replace("/", "_").lower()


def emit_cases_c(gen):
    cases = make_cases(gen)
    o = ["/* GENERATED by tools/mali/gen_pack_rs.py c-cases; included by tools/mali/pack_ref.c */"]
    o.append("#define PACK_REF_CASES {}".format(len(cases)))
    o.append("static void run_cases(void)")
    o.append("{")
    for s, cid, (mode, values) in cases:
        o.append("   {")
        o.append("      struct {} v = {{ {}_header }};".format(s.c, s.c))
        o.append("      struct {}_packed p;".format(s.c.lower()))
        for lf in gen.leaves(s):
            if lf.rust_path in values:
                o.append("      v.{} = {};".format(lf.c_path, leaf_value_c(gen, lf.field, values[lf.rust_path])))
        if not gen.leaves(s):
            o.append("      (void)v;")
        o.append("      {}_pack(&p, &v);".format(s.c))
        o.append("      print_case(\"{}\", &p, sizeof(p));".format(cid))
        o.append("   }")
    o.append("}")
    return "\n".join(o) + "\n"


def main(argv):
    if len(argv) != 3 or argv[1] not in ("rust", "cases", "c-cases"):
        sys.stderr.write(__doc__)
        return 2
    try:
        gen = Gen(argv[2])
        if argv[1] == "rust":
            sys.stdout.write(emit_rust(gen))
        elif argv[1] == "cases":
            sys.stdout.write(emit_cases_rust(gen))
        else:
            sys.stdout.write(emit_cases_c(gen))
    except GenError as e:
        sys.stderr.write("gen_pack_rs.py: {}\n".format(e))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
