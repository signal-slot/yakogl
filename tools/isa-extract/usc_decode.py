#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# A reference decoder for Rogue USC binaries, driven entirely by the table that
# pco_isa_extract.py produces.  It exists to answer one question: does the table
# describe the shaders this driver already runs on silicon?
#
# It is a verification instrument, not the product.  The shipping decoder and
# encoder are Rust, in src/shaderc/online/.  When the two disagree, the board
# decides, not this script.
#
#   tools/isa-extract/usc_decode.py --blobs        # our hand-verified blobs
#   tools/isa-extract/usc_decode.py --hex '58 a0 04 ...'
#
# Group layout (driver/docs/online-shaderc/isa.md "Instruction groups"), taken
# from Mesa's pco_binary.c:pco_encode_igrp() and pco_group_instrs.c:calc_lengths():
#
#   [header 2 or 3 bytes] [instructions, phases in REVERSE order]
#   [lower srcs] [upper srcs] [iss] [dests] [word pad] [align pad]
#
# with hdr.length counting 2-byte WORDS of the whole group, and hdr.da the byte
# offset at which the source/dest area begins (see calc_da()).

import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DEFAULT_TABLE = os.path.join(REPO, 'docs/online-shaderc/isa/usc-isa.json')

# The blobs this driver has run on silicon, and what they are.
BLOBS = [
    ('driver/src/pvr_usc_vs_passthrough_blob.h', 'pvr_usc_vs_passthrough',
     'passthrough VS: gl_Position = vec4(in.xyz, 1.0)'),
    ('driver/src/pvr_usc_clear_fs_blob.h', 'pvr_usc_clear_fs_words',
     'clear FS, one output dword'),
    ('driver/src/pvr_usc_clear_fs2_blob.h', 'pvr_usc_clear_fs2_words',
     'clear FS, two output dwords'),
    ('driver/src/pvr_usc_eot_blob.h', None, 'end-of-tile program'),
]


class Table:
    def __init__(self, path):
        with open(path) as f:
            self.t = json.load(f)
        self.bit_sets = self.t['bit_sets']
        self.field_types = self.t['field_types']
        self.enums = self.t['enums']

    def extract(self, bs_name, data, off, field_name):
        """Pull one logical field out of `data` at byte offset `off`.

        Pieces are listed most-significant first, so each one shifts what came
        before it up by its own width.
        """
        bs = self.bit_sets[bs_name]
        field = bs['fields'][field_name]
        value = 0
        for piece_name in field['pieces']:
            p = bs['pieces'][piece_name]
            idx = off + p['byte']
            if idx >= len(data):
                return None  # Ran off the end: this variant cannot be the one.
            mask = (1 << p['num_bits']) - 1
            value = (value << p['num_bits']) | ((data[idx] >> p['lo']) & mask)
        return value

    def decode_field(self, bs_name, data, off, field_name):
        """Extract a field and undo the encode transform, giving a raw + name."""
        raw = self.extract(bs_name, data, off, field_name)
        if raw is None:
            return None
        ft_name = self.bit_sets[bs_name]['fields'][field_name]['type']
        ft = self.field_types[ft_name]
        value = raw
        enc = ft.get('encode')
        # The three transforms pco uses; anything else is reported as raw so a
        # new one cannot be silently mis-decoded.
        if enc == '{} - 1':
            value = raw + 1
        elif enc == '{} % 16':
            value = raw if raw != 0 else 16
        elif enc == '{} >> 1':
            value = raw << 1
        elif enc == '{} >> 2':
            value = raw << 2
        elif enc is not None:
            return {'raw': raw, 'value': None, 'name': None,
                    'unhandled_encode': enc}
        name = None
        if ft['enum']:
            for e in self.enums[ft['enum']]['elems']:
                if e['value'] == value:
                    name = e['name']
                    break
        return {'raw': raw, 'value': value, 'name': name}

    def match_variants(self, bs_name, data, off):
        """Every variant of `bs_name` whose pinned fields match at `off`."""
        hits = []
        for vname, v in self.bit_sets[bs_name]['structs'].items():
            ok = True
            for fname, pin in v['pinned'].items():
                got = self.extract(bs_name, data, off, fname)
                if got is None or got != pin['value']:
                    ok = False
                    break
            if ok and off + v['num_bytes'] <= len(data):
                hits.append((vname, v))
        return hits

    def read_variant(self, bs_name, data, off, vname):
        v = self.bit_sets[bs_name]['structs'][vname]
        out = {}
        for sf_name, sf in v['settable'].items():
            out[sf_name] = self.decode_field(bs_name, data, off, sf['field'])
        return out


def decode_header(tab, data, off):
    """Decode a group header; returns (variant, fields, hdr_bytes)."""
    ext = (data[off + 1] >> 7) & 1
    if not ext:
        variant = 'main_brief'
        alutype = 'main'
    else:
        alu = (data[off + 2] >> 5) & 0b11
        variant = {0: 'main', 2: 'bitwise', 3: 'control'}.get(alu)
        if variant is None:
            raise ValueError(f'reserved alutype {alu} at byte {off + 2}')
        alutype = variant
    nbytes = tab.bit_sets['igrp_hdr']['structs'][variant]['num_bytes']
    return variant, tab.read_variant('igrp_hdr', data, off, variant), nbytes, alutype


