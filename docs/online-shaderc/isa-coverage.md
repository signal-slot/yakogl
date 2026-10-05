# What the offline table's USC actually uses

Counted by `cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online isa_coverage`, which regenerates this file with `YAKOGL_WRITE_ISA_COVERAGE=1`.
It is the code generator's target list: an instruction form or an operation that is not here is one nothing in the table needs, and one that is here with a large count is one the generator has to get right first.

Corpus: 2476 blobs of `SHC_BLOB_TABLE`, 685564 instruction groups.

## Operations

| | count |
|---|---:|
| `mbyp` | 239986 |
| `fmad` | 133042 |
| `fmul` | 108623 |
| `tst` | 58799 |
| `fadd` | 56654 |
| `movc` | 50065 |
| `bbyp0bm_imm32` | 38553 |
| `imadd32` | 31833 |
| `bbyp0bm` | 29446 |
| `bbyp0s1` | 29446 |
| `shift` | 25144 |
| `pck` | 22518 |
| `br` | 21526 |
| `br.skip_next` | 21526 |
| `br.next` | 18148 |
| `frcp` | 15772 |
| `wdf` | 11077 |
| `frsq` | 10581 |
| `logical` | 6676 |
| `smp` | 6442 |
| `ditrp` | 4171 |
| `cndst` | 3327 |
| `unpck` | 3127 |
| `cndend` | 2212 |
| `movwm` | 1408 |
| `fexp` | 1260 |
| `msk_bbyp0s1` | 1181 |
| `cndef` | 1166 |
| `uvsw.write` | 798 |
| `fred` | 794 |
| `cndsm` | 692 |
| `cndlt` | 680 |
| `imadd64` | 637 |
| `flog` | 402 |
| `fsinc` | 397 |
| `savmsk` | 280 |
| `alphaf` | 256 |
| `uvsw.emit.endtask` | 211 |
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
| `main.sngl` | 265033 |
| `main.fmad` | 133014 |
| `main.fmul` | 108623 |
| `main.tst_ext` | 57910 |
| `main.fadd` | 56654 |
| `main.movc_ext` | 51473 |
| `bitwise.phase0_imm32` | 38553 |
| `bitwise.phase0_src` | 30639 |
| `bitwise.phase2` | 25144 |
| `main.int32_64` | 22668 |
| `main.pck` | 22518 |
| `ctrl.branch` | 21526 |
| `ctrl.wdf` | 11077 |
| `main.int32_64_ext` | 9814 |
| `ctrl.cnd` | 8077 |
| `bitwise.phase1` | 6676 |
| `ctrl.ditr` | 4367 |
| `main.sngl_ext` | 3565 |
| `backend.smp_exta` | 3445 |
| `main.upck` | 3127 |
| `backend.smp_brief` | 2997 |
| `main.tst` | 889 |
| `backend.uvsw_write_imm` | 798 |
| `main.fred` | 794 |
| `backend.savmsk` | 280 |
| `backend.vistest_atst` | 256 |
| `backend.uvsw_emit_endtask` | 211 |
| `main.fmad_ext` | 28 |
| `ctrl.nop` | 20 |
| `backend.st_immbl_tiled` | 12 |
| `backend.uvsw_write_emit_endtask_imm` | 8 |

## Group headers

| | count |
|---|---:|
| `main` | 571305 |
| `bitwise` | 69192 |
| `control` | 45067 |

## Sections and padding

| | count |
|---|---:|
| `upper_src.1up_1b6i` | 608098 |
| `dst.1_1b6i` | 579343 |
| `iss.iss` | 571305 |
| `pad.word` | 375400 |
| `lower_src.1lo_1b6i` | 177420 |
| `lower_src.3lo_2b7i_2b7i_2b6i_2m` | 155459 |
| `lower_src.2lo_1b6i_1b5i` | 103590 |
| `lower_src.2lo_2b7i_2b7i_2m` | 83641 |
| `lower_src.1lo_3b11i_2m` | 68124 |
| `dst.1_3b11i` | 49024 |
| `lower_src.2lo_3b11i_2b8i_3m` | 32012 |
| `upper_src.2up_1b6i_1b5i` | 29728 |
| `lower_src.3lo_3b8i_2b8i_3b8i_3m` | 28328 |
| `upper_src.1up_3b11i` | 8407 |
| `upper_src.2up_2b7i_2b7i` | 2341 |
| `pad.align` | 1827 |
| `dst.2_1b7i_1b6i` | 12 |

