#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# Transcribe the Rogue USC instruction encodings into YakoGL's own ISA table.
#
# The encodings are read from Mesa's `pco_isa.py`, which describes them in a
# small Python DSL (`pco_pygen_common.py`).  That tree is NOT vendored here: it
# is imported from wherever it sits on the build host, named by --mesa or by
# $MESA_TREE, and only the encoding facts are written out, in our own schema.
#
#   Mesa's pco is Copyright (c) 2024 Imagination Technologies Ltd, MIT licensed.
#
# The output is docs/online-shaderc/isa/usc-isa.json; docs/online-shaderc/isa.md
# is the prose that explains it.  Regenerate with:
#
#   tools/isa-extract/pco_isa_extract.py -o docs/online-shaderc/isa/usc-isa.json
#
# `--check` re-generates and diffs against the committed file instead of
# writing it, so a stale table is a failure and not a surprise.

import argparse
import hashlib
import json
import os
import subprocess
import sys

DEFAULT_MESA = os.path.expanduser('~/pvr-lab/mesa-build/mesa-26.2.1')
PCO_SUBDIR = 'src/imagination/pco'

# The schema version of the JSON we emit.  Bump it when the shape changes, so a
# consumer that reads an older table says so instead of mis-reading it.
SCHEMA_VERSION = 1


def load_pco(mesa_tree):
    """Import Mesa's ISA description and return its pygen_common module."""
    pco_dir = os.path.join(mesa_tree, PCO_SUBDIR)
    isa_py = os.path.join(pco_dir, 'pco_isa.py')
    if not os.path.isfile(isa_py):
        sys.exit(
            f'{isa_py}: not found.\n'
            'Point --mesa (or $MESA_TREE) at a Mesa checkout that has '
            'src/imagination/pco; the tree is not vendored in this repository.'
        )
    sys.path.insert(0, pco_dir)
    import pco_pygen_common as common  # noqa: E402
    import pco_isa  # noqa: F401,E402  (importing it is what builds the tables)
    return common, pco_dir


def mesa_provenance(mesa_tree, pco_dir, common):
    """Record exactly which input produced this table."""
    version = None
    ver_file = os.path.join(mesa_tree, 'VERSION')
    if os.path.isfile(ver_file):
        with open(ver_file) as f:
            version = f.read().strip()

    describe = None
    try:
        describe = subprocess.run(
            ['git', 'describe', '--always', '--dirty'],
            cwd=mesa_tree, capture_output=True, text=True, timeout=10,
        ).stdout.strip() or None
    except (OSError, subprocess.SubprocessError):
        pass

    # Hash the two files the table is actually derived from, so a changed input
    # is visible even when the tree carries no version or git metadata.
    inputs = {}
    for name in ('pco_isa.py', 'pco_pygen_common.py'):
        path = os.path.join(pco_dir, name)
        with open(path, 'rb') as f:
            inputs[name] = hashlib.sha256(f.read()).hexdigest()

    return {
        'source': 'Mesa src/imagination/pco (pco_isa.py), transcribed, not vendored',
        'source_copyright': 'Copyright (c) 2024 Imagination Technologies Ltd.',
        'source_license': 'MIT',
        'mesa_version': version,
        'mesa_describe': describe,
        'input_sha256': inputs,
        'generator': 'tools/isa-extract/pco_isa_extract.py',
        'schema_version': SCHEMA_VERSION,
        'note': (
            'pco_isa.py carries no BVNC or feature gating: the encodings below '
            'are core-independent.  What differs per core in pco lives in the '
            'NIR lowering passes, not in these bit layouts.  Verified against '
            'BVNC 36.53.104.796 by decoding driver/src/pvr_usc_*_blob.h; see '
            'docs/online-shaderc/isa.md.'
        ),
    }


def base_type_name(t):
    return t.base_type.name


def enum_elems(common, enum):
    """Elements of an enum, resolving a subtype through its parent.

    A subtype stores no elements of its own: it is the parent enum narrowed to
    the values that still fit, so its elements are the parent's that fit.
    """
    if enum.elems is not None:
        return [
            {'name': name, 'value': e.value, 'string': e.string}
            for name, e in enum.elems.items()
        ]
    # A subtype's `parent` is the parent *Type*, whose `.enum` holds the
    # elements; the subtype keeps only the values that still fit.
    parent = enum.parent
    assert parent is not None, f'enum {enum.ename} has neither elems nor parent'
    return [
        {'name': name, 'value': e.value, 'string': e.string}
        for name, e in parent.enum.elems.items()
        if e.value in enum.valid
    ]


def dump_enums(common):
    out = {}
    for name, enum in common.field_enum_types.items():
        parent = enum.parent.tname if enum.parent is not None else None
        t = common.field_types[name]
        out[name] = {
            'num_bits': t.num_bits,
            'parent': parent,
            'is_bitset': enum.is_bitset,
            'elems': enum_elems(common, enum),
        }
    return out


def dump_field_types(common):
    out = {}
    for name, t in common.field_types.items():
        out[name] = {
            'base': base_type_name(t),
            'num_bits': t.num_bits,
            # dec_bits is the *decoded* width when it differs from the encoded
            # width; check/encode are the transform between the two.
            'dec_bits': t.dec_bits,
            'check': t.check,
            'encode': t.encode,
            'enum': t.enum.ename if t.enum is not None else None,
        }
    return out


def field_cname(bit_set, field_name):
    return f'{bit_set.name}_{field_name}'.upper()


