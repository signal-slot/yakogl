// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the STM32MP257F's GPU power path (tests only):
//! the RCC's `RCC_GPUCFGR`, the GPU's host-interface registers a power-up
//! reads, and a scripted OP-TEE SCMI server for the power domain (0x11) and
//! performance (0x13) protocols. Plan #322, issues #7 and #1281.
//!
//! Like [`crate::model::Model`] and [`crate::model_am62p::Am62pModel`] it
//! records every access in order and answers only what the drivers poll.
//! Offsets, bits, reset values and message layouts are restated here from
//! their sources, not shared with `soc::stm32mp2` or `gpu::vivante`, so a
//! wrong constant cannot agree with itself. The SCMI codec's own types
//! ([`scmi::Transport`], [`scmi::word`], [`scmi::header`], the status codes)
//! are used, not copied: the server answers at the level of that trait, one
//! message in, one reply out.
//!
//! # Sources
//!
//! - **`RCC_GPUCFGR`**, offset 0x83C of the RCC: bit 0 the GPU reset
//!   (`GPURST`), bit 1 the gate of both GPU clocks (`GPUEN`; `CK_BUS_GPU` 91
//!   and `CK_KER_GPU` 340 are one gate bit), bit 2 `GPULPEN` (not modelled).
//!   Read here from TF-A, ST's tag `v2.10-stm32mp-r3.1`,
//!   `include/drivers/st/stm32mp25_rcc.h:440` (`RCC_GPUCFGR U(0x83C)`) and
//!   `:3539-3541` (`RCC_GPUCFGR_GPURST BIT_32(0)`, `RCC_GPUCFGR_GPUEN
//!   BIT_32(1)`, `RCC_GPUCFGR_GPULPEN BIT_32(2)`), and from OP-TEE, ST's tag
//!   `4.0.0-stm32mp-r3.1`, `core/include/drivers/stm32mp25_rcc.h:437` and
//!   `:2024-2026` (the same values). That is the cross-check the r-rcc-src
//!   study made (TF-A, OP-TEE, Zephyr, U-Boot, Linux) and the register
//!   galcore 6.4.21 writes (r-galcore; its file and line are in that study's
//!   report, which is not in the tree). The clock and reset ids are the
//!   device tree's (#1281: `clocks = <&rcc 91>, <&rcc 340>`, `resets =
//!   <&rcc 75>`). **None of this is confirmed by RM0457, which could not be
//!   fetched** (r-rm0457): the offset, the bit positions, the reset bit's
//!   polarity (taken as 1 = held in reset, the MP2 RCC's `xxRST` convention,
//!   as `soc::stm32mp2::rcc` takes it) and the register's reset value, which
//!   no source read here states ([`Mp2Config::rcc_gpucfgr`] starts at 0:
//!   gated, not held in reset; a test can start from another value).
//! - **The GPU's registers** (rnndb `state_hi.xml.h`, vendored as
//!   `driver/src/vivante/hw/state_hi.xml.h`): `HI_CLOCK_CONTROL` 0x000
//!   (`:57`; `IDLE_3D` bit 16 `:67`, `IDLE_2D` bit 17 `:68`),
//!   `HI_IDLE_STATE` 0x004 (`:75`), `HI_INTR_ACKNOWLEDGE` 0x010 (`:121`),
//!   `HI_INTR_ENBL` 0x014 (`:128`), `HI_CHIP_IDENTITY` 0x018 (`:133`),
//!   `HI_CHIP_MODEL` 0x020 (`:146`), `HI_CHIP_REV` 0x024 (`:148`),
//!   `HI_CHIP_CUSTOMER_ID` 0x030 (`:154`), `HI_CHIP_PRODUCT_ID` 0x0A8
//!   (`:249`), `HI_CHIP_ECO_ID` 0x0E8 (`:253`). Linux's `etnaviv_hw_identify`
//!   (`drivers/gpu/drm/etnaviv/etnaviv_gpu.c`, v6.6) reads `CHIP_IDENTITY`
//!   first and, unless its family field is 0x01, `CHIP_MODEL`, `CHIP_REV`,
//!   `CHIP_CUSTOMER_ID`, then `CHIP_PRODUCT_ID` and `CHIP_ECO_ID`; so the
//!   model is at 0x020 and the revision at 0x024 (the item's brief wrote
//!   0x018 and 0x020; 0x018 is `CHIP_IDENTITY`). The values are #1281's
//!   day-one readings under ST's Linux: model 0x8000, revision 0x6205,
//!   product 0x80003, eco 0. The customer id was not read (Mesa's
//!   `GCNANOULTRA31_VIP2` row says 0x15, `gpu::vivante::hw`) and
//!   `CHIP_IDENTITY` was not recorded: both start at 0 here. All idle reads
//!   0x7fffffff (every module idle, `AXI_LP` bit 31 clear), as
//!   [`crate::model::Model`]'s GC does, once the domain is on, the clock runs
//!   and the reset is released. `HI_INTR_ENBL` is storage; `HI_INTR_ACKNOWLEDGE`
//!   reads the pending bits ([`Mp2Model::raise_irq`]) and clears them, as the
//!   board's handler expects (r-galcore: one read of 0x10 acknowledges, zero
//!   means not ours). Every other offset below the window is plain storage.
//! - **The window**: 0x800 bytes (#1281, `stm32mp255.dtsi:171`); nothing at
//!   or above it is read from the CPU (plan #322, section 6).
//! - **SCMI** (Arm DEN0056, v3.x): 4.1.2 the header (message id bits 7:0,
//!   type 9:8, protocol 17:10, token 27:18; a reply echoes it, then a signed
//!   32-bit status); 4.3 power domain (PROTOCOL_VERSION 0, PROTOCOL_ATTRIBUTES
//!   1, POWER_DOMAIN_ATTRIBUTES 3, POWER_STATE_SET 4, POWER_STATE_GET 5); 4.5
//!   performance (PERFORMANCE_DOMAIN_ATTRIBUTES 3, PERFORMANCE_LEVEL_SET 7,
//!   PERFORMANCE_LEVEL_GET 8). The server's answers are SCP-firmware's as
//!   ST's OP-TEE carries it (`core/lib/scmi-server/SCP-firmware`, tag above):
//!   - versions 0x20000 for both (`module/scmi_power_domain/include/internal/
//!     scmi_power_domain.h:26`, `module/scmi_perf/include/internal/scmi_perf.h:24`);
//!   - statuses (`module/scmi/include/mod_scmi_std.h:49-59`): 0 success, -1
//!     not supported, -2 invalid parameters, -3 denied, -4 not found, -5 out
//!     of range, -8 generic error, -10 protocol error;
//!   - POWER_STATE_SET (`module/scmi_power_domain/src/mod_scmi_power_domain.c`):
//!     a flag bit other than bit 0, or a state bit outside 0x4FFFFFFF, is
//!     INVALID_PARAMETERS (`:468-471`), an unknown domain NOT_FOUND
//!     (`:474-481`); bit 0 (asynchronous) is taken for a device domain
//!     (`:589-594`); the reply is status-only. The GPU's domain is a device
//!     domain whose ON enables the `vddgpu` regulator and OFF disables it
//!     (`product/optee-stm32mp2/module/stm32_pd/src/mod_stm32_pd.c:51-117`);
//!   - POWER_DOMAIN_ATTRIBUTES of a device domain: bit 29 (synchronous set)
//!     and, if the server was built with notifications, bit 31 (`:370-379`;
//!     not modelled), then the 16-byte name (`:391-394`), `gpu-pd` from
//!     OP-TEE's `domain-name` (`core/arch/arm/dts/stm32mp25-st-scmi-cfg.dtsi:37`);
//!   - PERFORMANCE_LEVEL_SET (`module/scmi_perf/src/scmi_perf_protocol_ops.c:658-727`):
//!     an unknown domain NOT_FOUND (`:674-679`), a level outside the OPP table
//!     OUT_OF_RANGE (`module/dvfs/src/mod_dvfs.c:551`, `:712-713`);
//!     otherwise **fire-and-forget**: the change is queued as an event and the
//!     reply is SUCCESS (`mod_dvfs.c:244-271`, `:569`; `:710-711`). The GPU's
//!     OPP change is refused later, inside OP-TEE, while the GPU's regulator is
//!     on (`core/drivers/stm32_gpu_opp.c:54-57`, `TEE_ERROR_ACCESS_DENIED`,
//!     "Change OPP not allowed when GPU is used"; `product/optee-stm32mp2/
//!     module/stm32_opp/src/mod_stm32_opp.c:168-184`; `arch/none/optee/src/
//!     arch_main.c:118-120` maps it to `FWK_E_ACCESS`), and the level stays
//!     (`mod_dvfs.c:780` updates it on success only). So the agent sees
//!     SUCCESS and an unchanged LEVEL_GET, not DENIED: the model answers so by
//!     default, records [`Fault::LevelWhileOn`], and a test that wants the
//!     refusal as a status scripts it with [`Mp2Model::force_status`]. A level
//!     equal to the current one changes nothing (`mod_dvfs.c:560-562`);
//!   - PERFORMANCE_DOMAIN_ATTRIBUTES (`scmi_perf_protocol_ops.c:391-415`):
//!     the attribute bits (`internal/scmi_perf.h:130-134`: set limits 31, set
//!     level 30, notifications 29 and 28, fast channel 27), rate limit 0
//!     ("Unsupported", `:405`), the sustained frequency in kHz and level
//!     (`:406-407`), the 16-byte name. The model answers set-level only; the
//!     board's names, OPP table and sustained point were not read
//!     ([`Mp2Config`]: level 800000 is the reference vectors' illustrative
//!     value, `tools/testdata/optee-scmi/scmi_msgs.txt`);
//!   - PROTOCOL_ATTRIBUTES as the reference vectors (`scmi_msgs.txt`): one
//!     power domain; two performance domains (the CPU's is 0, the GPU's 1:
//!     `core/arch/arm/plat-stm32mp2/scmi_server_scpfw.c:369-378` registers
//!     the CPU's first) with the power-in-mW bit; no statistics area.
//!
//!   The GPU's domains: power 0 (`PD_SCMI_GPU`, `core/include/dt-bindings/
//!   power/st,stm32mp25-power.h:9`), performance 1 (#1281,
//!   `stm32mp255.dtsi:176`).
//!
//! # Faults
//!
//! The cause of the OP-TEE panic when etnaviv loaded is not established
//! (docs/notes/pre-study-stm32mp257f-dk.md, "The etnaviv panic and what our
//! image avoids"); the optee-panic study's report is not in the tree and
//! names, as relayed, no condition beyond the note's. So every condition
//! below is the stricter reading, and the model records it as a [`Fault`]
//! and goes on (a test asserts [`Mp2Model::faults`] is empty):
//! - a GPU register access while power domain 0 is off, the clock gated or
//!   the reset held: a read answers [`POISON`], a write is dropped. Which of
//!   the three a real access survives is not known (a gated APB read may
//!   hang the bus, a read in reset may return reset values); each one faults
//!   here;
//! - a GPU access at or above the 0x800 window;
//! - an `RCC_GPUCFGR` write that starts the clock or releases the reset while
//!   power domain 0 is off: etnaviv's order (clocks in its runtime resume,
//!   `etnaviv_gpu_clk_enable`, before any hardware access) is the one that
//!   preceded the panic, galcore's (domain first) the one the image follows
//!   ("never the clock gates before the domain", the note). Gating and
//!   holding the reset are allowed in any state;
//! - an `RCC_GPUCFGR` write that releases the reset while the clock is gated
//!   (galcore's order is gate, assert, release; a reset without a clock does
//!   not propagate; no source states it);
//! - POWER_STATE_SET off while the clock still runs (the reverse of the order:
//!   reset held, gate off, then the domain; no source states it);
//! - PERFORMANCE_LEVEL_SET while power domain 0 is on (OP-TEE refuses the
//!   change inside, see above; the image sets the level before the domain);
//! - any other RCC register (only `RCC_GPUCFGR` belongs to this path: no PLL,
//!   no `CK_SCMI_PLL3`);
//! - an SCMI message of another protocol (answered NOT_SUPPORTED), or one
//!   that names the CPU's performance domain 0 (answered as the server
//!   would). Both are stricter than OP-TEE, which serves them.

