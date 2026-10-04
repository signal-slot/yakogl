#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# extract.py - read Mesa's pco ISA description and write the same facts as JSON.
#
# Mesa's PowerVR Rogue compiler (src/imagination/pco, MIT licence) keeps its
# instruction encoding as data rather than code.  pco_isa.py declares every bit
# set (named bit pieces at byte/bit positions, and fields assembled out of
# them) and every bit struct (one encodable instruction form: which fields it
# leaves settable, which it pins to a fixed value, and how many bytes it is).
# pco_map.py declares, per operation, which bit struct encodes it and which
# source, ISS and destination sections its instruction group carries.
#
# This script imports those declarations and writes them out as JSON;
# gen_tables.py turns that JSON into the Rust tables in src/shaderc/online/
# that our decoder and encoder share.  Nothing from Mesa is copied into this
# repository - the relationship is the one tools/glsl2usc.sh already has with
# pco_shc, and the generated Rust carries the MIT notice for the derived data.
#
# Usage: extract.py <path to mesa src/imagination/pco> <out.json>

import json
import os
import sys

# The marker in pco_map.py after which the file is nothing but encode_map()
# and group_map() calls.  We import the module once so its definitions exist,
# then re-run that tail with recording stand-ins, because the objects the real
# calls build hold Mesa's generated C, while the call arguments hold the facts.
MAP_TAIL_MARKER = "# Encode mappings."


def field_type_json(t):
    return {
        "tname": t.tname,
        "base": t.base_type.name,
        "num_bits": t.num_bits,
        "dec_bits": t.dec_bits,
        "check": t.check,
        "encode": t.encode,
        "enum": t.enum.ename if t.enum is not None else None,
        "enum_parent": t.enum.parent.enum.ename
        if (t.enum is not None and t.enum.parent is not None)
        else None,
    }


def collect_enums(C):
    enums = {}

    def add(t):
        if t.enum is None:
            return
        e = t.enum
        if e.ename in enums:
            return
        # A subtype (enum_subtype) carries no element table of its own: it is
        # the parent's elements narrowed to those that fit in fewer bits.
        elems = e.elems
        valid = e.valid
        if elems is None:
            assert e.parent is not None
            add_parent = e.parent.enum
            elems = {
                name: el
                for name, el in add_parent.elems.items()
                if el.value in valid
            }
        enums[e.ename] = {
            "name": e.ename,
            "cname": e.name,
            "num_bits": t.num_bits,
            "is_bitset": bool(e.is_bitset),
            "parent": e.parent.enum.ename if e.parent is not None else None,
            "elems": [
                {"name": name, "cname": el.cname, "value": el.value, "string": el.string}
                for name, el in elems.items()
            ],
        }

    for bs in C.bit_sets.values():
        for f in bs.fields.values():
            add(f.field_type)
    return enums


def collect_bit_sets(C):
    bit_sets = {}
    for bsname, bs in C.bit_sets.items():
        fields = {}
        # cname -> field name, so a bit struct's encode list can be mapped back.
        by_cname = {}
        for fname, f in bs.fields.items():
            fields[fname] = {
                "name": fname,
                "type": field_type_json(f.field_type),
                "reserved": f.reserved,
                "pieces": [
                    {"name": p.name, "byte": p.byte, "hi": p.hi_bit, "lo": p.lo_bit}
                    for p in f.pieces
                ],
            }
            by_cname[f.cname] = fname

        structs = []
        for sname, st in bs.bit_structs.items():
            enc = []
            for ef in st.encode_fields:
                fname = by_cname[ef.name]
                v = ef.value
                if isinstance(v, str) and v.startswith("s."):
                    enc.append({"field": fname, "kind": "settable", "struct_field": v[2:]})
                elif isinstance(v, str):
                    # A fixed enum element, stored by its C name.
                    ft = bs.fields[fname].field_type
                    assert ft.enum is not None, (bsname, sname, fname, v)
                    val = next(
                        el.value for el in ft.enum.elems.values() if el.cname.upper() == v
                    )
                    enc.append({"field": fname, "kind": "fixed", "value": val})
                else:
                    enc.append({"field": fname, "kind": "fixed", "value": int(v)})
            data = st.data
            structs.append(
                {
                    "name": sname,
                    "cname": st.name,
                    "bytes": st.num_bytes,
                    "encode": enc,
                    "struct_fields": list(st.struct_fields.keys()),
                    "src_spec": None
                    if data is None or not hasattr(data, "is_upper")
                    else {
                        "is_upper": data.is_upper,
                        "sbA_bits": data.sbA_bits,
                        "sA_bits": data.sA_bits,
                        "sbB_bits": data.sbB_bits,
                        "sB_bits": data.sB_bits,
                        "sbC_bits": data.sbC_bits,
                        "sC_bits": data.sC_bits,
                        "mux_bits": data.mux_bits,
                    },
                }
            )
        bit_sets[bsname] = {"name": bsname, "cname": bs.name, "fields": fields, "structs": structs}
    return bit_sets


