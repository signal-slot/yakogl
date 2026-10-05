# What the offline table's USC actually uses

Counted by `cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online isa_coverage`, which regenerates this file with `YAKOGL_WRITE_ISA_COVERAGE=1`.
It is the code generator's target list: an instruction form or an operation that is not here is one nothing in the table needs, and one that is here with a large count is one the generator has to get right first.

Corpus: 2268 blobs of `SHC_BLOB_TABLE`, 651768 instruction groups.

## Operations

| | count |
|---|---:|
| `mbyp` | 230080 |
| `fmad` | 116900 |
| `fmul` | 106543 |
| `tst` | 56264 |
| `fadd` | 53534 |
| `movc` | 48310 |
| `bbyp0bm_imm32` | 38358 |
| `imadd32` | 31053 |
| `bbyp0bm` | 29446 |
| `bbyp0s1` | 29446 |
| `shift` | 25144 |
| `pck` | 21920 |
| `br` | 19966 |
| `br.skip_next` | 19966 |
| `br.next` | 16588 |
| `frcp` | 15577 |
| `wdf` | 10687 |
| `frsq` | 10386 |
| `logical` | 6676 |
| `smp` | 6442 |
| `ditrp` | 3781 |
| `cndst` | 3327 |
| `unpck` | 2932 |
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
| `main.sngl` | 254932 |
| `main.fmad` | 116872 |
| `main.fmul` | 106543 |
| `main.tst_ext` | 55375 |
| `main.fadd` | 53534 |
| `main.movc_ext` | 49718 |
| `bitwise.phase0_imm32` | 38358 |
| `bitwise.phase0_src` | 30639 |
| `bitwise.phase2` | 25144 |
| `main.int32_64` | 22668 |
| `main.pck` | 21920 |
| `ctrl.branch` | 19966 |
| `ctrl.wdf` | 10687 |
| `main.int32_64_ext` | 9034 |
| `ctrl.cnd` | 8077 |
| `bitwise.phase1` | 6676 |
| `ctrl.ditr` | 3977 |
| `backend.smp_exta` | 3445 |
| `main.sngl_ext` | 3370 |
| `backend.smp_brief` | 2997 |
| `main.upck` | 2932 |
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
| `main` | 540044 |
| `bitwise` | 68997 |
| `control` | 42727 |

## Sections and padding

| | count |
|---|---:|
| `upper_src.1up_1b6i` | 576642 |
| `dst.1_1b6i` | 548862 |
| `iss.iss` | 540044 |
| `pad.word` | 351640 |
| `lower_src.1lo_1b6i` | 174430 |
| `lower_src.3lo_2b7i_2b7i_2b6i_2m` | 139122 |
| `lower_src.2lo_1b6i_1b5i` | 98715 |
| `lower_src.2lo_2b7i_2b7i_2m` | 83316 |
| `lower_src.1lo_3b11i_2m` | 62378 |
| `dst.1_3b11i` | 49024 |
| `lower_src.2lo_3b11i_2b8i_3m` | 31622 |
| `upper_src.2up_1b6i_1b5i` | 29728 |
| `lower_src.3lo_3b8i_2b8i_3b8i_3m` | 27535 |
| `upper_src.1up_3b11i` | 8407 |
| `upper_src.2up_2b7i_2b7i` | 2341 |
| `pad.align` | 1684 |
| `dst.2_1b7i_1b6i` | 12 |

