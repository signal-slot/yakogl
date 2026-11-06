#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Reference byte vectors for SCMI over OP-TEE on the STM32MP2 (GPU power).

Standard library only, independent of the Rust code: the Rust items (optee-msg,
mp2-model, st-power) test against the files written here.

    python3 tools/optee/scmi_vectors.py           write tools/testdata/optee-scmi/*.txt
    python3 tools/optee/scmi_vectors.py --check   exit 0 when the files match

Every vector is one line "name hexbytes" in a .txt fixture; lines starting with
'#' are comments that say where each field comes from.  tools/optee/README.md
has the sources (file:line at a tag) and the list of values not yet checked
against a real OP-TEE.  Sources are cited as:

  OPTEE   OP-TEE/optee_os tag 4.0.0   (upstream: message ABI, PTA header)
  ST      STMicroelectronics/optee_os tag 4.0.0-stm32mp-r3.1
          (the SCMI PTA and the SCP-firmware server the board's OP-TEE runs)
  SCPFW   core/lib/scmi-server/SCP-firmware inside the ST tree
  SPEC    Arm SCMI specification DEN0056 (header, protocols 0x11 and 0x13)
"""

import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_DIR = os.path.normpath(os.path.join(HERE, "..", "testdata", "optee-scmi"))

# ---------------------------------------------------------------------------
# Constants, each with its source.

# OPTEE core/arch/arm/include/sm/optee_smc.h:20-23,36-47 (SMC32, owner 50 TOS,
# owner 63 TOS API; fast = bit 31).
def smc_val(fast, owner, func):
    return (0x80000000 if fast else 0) | (owner << 24) | func

SMC_CALLS_UID = smc_val(True, 63, 0xFF01)             # optee_smc.h:89-92
SMC_CALLS_REVISION = smc_val(True, 63, 0xFF03)        # optee_smc.h:101-104
SMC_GET_OS_UUID = smc_val(True, 50, 0x0000)           # optee_smc.h:116-119
SMC_GET_OS_REVISION = smc_val(True, 50, 0x0001)       # optee_smc.h:131-134
SMC_EXCHANGE_CAPABILITIES = smc_val(True, 50, 9)      # optee_smc.h:320-322
SMC_CALL_WITH_ARG = smc_val(False, 50, 4)             # optee_smc.h:199-201
SMC_CALL_WITH_RPC_ARG = smc_val(False, 50, 18)        # optee_smc.h:202-203
assert SMC_CALL_WITH_ARG == 0x32000004
assert SMC_EXCHANGE_CAPABILITIES == 0xB2000009

RETURN_OK = 0                                         # optee_smc.h:695
# optee_smc.h:301-318
NSEC_CAP_UNIPROCESSOR = 1 << 0
SEC_CAP_HAVE_RESERVED_SHM = 1 << 0
SEC_CAP_DYNAMIC_SHM = 1 << 2
SEC_CAP_MEMREF_NULL = 1 << 4
SEC_CAP_ASYNC_NOTIF = 1 << 5
SEC_CAP_RPC_ARG = 1 << 6

# OPTEE core/include/optee_msg.h:21-33,71,335-340
ATTR_NONE = 0x0
ATTR_VALUE_INPUT = 0x1
ATTR_VALUE_OUTPUT = 0x2
ATTR_VALUE_INOUT = 0x3
ATTR_RMEM_INPUT = 0x5
ATTR_RMEM_OUTPUT = 0x6
ATTR_TMEM_OUTPUT = 0xA
ATTR_META = 1 << 8
ATTR_NONCONTIG = 1 << 9
CMD_OPEN_SESSION = 0
CMD_INVOKE_COMMAND = 1
CMD_CLOSE_SESSION = 2
CMD_REGISTER_SHM = 4
CMD_UNREGISTER_SHM = 5
LOGIN_PUBLIC = 0                                      # optee_msg.h:87

# OPTEE core/include/optee_msg.h:115-216 struct sizes.
ARG_HDR = struct.Struct("<8I")                        # 32 bytes
PARAM = struct.Struct("<4Q")                          # 32 bytes

# GlobalPlatform error codes, OPTEE lib/libutee/include/tee_api_defines.h:100-117
TEE_SUCCESS = 0
TEE_ERROR_BAD_PARAMETERS = 0xFFFF0006
TEE_ERROR_ITEM_NOT_FOUND = 0xFFFF0008
TEE_ERROR_NOT_SUPPORTED = 0xFFFF000A
TEE_ORIGIN_TEE = 3
TEE_ORIGIN_TRUSTED_APP = 4

# SCMI PTA: ST lib/libutee/include/pta_scmi_client.h:8-9 (uuid), :19, :59, :69
# (commands), :103 (MSG_HEADER); the upstream header has the same values.
PTA_SCMI_UUID = bytes.fromhex("a8cfe406d4f54a2e9f8da25dc754c099")
PTA_CMD_CAPABILITIES = 0
PTA_CMD_GET_CHANNEL_HANDLE = 3
PTA_CMD_PROCESS_MSG_CHANNEL = 4
PTA_CAPS_MSG_HEADER = 1 << 1

# SCMI (SPEC; SCPFW module/scmi/include/mod_scmi_header.h:14-17 positions,
# mod_scmi_std.h:49-59 status, :100,:132 protocol ids).
PROT_POWER = 0x11
PROT_PERF = 0x13
MSG_PROTOCOL_VERSION = 0x0
MSG_PROTOCOL_ATTRIBUTES = 0x1
# SCPFW mod_scmi_std.h:105-108 (power domain), :142-143 (performance)
MSG_PD_STATE_SET = 0x4
MSG_PD_STATE_GET = 0x5
MSG_PERF_LEVEL_SET = 0x7
MSG_PERF_LEVEL_GET = 0x8
SCMI_SUCCESS = 0
SCMI_NOT_SUPPORTED = -1
SCMI_INVALID_PARAMETERS = -2
SCMI_NOT_FOUND = -4
SCMI_OUT_OF_RANGE = -5
SCMI_GENERIC_ERROR = -8

# Values both protocols report in PROTOCOL_VERSION: SCPFW
# module/scmi_power_domain/include/internal/scmi_power_domain.h:26 and
# module/scmi_perf/include/internal/scmi_perf.h:24.
PROTO_VERSION = 0x20000
# SCPFW scmi_power_domain.h:27-31: ON is state id 0, type 0; OFF is type bit 30.
PD_STATE_ON = 0x00000000
PD_STATE_OFF = 1 << 30

# ---------------------------------------------------------------------------
# Illustrative values (not read from the board): marked UNVERIFIED in README.md.

GPU_PD = 0                       # st,stm32mp25-power.h:9 PD_SCMI_GPU; #1281
GPU_PERF = 1                     # #1281: the GPU node's scmi_perf 1
CHANNEL = 0                      # stm32mp251.dtsi:103-108 channel 0 (#1281)
PD_DOMAIN_COUNT = 1              # UNVERIFIED: only PD_SCMI_GPU is defined
PERF_DOMAIN_COUNT = 2            # UNVERIFIED: domain 1 exists, 0 assumed CPU
PERF_LEVEL = 800000              # UNVERIFIED: illustrative level
ARG_PA = 0x88000000              # illustrative physical addresses, inside
PAGELIST_PA = 0x88001000         # System RAM (0x84000000-0xfa7fffff, #1281)
SHM_PA = 0x88002000
SHM_SIZE = 0x1000
SHM_COOKIE = 0x1                 # tmem.shm_ref is the client's own handle
SESSION_ID = 0x1                 # UNVERIFIED: the id OP-TEE hands out
IN_OFFS = 0x0
OUT_OFFS = 0x80                  # illustrative: 128-byte reply buffer
OUT_MAX = 0x80
TOKEN = 0x01

# ---------------------------------------------------------------------------
# Encoders.


def hexs(b):
    return b.hex()


def u64regs(*regs):
    regs = list(regs) + [0] * (8 - len(regs))
    return struct.pack("<8Q", *regs)


def param(attr, a=0, b=0, c=0):
    return PARAM.pack(attr, a, b, c)


def arg(cmd, func, session, ret, ret_origin, params):
    return (ARG_HDR.pack(cmd, func, session, 0, 0, ret, ret_origin,
                         len(params)) + b"".join(params))


def scmi_header(msg_id, proto, token=TOKEN, mtype=0):
    # SCPFW mod_scmi_header.h:14-17 (positions 0, 8, 10, 18), SPEC 4.2.2.2
    return struct.pack("<I", msg_id | (mtype << 8) | (proto << 10) |
                       (token << 18))


def scmi_msg(proto, msg_id, *words, token=TOKEN):
    return scmi_header(msg_id, proto, token) + b"".join(
        struct.pack("<I", w & 0xFFFFFFFF) for w in words)


def invoke_msg(session, in_len, out_len):
    """PTA_SCMI_CMD_PROCESS_MSG_CHANNEL: ST scmi.c cmd_process_msg_channel,
    types VALUE_INPUT, MEMREF_INPUT, MEMREF_OUTPUT, NONE."""
    return arg(CMD_INVOKE_COMMAND, PTA_CMD_PROCESS_MSG_CHANNEL, session, 0, 0, [
        param(ATTR_VALUE_INPUT, CHANNEL),
        param(ATTR_RMEM_INPUT, IN_OFFS, in_len, SHM_COOKIE),
        param(ATTR_RMEM_OUTPUT, OUT_OFFS, OUT_MAX, SHM_COOKIE),
    ]), (session, in_len, out_len)


def invoke_msg_reply(session, in_len, out_len, ret=TEE_SUCCESS,
                     origin=TEE_ORIGIN_TRUSTED_APP, channel=CHANNEL):
    # entry_std.c copy_out_param(): only the RMEM_OUTPUT size is written back
    return arg(CMD_INVOKE_COMMAND, PTA_CMD_PROCESS_MSG_CHANNEL, session, ret,
               origin, [
                   param(ATTR_VALUE_INPUT, channel),
                   param(ATTR_RMEM_INPUT, IN_OFFS, in_len, SHM_COOKIE),
                   param(ATTR_RMEM_OUTPUT, OUT_OFFS, out_len, SHM_COOKIE),
               ])


# ---------------------------------------------------------------------------
# Fixture content: each entry is (name, bytes, comment).


class File:
    def __init__(self, title, header):
        self.title = title
        self.header = header
        self.items = []

    def add(self, name, data, note):
        assert " " not in name and name not in [n for n, _, _ in self.items]
        self.items.append((name, data, note))

    def render(self):
        out = ["# " + self.title, "#"]
        out += ["# " + l if l else "#" for l in self.header.strip().split("\n")]
        out += ["#", "# Format: one vector per line, `name hexbytes` (no spaces",
                "# inside the hex).  `#` lines are comments.  Generated by",
                "# tools/optee/scmi_vectors.py; do not edit by hand.", ""]
        for name, data, note in self.items:
            for l in note.strip().split("\n"):
                out.append("# " + l)
            out.append("%s %s" % (name, hexs(data)))
            out.append("")
        return "\n".join(out).rstrip("\n") + "\n"


def build_smc():
    f = File("OP-TEE SMC register sets (SMC32 calling convention)", """
Each vector is x0..x7 as eight little-endian u64, 64 bytes.  `call_` vectors
are the registers the non-secure side sets; `ret_` vectors are what the secure
side returns in the same registers (unused registers are zero here; the
secure side preserves a4-a7, SMCCC says x4-x7 are preserved).
Sources: OPTEE core/arch/arm/include/sm/optee_smc.h at tag 4.0.0.
Capability replies follow ST tag 4.0.0-stm32mp-r3.1 plat-stm32mp2/conf.mk and
OPTEE core/arch/arm/tee/entry_fast.c:67-121 (tee_entry_exchange_capabilities).""")
    f.add("call_calls_uid", u64regs(SMC_CALLS_UID),
          "x0 = OPTEE_SMC_CALLS_UID = FAST|owner 63|func 0xFF01 "
          "(optee_smc.h:89-92)")
    f.add("ret_calls_uid", u64regs(0x384FB3E0, 0xE7F811E3, 0xAF630002,
                                   0xA5D5C51B),
          "x0..x3 = OPTEE_MSG_UID_0..3 (optee_msg.h:251-254)")
    f.add("call_calls_revision", u64regs(SMC_CALLS_REVISION),
          "x0 = OPTEE_SMC_CALLS_REVISION (optee_smc.h:101-104)")
    f.add("ret_calls_revision", u64regs(2, 0),
          "x0 = OPTEE_MSG_REVISION_MAJOR 2, x1 = MINOR 0 "
          "(optee_msg.h:262-263; entry_fast.c:346-347)")
    f.add("call_get_os_uuid", u64regs(SMC_GET_OS_UUID),
          "x0 = OPTEE_SMC_CALL_GET_OS_UUID (optee_smc.h:116-119)")
    f.add("ret_get_os_uuid", u64regs(0x486178E0, 0xE7F811E3, 0xBC5E0002,
                                     0xA5D5C51B),
          "x0..x3 = OPTEE_MSG_OS_OPTEE_UUID_0..3 (optee_msg.h:275-278)")
    f.add("call_get_os_revision", u64regs(SMC_GET_OS_REVISION),
          "x0 = OPTEE_SMC_CALL_GET_OS_REVISION (optee_smc.h:131-134)")
    f.add("ret_get_os_revision", u64regs(4, 0),
          "x0 = CFG_OPTEE_REVISION_MAJOR 4 (ST mk/config.mk:123; entry_fast.c"
          ":360-361), x1 = MINOR (ST config: UNVERIFIED, 0 here)")
    f.add("call_exchange_capabilities",
          u64regs(SMC_EXCHANGE_CAPABILITIES, 0),
          "x0 = OPTEE_SMC_EXCHANGE_CAPABILITIES (FAST|owner 50|func 9, "
          "optee_smc.h:320-322); x1 = non-secure capabilities, 0 = SMP "
          "(bit 0 UNIPROCESSOR is the only legal bit, entry_fast.c:85)")
    sec = (SEC_CAP_DYNAMIC_SHM | SEC_CAP_MEMREF_NULL | SEC_CAP_ASYNC_NOTIF |
           SEC_CAP_RPC_ARG)
    f.add("ret_exchange_capabilities",
          u64regs(RETURN_OK, sec, 64 + 5, 4),
          "x0 = OK; x1 = DYNAMIC_SHM|MEMREF_NULL|ASYNC_NOTIF|RPC_ARG = 0x74 "
          "(entry_fast.c:94-119 with ST plat-stm32mp2/conf.mk:157 "
          "CFG_CORE_RESERVED_SHM=n, :113 ASYNC_NOTIF=y, mk/config.mk:678 "
          "DYN_SHM=y); x2 = NOTIF_VALUE_MAX = 64 + CFG_NUM_THREADS 5 "
          "(kernel/notif.h:28-38, conf.mk:161); x3 = "
          "THREAD_RPC_MAX_NUM_PARAMS 4 (kernel/thread.h:27).  UNVERIFIED on "
          "the board: the build's real config")
    f.add("call_with_rpc_arg",
          u64regs(SMC_CALL_WITH_RPC_ARG, ARG_PA >> 32, ARG_PA & 0xFFFFFFFF, 0),
          "x0 = OPTEE_SMC_CALL_WITH_RPC_ARG (STD|owner 50|func 18, "
          "optee_smc.h:202-203); x1 = upper, x2 = lower 32 bits of the "
          "physical address of struct optee_msg_arg (optee_smc.h:148-150); "
          "x3 = cache attribute, unused for dynamic shm.  The buffer must "
          "also hold the RPC arg space after the call arg (optee_smc.h:138-142)")
    f.add("call_with_arg",
          u64regs(SMC_CALL_WITH_ARG, ARG_PA >> 32, ARG_PA & 0xFFFFFFFF, 0),
          "x0 = OPTEE_SMC_CALL_WITH_ARG (STD|owner 50|func 4, "
          "optee_smc.h:199-201), registers as above, no RPC arg space")
    f.add("ret_call_ok", u64regs(RETURN_OK),
          "x0 = OPTEE_SMC_RETURN_OK; the result is in struct optee_msg_arg "
          "(optee_smc.h:171-175,695)")
    return f


def page_list():
    # optee_msg.h:44-69: 511 page addresses then next_page_data
    pages = [SHM_PA] + [0] * 510 + [0]
    return struct.pack("<512Q", *pages)


def build_msg_arg():
    f = File("OP-TEE struct optee_msg_arg frames for the SCMI PTA", """
struct optee_msg_arg (OPTEE core/include/optee_msg.h:204-216): cmd, func,
session, cancel_id, pad, ret, ret_origin, num_params (eight u32 LE, 32 bytes),
then num_params struct optee_msg_param of 32 bytes: attr u64, then a, b, c
(value) or buf_ptr/offs, size, shm_ref (tmem/rmem) as u64 (:176-185).
Attribute types: optee_msg.h:21-33 (VALUE_INPUT 1, VALUE_OUTPUT 2, VALUE_INOUT
3, RMEM_INPUT 5, RMEM_OUTPUT 6, TMEM_OUTPUT 0xa); META bit 8 (:43),
NONCONTIG bit 9 (:71).  Commands: optee_msg.h:335-340.  Secure-side handling:
OPTEE core/tee/entry_std.c (open_session meta :315-345 and handler :352-399,
close :400-418, invoke :420-455, register_shm :483-504, unregister :506-518).
Illustrative addresses (not board values): arg buffer 0x88000000, page list
0x88001000, shared buffer 0x88002000 (one 4 KiB page, cookie 1), session id 1.
The SCMI request sits at offset 0 of the shared buffer, the reply buffer at
offset 0x80 with size 0x80.  Session id, origin codes of replies are
derived from the source and UNVERIFIED on the board (see README.md).""")
    # register / unregister
    f.add("register_shm_req", arg(CMD_REGISTER_SHM, 0, 0, 0, 0, [
        param(ATTR_TMEM_OUTPUT | ATTR_NONCONTIG, PAGELIST_PA, SHM_SIZE,
              SHM_COOKIE)]),
          "cmd 4; one param, attr TMEM_OUTPUT|NONCONTIG = 0x20a (entry_std.c:"
          "490-492 checks exactly this; optee_msg.h:319-322 comment says "
          "INPUT, the code is authoritative); buf_ptr = page list PA | "
          "offset of the buffer in its first page (0, optee_msg.h:62-63), "
          "size = 0x1000, shm_ref = cookie")
    f.add("register_shm_reply", arg(CMD_REGISTER_SHM, 0, 0, 0, 0, [
        param(ATTR_TMEM_OUTPUT | ATTR_NONCONTIG, PAGELIST_PA, SHM_SIZE,
              SHM_COOKIE)]),
          "ret 0 (entry_std.c:488-504), ret_origin untouched (0), request echoed")
    f.add("register_shm_bad_reply", arg(CMD_REGISTER_SHM, 0, 0,
                                        TEE_ERROR_BAD_PARAMETERS, 0, [
        param(ATTR_TMEM_OUTPUT, PAGELIST_PA, SHM_SIZE, SHM_COOKIE)]),
          "error reply: the same request without NONCONTIG is rejected with "
          "TEE_ERROR_BAD_PARAMETERS (entry_std.c:488-492), origin untouched")
    f.add("unregister_shm_req", arg(CMD_UNREGISTER_SHM, 0, 0, 0, 0, [
        param(ATTR_RMEM_INPUT, 0, 0, SHM_COOKIE)]),
          "cmd 5; param RMEM_INPUT, offs 0, size 0, shm_ref = cookie "
          "(optee_msg.h:323-330; entry_std.c:506-512 reads only shm_ref)")
    f.add("unregister_shm_reply", arg(CMD_UNREGISTER_SHM, 0, 0, 0, 0, [
        param(ATTR_RMEM_INPUT, 0, 0, SHM_COOKIE)]),
          "ret 0 when the cookie is known (entry_std.c:506-514)")
    # open session
    zero16 = b"\0" * 16
    open_params = [
        PARAM.pack(ATTR_META | ATTR_VALUE_INPUT,
                   *struct.unpack("<QQ", PTA_SCMI_UUID), 0),
        PARAM.pack(ATTR_META | ATTR_VALUE_INPUT,
                   *struct.unpack("<QQ", zero16), LOGIN_PUBLIC),
    ]
    f.add("open_session_req", arg(CMD_OPEN_SESSION, 0, 0, 0, 0, open_params),
          "cmd 0; two META|VALUE_INPUT params (attr 0x101, entry_std.c:320-"
          "327): param0 value a/b = the 16 UUID octets in RFC 4122 order "
          "a8cfe406-d4f5-4a2e-9f8d-a25dc754c099 (ST pta_scmi_client.h:8-9; "
          "OPTEE core/tee/uuid.c:23-30 reads big-endian fields from the octets), "
          "param1 value a/b = client UUID (zero for PUBLIC) and c = login "
          "TEE_LOGIN_PUBLIC 0 (entry_std.c:330-345)")
    f.add("open_session_reply", arg(CMD_OPEN_SESSION, 0, SESSION_ID, 0,
                                    TEE_ORIGIN_TRUSTED_APP, open_params),
          "session = id handed out (UNVERIFIED value 1, entry_std.c:392-395), "
          "ret 0, origin 4 TRUSTED_APP (pseudo_ta.c:151,208, UNVERIFIED), "
          "params echoed")
    bad_uuid = bytearray(PTA_SCMI_UUID)
    bad_uuid[15] ^= 0xFF
    bad_params = [
        PARAM.pack(ATTR_META | ATTR_VALUE_INPUT,
                   *struct.unpack("<QQ", bytes(bad_uuid)), 0), open_params[1]]
    f.add("open_session_unknown_uuid_reply",
          arg(CMD_OPEN_SESSION, 0, 0, TEE_ERROR_ITEM_NOT_FOUND,
              TEE_ORIGIN_TEE, bad_params),
          "error reply for an unknown UUID (last octet flipped): session 0 "
          "(entry_std.c:392-395), ret the open-session error, origin TEE_ORIGIN_TEE "
          "initial value (entry_std.c:352-399).  Code and origin UNVERIFIED on the board")
    # capabilities
    f.add("invoke_capabilities_req", arg(CMD_INVOKE_COMMAND,
                                         PTA_CMD_CAPABILITIES, SESSION_ID, 0,
                                         0, [param(ATTR_VALUE_OUTPUT)]),
          "func 0 PTA_SCMI_CMD_CAPABILITIES; one VALUE_OUTPUT param "
          "(ST pta_scmi_client.h:19; ST scmi.c:46 cmd_capabilities)")
    f.add("invoke_capabilities_reply", arg(CMD_INVOKE_COMMAND,
                                           PTA_CMD_CAPABILITIES, SESSION_ID, 0,
                                           TEE_ORIGIN_TRUSTED_APP,
                                           [param(ATTR_VALUE_OUTPUT,
                                                  PTA_CAPS_MSG_HEADER, 0)]),
          "value a = capability mask, MSG_HEADER bit 1 = 0x2 (ST "
          "pta_scmi_client.h:103; ST mk/config.mk:787 "
          "_CFG_SCMI_PTA_MSG_HEADER=y); ST's PTA may add bit 31 OCALL2 "
          "(ST pta_scmi_client.h:109) - UNVERIFIED on the board; b = 0; "
          "copied out by copy_out_param (entry_std.c:275-300)")
    chan_req = [param(ATTR_VALUE_INOUT, CHANNEL, PTA_CAPS_MSG_HEADER)]
    f.add("invoke_get_channel_handle_req", arg(
        CMD_INVOKE_COMMAND, PTA_CMD_GET_CHANNEL_HANDLE, SESSION_ID, 0, 0,
        chan_req),
          "func 3 PTA_SCMI_CMD_GET_CHANNEL_HANDLE; VALUE_INOUT, a = channel "
          "id 0 (stm32mp251.dtsi:103-108, #1281), b = requested caps "
          "MSG_HEADER (ST scmi.c cmd_get_channel_handle)")
    f.add("invoke_get_channel_handle_reply", arg(
        CMD_INVOKE_COMMAND, PTA_CMD_GET_CHANNEL_HANDLE, SESSION_ID, 0,
        TEE_ORIGIN_TRUSTED_APP, chan_req),
          "with CFG_SCMI_SCPFW the PTA only checks the channel id and leaves "
          "value a/b as passed, so the handle is the channel id 0 "
          "(ST scmi.c:373-376; scmi_server.c:36-48)")
    f.add("invoke_get_channel_handle_bad_reply", arg(
        CMD_INVOKE_COMMAND, PTA_CMD_GET_CHANNEL_HANDLE, SESSION_ID,
        TEE_ERROR_BAD_PARAMETERS, TEE_ORIGIN_TRUSTED_APP,
        [param(ATTR_VALUE_INOUT, 7, PTA_CAPS_MSG_HEADER)]),
          "error reply: unknown channel id 7 gives TEE_ERROR_BAD_PARAMETERS "
          "(scmi_server.c:40-42); origin per pseudo_ta.c:151,208 UNVERIFIED")
    # close
    f.add("close_session_req", arg(CMD_CLOSE_SESSION, 0, SESSION_ID, 0, 0, []),
          "cmd 2; session id; no params (entry_std.c:400-418 rejects any)")
    f.add("close_session_reply", arg(CMD_CLOSE_SESSION, 0, SESSION_ID, 0,
                                     TEE_ORIGIN_TEE, []),
          "ret 0, origin TEE_ORIGIN_TEE 3 (entry_std.c:416-417)")
    f.add("close_session_bad_params_reply", arg(
        CMD_CLOSE_SESSION, 0, SESSION_ID, TEE_ERROR_BAD_PARAMETERS,
        TEE_ORIGIN_TEE, [param(ATTR_NONE)]),
          "error reply: a close with one param is rejected "
          "(entry_std.c:400-418)")
    f.add("page_list_one_page", page_list(),
          "the 4096-byte page list register_shm_req points at: 511 page "
          "addresses (entry 0 = 0x88002000, the rest zero) then "
          "next_page_data = 0 (optee_msg.h:44-69; not an arg frame)")
    return f


def scmi_cases():
    """(name, request bytes, reply bytes, note); replies per SCPFW."""
    cases = []

    def ok(proto, mid, *words):
        return scmi_msg(proto, mid, SCMI_SUCCESS, *words)

    # protocol/message ids for each protocol
    for proto, pname, doms, attr in (
            (PROT_POWER, "pd", PD_DOMAIN_COUNT, PD_DOMAIN_COUNT),
            (PROT_PERF, "perf", PERF_DOMAIN_COUNT,
             (1 << 16) | PERF_DOMAIN_COUNT)):
        cases.append((
            "%s_protocol_version" % pname,
            scmi_msg(proto, MSG_PROTOCOL_VERSION),
            ok(proto, MSG_PROTOCOL_VERSION, PROTO_VERSION),
            "PROTOCOL_VERSION (message 0, no payload); reply status 0, "
            "version 0x20000 (SCPFW %s)" % (
                "scmi_power_domain/src/mod_scmi_power_domain.c:268-285, "
                "internal/scmi_power_domain.h:26" if proto == PROT_POWER else
                "scmi_perf/include/internal/scmi_perf.h:24")))
        if proto == PROT_POWER:
            note = ("PROTOCOL_ATTRIBUTES (message 1, no payload); reply "
                    "status, attributes = domain count (%d, UNVERIFIED: the "
                    "board's OP-TEE device tree), statistics address low/"
                    "high/length 0 (mod_scmi_power_domain.c:287-298, "
                    "internal/scmi_power_domain.h)" % doms)
            reply = ok(proto, MSG_PROTOCOL_ATTRIBUTES, doms, 0, 0, 0)
        else:
            note = ("PROTOCOL_ATTRIBUTES (message 1); reply attributes = "
                    "power-in-mW bit 16 set | domain count %d (UNVERIFIED: "
                    "the board's domain count), statistics 0 (no "
                    "MOD_STATISTICS assumed, UNVERIFIED) "
                    "(scmi_perf/src/scmi_perf_protocol_ops.c:267-290, "
                    "internal/scmi_perf.h)" % doms)
            reply = ok(proto, MSG_PROTOCOL_ATTRIBUTES, attr, 0, 0, 0)
        cases.append(("%s_protocol_attributes" % pname,
                      scmi_msg(proto, MSG_PROTOCOL_ATTRIBUTES), reply, note))

    # power domain
    cases.append((
        "pd_state_set_on",
        scmi_msg(PROT_POWER, MSG_PD_STATE_SET, 0, GPU_PD, PD_STATE_ON),
        ok(PROT_POWER, MSG_PD_STATE_SET),
        "POWER_STATE_SET (message 4), payload flags 0 (sync), domain_id 0 "
        "(GPU), power_state 0 = ON (SPEC 4.3.2.5; SCPFW "
        "internal/scmi_power_domain.h); the sync reply is sent from the module's event handler, which was not "
        "read; assumed status-only like the error and async replies "
        "(mod_scmi_power_domain.c:589-617) - UNVERIFIED on the board"))
    cases.append((
        "pd_state_set_off",
        scmi_msg(PROT_POWER, MSG_PD_STATE_SET, 0, GPU_PD, PD_STATE_OFF),
        ok(PROT_POWER, MSG_PD_STATE_SET),
        "POWER_STATE_SET, power_state 0x40000000 = OFF (bit 30 device state "
        "type, id 0; internal/scmi_power_domain.h)"))
    cases.append((
        "pd_state_set_async_flag_rejected",
        scmi_msg(PROT_POWER, MSG_PD_STATE_SET, 2, GPU_PD, PD_STATE_ON),
        scmi_msg(PROT_POWER, MSG_PD_STATE_SET, SCMI_INVALID_PARAMETERS),
        "error: a flag bit outside bit 0 gives INVALID_PARAMETERS (-2), "
        "status-only reply (mod_scmi_power_domain.c:464-472, 534-541)"))
    cases.append((
        "pd_state_set_unknown_domain",
        scmi_msg(PROT_POWER, MSG_PD_STATE_SET, 0, 9, PD_STATE_ON),
        scmi_msg(PROT_POWER, MSG_PD_STATE_SET, SCMI_NOT_FOUND),
        "error: domain 9 does not exist, NOT_FOUND (-4), status-only reply "
        "(mod_scmi_power_domain.c:478-482)"))
    cases.append((
        "pd_state_get_on",
        scmi_msg(PROT_POWER, MSG_PD_STATE_GET, GPU_PD),
        ok(PROT_POWER, MSG_PD_STATE_GET, PD_STATE_ON),
        "POWER_STATE_GET (message 5), domain_id 0; reply status, "
        "power_state 0 = ON for a device-type domain "
        "(mod_scmi_power_domain.c:640-735, :198-203 map)"))
    cases.append((
        "pd_state_get_off",
        scmi_msg(PROT_POWER, MSG_PD_STATE_GET, GPU_PD),
        ok(PROT_POWER, MSG_PD_STATE_GET, PD_STATE_OFF),
        "POWER_STATE_GET reply power_state 0x40000000 = OFF"))
    cases.append((
        "pd_state_get_unknown_domain",
        scmi_msg(PROT_POWER, MSG_PD_STATE_GET, 9),
        scmi_msg(PROT_POWER, MSG_PD_STATE_GET, SCMI_NOT_FOUND),
        "error: NOT_FOUND (-4), status-only reply "
        "(mod_scmi_power_domain.c:667-671, 726-732)"))

    # performance
    cases.append((
        "perf_level_set",
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_SET, GPU_PERF, PERF_LEVEL),
        ok(PROT_PERF, MSG_PERF_LEVEL_SET),
        "PERFORMANCE_LEVEL_SET (message 7), domain_id 1 (GPU), "
        "performance_level 800000 (ILLUSTRATIVE, UNVERIFIED: the board's "
        "OPP table); fire-and-forget success reply = status only "
        "(scmi_perf_protocol_ops.c:658-727, internal/scmi_perf.h)"))
    cases.append((
        "perf_level_set_unknown_domain",
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_SET, 7, PERF_LEVEL),
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_SET, SCMI_NOT_FOUND),
        "error: domain 7 >= domain_count gives NOT_FOUND (-4) "
        "(scmi_perf_protocol_ops.c:673-678)"))
    cases.append((
        "perf_level_set_out_of_range",
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_SET, GPU_PERF, 0xFFFFFFFF),
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_SET, SCMI_OUT_OF_RANGE),
        "error: a level the DVFS module reports FWK_E_RANGE for gives "
        "OUT_OF_RANGE (-5) (scmi_perf_protocol_ops.c:695-712); whether "
        "0xffffffff is out of range on this board is UNVERIFIED"))
    cases.append((
        "perf_level_get",
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_GET, GPU_PERF),
        ok(PROT_PERF, MSG_PERF_LEVEL_GET, PERF_LEVEL),
        "PERFORMANCE_LEVEL_GET (message 8), domain_id 1; reply status and "
        "performance_level (scmi_perf_protocol_ops.c:729-790, "
        "internal/scmi_perf.h); level value illustrative"))
    cases.append((
        "perf_level_get_unknown_domain",
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_GET, 7),
        scmi_msg(PROT_PERF, MSG_PERF_LEVEL_GET, SCMI_NOT_FOUND),
        "error: NOT_FOUND (-4), status-only reply "
        "(scmi_perf_protocol_ops.c:739-744)"))
    return cases


def build_scmi():
    f = File("SCMI messages for the GPU: power domain 0x11 and performance 0x13",
             """
Each message is the 32-bit MSG-format header followed by the payload words, all
little-endian (OP-TEE's MSG shared-memory transport: ST
core/lib/scmi-server/SCP-firmware/module/msg_smt/src/mod_msg_smt.c:26,183-198,
258-259; upstream OPTEE core/drivers/scmi-msg/shm_msg.c:16-60).
Header (SPEC 4.2.2.2; SCPFW module/scmi/include/mod_scmi_header.h:14-17):
bits 7:0 message id, 9:8 message type (0 command), 17:10 protocol id,
27:18 token.  Token here is 1.  A reply echoes the request header verbatim
(mod_msg_smt.c:259) and carries int32 status first, then the payload.
Protocol ids 0x11 power domain, 0x13 performance (SCPFW mod_scmi_std.h:100,
:132).  Message ids: PROTOCOL_VERSION 0, PROTOCOL_ATTRIBUTES 1; power domain
POWER_STATE_SET 4, POWER_STATE_GET 5 (mod_scmi_std.h:105-108); performance
LEVEL_SET 7, LEVEL_GET 8 (mod_scmi_std.h:138-143).  Status codes: SCPFW
mod_scmi_std.h:49-59 (0 success, -1 not supported, -2 invalid parameters,
-4 not found, -5 out of range, -8 generic error).  GPU domains: power domain
0 (PD_SCMI_GPU, ST core/include/dt-bindings/power/st,stm32mp25-power.h:9),
performance domain 1 (#1281, stm32mp255.dtsi:176).""")
    for name, req, rep, note in scmi_cases():
        f.add("req_" + name, req, note)
        f.add("rep_" + name, rep, "reply to req_%s" % name)
    return f


def build_invoke():
    f = File("OP-TEE invoke frames carrying each SCMI message", """
For each request in scmi_msgs.txt, the PTA_SCMI_CMD_PROCESS_MSG_CHANNEL invoke
(cmd 1, func 4; ST core/pta/scmi.c cmd_process_msg_channel, expected
parameter types VALUE_INPUT, MEMREF_INPUT, MEMREF_OUTPUT, NONE) and its
reply.  Parameter 0: value a = channel id 0.  Parameter 1: RMEM_INPUT,
offs 0, size = request length, shm_ref = cookie 1.  Parameter 2: RMEM_OUTPUT,
offs 0x80, size 0x80, shm_ref = cookie 1 (the shared buffer registered by
register_shm_req in msg_arg.txt).  The reply rewrites parameter 2's size to
the reply length (scmi.c:174; entry_std.c copy_out_param :275-300);
ret 0, origin TRUSTED_APP (pseudo_ta.c:151,208; UNVERIFIED on the board).
The shared-buffer bytes themselves are the req_/rep_ vectors of
scmi_msgs.txt (request at offset 0, reply at offset 0x80).  The session id is
1 (illustrative, UNVERIFIED).""")
    for name, req, rep, note in scmi_cases():
        frame, _ = invoke_msg(SESSION_ID, len(req), len(rep))
        f.add("invoke_" + name + "_req", frame,
              "invoke carrying req_%s (%d bytes in)" % (name, len(req)))
        f.add("invoke_" + name + "_reply",
              invoke_msg_reply(SESSION_ID, len(req), len(rep)),
              "reply: output size %d = reply length of rep_%s" % (len(rep),
                                                                  name))
    f.add("invoke_unknown_channel_reply",
          invoke_msg_reply(SESSION_ID, 8, 0x80, TEE_ERROR_BAD_PARAMETERS,
                           TEE_ORIGIN_TRUSTED_APP, channel=7),
          "error reply: channel id 7 is unknown to the server, "
          "scmi_server_get_channel fails with TEE_ERROR_BAD_PARAMETERS "
          "(scmi_server.c:36-42, called from scmi_server_msg_process_thread "
          ":62-74), the PTA returns it (scmi.c:166-176) and the output size "
          "is left as passed (written only on success, scmi.c:173-174); "
          "origin per pseudo_ta.c:151,208, UNVERIFIED on the board")
    return f


SPDX = ("# SPDX-License-Identifier: Apache-2.0\n"
        "# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")

FILES = {
    "smc.txt": build_smc,
    "msg_arg.txt": build_msg_arg,
    "scmi_msgs.txt": build_scmi,
    "invoke_frames.txt": build_invoke,
}


def render_all():
    return {name: SPDX + fn().render() for name, fn in FILES.items()}


def main(argv):
    check = "--check" in argv
    unknown = [a for a in argv[1:] if a != "--check"]
    if unknown:
        sys.stderr.write("usage: scmi_vectors.py [--check]\n")
        return 2
    rendered = render_all()
    bad = 0
    for name, text in rendered.items():
        path = os.path.join(OUT_DIR, name)
        if check:
            try:
                with open(path, encoding="utf-8") as fh:
                    cur = fh.read()
            except OSError:
                cur = None
            if cur != text:
                sys.stderr.write("%s: differs from the generator\n" % path)
                bad += 1
        else:
            os.makedirs(OUT_DIR, exist_ok=True)
            with open(path, "w", encoding="utf-8", newline="\n") as fh:
                fh.write(text)
    if check:
        extra = sorted(set(os.listdir(OUT_DIR)) - set(rendered)) if \
            os.path.isdir(OUT_DIR) else []
        for e in extra:
            sys.stderr.write("%s: not produced by the generator\n" % e)
            bad += 1
        return 1 if bad else 0
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
