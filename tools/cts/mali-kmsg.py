#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Classify panthor kernel messages taken around one dEQP chunk.

  tools/cts/mali-kmsg.py [--json] [FILE|-]

Reads a kernel message capture (dmesg text with or without timestamps, or
`dmesg -r` with <N> prefixes) and prints one line:

  kmsg: resets=N timeouts=N mmu_faults=N fw_faults=N gpu_faults=N other_panthor=N first=<line|->

Only panthor's lines count: those naming 'panthor' or the device fb000000.gpu.
Each such line falls in exactly one class (first matching row of PATTERNS).
Benign lines (BENIGN: probe and info messages) are not counted. other_panthor
is every panthor line that no row matches, e.g. the CS_FATAL / CS_FAULT
reports of panthor_sched.c. first= is the first counted panthor line, with the
<N> prefix and timestamp removed, cut to FIRST_MAX characters.

Importable: classify(lines) -> dict with the same keys.

The patterns come from the Linux v6.18 drivers/gpu/drm/panthor sources. The
format strings were read from the v6.18 tag; the tail of panthor_sched.c past
panthor_group_create() was not read, so messages there land in other_panthor.
Multi-line messages (one drm_err with embedded newlines) are counted once, on
the line that names the device.
"""
import json
import re
import sys

FIRST_MAX = 160
DEVICE = re.compile(r'panthor|fb000000\.gpu')
PREFIX = re.compile(r'^\s*(<\d+>)?\s*(\[\s*\d+\.\d+\]|\d{4}-\d\d-\d\dT\S+)?\s*')

# (class, regex, source file: function). First match wins, so the specific
# reset rows come before the firmware rows.
PATTERNS = [
    # panthor_device.c: panthor_device_reset_work
    ('resets', r'Failed to boot MCU after reset'),
    # panthor_device.c: panthor_device_resume
    ('resets', r'Fast reset failed'),
    # panthor_fw.c: panthor_fw_ping_work
    ('resets', r'FW ping timeout, scheduling a reset'),
    # panthor_fw.c: panthor_fw_post_reset
    ('resets', r'FW \S+ reset failed'),
    # panthor_gpu.c: panthor_gpu_soft_reset
    ('resets', r'Soft reset timeout'),
    # panthor_sched.c: queue_timedout_job
    ('timeouts', r'job timeout: pid='),
    # panthor_sched.c: csg_slot_process_progress_timer_event_locked
    ('timeouts', r'CSG_PROGRESS_TIMER_EVENT: |CSG slot \d+ progress timeout'),
    # panthor_sched.c: csgs_upd_ctx_apply_locked
    ('timeouts', r'CSG \d+ update request timedout'),
    # panthor_sched.c: panthor_sched_suspend
    ('timeouts', r'CSG suspend failed, escalating to termination'),
    # panthor_gpu.c: panthor_gpu_flush_caches
    ('timeouts', r'Flush caches timeout'),
    # panthor_gpu.c: panthor_gpu_block_power_off, panthor_gpu_block_power_on
    ('timeouts', r'timeout waiting on \S+ power transition|timeout waiting on \S+ readiness'),
    # panthor_mmu.c: panthor_mmu_irq_handler
    ('mmu_faults', r'Unhandled Page fault in AS\d+'),
    # panthor_mmu.c: wait_ready
    ('mmu_faults', r'AS_ACTIVE bit stuck'),
    # panthor_mmu.c: panthor_vm_unmap_pages
    ('mmu_faults', r'failed to unmap range'),
    # panthor_gpu.c: panthor_gpu_irq_handler
    ('gpu_faults', r'GPU Fault 0x[0-9a-fA-F]+ \(|GPU Fault in protected mode'),
    # panthor_fw.c: panthor_fw_start, panthor_fw_stop, panthor_fw_pre_reset,
    # panthor_fw_load*, panthor_fw_read_build_info, panthor_fw_init_ifaces,
    # panthor_init_cs_iface, panthor_init_csg_iface
    ('fw_faults', r'Failed to boot MCU|Failed to stop MCU|Failed to cleanly suspend MCU'
                  r'|Firmware (too small|entry too long|corrupted|contains|build info corrupt|image is truncated|entry isn)'
                  r'|Unsupported non-optional entry|Failed to load firmware image|Invalid firmware magic'
                  r'|Unsupported firmware binary header|Shared interface region not found'
                  r'|Interface at 0x\S+ must be shared|Firmware version is 0'
                  r'|Invalid (stream|group|global) control|Invalid number of control groups'
                  r'|Expecting identical C'),
]
PATTERNS = [(c, re.compile(r)) for c, r in PATTERNS]

# Info-level panthor lines, not counted: panthor_device.c panthor_clk_init
# ("clock rate = "), panthor_fw.c panthor_fw_read_build_info ("Firmware git
# sha") and panthor_fw_init_ifaces ("CSF FW using interface"), panthor_gpu.c
# panthor_gpu_l2_power_on ("using only 1st core group"), and the probe
# messages of panthor_drv.c.
BENIGN = re.compile(r'EM: created perf domain|clock rate = |Firmware git sha|CSF FW using interface|using only 1st core group|Initialized panthor|GPU information|mali-g\d+|Mali-G\d+')

KEYS = ('resets', 'timeouts', 'mmu_faults', 'fw_faults', 'gpu_faults', 'other_panthor')


def classify(lines):
    r = {k: 0 for k in KEYS}
    r['first'] = '-'
    for raw in lines:
        raw = raw.rstrip('\r\n')
        if not DEVICE.search(raw):
            continue
        text = PREFIX.sub('', raw, count=1).strip()
        if BENIGN.search(text):
            continue
        for cls, rx in PATTERNS:
            if rx.search(text):
                break
        else:
            cls = 'other_panthor'
        r[cls] += 1
        if r['first'] == '-':
            r['first'] = text[:FIRST_MAX]
    return r


def line(r):
    return 'kmsg: ' + ' '.join('%s=%d' % (k, r[k]) for k in KEYS) + ' first=' + r['first']


def main(argv):
    args = [a for a in argv if a != '--json']
    as_json = len(args) != len(argv)
    if len(args) > 1:
        sys.stderr.write('usage: mali-kmsg.py [--json] [FILE|-]\n')
        return 2
    try:
        if not args or args[0] == '-':
            lines = sys.stdin.read().splitlines()
        else:
            with open(args[0], errors='replace') as f:
                lines = f.read().splitlines()
    except OSError as e:
        sys.stderr.write('mali-kmsg.py: %s\n' % e)
        return 2
    r = classify(lines)
    print(json.dumps(r) if as_json else line(r))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