# Which phases an oporg value says are present, in encode order (reverse phase).
OPORG_PHASES = {
    'p0': ['p0'],
    'p2': ['p2'],
    'be': ['backend'],
    'p0_p1': ['p0', 'p1'],
    'p0_p2': ['p0', 'p2'],
    'p0_p1_p2': ['p0', 'p1', 'p2'],
    'p0_p2_be': ['p0', 'p2', 'backend'],
    'p0_p1_p2_be': ['p0', 'p1', 'p2', 'backend'],
}


def decode_group(tab, data, off):
    """Decode one instruction group starting at `off`."""
    hdr_variant, hdr, hdr_bytes, alutype = decode_header(tab, data, off)

    length = hdr['length']['value']
    total = length * 2          # hdr.length counts 2-byte words.
    da = hdr['da']['value']

    g = {
        'offset': off,
        'total_bytes': total,
        'hdr_variant': hdr_variant,
        'hdr_bytes': hdr_bytes,
        'alutype': alutype,
        'da': da,
        'fields': hdr,
        'instrs': [],
        'notes': [],
    }

    if alutype == 'main':
        oporg = hdr['oporg']['name']
        phases = OPORG_PHASES.get(oporg)
        if phases is None:
            g['notes'].append(f'unknown oporg {oporg}')
            return g
        # Encoded in reverse phase order: backend first, then p2, p1, p0.
        order = list(reversed(phases))
    elif alutype == 'control':
        order = ['ctrl']
    else:
        order = ['bitwise']

    cur = off + hdr_bytes
    for phase in order:
        bs_name = {'backend': 'backend', 'ctrl': 'ctrl',
                   'bitwise': 'bitwise'}.get(phase, 'main')
        hits = tab.match_variants(bs_name, data, cur)
        if not hits:
            g['notes'].append(f'{phase}: no variant of {bs_name} matches '
                              f'at byte {cur - off} of the group')
            break
        # Prefer the longest match: the extension bits that pick a longer
        # variant are themselves pinned, so a longer hit is the specific one.
        hits.sort(key=lambda h: -h[1]['num_bytes'])
        best_len = hits[0][1]['num_bytes']
        tied = [h for h in hits if h[1]['num_bytes'] == best_len]
        vname, v = tied[0]
        entry = {
            'phase': phase,
            'bit_set': bs_name,
            'variant': vname,
            'bytes': v['num_bytes'],
            'at': cur - off,
            'fields': tab.read_variant(bs_name, data, cur, vname),
        }
        if len(tied) > 1:
            # Report the ambiguity instead of hiding it behind a pick.
            entry['ambiguous_with'] = [n for n, _ in tied[1:]]
        g['instrs'].append(entry)
        cur += v['num_bytes']

    g['instr_end'] = cur - off

    # The first (lower) source operand, where one is present.  The area holds
    # lower srcs, upper srcs, iss then dests; splitting it in general needs each
    # op's source and destination counts (pco_map.py), so only the first group
    # is decoded here and the rest is left as bytes.  A lower and an upper
    # variant carry identical pinned bits and are told apart by position, the
    # lower ones coming first -- so the first match IS the lower one.
    g['first_src'] = None
    if cur < off + g['total_bytes'] and cur < len(data) and data[cur] != 0xff:
        hits = [h for h in tab.match_variants('src', data, cur)
                if h[0].startswith('1lo') or h[0].startswith('2lo')
                or h[0].startswith('3lo')]
        if hits:
            hits.sort(key=lambda h: -h[1]['num_bytes'])
            vname, v = hits[0]
            g['first_src'] = {
                'variant': vname,
                'bytes': v['num_bytes'],
                'at': cur - off,
                'fields': tab.read_variant('src', data, cur, vname),
            }

    # da is the strongest cross-check we have on the instruction lengths, but
    # it is NOT simply the instruction end: calc_da() (pco_group_instrs.c:26)
    # counts only the phases encoded before p1, i.e. phases *after* p1 in the
    # phase order, because those are the ones the hardware skips to reach the
    # source/dest area.  For BITWISE every phase counts; for CONTROL it is the
    # ctrl instruction, or 0 when the group has no sources or dests at all.
    g['da_expect'] = compute_da(g, hdr_bytes)
    g['da_matches'] = (da == g['da_expect']) if g['da_expect'] is not None else None
    return g


# Phase order, lowest first, as pco's OP_PHASE enum numbers them.  da counts
# only the phases strictly after p1.
PHASE_ORDER = ['p0', 'p1', 'p2', 'p2_tst', 'p2_mov', 'backend']


