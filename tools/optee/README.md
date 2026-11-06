# tools/optee: reference vectors for SCMI over OP-TEE (STM32MP2)

`scmi_vectors.py` writes the fixtures in `tools/testdata/optee-scmi/` (Python 3,
standard library only). `python3 tools/optee/scmi_vectors.py --check` exits 0
when the files match the generator; run it without `--check` to regenerate.
The vectors are independent of the Rust code: `optee-msg`, `mp2-model` and
`st-power` test against them. Part of #7 and #1281 (STM32MP257F-DK port).

| File | Content |
|---|---|
| `smc.txt` | SMC32 register sets x0..x7 (u64 LE x 8): CALLS_UID/REVISION, GET_OS_UUID/REVISION, EXCHANGE_CAPABILITIES, CALL_WITH_ARG, CALL_WITH_RPC_ARG |
| `msg_arg.txt` | `struct optee_msg_arg` frames: register/unregister shm (and the page list), open session to the SCMI PTA, PTA capabilities, get channel handle, close session; each with its reply and an error reply |
| `scmi_msgs.txt` | SCMI messages (MSG header + payload) and OP-TEE's replies: PROTOCOL_VERSION, PROTOCOL_ATTRIBUTES of 0x11 and 0x13; power domain POWER_STATE_SET on/off, POWER_STATE_GET; performance LEVEL_SET, LEVEL_GET; error statuses |
| `invoke_frames.txt` | the `PTA_SCMI_CMD_PROCESS_MSG_CHANNEL` (cmd 4) invoke frame carrying each SCMI request, and its reply |

Each fixture has a header and a `#` comment above every vector naming the
source of every field; this README holds the sources and the open list.

## Transport the vectors encode

SMC `CALL_WITH_RPC_ARG` (or `CALL_WITH_ARG`) with the physical address of a
`struct optee_msg_arg` -> open session to the SCMI pseudo-TA (UUID
`a8cfe406-d4f5-4a2e-9f8d-a25dc754c099`) -> `GET_CHANNEL_HANDLE` (func 3,
channel 0, caps MSG_HEADER) -> `PROCESS_MSG_CHANNEL` (func 4): value input =
channel, input memref = SCMI message, output memref = reply buffer, both
registered (RMEM) through `REGISTER_SHM` (attr `TMEM_OUTPUT|NONCONTIG`, a page
list). With ST's SCP-firmware server (`CFG_SCMI_SCPFW`) the handle returned is
the channel id itself.

## Sources

Two trees were read (cloned fresh, not from memory):

- **OPTEE** = `OP-TEE/optee_os` tag `4.0.0`, commit
  `2a5b1d1232f582056184367fb58a425ac7478ec6` (`git ls-remote` peeled tag):
  `core/include/optee_msg.h`, `core/arch/arm/include/sm/optee_smc.h`,
  `core/arch/arm/tee/entry_fast.c`, `core/tee/entry_std.c`, `core/tee/uuid.c`,
  `core/kernel/pseudo_ta.c`, `core/include/kernel/notif.h`,
  `core/include/kernel/thread.h`, `lib/libutee/include/tee_api_defines.h`.
