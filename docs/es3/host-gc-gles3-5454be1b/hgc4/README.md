# HOST-ONLY GC7000UL dEQP-GLES3, part 4: functional.texture

**HOST-ONLY.** Every number here comes from a run on a host with the Linux
OSAL, which keeps the command stream instead of executing it: every
`glReadPixels` reads black, so every render verdict (Pass, Fail by image
comparison or verification) is meaningless and must not be quoted as a board
result. What is real is each case's compile and link outcome.

- Commit (library and harness): `5454be1b6bf561d9ed3ee686c5e039bc323147c4`
- GPU: vivante (GC7000UL's online back end), module gles3, `REPORT_ES2=0`
- VK-GL-CTS: `93bca01861b0e3ef3c387027a9791e6d065f900c` with the yakogl target
  applied
- Command, from the worktree of the commit:

```
GPU=vivante MODULE=gles3 REPORT_ES2=0 CTS=<deps>/VK-GL-CTS OUT=<deps>/hgc-5454be1b/hgc4 \
  tools/cts/host-deqp-run.sh dEQP-GLES3.functional.texture
```

## Result (HOST-ONLY)

| | cases |
|---|---|
| asked (names containing `dEQP-GLES3.functional.texture`) | 5105 |
| begun | 5105 |
| not executed | 0 |
| aborted chunks (`aborted.txt`) | 0 |

Process exits: 11 with exit 0, 41 with exit 1 (a chunk with a failing case
exits 1); no chunk needed more than one process.

Status of the 5105 begun cases (HOST-ONLY): Pass 19, Fail 3748,
NotSupported 1338, Crash 0, other 0 (19 + 3748 + 1338 = 5105).

Compile refusals: 432, all on cases dEQP failed, one reason: the code
generator refused the fragment shader with `Select(Unsupported("an ES 3.00
texture look-up (a shadow sampler, an integer result, a gradient, an offset or
a cube fetch) is not selected yet"))`. They are the whole of
`functional.texture.shadow` (432 cases). The other Fail verdicts are image
comparison or verification failures (2096 and 1084 cases) and GL errors on
texture upload or specification (about 140 cases); those verdicts depend on
the black read-back and are HOST-ONLY.

For comparison, `docs/es3/host-gles3-tip.tsv` has vivante
`functional.texture` as 19 / 3428 / 1658 (pass / fail / notsupported) at its
earlier integration tip; this part differs by 320 cases moving from
NotSupported to Fail, HOST-ONLY both.

## Files

- `group-counts.tsv`: per subgroup of `functional.texture` and a total row;
  columns of `docs/es3/host-gles3-tip.tsv` (gpu group pass fail notsupported
  crash other). HOST-ONLY.
- `notpass.tsv`: `case<TAB>status` for the 5086 cases that are not Pass.
  HOST-ONLY.
