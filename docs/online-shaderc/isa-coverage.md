# What the offline table's USC actually uses

Counted by `cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online isa_coverage`, which regenerates this file with `YAKOGL_WRITE_ISA_COVERAGE=1`.
It is the code generator's target list: an instruction form or an operation that is not here is one nothing in the table needs, and one that is here with a large count is one the generator has to get right first.

Corpus: 2248 blobs of `SHC_BLOB_TABLE`, 673305 instruction groups.

## Operations

| | count |
|---|---:|
| `mbyp` | 253332 |
| `fmad` | 117069 |
| `fmul` | 108826 |
| `tst` | 68385 |
| `movc` | 60465 |
| `fadd` | 55159 |
| `bbyp0bm_imm32` | 39058 |
| `bbyp0bm` | 33475 |
| `bbyp0s1` | 33475 |
| `imadd32` | 31374 |
| `shift` | 26167 |
| `pck` | 24392 |
| `br` | 19808 |
| `br.skip_next` | 19808 |
| `br.next` | 16526 |
| `frcp` | 16515 |
| `wdf` | 10580 |
| `frsq` | 10386 |
| `logical` | 10344 |
| `smp` | 6352 |
| `ditrp` | 3764 |
| `unpck` | 3247 |
| `cndst` | 3243 |
| `cndend` | 2122 |
| `fexp` | 1645 |
| `msk_bbyp0s1` | 1512 |
| `movwm` | 1408 |
| `cndef` | 1160 |
| `fred` | 794 |
| `uvsw.write` | 792 |
| `cndsm` | 698 |
| `cndlt` | 686 |
| `imadd64` | 637 |
| `flog` | 402 |
| `fsinc` | 397 |
| `savmsk` | 280 |
| `alphaf` | 256 |
| `uvsw.emit.endtask` | 208 |
| `ditr` | 196 |
| `fdsx` | 100 |
| `fdsxf` | 100 |
| `fdsy` | 100 |
| `fdsyf` | 100 |
| `nop` | 20 |
| `add64_32` | 12 |
| `msk_lsl` | 12 |
| `st.tiled` | 12 |
| `uvsw.write.emit.endtask` | 8 |

## Instruction forms

| | count |
|---|---:|
| `main.sngl` | 273029 |
| `main.fmad` | 117041 |
| `main.fmul` | 108826 |
| `main.tst_ext` | 66089 |
| `main.movc_ext` | 61873 |
| `main.fadd` | 55159 |
| `bitwise.phase0_imm32` | 39058 |
| `bitwise.phase0_src` | 34999 |
| `bitwise.phase2` | 26167 |
| `main.pck` | 24392 |
| `main.int32_64` | 23023 |
| `ctrl.branch` | 19808 |
| `ctrl.wdf` | 10580 |
| `bitwise.phase1` | 10344 |
| `main.sngl_ext` | 9848 |
| `main.int32_64_ext` | 9000 |
| `ctrl.cnd` | 7909 |
| `ctrl.ditr` | 3960 |
| `backend.smp_exta` | 3355 |
| `main.upck` | 3247 |
| `backend.smp_brief` | 2997 |
| `main.tst` | 2296 |
| `main.fred` | 794 |
| `backend.uvsw_write_imm` | 792 |
| `backend.savmsk` | 280 |
| `backend.vistest_atst` | 256 |
| `backend.uvsw_emit_endtask` | 208 |
| `main.fmad_ext` | 28 |
| `ctrl.nop` | 20 |
| `backend.st_immbl_tiled` | 12 |
| `backend.uvsw_write_emit_endtask_imm` | 8 |

## Group headers

| | count |
|---|---:|
| `main` | 556971 |
| `bitwise` | 74057 |
| `control` | 42277 |

## Sections and padding

| | count |
|---|---:|
| `upper_src.1up_1b6i` | 597051 |
| `dst.1_1b6i` | 575362 |
| `iss.iss` | 556971 |
| `pad.word` | 365679 |
| `lower_src.1lo_1b6i` | 180230 |
| `lower_src.3lo_2b7i_2b7i_2b6i_2m` | 143060 |
| `lower_src.2lo_1b6i_1b5i` | 101228 |
| `lower_src.2lo_2b7i_2b7i_2m` | 84218 |
| `lower_src.1lo_3b11i_2m` | 61362 |
| `dst.1_3b11i` | 44566 |
| `lower_src.2lo_3b11i_2b8i_3m` | 41185 |
| `upper_src.2up_1b6i_1b5i` | 30998 |
| `lower_src.3lo_3b8i_2b8i_3b8i_3m` | 27654 |
| `upper_src.1up_3b11i` | 8499 |
| `upper_src.2up_2b7i_2b7i` | 2389 |
| `pad.align` | 1667 |
| `dst.2_1b7i_1b6i` | 12 |

