#!/usr/bin/env python3
"""Download a URL to a file with progress logging. Usage: fetch.py <url> <dest>"""
import sys, urllib.request, os, time

url, dest = sys.argv[1], sys.argv[2]
os.makedirs(os.path.dirname(dest), exist_ok=True)
tmp = dest + ".part"
req = urllib.request.Request(url, headers={'User-Agent': 'yakogl-stage1-fetch'})
t0 = time.time()
with urllib.request.urlopen(req, timeout=60) as r, open(tmp, 'wb') as f:
    total = 0
    while True:
        chunk = r.read(1 << 20)
        if not chunk:
            break
        f.write(chunk)
        total += len(chunk)
        if total % (32 << 20) < (1 << 20):
            print(f"{total >> 20} MB  {int(time.time()-t0)}s  {url}", flush=True)
os.replace(tmp, dest)
print(f"DONE {dest}  {total} bytes  {int(time.time()-t0)}s", flush=True)