use crate::firmware::scmi;
use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The RCC's fake base (only `RCC_GPUCFGR` is modelled).
pub const RCC_BASE: usize = 0x1000_0000;
/// The GPU's fake base; 0x1000 decodes, 0x800 is the window.
pub const GPU_BASE: usize = 0x2000_0000;
const RCC_SIZE: usize = 0x1000;
const GPU_DECODE: usize = 0x1000;
const GPU_WINDOW: u32 = 0x800;

/// What a faulting GPU read answers. It matches the interconnect's absent
/// pattern `gpu::vivante::hw::reg_absent` knows (0xdeadde..), so an identify
/// against the model refuses it as the i.MX8MP's did on its board.
pub const POISON: u32 = 0xdead_dead;

// RCC (see the module text).
const GPUCFGR: u32 = 0x83c;
const GPU_RST: u32 = 1 << 0;
const GPU_EN: u32 = 1 << 1;
// GPU (state_hi.xml.h).
const CLOCK_CONTROL: u32 = 0x000;
const IDLE_STATE: u32 = 0x004;
const INTR_ACK: u32 = 0x010;
const INTR_ENBL: u32 = 0x014;
const CHIP_IDENTITY: u32 = 0x018;
const CHIP_MODEL: u32 = 0x020;
const CHIP_REV: u32 = 0x024;
const CHIP_CUSTOMER_ID: u32 = 0x030;
const CHIP_PRODUCT_ID: u32 = 0x0a8;
const CHIP_ECO_ID: u32 = 0x0e8;
const IDLE_ALL: u32 = 0x7fff_ffff;
/// FE, PE and SH busy: what a part that never settles shows.
const IDLE_STUCK: u32 = 0x7fff_fff2;
const IDLE_3D_2D: u32 = (1 << 16) | (1 << 17);
// SCMI (DEN0056; SCP-firmware's values in the module text).
const P_POWER: u8 = 0x11;
const P_PERF: u8 = 0x13;
const M_VERSION: u8 = 0x0;
const M_ATTRIBUTES: u8 = 0x1;
const M_DOMAIN_ATTRIBUTES: u8 = 0x3;
const M_PD_STATE_SET: u8 = 0x4;
const M_PD_STATE_GET: u8 = 0x5;
const M_PERF_LEVEL_SET: u8 = 0x7;
const M_PERF_LEVEL_GET: u8 = 0x8;
const PD_ON: u32 = 0;
const PD_OFF: u32 = 1 << 30;
/// `SCMI_PD_POWER_STATE_SET_POWER_STATE_MASK`.
const PD_STATE_MASK: u32 = 0x4fff_ffff;
const PD_FLAG_ASYNC: u32 = 1 << 0;
const PD_ATTR_SET_SYNC: u32 = 1 << 29;
const PERF_ATTR_CAN_SET_LEVEL: u32 = 1 << 30;
const PERF_ATTR_POWER_MW: u32 = 1 << 16;
const GPU_PD: u32 = 0;
const CPU_PERF: u32 = 0;
const GPU_PERF: u32 = 1;