def resolve_pinned(common, field, value):
    """Turn a pinned field value into {value, name}.

    bit_struct() has already converted an enum's element name into its C name,
    so map that back to the element and its number.  Everything else is already
    an int (a bool having been narrowed to 0/1).
    """
    if not isinstance(value, str):
        return {'value': int(value), 'name': None}

    enum = field.field_type.enum
    assert enum is not None, (
        f'pinned string value {value!r} on non-enum field {field.name}'
    )
    elems = enum.elems
    if elems is None:
        elems = enum.parent.enum.elems
    for elem_name, elem in elems.items():
        if elem.cname.upper() == value:
            return {'value': elem.value, 'name': elem_name}
    raise AssertionError(f'pinned value {value!r} not an element of {enum.ename}')


def dump_bit_structs(common, bit_set):
    out = {}
    for name, bs in bit_set.bit_structs.items():
        pinned = {}
        for ef in bs.encode_fields:
            # Recover the bit_set field this encode entry belongs to.
            field_name = None
            for candidate in bit_set.fields:
                if field_cname(bit_set, candidate) == ef.name:
                    field_name = candidate
                    break
            assert field_name is not None, f'unknown encode field {ef.name}'
            field = bit_set.fields[field_name]

            # 's.<x>' means "taken from the struct", i.e. settable, not pinned.
            if isinstance(ef.value, str) and ef.value.startswith('s.'):
                continue
            # A reserved field is always that value in every variant, so it is
            # not a discriminator; it is already recorded on the field itself.
            if field.reserved is not None:
                continue
            pinned[field_name] = resolve_pinned(common, field, ef.value)

        settable = {}
        for sf_name, sf in bs.struct_fields.items():
            # The struct field's own name can differ from the bit_set field it
            # is mapped to (e.g. struct 'cc' <- field 'cc1' in main_brief).
            mapped = None
            for candidate in bit_set.fields:
                cand_cname = field_cname(bit_set, candidate)
                for ef in bs.encode_fields:
                    if ef.name == cand_cname and ef.value == f's.{sf_name}':
                        mapped = candidate
                        break
                if mapped:
                    break
            assert mapped is not None, f'no field mapped to struct field {sf_name}'
            settable[sf_name] = {
                'field': mapped,
                'type': sf.type.tname,
                'bits': sf.bits,
            }

        out[name] = {
            'num_bytes': bs.num_bytes,
            'pinned': pinned,
            'settable': settable,
        }
    return out


def dump_bit_sets(common):
    out = {}
    for name, bit_set in common.bit_sets.items():
        pieces = {
            p_name: {
                'byte': p.byte,
                'hi': p.hi_bit,
                'lo': p.lo_bit,
                'num_bits': p.num_bits,
            }
            for p_name, p in bit_set.pieces.items()
        }
        fields = {
            f_name: {
                'type': f.field_type.tname,
                # Pieces are listed most-significant first: concatenating them
                # in this order reconstructs the field's value.
                'pieces': [p.name for p in f.pieces],
                'num_bits': f.field_type.num_bits,
                'reserved': f.reserved,
            }
            for f_name, f in bit_set.fields.items()
        }
        out[name] = {
            'pieces': pieces,
            'fields': fields,
            'structs': dump_bit_structs(common, bit_set),
        }
    return out


def build(mesa_tree):
    common, pco_dir = load_pco(mesa_tree)
    return {
        'provenance': mesa_provenance(mesa_tree, pco_dir, common),
        'enums': dump_enums(common),
        'field_types': dump_field_types(common),
        'bit_sets': dump_bit_sets(common),
    }


def render(table):
    return json.dumps(table, indent=2, sort_keys=False) + '\n'


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--mesa', default=os.environ.get('MESA_TREE', DEFAULT_MESA),
                    help='Mesa checkout holding src/imagination/pco')
    ap.add_argument('-o', '--output', help='write the table here')
    ap.add_argument('--check', action='store_true',
                    help='compare against --output instead of writing it')
    ap.add_argument('--stats', action='store_true',
                    help='print a summary of what was transcribed')
    args = ap.parse_args()

    table = build(args.mesa)
    text = render(table)

    if args.stats:
        bs = table['bit_sets']
        print(f'enums        {len(table["enums"])}')
        print(f'field types  {len(table["field_types"])}')
        print(f'bit sets     {len(bs)}')
        for name, s in bs.items():
            print(f'  {name:10s} pieces {len(s["pieces"]):3d} '
                  f'fields {len(s["fields"]):3d} variants {len(s["structs"]):3d}')
        print(f'variants     {sum(len(s["structs"]) for s in bs.values())}')

    if args.check:
        if not args.output:
            sys.exit('--check needs --output')
        with open(args.output) as f:
            current = f.read()
        if current != text:
            sys.exit(f'{args.output}: stale; re-run without --check to regenerate')
        print(f'{args.output}: up to date')
        return

    if args.output:
        os.makedirs(os.path.dirname(args.output), exist_ok=True)
        with open(args.output, 'w') as f:
            f.write(text)
        # Report the artefact as it landed on disk, not as it was generated.
        with open(args.output, 'rb') as f:
            written = f.read()
        print(f'{args.output}: {len(written)} bytes, '
              f'sha256 {hashlib.sha256(written).hexdigest()[:16]}')
    else:
        sys.stdout.write(text)


if __name__ == '__main__':
    main()