- **ST** = `STMicroelectronics/optee_os` tag `4.0.0-stm32mp-r3.1`, commit
  `0063536697f1d5337e80d43507c9887e49e36b43` (the tag the pre-study note
  names; the board's exact release is unknown, #1281): `core/pta/scmi.c`,
  `lib/libutee/include/pta_scmi_client.h`, `core/lib/scmi-server/scmi_server.c`,
  `core/arch/arm/plat-stm32mp2/{conf.mk,scmi_server_scpfw.c}`,
  `core/lib/scmi-server/conf-optee-stm32mp2.mk`, `mk/config.mk`,
  `core/include/dt-bindings/power/st,stm32mp25-power.h`.
- **SCPFW** = the SCP-firmware tree inside the ST tree
  (`core/lib/scmi-server/SCP-firmware`, a plain directory in the tag; its own
  upstream commit is not recorded in the tag): `module/scmi/include/mod_scmi_header.h`,
  `mod_scmi_std.h`, `module/msg_smt/src/mod_msg_smt.c`,
  `module/scmi_power_domain/src/mod_scmi_power_domain.c`,
  `module/scmi_power_domain/include/internal/scmi_power_domain.h`,
  `module/scmi_perf/src/scmi_perf_protocol_ops.c`,
  `module/scmi_perf/include/internal/scmi_perf.h`.
- **SPEC**: Arm SCMI specification DEN0056 (message header; protocols 0x11
  and 0x13). Not fetched here; the field layout and the status codes were taken
  from SCPFW, which implements them. Cross-check against the PDF when the
  Rust codec lands.
- #1281 / `docs/notes/pre-study-stm32mp257f-dk.md`: the GPU's power domain 0,
  performance domain 1, channel 0 (`stm32mp251.dtsi:103-108`,
  `stm32mp255.dtsi:176`).

The file:line numbers in the fixture comments refer to those tags.

## Findings that differ from what one would assume

- `REGISTER_SHM` uses `TMEM_OUTPUT|NONCONTIG` (`0x20a`) in the code
  (`entry_std.c:490-492`); the comment in `optee_msg.h:319-322` says INPUT. The
  code is followed.
- The board's OP-TEE is built with `CFG_SCMI_SCPFW=y` (ST `plat-stm32mp2/
  conf.mk:205`), so the performance and power-domain protocols are SCP-firmware
  modules, not the upstream `core/drivers/scmi-msg` drivers (which have no
  performance or power-domain protocol at 4.0.0).
- With SCPFW, `GET_CHANNEL_HANDLE` only validates the channel and leaves
  value a/b unchanged (`scmi.c:373-376`): the handle is the channel id.
- ST's PTA header also has commands 2048/2049 (OCALL2 threads) and capability
  bit 31; the reference vectors cover only the plain commands 0, 3 and 4. Which
  one ST's Linux 6.6 transport uses on this board is not known.

## Not yet checked against a real OP-TEE (all marked UNVERIFIED in the fixtures)

Everything below is derived from source, not from a wire capture of the
board's OP-TEE.

1. The board's build configuration: `EXCHANGE_CAPABILITIES` reply `x1 = 0x74`,
   `x2 = 69` (`64 + CFG_NUM_THREADS 5`, the default), `x3 = 4`; the OS revision
   minor 0.
2. Session id of the reply (`1`), and the `ret_origin` of replies: 4
   (TRUSTED_APP) for PTA calls, 3 (TEE) for close and failed opens. The values
   follow `pseudo_ta.c:151,208` and `entry_std.c`, not a trace.
3. The capability reply of the PTA (`0x2`; ST may OR in bit 31).
4. The error replies: unknown UUID gives `ITEM_NOT_FOUND`
   (the code `tee_ta_open_session` returns was not read), unknown channel gives
   `BAD_PARAMETERS`, unregistered shm cookie in an invoke gives an error that is
   not encoded.
5. SCMI domain counts: power domain protocol 1 (only `PD_SCMI_GPU` exists in
   the binding header), performance protocol 2 (domain 1 exists; domain 0
   assumed CPU DVFS). The real counts come from OP-TEE's device tree.
6. PERFORMANCE_LEVEL value 800000 and the out-of-range case (`0xffffffff`):
   the OPP table of the GPU performance domain was not read.
7. The success reply of a synchronous POWER_STATE_SET: the status word alone
   (the event handler that sends it was not read); the async path and every
   error reply were read (`mod_scmi_power_domain.c:518-617`).
8. PROTOCOL_ATTRIBUTES of the performance protocol: power-in-mW bit set and zero
   statistics fields assume no `MOD_STATISTICS` module.
9. Token value 1 and message type 0 are the generator's choice; a reply echoes
   the request header verbatim (`mod_msg_smt.c:259`).
10. All physical addresses (arg buffer, page list, shared buffer) and the cookie
    are illustrative, inside System RAM; the real ones come from the driver's
    allocation.
11. A secure-side RPC during a call (`OPTEE_SMC_RETURN_RPC_*`, e.g. shm alloc
    or foreign interrupt) is not encoded; a client must handle it
    (`optee_smc.h:583-692`).
12. PROTOCOL_VERSION `0x20000` for both protocols is what SCPFW reports
    (`scmi_power_domain.h:26`, `scmi_perf.h:24`); not read from the board.
