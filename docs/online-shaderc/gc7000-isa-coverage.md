# What the GC7000 offline table's shaders actually use

Counted by `cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online isa_coverage`, which regenerates this file with `YAKOGL_WRITE_ISA_COVERAGE=1`.
It is the code generator's target list: an operation that is not here is one nothing in the table needs, and one here with a large count is one the generator has to get right first.

Corpus: the GC7000 `SHC_BLOB_TABLE` plus the backend's two internal clear shaders, 309 shaders and 13854 instructions; 38 of the description's 90 operations appear.

| operation | count |
|---|---:|
| `mad` | 3358 |
| `mul` | 2584 |
| `mov` | 2165 |
| `add` | 1091 |
| `select` | 606 |
| `sqrt` | 554 |
| `imullo0` | 533 |
| `cmp` | 438 |
| `rcp` | 381 |
| `texld` | 280 |
| `dp3` | 210 |
| `branch_unary` | 208 |
| `and` | 170 |
| `rshift` | 145 |
| `rsq` | 142 |
| `dp2` | 137 |
| `exp` | 116 |
| `branch` | 95 |
| `frc` | 83 |
| `texldl` | 78 |
| `txf` | 65 |
| `or` | 54 |
| `not` | 53 |
| `i2f` | 51 |
| `f2i` | 44 |
| `log` | 44 |
| `floor` | 36 |
| `sin` | 29 |
| `iabs` | 27 |
| `texkill` | 16 |
| `xor` | 13 |
| `cos` | 10 |
| `nop` | 10 |
| `dsx` | 8 |
| `dsy` | 8 |
| `movar` | 7 |
| `ceil` | 3 |
| `sign` | 2 |