def compute_da(g, hdr_bytes):
    """Re-derive the header's decode-assist from the instructions we decoded."""
    if g['alutype'] == 'control':
        return None  # Depends on whether the group has srcs/dests at all.
    da = hdr_bytes
    for i in g['instrs']:
        if g['alutype'] == 'bitwise':
            da += i['bytes']
        elif i['phase'] in PHASE_ORDER and \
                PHASE_ORDER.index(i['phase']) > PHASE_ORDER.index('p1'):
            da += i['bytes']
    return da


def decode_shader(tab, data):
    groups = []
    off = 0
    while off < len(data):
        if data[off] == 0xff:           # word padding
            off += 1
            continue
        if (data[off] & 0xf0) == 0xf0:  # alignment padding run
            words = data[off] & 0x0f
            off += words * 2
            continue
        g = decode_group(tab, data, off)
        groups.append(g)
        if not g['total_bytes']:
            break
        off += g['total_bytes']
    return groups


def parse_blob(path, symbol):
    """Pull the uint32 array out of a C header, little-endian to bytes."""
    with open(path) as f:
        text = f.read()
    if symbol:
        m = re.search(re.escape(symbol) + r'\s*\[[^\]]*\]\s*=\s*\{(.*?)\}', text, re.S)
    else:
        m = re.search(r'uint32_t\s+\w+\s*\[[^\]]*\]\s*=\s*\{(.*?)\}', text, re.S)
    if not m:
        return None
    words = [int(w, 16) for w in re.findall(r'0x([0-9a-fA-F]+)u?', m.group(1))]
    data = bytearray()
    for w in words:
        data += w.to_bytes(4, 'little')
    return bytes(data)


def show(groups, data):
    for g in groups:
        print(f"  group @{g['offset']:#05x}  {g['total_bytes']} bytes  "
              f"hdr={g['hdr_variant']}({g['hdr_bytes']}B) alutype={g['alutype']}")
        f = g['fields']
        bits = []
        for k in ('oporg', 'opcnt', 'ctrlop', 'rpt', 'end', 'cc', 'atom',
                  'olchk', 'w0p', 'w1p'):
            if k in f and f[k] is not None:
                v = f[k]
                bits.append(f"{k}={v['name'] or v['value']}")
        print(f"    hdr: da={g['da']} " + ' '.join(bits))
        for i in g['instrs']:
            extra = ''
            if 'ambiguous_with' in i:
                extra = f"  AMBIGUOUS with {', '.join(i['ambiguous_with'])}"
            print(f"    [{i['phase']:8s}] {i['variant']:24s} "
                  f"{i['bytes']}B @+{i['at']}{extra}")
            named = {k: (v['name'] or v['value'])
                     for k, v in i['fields'].items() if v is not None}
            if named:
                print(f"               {named}")
        # Separate the operand bytes from the 0xff run that pads the group,
        # so the report never presents padding as decoded operands.
        end = g['offset'] + g['total_bytes']
        pad = 0
        while end - pad - 1 >= g['offset'] + g['instr_end'] and \
                data[end - pad - 1] == 0xff:
            pad += 1
        tail = g['total_bytes'] - g['instr_end'] - pad
        print(f"    srcs/iss/dests: +{g['instr_end']}..+"
              f"{g['instr_end'] + tail} ({tail} bytes, {pad} bytes 0xff pad)"
              + ('  da=%d as calc_da predicts' % g['da'] if g.get('da_matches')
                 else f"  da={g['da']} but calc_da predicts {g.get('da_expect')}"
                 if g.get('da_expect') is not None else f"  da={g['da']}"))
        if g.get('first_src'):
            fs = g['first_src']
            named = {k: (v['name'] if v['name'] is not None else v['value'])
                     for k, v in fs['fields'].items() if v is not None}
            print(f"    first src: {fs['variant']} {fs['bytes']}B @+{fs['at']} "
                  f"{named}")
        for n in g['notes']:
            print(f"    NOTE: {n}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--table', default=DEFAULT_TABLE)
    ap.add_argument('--blobs', action='store_true',
                    help="decode the driver's hand-verified USC blobs")
    ap.add_argument('--hex', help='decode a raw byte string')
    args = ap.parse_args()

    tab = Table(args.table)

    if args.hex:
        data = bytes(int(b, 16) for b in args.hex.split())
        show(decode_shader(tab, data), data)
        return

    if not args.blobs:
        ap.error('give --blobs or --hex')

    rc = 0
    for rel, symbol, what in BLOBS:
        path = os.path.join(REPO, rel)
        data = parse_blob(path, symbol)
        print(f'== {rel} — {what}')
        if data is None:
            print('   no uint32 array found; skipped')
            rc = 1
            continue
        print(f'   {len(data)} bytes')
        try:
            groups = decode_shader(tab, data)
        except Exception as e:  # noqa: BLE001 - report, do not mask
            print(f'   DECODE FAILED: {e}')
            rc = 1
            continue
        show(groups, data)
        covered = sum(g['total_bytes'] for g in groups)
        print(f'   groups={len(groups)} bytes accounted={covered}/{len(data)}')
        if covered != len(data):
            print('   MISMATCH: the group lengths do not cover the blob')
            rc = 1
        print()
    sys.exit(rc)


if __name__ == '__main__':
    main()
