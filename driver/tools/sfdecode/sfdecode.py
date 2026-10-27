#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Decode a raw PowerVR FW trace (space-separated hex dwords on stdin) into
# human-readable SF lines, using the SFID table in the DDK's rgx_fwif_sf.h.
#
# Usage:
#   cat trace.txt | ./sfdecode.py
#   ./sfdecode.py /path/to/rgx_fwif_sf.h  < trace.txt      (another DDK's table)
#
# The header is the DDK's include/rgx_fwif_sf.h, vendored in
# third_party/ti-img-rogue-ddk-25.2 (the default); SFID_HEADER overrides it.
# An SF entry is: sfid dword + u64 timestamp + args.
import re, sys, os
_VENDORED = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..",
                         "third_party", "ti-img-rogue-ddk-25.2", "include", "rgx_fwif_sf.h")
HDR = (sys.argv[1] if len(sys.argv) > 1
       else os.environ.get("SFID_HEADER", _VENDORED))
hdr = open(HDR).read()

# group order
groups={}
for i,m in enumerate(re.findall(r'X\(RGXFW_GROUP_\w+,\s*(\w+)\)', hdr)):
    groups["RGXFW_GROUP_"+m]=i

# SFID list: X(a, GROUP, NAME, "str", e)  -- multiline; strip line continuations
body=hdr.split("RGXFW_LOG_SFIDLIST")[1]
body=body.replace("\\\n"," ")
sfs={}
for m in re.finditer(r'X\(\s*(\d+)\s*,\s*(RGXFW_GROUP_\w+)\s*,\s*(\w+)\s*,\s*"((?:[^"\\]|\\.)*)"\s*,\s*(\d+)\s*\)', body):
    a=int(m.group(1)); grp=m.group(2); name=m.group(3); s=m.group(4); e=int(m.group(5))
    g=groups[grp]
    sfid=(a | ((g&0x10)<<7) | ((g&0xF)<<12) | (e<<16) | 0x70000000)&0xffffffff
    sfs[sfid]=(name,s,e)

def valid(x): return (x&0xFFF00000)==0x70000000

# read dwords from stdin (space/hex)
data=[]
for line in sys.stdin:
    for tok in re.findall(r'[0-9a-fA-F]{8}', line):
        data.append(int(tok,16))

i=0
while i < len(data):
    d=data[i]
    if valid(d) and d in sfs:
        name,fmt,argn=sfs[d]
        ts=data[i+1] | (data[i+2]<<32) if i+2<len(data) else 0
        args=data[i+3:i+3+argn]
        try: out=fmt % tuple(args) if argn else fmt
        except: out=fmt+" args="+str([hex(x) for x in args])
        print(f"[{i:3}] {name}: {out}")
        i+=3+argn
    else:
        i+=1