/// The model's starting state and its answers.
#[derive(Clone, Debug)]
pub struct Mp2Config {
    /// `RCC_GPUCFGR` at start (its reset value is not known).
    pub rcc_gpucfgr: u32,
    /// Power domain 0 on at start.
    pub domain_on: bool,
    /// The levels of performance domains 0 (CPU) and 1 (GPU) at start.
    pub levels: [u32; 2],
    /// The GPU's OPP table: LEVEL_SET of anything else is OUT_OF_RANGE.
    /// 800000 is the vectors' illustrative level, not the board's table.
    pub gpu_levels: Vec<u32>,
    /// PROTOCOL_VERSION of power domain and of performance.
    pub versions: [u32; 2],
    /// The 16-byte names POWER_DOMAIN_ATTRIBUTES and
    /// PERFORMANCE_DOMAIN_ATTRIBUTES answer (power domain 0; performance
    /// domains 0 and 1). Only the power domain's is a source's.
    pub pd_name: &'static str,
    pub perf_names: [&'static str; 2],
    pub model: u32,
    pub revision: u32,
    pub product_id: u32,
    pub eco_id: u32,
    pub customer_id: u32,
    pub identity: u32,
    /// `HI_IDLE_STATE` never reports every module idle.
    pub never_idle: bool,
}

impl Default for Mp2Config {
    fn default() -> Self {
        Mp2Config {
            rcc_gpucfgr: 0,
            domain_on: false,
            levels: [1_200_000, 800_000],
            gpu_levels: vec![800_000],
            versions: [0x2_0000, 0x2_0000],
            pd_name: "gpu-pd",
            perf_names: ["cpu", "gpu"],
            model: 0x8000,
            revision: 0x6205,
            product_id: 0x8_0003,
            eco_id: 0,
            customer_id: 0,
            identity: 0,
            never_idle: false,
        }
    }
}

/// Why a GPU access faulted (the first that holds, in this order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Off {
    Domain,
    Clock,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A GPU access (offset, write) with the GPU not powered.
    Gpu { off: u32, write: bool, why: Off },
    /// A GPU access at or above the 0x800 window.
    Window { off: u32, write: bool },
    /// `RCC_GPUCFGR` written to run the clock or release the reset with
    /// power domain 0 off.
    RccBeforeDomain { val: u32 },
    /// `RCC_GPUCFGR` written to release the reset with the clock gated.
    ReleaseWithoutClock { val: u32 },
    /// An RCC register other than `RCC_GPUCFGR`.
    RccOther { off: u32, write: bool },
    /// Power domain 0 turned off with the clock running.
    DomainOffClockOn,
    /// PERFORMANCE_LEVEL_SET of the GPU's domain while power domain 0 is on.
    LevelWhileOn { level: u32 },
    /// An SCMI protocol other than 0x11 and 0x13.
    Protocol { proto: u8, msg: u8 },
    /// A message naming the CPU's performance domain.
    CpuDomain { msg: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// RCC read and write (offset, value).
    RccR(u32, u32),
    RccW(u32, u32),
    /// GPU read and write (offset, value; a faulting read logs [`POISON`]).
    GpuR(u32, u32),
    GpuW(u32, u32),
    /// One SCMI exchange, request and reply bytes as transferred (an empty
    /// reply for a transport failure).
    Scmi {
        tx: Vec<u8>,
        rx: Vec<u8>,
    },
    Fault(Fault),
}

struct State {
    cfg: Mp2Config,
    rcc: u32,
    domain_on: bool,
    levels: [u32; 2],
    /// Pending interrupt bits, read and cleared through `HI_INTR_ACKNOWLEDGE`.
    irq: u32,
    /// Plain storage: every GPU offset below the window that is not answered
    /// from state (`HI_INTR_ENBL` included).
    regs: BTreeMap<u32, u32>,
    log: Vec<Event>,
    /// Forced status per (protocol, message): the reply is status-only and
    /// the server changes nothing.
    forced: Vec<(u8, u8, i32)>,
    transport_error: Option<i32>,
}

/// The model. See the module text.
pub struct Mp2Model(RefCell<State>);

impl Default for Mp2Model {
    fn default() -> Self {
        Mp2Model::new()
    }
}

/// The SCMI server's end of the channel ([`Mp2Model::scmi`]): a
/// [`scmi::Transport`] a client or `soc::stm32mp2::power` drives.
pub struct Mp2Scmi<'a> {
    model: &'a Mp2Model,
}

impl Mp2Model {
    pub fn new() -> Mp2Model {
        Mp2Model::with_config(Mp2Config::default())
    }

    pub fn with_config(cfg: Mp2Config) -> Mp2Model {
        let s = State {
            rcc: cfg.rcc_gpucfgr,
            domain_on: cfg.domain_on,
            levels: cfg.levels,
            irq: 0,
            regs: BTreeMap::new(),
            log: Vec::new(),
            forced: Vec::new(),
            transport_error: None,
            cfg,
        };
        Mp2Model(RefCell::new(s))
    }

