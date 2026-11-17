# HOST-ONLY GC7000UL dEQP-GLES3 at 5454be1b: the merged tip and not-pass list

**HOST-ONLY.** Every number here comes from host runs with the Linux OSAL,
which keeps the command stream instead of executing it: `glReadPixels` reads
black, so a render verdict (Pass, or Fail by image comparison) is meaningless
and must not be quoted as a board result. What is real is each case's compile
and link outcome, its GL errors and its NotSupported reason.

- Commit (library and harness): `5454be1b6bf561d9ed3ee686c5e039bc323147c4`;
  GPU vivante (the GC7000UL's online back end), module gles3.
- `docs/es3/host-gc-gles3-tip.tsv`: per group, the columns of
  `docs/es3/host-gles3-tip.tsv` (`gpu group pass fail notsupported crash
  other`), 42,810 cases. `tools/cts/gles3-triage.py --host` reads it.
- `docs/es3/host-gc-gles3-notpass.tsv`: the counting not-pass list in the
  `host-gles3-tally.py` layout (`gpu case status kind message rc owner`, rc and
  owner empty), sorted by case: refusals, wrong GL errors, crashes and compile
  failures. Render Fails are not listed.

Both files have the header line `host-gles3-tally.py` writes, then one
`# merged from ...` line naming where the parts were read from.

## Where the 74 part files are

The run was split into six parts (hgc1..hgc6), each written with
`tools/cts/host-gles3-tally.py --part`. **Only three of them are in this
tree.** The other three were never merged to main. They are read from the
commits of their result branches:

| part | files | cases | covers | where the files are |
|---|---|---|---|---|
| hgc1 | 1 (`operator.tsv`) | 6,402 | `functional.shaders.operator` | this tree: `hgc1/` (added in f114aa05) |
| hgc2 | 3 (`builtin_functions.part`, `matrix.part`, `random.part`) | 6,008 | those three `functional.shaders` subgroups | **not in this tree**: commit `550d00ed`, `docs/es3/host-gc-gles3-5454be1b/hgc2/` |
| hgc3 | 33 (`*.tsv`) | 8,206 | the other 33 `functional.shaders` subgroups | this tree: `hgc3/` (added in 47edb814) |
| hgc4 | 1 (`texture.part`) | 5,105 | `functional.texture` | **not in this tree**: commit `72658f77`, `docs/es3/host-gc-gles3-5454be1b/hgc4/` |
| hgc5 | 3 (`fbo.part`, `fragment_ops.part`, `fragment_out.part`) | 6,138 | those three groups | **not in this tree**: commit `b22499c4`, `docs/es3/host-gc-gles3-5454be1b/hgc5/` |
| hgc6 | 33 (`*.tsv`) | 10,951 | the remaining 32 `functional.*` groups (one file each) and `info.tsv` (the 6 `dEQP-GLES3.info.*` cases) | this tree: `hgc6/` (added in 5afe26cd) |
| | **74** | **42,810** | | 67 files in this tree, 7 not |

The 7 files that are not in this tree have the same format as the 67 that
are; only the extension differs. Read them with `git show <commit>:<path>`.
On origin, the commits are on the branches `connected/01a1e248-5b10-7116-a813-428fa78f08d0` (hgc2),
`connected/01a1e216-5d7d-7409-a058-96964121e7e2` (hgc4) and
`connected/01a1e2c1-e6ee-7234-8896-241a578a31a3` (hgc5). The figures
below and both TSVs therefore cannot be rebuilt from this tree alone. They
need those three commits too.

The part headers name the commit as `5454be1b` or `5454be1b6b`. Both are
prefixes of the commit above. The merged files print `5454be1b`.

## Result (HOST-ONLY)

Status of the 42,810 cases: Pass 8,062, Fail 32,434, NotSupported 1,877,
Crash 0, other 437 (all `functional.shaders`).

The not-pass list has 3,930 rows: 2,053 Fail and 1,877 NotSupported. By kind:

| kind | rows | status |
|---|---|---|
| refusal | 1,870 | NotSupported |
| gl-error | 1,295 | 1,288 Fail, 7 NotSupported |
| compile | 765 | Fail |
| crash | 0 | |

The other 30,381 Fails are render verdicts. They are meaningless on the host
and not listed.

By group (rows of the not-pass list):

| group | refusal | gl-error | compile |
|---|---|---|---|
| functional.texture | 1,338 | 65 | 432 |
| functional.shaders | 0 | 912 | 330 |
| functional.fbo | 248 | 135 | 0 |
| functional.fragment_out | 115 | 150 | 0 |
| functional.draw_buffers_indexed | 76 | 0 | 0 |
| functional.multisample | 48 | 0 | 0 |
| functional.read_pixels | 0 | 24 | 0 |
| functional.shader_api | 14 | 0 | 0 |
| functional.rasterization | 12 | 0 | 0 |
| functional.pbo | 6 | 4 | 0 |
| functional.state_query | 9 | 1 | 0 |
| functional.negative_api | 4 | 4 | 0 |
| functional.transform_feedback | 0 | 0 | 2 |
| functional.uniform_api | 0 | 0 | 1 |

## Coverage check and reproduction

The merge was checked on all 74 files: the 67 from this tree and the 7 read
with `git show` from `550d00ed`, `72658f77` and `b22499c4`. Each file loads
with `load_part` of `host-gles3-tally.py`. Each file's case count equals the
number of `tools/cts/gles3-mustpass.txt` names under its subgroup or group,
and so does the sum of its tip row. Every not-pass row lies under its file's
prefix. Every mustpass name falls under exactly one file. No case is in the
not-pass list twice.

The commands below redo the merge from the repository root. Like the check,
they need the three commits above. They copy the 74 files to a scratch
directory, merge them and assert two things: each group's total equals its
count in the mustpass, and no not-pass case appears twice. Then they write
both TSVs. The result matches the files in the tree byte for byte.

```
set -eu
D=docs/es3/host-gc-gles3-5454be1b
S=$TMPDIR/hgc-parts
mkdir -p $S
cp $D/hgc1/*.tsv $D/hgc3/*.tsv $D/hgc6/*.tsv $S/
for cp in 550d00ed:hgc2 72658f77:hgc4 b22499c4:hgc5; do
  c=${cp%%:*} p=${cp#*:}
  for f in $(git ls-tree --name-only $c $D/$p/ | grep '\.part$'); do
    git show $c:$f > $S/$p-$(basename $f)
  done
done
ls $S | wc -l    # 74
python3 -I - $S <<'EOF'
import glob, importlib.util, sys
spec = importlib.util.spec_from_file_location('t', 'tools/cts/host-gles3-tally.py')
t = importlib.util.module_from_spec(spec); spec.loader.exec_module(t)
tip, notpass = {}, []
for p in map(t.load_part, sorted(glob.glob(sys.argv[1] + '/*'))):
    for ln in p['tip']:
        f = ln.split('\t')
        tip[f[1]] = [a + int(b) for a, b in zip(tip.get(f[1], [0] * 5), f[2:])]
    notpass += p['notpass']
notpass.sort(key=lambda ln: ln.split('\t')[1])
must = {}
for c in open('tools/cts/gles3-mustpass.txt').read().split():
    g = t.GROUP_RE.match(c).group(1)
    must[g] = must.get(g, 0) + 1
assert must == {g: sum(n) for g, n in tip.items()}, 'a group not covered once'
cases = [ln.split('\t')[1] for ln in notpass]
assert len(cases) == len(set(cases)), 'a not-pass case twice'
src = ('# merged from 74 part files: hgc1, hgc3, hgc6 (67) in docs/es3/host-gc-gles3-5454be1b/; '
       'hgc2, hgc4, hgc5 (7) are not in this tree, read from commits 550d00ed, 72658f77, b22499c4; '
       'how: docs/es3/host-gc-gles3-5454be1b/README.md')
head = t.tip_head('gles3', 42810, '5454be1b')
rows = ['\t'.join(['vivante', g] + [str(n) for n in tip[g]]) for g in sorted(tip)]
open('docs/es3/host-gc-gles3-tip.tsv', 'w').write('\n'.join(head[:1] + [src] + head[1:] + rows) + '\n')
head = t.notpass_head('5454be1b')
open('docs/es3/host-gc-gles3-notpass.tsv', 'w').write('\n'.join(head[:1] + [src] + head[1:] + notpass) + '\n')
EOF
```