def collect_maps(pco_dir):
    """Re-run the tail of pco_map.py with recording stand-ins for its two
    registration calls, and return what each call was actually given."""
    import pco_map

    src = open(os.path.join(pco_dir, "pco_map.py")).read()
    idx = src.index(MAP_TAIL_MARKER)
    tail = src[idx:]

    encode_calls = []
    group_calls = []

    def const_fields(isa_op, isa_op_fields):
        """The field values this encoding pins to a constant.  They are what
        tells two operations that share a bit struct apart (I_SNGL is FRCP
        with sngl_op=rcp and MBYP with sngl_op=byp), so the decoder needs
        them to name what it just read."""
        out = {}
        for field, val_spec in isa_op_fields:
            if isinstance(val_spec, bool):
                out[field] = int(val_spec)
            elif isinstance(val_spec, int):
                out[field] = val_spec
            elif isinstance(val_spec, str):
                sf = isa_op.struct_fields.get(field)
                if sf is None or sf.type.base_type.name != "enum":
                    continue
                enum = sf.type.enum
                if val_spec not in enum.elems:
                    continue
                out[field] = enum.elems[val_spec].value
        return out

    def rec_encode_map(op, encodings, op_ref_maps):
        encode_calls.append(
            {
                "op": op.name,
                "encodings": [
                    {"struct": enc[0].name, "consts": const_fields(enc[0], enc[1])}
                    for enc in encodings
                ],
                "phases": [m[0] for m in op_ref_maps],
            }
        )

    def rec_group_map(op, hdr, enc_ops, srcs=[], iss=[], dests=[]):
        group_calls.append(
            {
                "op": op.name,
                "hdr": hdr[0].name,
                "enc_ops": [(e[0], e[1].name) for e in enc_ops],
                "srcs": [s[-1] for s in srcs],
                "iss": [i[0] for i in iss],
                "dests": [d[-1] for d in dests],
            }
        )

    ns = dict(pco_map.__dict__)
    ns["encode_map"] = rec_encode_map
    ns["group_map"] = rec_group_map
    exec(compile(tail, "pco_map.py<tail>", "exec"), ns)

    return encode_calls, group_calls


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: extract.py <mesa src/imagination/pco> <out.json>")
    pco_dir, out_path = sys.argv[1], sys.argv[2]
    if not os.path.isfile(os.path.join(pco_dir, "pco_isa.py")):
        sys.exit(f"{pco_dir}: no pco_isa.py there")
    sys.path.insert(0, os.path.abspath(pco_dir))

    import pco_pygen_common as C
    import pco_isa  # noqa: F401  (populates C.bit_sets)

    encode_calls, group_calls = collect_maps(os.path.abspath(pco_dir))

    doc = {
        "source": "Mesa src/imagination/pco (MIT); facts extracted, not copied",
        "enums": collect_enums(C),
        "bit_sets": collect_bit_sets(C),
        "encode_maps": encode_calls,
        "group_maps": group_calls,
    }
    with open(out_path, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
        f.write("\n")

    nstructs = sum(len(bs["structs"]) for bs in doc["bit_sets"].values())
    print(
        f"bit sets {len(doc['bit_sets'])}, bit structs {nstructs}, "
        f"enums {len(doc['enums'])}, encode maps {len(encode_calls)}, "
        f"group maps {len(group_calls)} -> {out_path}",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