    /// A transport to the scripted SCMI server.
    pub fn scmi(&self) -> Mp2Scmi<'_> {
        Mp2Scmi { model: self }
    }

    /// Answer every (proto, msg) with `status` from now on (SUCCESS lifts it).
    pub fn force_status(&self, proto: u8, msg: u8, status: i32) {
        let mut s = self.0.borrow_mut();
        s.forced.retain(|f| (f.0, f.1) != (proto, msg));
        if status != scmi::STATUS_SUCCESS {
            s.forced.push((proto, msg, status));
        }
    }

    /// Fail every transfer with `code` (None: work again).
    pub fn fail_transport(&self, code: Option<i32>) {
        self.0.borrow_mut().transport_error = code;
    }

    /// Raise interrupt bits: `HI_INTR_ACKNOWLEDGE` shows them until read.
    pub fn raise_irq(&self, bits: u32) {
        self.0.borrow_mut().irq |= bits;
    }

    pub fn events(&self) -> Vec<Event> {
        self.0.borrow().log.clone()
    }

    pub fn faults(&self) -> Vec<Fault> {
        self.0
            .borrow()
            .log
            .iter()
            .filter_map(|e| match e {
                Event::Fault(f) => Some(*f),
                _ => None,
            })
            .collect()
    }

    /// The SCMI exchanges in order (request, reply).
    pub fn messages(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.0
            .borrow()
            .log
            .iter()
            .filter_map(|e| match e {
                Event::Scmi { tx, rx } => Some((tx.clone(), rx.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn clear_log(&self) {
        self.0.borrow_mut().log.clear();
    }

    pub fn domain_on(&self) -> bool {
        self.0.borrow().domain_on
    }

    pub fn rcc(&self) -> u32 {
        self.0.borrow().rcc
    }

    /// The GPU's performance level (domain 1).
    pub fn gpu_level(&self) -> u32 {
        self.0.borrow().levels[GPU_PERF as usize]
    }

    /// `HI_INTR_ENBL` as written.
    pub fn intr_enbl(&self) -> u32 {
        self.0.borrow().regs.get(&INTR_ENBL).copied().unwrap_or(0)
    }

    fn gpu_off(s: &State) -> Option<Off> {
        if !s.domain_on {
            Some(Off::Domain)
        } else if s.rcc & GPU_EN == 0 {
            Some(Off::Clock)
        } else if s.rcc & GPU_RST != 0 {
            Some(Off::Reset)
        } else {
            None
        }
    }

    fn gpu_read(s: &mut State, off: u32) -> u32 {
        if off >= GPU_WINDOW {
            s.log.push(Event::GpuR(off, POISON));
            s.log.push(Event::Fault(Fault::Window { off, write: false }));
            return POISON;
        }
        if let Some(why) = Self::gpu_off(s) {
            s.log.push(Event::GpuR(off, POISON));
            s.log.push(Event::Fault(Fault::Gpu { off, write: false, why }));
            return POISON;
        }
        let stored = s.regs.get(&off).copied().unwrap_or(0);
        let c = &s.cfg;
        let v = match off {
            CLOCK_CONTROL => stored | if c.never_idle { 0 } else { IDLE_3D_2D },
            IDLE_STATE => {
                if c.never_idle {
                    IDLE_STUCK
                } else {
                    IDLE_ALL
                }
            }
            INTR_ACK => {
                let v = s.irq;
                s.irq = 0;
                v
            }
            CHIP_IDENTITY => c.identity,
            CHIP_MODEL => c.model,
            CHIP_REV => c.revision,
            CHIP_CUSTOMER_ID => c.customer_id,
            CHIP_PRODUCT_ID => c.product_id,
            CHIP_ECO_ID => c.eco_id,
            _ => stored,
        };
        s.log.push(Event::GpuR(off, v));
        v
    }

    fn gpu_write(s: &mut State, off: u32, val: u32) {
        s.log.push(Event::GpuW(off, val));
        if off >= GPU_WINDOW {
            s.log.push(Event::Fault(Fault::Window { off, write: true }));
            return;
        }
        if let Some(why) = Self::gpu_off(s) {
            s.log.push(Event::Fault(Fault::Gpu { off, write: true, why }));
            return;
        }
        match off {
            // Read-only, or read-to-clear.
            IDLE_STATE | INTR_ACK | CHIP_IDENTITY | CHIP_MODEL | CHIP_REV | CHIP_CUSTOMER_ID | CHIP_PRODUCT_ID
            | CHIP_ECO_ID => {}
            CLOCK_CONTROL => {
                s.regs.insert(off, val & !IDLE_3D_2D);
            }
            _ => {
                s.regs.insert(off, val);
            }
        }
    }

    fn rcc_write(s: &mut State, off: u32, val: u32) {
        s.log.push(Event::RccW(off, val));
        if off != GPUCFGR {
            s.log.push(Event::Fault(Fault::RccOther { off, write: true }));
            return;
        }
        let starts = val & GPU_EN != 0 && s.rcc & GPU_EN == 0;
        let releases = val & GPU_RST == 0 && s.rcc & GPU_RST != 0;
        if !s.domain_on && (starts || releases) {
            s.log.push(Event::Fault(Fault::RccBeforeDomain { val }));
        }
        if releases && val & GPU_EN == 0 {
            s.log.push(Event::Fault(Fault::ReleaseWithoutClock { val }));
        }
        s.rcc = val;
    }

    fn decode(addr: usize) -> (bool, u32) {
        assert!(addr.is_multiple_of(4), "unaligned access at {addr:#x}");
        if (RCC_BASE..RCC_BASE + RCC_SIZE).contains(&addr) {
            (true, (addr - RCC_BASE) as u32)
        } else if (GPU_BASE..GPU_BASE + GPU_DECODE).contains(&addr) {
            (false, (addr - GPU_BASE) as u32)
        } else {
            panic!("access outside every modelled block: {addr:#x}");
        }
    }

    /// One SCMI message: the reply bytes (header echoed, status, values).
    fn serve(s: &mut State, tx: &[u8]) -> Vec<u8> {
        let hdr = scmi::word(tx, 0);
        let msg = (hdr & 0xff) as u8;
        let kind = (hdr >> 8) & 0x3;
        let proto = ((hdr >> 10) & 0xff) as u8;
        let args: Vec<u32> = (1..tx.len() / 4).map(|i| scmi::word(tx, i)).collect();
        let (status, values) = Self::answer(s, kind, proto, msg, &args, tx.len() % 4 != 0);
        let mut rx = Vec::with_capacity(8 + 4 * values.len());
        rx.extend_from_slice(&hdr.to_le_bytes());
        rx.extend_from_slice(&status.to_le_bytes());
        if status == scmi::STATUS_SUCCESS {
            for v in values {
                rx.extend_from_slice(&v.to_le_bytes());
            }
        }
        rx
    }

    fn answer(s: &mut State, kind: u32, proto: u8, msg: u8, a: &[u32], ragged: bool) -> (i32, Vec<u32>) {
        if proto != P_POWER && proto != P_PERF {
            s.log.push(Event::Fault(Fault::Protocol { proto, msg }));
            return (scmi::STATUS_NOT_SUPPORTED, vec![]);
        }
        if let Some(f) = s.forced.iter().find(|f| (f.0, f.1) == (proto, msg)) {
            return (f.2, vec![]);
        }
        // The payload length each message takes (words).
        let want = match (proto, msg) {
            (_, M_VERSION) | (_, M_ATTRIBUTES) => 0,
            (_, M_DOMAIN_ATTRIBUTES) => 1,
            (P_POWER, M_PD_STATE_SET) => 3,
            (P_POWER, M_PD_STATE_GET) => 1,
            (P_PERF, M_PERF_LEVEL_SET) => 2,
            (P_PERF, M_PERF_LEVEL_GET) => 1,
            _ => return (scmi::STATUS_NOT_SUPPORTED, vec![]),
        };
        if kind != 0 || ragged || a.len() != want {
            return (scmi::STATUS_PROTOCOL_ERROR, vec![]);
        }
        let ver = |i: usize| s.cfg.versions[i];
        match (proto, msg) {
            (P_POWER, M_VERSION) => (0, vec![ver(0)]),
            (P_PERF, M_VERSION) => (0, vec![ver(1)]),
            // Domain count; the statistics area (address low, high, length) 0.
            (P_POWER, M_ATTRIBUTES) => (0, vec![1, 0, 0, 0]),
            // Power in mW (bit 16), two domains.
            (P_POWER, M_DOMAIN_ATTRIBUTES) => {
                if a[0] != GPU_PD {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                let mut v = vec![PD_ATTR_SET_SYNC];
                v.extend(name_words(s.cfg.pd_name));
                (0, v)
            }
            (P_PERF, M_ATTRIBUTES) => (0, vec![PERF_ATTR_POWER_MW | 2, 0, 0, 0]),
            (P_PERF, M_DOMAIN_ATTRIBUTES) => {
                let d = a[0];
                if d > GPU_PERF {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                if d == CPU_PERF {
                    s.log.push(Event::Fault(Fault::CpuDomain { msg }));
                }
                // Set-level allowed; rate limit 0; the sustained frequency
                // (kHz, the level's value here) and level; the name.
                let lvl = s.levels[d as usize];
                let mut v = vec![PERF_ATTR_CAN_SET_LEVEL, 0, lvl, lvl];
                v.extend(name_words(s.cfg.perf_names[d as usize]));
                (0, v)
            }
            (P_POWER, M_PD_STATE_SET) => {
                let (flags, d, state) = (a[0], a[1], a[2]);
                if flags & !PD_FLAG_ASYNC != 0 || state & !PD_STATE_MASK != 0 {
                    return (scmi::STATUS_INVALID_PARAMETERS, vec![]);
                }
                if d != GPU_PD {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                match state {
                    PD_ON => s.domain_on = true,
                    PD_OFF => {
                        if s.rcc & GPU_EN != 0 {
                            s.log.push(Event::Fault(Fault::DomainOffClockOn));
                        }
                        s.domain_on = false;
                    }
                    // Another device state id: the model's choice (SCP-firmware
                    // hands it to the power domain module, not read).
                    _ => return (scmi::STATUS_INVALID_PARAMETERS, vec![]),
                }
                (0, vec![])
            }
            (P_POWER, M_PD_STATE_GET) => {
                if a[0] != GPU_PD {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                (0, vec![if s.domain_on { PD_ON } else { PD_OFF }])
            }
            (P_PERF, M_PERF_LEVEL_SET) => {
                let (d, lvl) = (a[0], a[1]);
                if d > GPU_PERF {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                if d == CPU_PERF {
                    s.log.push(Event::Fault(Fault::CpuDomain { msg }));
                    s.levels[0] = lvl;
                    return (0, vec![]);
                }
                if !s.cfg.gpu_levels.contains(&lvl) {
                    return (scmi::STATUS_OUT_OF_RANGE, vec![]);
                }
                if s.domain_on {
                    // Fire-and-forget SUCCESS; the change is refused inside
                    // OP-TEE and the level stays (the module text).
                    s.log.push(Event::Fault(Fault::LevelWhileOn { level: lvl }));
                    return (0, vec![]);
                }
                s.levels[1] = lvl;
                (0, vec![])
            }
            (P_PERF, M_PERF_LEVEL_GET) => {
                let d = a[0];
                if d > GPU_PERF {
                    return (scmi::STATUS_NOT_FOUND, vec![]);
                }
                if d == CPU_PERF {
                    s.log.push(Event::Fault(Fault::CpuDomain { msg }));
                }
                (0, vec![s.levels[d as usize]])
            }
            _ => unreachable!(),
        }
    }
}

/// A 16-byte NUL-padded name as four little-endian words.
fn name_words(name: &str) -> [u32; 4] {
    let n = name.as_bytes();
    assert!(n.len() < 16, "name {name:?} does not fit 16 bytes with its NUL");
    let mut b = [0u8; 16];
    b[..n.len()].copy_from_slice(n);
    core::array::from_fn(|i| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]))
}

impl scmi::Transport for Mp2Scmi<'_> {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        let mut s = self.model.0.borrow_mut();
        if let Some(code) = s.transport_error {
            s.log.push(Event::Scmi { tx: tx.to_vec(), rx: vec![] });
            return Err(code);
        }
        if tx.len() < 4 {
            s.log.push(Event::Scmi { tx: tx.to_vec(), rx: vec![] });
            return Err(-crate::errno::EINVAL);
        }
        let reply = Mp2Model::serve(&mut s, tx);
        s.log.push(Event::Scmi { tx: tx.to_vec(), rx: reply.clone() });
        if reply.len() > rx.len() {
            return Err(-crate::errno::EMSGSIZE);
        }
        rx[..reply.len()].copy_from_slice(&reply);
        Ok(reply.len())
    }
}

impl Mmio for Mp2Model {
    fn read32(&self, addr: usize) -> u32 {
        let (rcc, off) = Self::decode(addr);
        let mut s = self.0.borrow_mut();
        if !rcc {
            return Self::gpu_read(&mut s, off);
        }
        if off != GPUCFGR {
            s.log.push(Event::RccR(off, 0));
            s.log.push(Event::Fault(Fault::RccOther { off, write: false }));
            return 0;
        }
        let v = s.rcc;
        s.log.push(Event::RccR(off, v));
        v
    }

    fn write32(&self, addr: usize, val: u32) {
        let (rcc, off) = Self::decode(addr);
        let mut s = self.0.borrow_mut();
        if rcc {
            Self::rcc_write(&mut s, off, val)
        } else {
            Self::gpu_write(&mut s, off, val)
        }
    }

    fn read8(&self, addr: usize) -> u8 {
        panic!("8-bit read at {addr:#x} of a 32-bit block");
    }

    fn write8(&self, addr: usize, _val: u8) {
        panic!("8-bit write at {addr:#x} of a 32-bit block");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::scmi::{Client, Transport};
    use crate::gpu::vivante::hw;
    use crate::model::TestEnv;
    use crate::soc::stm32mp2::power::{self, Config, Error, Log, Step};
    use crate::soc::stm32mp2::rcc::GpuRcc;

    const VECTORS: &str = include_str!("../../../tools/testdata/optee-scmi/scmi_msgs.txt");

    fn vector(name: &str) -> Vec<u8> {
        let hex = VECTORS
            .lines()
            .find_map(|l| l.strip_prefix(name).and_then(|r| r.strip_prefix(' ')))
            .unwrap_or_else(|| panic!("vector {name}"));
        (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// Send the named request, compare the reply with the named reply.
    fn exchange(m: &Mp2Model, req: &str, rep: &str) {
        let tx = vector(req);
        let mut rx = [0u8; scmi::MAX_MSG];
        let n = m.scmi().transfer(&tx, &mut rx).expect(req);
        assert_eq!(rx[..n], vector(rep)[..], "{req}");
    }

    fn raw(m: &Mp2Model, words: &[u32]) -> Vec<u8> {
        let tx: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut rx = [0u8; scmi::MAX_MSG];
        let n = m.scmi().transfer(&tx, &mut rx).unwrap();
        rx[..n].to_vec()
    }

    fn words(b: &[u8]) -> Vec<u32> {
        (0..b.len() / 4).map(|i| scmi::word(b, i)).collect()
    }

    /// A model with the GPU on: domain 0 on, clock running, reset released.
    fn powered() -> Mp2Model {
        Mp2Model::with_config(Mp2Config { domain_on: true, rcc_gpucfgr: 0b10, ..Default::default() })
    }

    const HDR_PD_ATTR: u32 = scmi::header(0x11, 3, 1);
    const HDR_PERF_ATTR: u32 = scmi::header(0x13, 3, 1);

    #[test]
    fn mp2_model_reset_values() {
        let m = Mp2Model::new();
        assert_eq!(m.read32(RCC_BASE + 0x83c), 0);
        assert!(!m.domain_on());
        assert_eq!(m.gpu_level(), 800_000);
        assert_eq!(m.intr_enbl(), 0);
        assert_eq!(m.events(), vec![Event::RccR(0x83c, 0)]);

        // The GPU's registers, once it is on: the identity #1281 read, idle,
        // the idle bits of CLOCK_CONTROL, nothing pending, nothing enabled.
        let m = powered();
        for (off, want) in [
            (0x018, 0),
            (0x020, 0x8000),
            (0x024, 0x6205),
            (0x030, 0),
            (0x0a8, 0x8_0003),
            (0x0e8, 0),
            (0x004, 0x7fff_ffff),
            (0x000, 0x0003_0000),
            (0x010, 0),
            (0x014, 0),
            (0x7fc, 0),
        ] {
            assert_eq!(m.read32(GPU_BASE + off), want, "offset {off:#x}");
        }
        assert!(m.faults().is_empty());
        assert_eq!(m.events().len(), 11);
    }

    #[test]
    fn mp2_model_register_storage_and_irq() {
        let m = powered();
        // INTR_ENBL and an unmodelled offset store; identity is read-only;
        // CLOCK_CONTROL keeps what was written and adds the idle bits.
        m.write32(GPU_BASE + 0x14, 0xffff_ffff);
        m.write32(GPU_BASE + 0x7fc, 0x1234);
        m.write32(GPU_BASE + 0x20, 0x7000);
        m.write32(GPU_BASE, 0x0003_0900); // CLOCK_CONTROL, offset 0
        assert_eq!(m.read32(GPU_BASE + 0x14), 0xffff_ffff);
        assert_eq!(m.intr_enbl(), 0xffff_ffff);
        assert_eq!(m.read32(GPU_BASE + 0x7fc), 0x1234);
        assert_eq!(m.read32(GPU_BASE + 0x20), 0x8000);
        assert_eq!(m.read32(GPU_BASE), 0x0003_0900);
        // INTR_ACKNOWLEDGE: pending bits read once, then clear.
        m.raise_irq(1 << 3);
        assert_eq!(m.read32(GPU_BASE + 0x10), 1 << 3);
        assert_eq!(m.read32(GPU_BASE + 0x10), 0);
        assert!(m.faults().is_empty());

        let m = Mp2Model::with_config(Mp2Config {
            never_idle: true,
            domain_on: true,
            rcc_gpucfgr: 2,
            ..Default::default()
        });
        assert_eq!(m.read32(GPU_BASE + 0x04), 0x7fff_fff2);
        assert_eq!(m.read32(GPU_BASE), 0, "no idle bits in CLOCK_CONTROL");
    }

    #[test]
    fn mp2_model_gpu_faults_each_condition() {
        let m = Mp2Model::new();
        assert_eq!(m.read32(GPU_BASE + 0x20), POISON);
        m.write32(GPU_BASE + 0x14, 1);
        assert_eq!(
            m.faults(),
            vec![
                Fault::Gpu { off: 0x20, write: false, why: Off::Domain },
                Fault::Gpu { off: 0x14, write: true, why: Off::Domain },
            ]
        );
        assert_eq!(m.intr_enbl(), 0, "a faulting write is dropped");
        assert!(hw::reg_absent(POISON), "the poison is what identify refuses");

        let m = Mp2Model::with_config(Mp2Config { domain_on: true, ..Default::default() });
        assert_eq!(m.read32(GPU_BASE + 0x4), POISON);
        m.write32(RCC_BASE + 0x83c, 0b11);
        assert_eq!(m.read32(GPU_BASE + 0x4), POISON);
        m.write32(RCC_BASE + 0x83c, 0b10);
        assert_eq!(m.read32(GPU_BASE + 0x4), 0x7fff_ffff);
        assert_eq!(m.read32(GPU_BASE + 0x800), POISON);
        m.write32(GPU_BASE + 0xffc, 0);
        assert_eq!(
            m.faults(),
            vec![
                Fault::Gpu { off: 4, write: false, why: Off::Clock },
                Fault::Gpu { off: 4, write: false, why: Off::Reset },
                Fault::Window { off: 0x800, write: false },
                Fault::Window { off: 0xffc, write: true },
            ]
        );
        // The log carries the poison the read answered.
        assert!(m.events().contains(&Event::GpuR(0x800, POISON)));
    }

    #[test]
    fn mp2_model_rcc_faults() {
        let m = Mp2Model::new();
        m.write32(RCC_BASE + 0x83c, 0b01); // reset held, gated: allowed
        m.write32(RCC_BASE + 0x83c, 0b11); // clock before the domain
        m.write32(RCC_BASE + 0x83c, 0b10); // reset released before the domain
        m.write32(RCC_BASE + 0x83c, 0b00); // gated: allowed
        assert_eq!(m.read32(RCC_BASE + 0x28c), 0);
        m.write32(RCC_BASE + 0x290, 1);
        assert_eq!(
            m.faults(),
            vec![
                Fault::RccBeforeDomain { val: 0b11 },
                Fault::RccBeforeDomain { val: 0b10 },
                Fault::RccOther { off: 0x28c, write: false },
                Fault::RccOther { off: 0x290, write: true },
            ]
        );
        assert_eq!(m.rcc(), 0, "the register still took every write");

        // Reset released with the clock gated, the domain on.
        let m = Mp2Model::with_config(Mp2Config { domain_on: true, rcc_gpucfgr: 0b01, ..Default::default() });
        m.write32(RCC_BASE + 0x83c, 0b00);
        assert_eq!(m.faults(), vec![Fault::ReleaseWithoutClock { val: 0 }]);

        // Domain off while the clock runs.
        let m = powered();
        exchange(&m, "req_pd_state_set_off", "rep_pd_state_set_off");
        assert_eq!(m.faults(), vec![Fault::DomainOffClockOn]);
        assert!(!m.domain_on());
    }

    #[test]
    fn mp2_model_scmi_vectors_round_trip() {
        let m = Mp2Model::new();
        for v in [
            "pd_protocol_version",
            "pd_protocol_attributes",
            "perf_protocol_version",
            "perf_protocol_attributes",
            "perf_level_get",
            "perf_level_set",
            "perf_level_set_unknown_domain",
            "perf_level_set_out_of_range",
            "perf_level_get_unknown_domain",
            "pd_state_get_off",
            "pd_state_set_async_flag_rejected",
            "pd_state_set_unknown_domain",
            "pd_state_get_unknown_domain",
            "pd_state_set_on",
            "pd_state_get_on",
            "pd_state_set_off",
        ] {
            exchange(&m, &format!("req_{v}"), &format!("rep_{v}"));
        }
        assert!(m.faults().is_empty());
        let msgs = m.messages();
        assert_eq!(msgs.len(), 16);
        assert_eq!(msgs[0], (vector("req_pd_protocol_version"), vector("rep_pd_protocol_version")));
        assert_eq!(msgs[15], (vector("req_pd_state_set_off"), vector("rep_pd_state_set_off")));
    }

    #[test]
    fn mp2_model_scmi_domain_attributes() {
        let m = Mp2Model::new();
        let gpu_pd = words(&raw(&m, &[HDR_PD_ATTR, 0]));
        assert_eq!(
            gpu_pd,
            vec![HDR_PD_ATTR, 0, 1 << 29, u32::from_le_bytes(*b"gpu-"), u32::from_le_bytes(*b"pd\0\0"), 0, 0]
        );
        let r = words(&raw(&m, &[HDR_PD_ATTR, 1]));
        assert_eq!(r, vec![HDR_PD_ATTR, scmi::STATUS_NOT_FOUND as u32]);
        let r = words(&raw(&m, &[HDR_PERF_ATTR, 1]));
        assert_eq!(r, vec![HDR_PERF_ATTR, 0, 1 << 30, 0, 800_000, 800_000, u32::from_le_bytes(*b"gpu\0"), 0, 0, 0]);
        let r = words(&raw(&m, &[HDR_PERF_ATTR, 2]));
        assert_eq!(r, vec![HDR_PERF_ATTR, scmi::STATUS_NOT_FOUND as u32]);
        // A short payload is a protocol error; a response-type header too.
        let r = words(&raw(&m, &[HDR_PERF_ATTR]));
        assert_eq!(r, vec![HDR_PERF_ATTR, scmi::STATUS_PROTOCOL_ERROR as u32]);
        let r = words(&raw(&m, &[HDR_PERF_ATTR | (2 << 8), 1]));
        assert_eq!(r, vec![HDR_PERF_ATTR | (2 << 8), scmi::STATUS_PROTOCOL_ERROR as u32]);
        assert!(m.faults().is_empty());
        // The CPU's domain is answered, and a fault.
        raw(&m, &[HDR_PERF_ATTR, 0]);
        assert_eq!(m.faults(), vec![Fault::CpuDomain { msg: 3 }]);
    }

    #[test]
    fn mp2_model_scmi_client_and_statuses() {
        let m = Mp2Model::new();
        let mut c = Client::new(m.scmi());
        assert_eq!(c.version(0x11), Ok(0x2_0000));
        assert_eq!(c.version(0x13), Ok(0x2_0000));
        assert_eq!(c.perf_level_get(1), Ok(800_000));
        assert_eq!(c.perf_level_set(1, 800_000), Ok(()));
        assert_eq!(c.power_get(0), Ok(scmi::POWER_OFF));
        assert_eq!(c.power_set(0, scmi::POWER_ON), Ok(()));
        assert_eq!(c.power_get(0), Ok(scmi::POWER_ON));
        assert!(m.faults().is_empty());
        // Scripted statuses: the reply is status-only and nothing changes.
        m.force_status(0x11, 4, scmi::STATUS_DENIED);
        assert_eq!(c.power_set(0, scmi::POWER_OFF), Err(scmi::Error::Status(scmi::STATUS_DENIED)));
        assert!(m.domain_on(), "a refused request changes nothing");
        m.force_status(0x13, 8, scmi::STATUS_NOT_FOUND);
        assert_eq!(c.perf_level_get(1), Err(scmi::Error::Status(scmi::STATUS_NOT_FOUND)));
        m.force_status(0x13, 8, scmi::STATUS_SUCCESS);
        assert_eq!(c.perf_level_get(1), Ok(800_000));
        m.fail_transport(Some(-5));
        assert_eq!(c.version(0x11), Err(scmi::Error::Transport(-5)));
        m.fail_transport(None);
        // Another protocol: NOT_SUPPORTED, and a fault.
        assert_eq!(c.clock_rate_get(0), Err(scmi::Error::Status(scmi::STATUS_NOT_SUPPORTED)));
        // An unknown message of a known protocol: NOT_SUPPORTED, no fault.
        assert_eq!(c.call(0x11, 0x9, &[0], 0).err(), Some(scmi::Error::Status(scmi::STATUS_NOT_SUPPORTED)));
        assert_eq!(m.faults(), vec![Fault::Protocol { proto: 0x14, msg: 6 }]);
        // Every message is recorded byte for byte, with its token.
        let msgs = m.messages();
        assert_eq!(msgs.len(), 13);
        // LEVEL_GET of domain 1 with token 2, and its reply.
        assert_eq!(msgs[2].0, [8, 0x4c, 0x08, 0, 1, 0, 0, 0]);
        assert_eq!(msgs[2].1, [8, 0x4c, 0x08, 0, 0, 0, 0, 0, 0x00, 0x35, 0x0c, 0]);
        // The transport failure (token 10): the request, an empty reply.
        assert_eq!(msgs[10].0, [0, 0x44, 0x28, 0]);
        assert!(msgs[10].1.is_empty());
    }

    #[test]
    fn mp2_model_level_set_while_on_is_fire_and_forget() {
        let m = Mp2Model::with_config(Mp2Config { gpu_levels: vec![400_000, 800_000], ..Default::default() });
        let mut c = Client::new(m.scmi());
        c.power_set(0, scmi::POWER_ON).unwrap();
        // SUCCESS on the wire, the level unchanged, a fault recorded.
        assert_eq!(c.perf_level_set(1, 400_000), Ok(()));
        assert_eq!(c.perf_level_get(1), Ok(800_000));
        assert_eq!(m.faults(), vec![Fault::LevelWhileOn { level: 400_000 }]);
        // A level outside the table is refused before that.
        assert_eq!(c.perf_level_set(1, 1), Err(scmi::Error::Status(scmi::STATUS_OUT_OF_RANGE)));
        // Scripted as a status for a client that expects one.
        m.force_status(0x13, 7, scmi::STATUS_DENIED);
        assert_eq!(c.perf_level_set(1, 400_000), Err(scmi::Error::Status(scmi::STATUS_DENIED)));
        assert_eq!(m.faults().len(), 1);
        // Off again: the change goes through.
        c.power_set(0, scmi::POWER_OFF).unwrap();
        m.force_status(0x13, 7, scmi::STATUS_SUCCESS);
        assert_eq!(c.perf_level_set(1, 400_000), Ok(()));
        assert_eq!(m.gpu_level(), 400_000);
        assert_eq!(m.faults().len(), 1);
    }

    #[test]
    fn mp2_model_power_state_set_flags_and_states() {
        let m = Mp2Model::new();
        let hdr = scmi::header(0x11, 4, 1);
        // Asynchronous (flag bit 0) is taken for a device domain.
        assert_eq!(words(&raw(&m, &[hdr, 1, 0, 0])), vec![hdr, 0]);
        assert!(m.domain_on());
        // A state bit outside the mask, and an unknown device state id.
        assert_eq!(words(&raw(&m, &[hdr, 0, 0, 0x8000_0000])), vec![hdr, scmi::STATUS_INVALID_PARAMETERS as u32]);
        assert_eq!(words(&raw(&m, &[hdr, 0, 0, 1])), vec![hdr, scmi::STATUS_INVALID_PARAMETERS as u32]);
        assert!(m.domain_on());
        assert!(m.faults().is_empty());
    }

    struct Lines(Vec<String>);
    impl Log for Lines {
        fn line(&mut self, s: &str) {
            self.0.push(s.to_string());
        }
    }

    /// galcore's order (r-galcore; note section 4): the level, power domain
    /// 0, the RCC gate and reset, then the GPU; it ends with no fault and
    /// the identity #1281 read under Linux. Then the reverse.
    #[test]
    fn mp2_model_power_up_order_and_identity() {
        let m = Mp2Model::new();
        let rcc = GpuRcc::new(&m, RCC_BASE);
        let mut log = Lines(Vec::new());
        let up = power::power_up(&mut m.scmi(), &rcc, |off| m.read32(GPU_BASE + off), &mut log, &Config::DK)
            .expect("power up");
        assert_eq!(
            (up.level_before, up.level, up.rcc, up.idle, up.idle_reads),
            (800_000, 800_000, 0b10, 0x7fff_ffff, 1)
        );
        assert!(m.domain_on());
        let env = TestEnv::default();
        let id = hw::identify(&m, GPU_BASE, &env).expect("identify");
        assert_eq!((id.model, id.revision, id.product_id, id.eco_id), (0x8000, 0x6205, 0x8_0003, 0));
        assert!(id.from_table, "the ST row of the database");
        m.write32(GPU_BASE + 0x14, 0xffff_ffff);
        assert_eq!(m.read32(GPU_BASE + 0x14), 0xffff_ffff);
        assert!(m.faults().is_empty(), "{:?}", m.faults());

        // The order on the wire: perf version, get, set, get; power version,
        // set on, get; then the RCC, then the GPU.
        let msgs = m.messages();
        let ids: Vec<(u32, u32)> =
            msgs.iter().map(|(tx, _)| ((scmi::word(tx, 0) >> 10) & 0xff, scmi::word(tx, 0) & 0xff)).collect();
        assert_eq!(ids, vec![(0x13, 0), (0x13, 8), (0x13, 7), (0x13, 8), (0x11, 0), (0x11, 4), (0x11, 5)]);
        let ev = m.events();
        let last_scmi = ev.iter().rposition(|e| matches!(e, Event::Scmi { .. })).unwrap();
        let first_rcc = ev.iter().position(|e| matches!(e, Event::RccW(..))).unwrap();
        let first_gpu = ev.iter().position(|e| matches!(e, Event::GpuR(..) | Event::GpuW(..))).unwrap();
        assert!(last_scmi < first_rcc && first_rcc < first_gpu);
        assert_eq!(
            ev[first_rcc..first_rcc + 7],
            [
                Event::RccW(0x83c, 0b10),
                Event::RccR(0x83c, 0b10),
                Event::RccW(0x83c, 0b11),
                Event::RccR(0x83c, 0b11),
                Event::RccW(0x83c, 0b10),
                Event::RccR(0x83c, 0b10),
                Event::GpuR(0x004, 0x7fff_ffff),
            ]
        );

        power::power_down(&mut m.scmi(), &rcc, &mut log, &Config::DK).expect("power down");
        assert!(!m.domain_on());
        assert_eq!(m.rcc(), 0b01);
        assert!(m.faults().is_empty(), "{:?}", m.faults());
    }

    /// The driver's read-back catches the silent refusal of a level change
    /// with the domain on, before any RCC write.
    #[test]
    fn mp2_model_power_up_with_domain_on_stops_at_the_level() {
        let m = Mp2Model::with_config(Mp2Config {
            domain_on: true,
            gpu_levels: vec![400_000, 800_000],
            ..Default::default()
        });
        let rcc = GpuRcc::new(&m, RCC_BASE);
        let mut log = Lines(Vec::new());
        let cfg = Config { level: power::Level::Fixed(400_000), ..Config::DK };
        let e = power::power_up(&mut m.scmi(), &rcc, |_| panic!("no GPU read"), &mut log, &cfg);
        assert_eq!(e, Err(Error::Readback { step: Step::PerfLevelCheck, want: 400_000, got: 800_000 }));
        assert_eq!(m.faults(), vec![Fault::LevelWhileOn { level: 400_000 }]);
        assert!(!m.events().iter().any(|e| matches!(e, Event::RccW(..))));
    }

    /// The wrong order: the clock before the domain, and an identity read
    /// with the domain off, are both caught.
    #[test]
    fn mp2_model_wrong_order_faults() {
        let m = Mp2Model::new();
        let rcc = GpuRcc::new(&m, RCC_BASE);
        rcc.enable_clock();
        let env = TestEnv::default();
        assert!(hw::identify(&m, GPU_BASE, &env).is_err());
        let f = m.faults();
        assert_eq!(f[0], Fault::RccBeforeDomain { val: 0b10 });
        assert_eq!(f[1], Fault::Gpu { off: 0x18, write: false, why: Off::Domain });
    }
}
