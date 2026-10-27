// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The device, its VM, buffers, a group with one queue, a tiler heap, and
//! submit/wait, over panthor's uAPI.
//!
//! The ioctl sequence is Mesa 26.2.1's (MIT): device queries
//! (`panthor_kmod_dev_create`, src/panfrost/lib/kmod/panthor_kmod.c:214-332),
//! VM create (panthor_kmod.c:810-863), BO create and mmap (panthor_kmod.c:385-439,
//! 559-571), synchronous VM bind (panthor_kmod.c:1142-1191), group and tiler
//! heap create (`csf_init_context`, src/gallium/drivers/panfrost/pan_csf.c:1816-1864)
//! and group submit with a syncobj signal operation (`csf_submit_batch`,
//! pan_csf.c:896-950). Nothing here guesses a GPU address: a VA is used only
//! after the bind that maps it returned 0, and every address printed is one
//! the kernel returned or one this process bound.

use crate::uapi::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fmt;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

/// What went wrong, and in which call.
#[derive(Debug)]
pub enum Error {
    /// An ioctl or system call failed. `what` names it and its arguments.
    Os { what: String, err: io::Error },
    /// A request this crate refuses before the kernel sees it.
    Refused(String),
    /// A command stream instruction that did not encode.
    Cs(crate::cs::CsError),
    /// The group reports a timeout or a fatal fault after a wait.
    GroupFault { state: u32, fatal_queues: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Os { what, err } => write!(f, "{what}: {err}"),
            Error::Refused(s) => write!(f, "refused: {s}"),
            Error::Cs(e) => write!(f, "command stream: {e}"),
            Error::GroupFault { state, fatal_queues } => {
                // DRM_IOCTL_PANTHOR_GROUP_GET_STATE returns only these two
                // fields (panthor_drm.h:924): the fault class (a tiler OOM
                // or another) is not in the uAPI, so it is not guessed here;
                // the kernel log of the run names it.
                write!(
                    f,
                    "group state flags {state:#x} ({}), fatal_queues bitmask {fatal_queues:#x}; the uAPI gives no fault class (OOM or other): read the kernel log",
                    group_state_names(*state)
                )
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::cs::CsError> for Error {
    fn from(e: crate::cs::CsError) -> Self {
        Error::Cs(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn group_state_names(state: u32) -> String {
    let mut v = Vec::new();
    if state & DRM_PANTHOR_GROUP_STATE_TIMEDOUT != 0 {
        v.push("TIMEDOUT");
    }
    if state & DRM_PANTHOR_GROUP_STATE_FATAL_FAULT != 0 {
        v.push("FATAL_FAULT");
    }
    if state & DRM_PANTHOR_GROUP_STATE_INNOCENT != 0 {
        v.push("INNOCENT");
    }
    if v.is_empty() {
        "usable".into()
    } else {
        v.join("|")
    }
}

/// `ioctl(fd, req, arg)`, retried on EINTR and EAGAIN as libdrm's
/// `drmIoctl` does.
fn ioctl<T>(fd: i32, req: u32, arg: &mut T, what: impl FnOnce() -> String) -> Result<()> {
    loop {
        // SAFETY: `arg` is a live, exclusively borrowed #[repr(C)] struct of
        // the size `req` encodes (pinned by uapi::tests); pointers inside it
        // point at buffers the caller keeps alive for the call.
        let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
        if r == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) | Some(libc::EAGAIN) => continue,
            _ => return Err(Error::Os { what: what(), err }),
        }
    }
}

/// The DRM driver's name and version (`DRM_IOCTL_VERSION`).
#[derive(Debug, Clone)]
pub struct DriverVersion {
    pub name: String,
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
    pub date: String,
    pub desc: String,
}

impl DriverVersion {
    fn at_least(&self, major: i32, minor: i32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

/// An open panthor render node and what it reports about the GPU.
pub struct Device {
    fd: OwnedFd,
    pub path: String,
    pub driver: DriverVersion,
    pub gpu: DrmPanthorGpuInfo,
    pub csif: DrmPanthorCsifInfo,
    /// `None` below panthor 1.1, where the query does not exist
    /// (panthor_kmod.c:265-277): absent, not zero.
    pub timestamp: Option<DrmPanthorTimestampInfo>,
    /// `None` below panthor 1.2 (panthor_kmod.c:300-318).
    pub priorities: Option<DrmPanthorGroupPrioritiesInfo>,
}

impl Device {
    /// Open `path` (e.g. /dev/dri/renderD128), check that the driver is
    /// panthor and read its GPU, CSIF, timestamp and priority queries.
    pub fn open(path: &str) -> Result<Device> {
        let c = CString::new(path).map_err(|_| Error::Refused(format!("path {path:?} holds a NUL")))?;
        // SAFETY: `c` is a NUL-terminated string that outlives the call.
        let raw = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(Error::Os { what: format!("open {path}"), err: io::Error::last_os_error() });
        }
        // SAFETY: `raw` is a file descriptor this process just opened and
        // owns alone.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let driver = Self::version(fd.as_raw_fd())?;
        if driver.name != "panthor" {
            return Err(Error::Refused(format!("{path} is driven by {:?}, not panthor", driver.name)));
        }
        let mut dev = Device {
            fd,
            path: path.into(),
            driver,
            gpu: Default::default(),
            csif: Default::default(),
            timestamp: None,
            priorities: None,
        };
        let mut gpu = DrmPanthorGpuInfo::default();
        dev.query(DRM_PANTHOR_DEV_QUERY_GPU_INFO, "GPU_INFO", &mut gpu)?;
        dev.gpu = gpu;
        let mut csif = DrmPanthorCsifInfo::default();
        dev.query(DRM_PANTHOR_DEV_QUERY_CSIF_INFO, "CSIF_INFO", &mut csif)?;
        dev.csif = csif;
        if dev.driver.at_least(1, 1) {
            let mut t = DrmPanthorTimestampInfo::default();
            dev.query(DRM_PANTHOR_DEV_QUERY_TIMESTAMP_INFO, "TIMESTAMP_INFO", &mut t)?;
            dev.timestamp = Some(t);
        }
        if dev.driver.at_least(1, 2) {
            let mut p = DrmPanthorGroupPrioritiesInfo::default();
            dev.query(DRM_PANTHOR_DEV_QUERY_GROUP_PRIORITIES_INFO, "GROUP_PRIORITIES_INFO", &mut p)?;
            dev.priorities = Some(p);
        }
        Ok(dev)
    }

    fn version(fd: i32) -> Result<DriverVersion> {
        let mut name = [0u8; 64];
        let mut date = [0u8; 64];
        let mut desc = [0u8; 256];
        let mut v = DrmVersion {
            version_major: 0,
            version_minor: 0,
            version_patchlevel: 0,
            name_len: name.len(),
            name: name.as_mut_ptr(),
            date_len: date.len(),
            date: date.as_mut_ptr(),
            desc_len: desc.len(),
            desc: desc.as_mut_ptr(),
        };
        ioctl(fd, DRM_IOCTL_VERSION, &mut v, || "DRM_IOCTL_VERSION".into())?;
        let s = |buf: &[u8], len: usize| String::from_utf8_lossy(&buf[..len.min(buf.len())]).into_owned();
        Ok(DriverVersion {
            name: s(&name, v.name_len),
            major: v.version_major,
            minor: v.version_minor,
            patch: v.version_patchlevel,
            date: s(&date, v.date_len),
            desc: s(&desc, v.desc_len),
        })
    }

    fn query<T>(&self, type_: u32, name: &str, out: &mut T) -> Result<()> {
        let mut q = DrmPanthorDevQuery { type_, size: core::mem::size_of::<T>() as u32, pointer: out as *mut T as u64 };
        self.ioctl(DRM_IOCTL_PANTHOR_DEV_QUERY, &mut q, || format!("DRM_IOCTL_PANTHOR_DEV_QUERY {name}"))
    }

    fn ioctl<T>(&self, req: u32, arg: &mut T, what: impl FnOnce() -> String) -> Result<()> {
        ioctl(self.fd.as_raw_fd(), req, arg, what)
    }

    /// Every number the device reports, one per line.
    pub fn print_info(&self) {
        let d = &self.driver;
        let g = &self.gpu;
        let c = &self.csif;
        println!(
            "device {}: driver {} {}.{}.{} ({}, {})",
            self.path, d.name, d.major, d.minor, d.patch, d.date, d.desc
        );
        println!(
            "gpu_id {:#010x}: arch {}.{} rev {}, product major {}, version {}.{} status {}",
            g.gpu_id,
            g.arch_major(),
            g.arch_minor(),
            g.arch_rev(),
            g.product_major(),
            g.version_major(),
            g.version_minor(),
            g.version_status()
        );
        println!("gpu_rev {:#010x}", g.gpu_rev);
        println!(
            "csf_id {:#010x}: CS hw {}.{} rev {}, MCU {}.{} rev {}",
            g.csf_id,
            g.cshw_major(),
            g.cshw_minor(),
            g.cshw_rev(),
            g.mcu_major(),
            g.mcu_minor(),
            g.mcu_rev()
        );
        println!(
            "l2_features {:#010x} tiler_features {:#010x} mem_features {:#010x} core_features {:#010x}",
            g.l2_features, g.tiler_features, g.mem_features, g.core_features
        );
        println!("mmu_features {:#010x}: VA bits {}", g.mmu_features, g.mmu_va_bits());
        println!(
            "thread_features {:#010x} max_threads {} max_workgroup_size {} max_barrier_size {}",
            g.thread_features, g.max_threads, g.thread_max_workgroup_size, g.thread_max_barrier_size
        );
        println!("coherency_features {:#010x}", g.coherency_features);
        println!(
            "texture_features {:#010x} {:#010x} {:#010x} {:#010x}",
            g.texture_features[0], g.texture_features[1], g.texture_features[2], g.texture_features[3]
        );
        println!(
            "as_present {:#x} shader_present {:#x} ({} cores) l2_present {:#x} tiler_present {:#x}",
            g.as_present,
            g.shader_present,
            g.shader_present.count_ones(),
            g.l2_present,
            g.tiler_present
        );
        println!("gpu_features {:#018x}", g.gpu_features);
        println!(
            "csif: csg_slot_count {} cs_slot_count {} cs_reg_count {} scoreboard_slot_count {} unpreserved_cs_reg_count {}",
            c.csg_slot_count, c.cs_slot_count, c.cs_reg_count, c.scoreboard_slot_count, c.unpreserved_cs_reg_count
        );
        match &self.timestamp {
            Some(t) => println!(
                "timestamp: frequency {} Hz, current {:#x}, offset {:#x}",
                t.timestamp_frequency, t.current_timestamp, t.timestamp_offset
            ),
            None => println!("timestamp: absent (panthor {}.{} < 1.1)", d.major, d.minor),
        }
        match &self.priorities {
            Some(p) => println!("group priorities allowed mask {:#x}", p.allowed_mask),
            None => println!("group priorities: absent (panthor {}.{} < 1.2)", d.major, d.minor),
        }
    }

    /// A binary DRM syncobj, unsignalled.
    pub fn syncobj(&self) -> Result<Syncobj<'_>> {
        let mut c = DrmSyncobjCreate::default();
        self.ioctl(DRM_IOCTL_SYNCOBJ_CREATE, &mut c, || "DRM_IOCTL_SYNCOBJ_CREATE".into())?;
        Ok(Syncobj { dev: self, handle: c.handle })
    }
}

/// A DRM syncobj owned by this process.
pub struct Syncobj<'d> {
    dev: &'d Device,
    pub handle: u32,
}

/// The result of a wait that did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// Signalled; the wait call took this long.
    Signalled(Duration),
    /// Not signalled within the timeout.
    TimedOut,
}

impl Syncobj<'_> {
    /// Wait until the fence attached to the syncobj signals (waiting for the
    /// fence to be attached first, `WAIT_FOR_SUBMIT`).
    pub fn wait(&self, timeout: Duration) -> Result<Waited> {
        let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `now` is a valid timespec to write.
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
            return Err(Error::Os { what: "clock_gettime".into(), err: io::Error::last_os_error() });
        }
        let deadline = i64::try_from(timeout.as_nanos())
            .ok()
            .and_then(|t| (now.tv_sec * 1_000_000_000 + now.tv_nsec).checked_add(t))
            .ok_or_else(|| {
                Error::Refused(format!("wait timeout {timeout:?} overflows the 64-bit nanosecond deadline"))
            })?;
        let handles = [self.handle];
        let mut w = DrmSyncobjWait {
            handles: handles.as_ptr() as u64,
            timeout_nsec: deadline,
            count_handles: 1,
            flags: DRM_SYNCOBJ_WAIT_FLAGS_WAIT_ALL | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT,
            ..Default::default()
        };
        let t0 = Instant::now();
        match self.dev.ioctl(DRM_IOCTL_SYNCOBJ_WAIT, &mut w, || format!("DRM_IOCTL_SYNCOBJ_WAIT {}", self.handle)) {
            Ok(()) => Ok(Waited::Signalled(t0.elapsed())),
            Err(Error::Os { err, .. }) if err.raw_os_error() == Some(libc::ETIME) => Ok(Waited::TimedOut),
            Err(e) => Err(e),
        }
    }
}

impl Drop for Syncobj<'_> {
    fn drop(&mut self) {
        let mut d = DrmSyncobjDestroy { handle: self.handle, pad: 0 };
        if let Err(e) = self.dev.ioctl(DRM_IOCTL_SYNCOBJ_DESTROY, &mut d, || "DRM_IOCTL_SYNCOBJ_DESTROY".into()) {
            eprintln!("syncobj {}: {e}", self.handle);
        }
    }
}

/// The first user VA this crate hands out: VA 0 stays unmapped, so a null
/// descriptor pointer faults instead of reading a buffer.
pub const VA_BASE: u64 = 0x0100_0000;

/// The VA ranges of one VM: a bump allocator that never reuses a range (no
/// free path, so no use-after-free) and the intervals that are bound now.
/// The kernel remaps silently when a VM_BIND map overlaps an existing
/// mapping (Linux v6.18 panthor_mmu.c:2100-2169), so every bind is checked
/// against this set first.
struct VaMap {
    next: u64,
    /// start -> (end, label) of every bound range; none overlap.
    bound: BTreeMap<u64, (u64, String)>,
}

impl VaMap {
    fn new() -> Self {
        VaMap { next: VA_BASE, bound: BTreeMap::new() }
    }

    /// The bound range that overlaps `[va, end)`, if any.
    fn overlap(&self, va: u64, end: u64) -> Option<(u64, u64, &str)> {
        // The ranges do not overlap each other, so the last one that starts
        // below `end` is the only candidate.
        let (&s, (e, l)) = self.bound.range(..end).next_back()?;
        (*e > va).then_some((s, *e, l.as_str()))
    }

    /// Refuse `[va, va+size)` if it overlaps a bound range, naming both.
    fn check_free(&self, label: &str, va: u64, size: u64) -> std::result::Result<(), String> {
        let end = va.checked_add(size).ok_or_else(|| format!("{label}: {va:#x} + {size:#x} overflows"))?;
        match self.overlap(va, end) {
            Some((s, e, l)) => Err(format!("{label} [{va:#x}, {end:#x}) overlaps {l} [{s:#x}, {e:#x}) (a VM_BIND over a mapping would remap it silently)")),
            None => Ok(()),
        }
    }

    /// The next free range of `size` bytes aligned to `align`, below
    /// `limit`, skipping ranges that were bound at a chosen address.
    fn alloc(&mut self, size: u64, align: u64, limit: u64) -> Option<u64> {
        loop {
            let va = self.next.next_multiple_of(align);
            let end = va.checked_add(size).filter(|&e| e <= limit)?;
            if let Some((_, e, _)) = self.overlap(va, end) {
                self.next = e;
                continue;
            }
            self.next = end;
            return Some(va);
        }
    }

    fn claim(&mut self, label: &str, va: u64, size: u64) -> std::result::Result<(), String> {
        self.check_free(label, va, size)?;
        self.bound.insert(va, (va + size, label.to_string()));
        Ok(())
    }

    fn release(&mut self, va: u64) {
        self.bound.remove(&va);
    }
}

/// A GPU address space. User VAs come from a bump allocator that never
/// reuses a range (no free path, so no use-after-free); its high-water mark
/// is printed on drop. A range bound at a chosen address is refused if it
/// overlaps another, and the bump allocator steps over it.
pub struct Vm<'d> {
    pub dev: &'d Device,
    pub id: u32,
    /// The user VA range the kernel chose (VAs [0, range)).
    pub user_va_range: u64,
    va: RefCell<VaMap>,
    binds: Cell<u32>,
    live: Cell<u32>,
    live_bytes_high_water: Cell<u64>,
    live_bytes: Cell<u64>,
}

impl<'d> Vm<'d> {
    /// `DRM_IOCTL_PANTHOR_VM_CREATE` with `user_va_range` 0: the kernel picks
    /// the split and returns it (panthor_drm.h:443-463).
    pub fn create(dev: &'d Device) -> Result<Vm<'d>> {
        let mut c = DrmPanthorVmCreate::default();
        dev.ioctl(DRM_IOCTL_PANTHOR_VM_CREATE, &mut c, || "DRM_IOCTL_PANTHOR_VM_CREATE".into())?;
        let vm = Vm {
            dev,
            id: c.id,
            user_va_range: c.user_va_range,
            va: RefCell::new(VaMap::new()),
            binds: Cell::new(0),
            live: Cell::new(0),
            live_bytes_high_water: Cell::new(0),
            live_bytes: Cell::new(0),
        };
        if c.user_va_range <= VA_BASE {
            return Err(Error::Refused(format!(
                "VM {}: user VA range {:#x} leaves nothing above {VA_BASE:#x}",
                c.id, c.user_va_range
            )));
        }
        Ok(vm)
    }

    /// `DRM_IOCTL_PANTHOR_VM_GET_STATE`: true while usable.
    pub fn usable(&self) -> Result<bool> {
        let mut s = DrmPanthorVmGetState { vm_id: self.id, state: 0 };
        self.dev
            .ioctl(DRM_IOCTL_PANTHOR_VM_GET_STATE, &mut s, || format!("DRM_IOCTL_PANTHOR_VM_GET_STATE {}", self.id))?;
        Ok(s.state == DRM_PANTHOR_VM_STATE_USABLE)
    }

    fn alloc_va(&self, size: u64, align: u64) -> Result<u64> {
        self.va.borrow_mut().alloc(size, align, self.user_va_range).ok_or_else(|| {
            Error::Refused(format!("VM {}: no {size:#x} bytes of user VA left (aligned {align:#x})", self.id))
        })
    }

    fn bind_op(&self, op: DrmPanthorVmBindOp, what: impl FnOnce() -> String) -> Result<()> {
        let ops = [op];
        let mut b = DrmPanthorVmBind { vm_id: self.id, flags: 0, ops: DrmPanthorObjArray::of(&ops) };
        self.dev.ioctl(DRM_IOCTL_PANTHOR_VM_BIND, &mut b, what)
    }
}

impl Drop for Vm<'_> {
    fn drop(&mut self) {
        println!(
            "arena vm {} VA: high-water {:#x} (bump from {VA_BASE:#x}, {:#x} bytes handed out of {:#x}); {} binds, {} live at drop, live bytes high-water {:#x}",
            self.id,
            self.va.borrow().next,
            self.va.borrow().next - VA_BASE,
            self.user_va_range,
            self.binds.get(),
            self.live.get(),
            self.live_bytes_high_water.get()
        );
        let mut d = DrmPanthorVmDestroy { id: self.id, pad: 0 };
        if let Err(e) = self.dev.ioctl(DRM_IOCTL_PANTHOR_VM_DESTROY, &mut d, || "DRM_IOCTL_PANTHOR_VM_DESTROY".into()) {
            eprintln!("vm {}: {e}", self.id);
        }
    }
}

/// How the GPU maps a buffer (`DRM_PANTHOR_VM_BIND_OP_MAP_*`,
/// panthor_drm.h:480-522). Mesa maps a non-executable buffer NOEXEC and an
/// executable one READONLY (panthor_kmod.c:1163-1169).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Map {
    /// Data the GPU reads and writes; not executable.
    Data,
    /// Shader code: read-only, executable.
    Code,
    /// Data, GPU-uncached.
    Uncached,
}

impl Map {
    fn flags(self) -> u32 {
        DRM_PANTHOR_VM_BIND_OP_TYPE_MAP
            | match self {
                Map::Data => DRM_PANTHOR_VM_BIND_OP_MAP_NOEXEC,
                Map::Code => DRM_PANTHOR_VM_BIND_OP_MAP_READONLY,
                Map::Uncached => DRM_PANTHOR_VM_BIND_OP_MAP_NOEXEC | DRM_PANTHOR_VM_BIND_OP_MAP_UNCACHED,
            }
    }
}

/// A buffer object private to one VM, CPU-mapped and bound at a GPU VA.
pub struct Bo<'v> {
    vm: &'v Vm<'v>,
    pub handle: u32,
    pub size: u64,
    cpu: *mut u8,
    va: u64,
    pub label: String,
}

impl<'v> Bo<'v> {
    /// Create `size` bytes (rounded up to pages by the kernel), map them for
    /// the CPU and bind them at the next free VA aligned to `align`.
    pub fn new(vm: &'v Vm<'v>, label: &str, size: u64, map: Map, align: u64) -> Result<Bo<'v>> {
        let mut bo = Self::create(vm, label, size)?;
        let va = vm.alloc_va(bo.size, align.max(4096))?;
        bo.bind(va, map)?;
        Ok(bo)
    }

    /// The same, bound at a VA the caller chooses (e.g. the address a
    /// reference trace used). A bind the kernel refuses is returned as the
    /// error; no other address is tried.
    pub fn new_at(vm: &'v Vm<'v>, label: &str, size: u64, map: Map, va: u64) -> Result<Bo<'v>> {
        if va < VA_BASE || va.checked_add(size).is_none_or(|e| e > vm.user_va_range) {
            return Err(Error::Refused(format!(
                "{label}: VA {va:#x} + {size:#x} is outside the user range [{VA_BASE:#x}, {:#x})",
                vm.user_va_range
            )));
        }
        vm.va.borrow().check_free(label, va, size).map_err(Error::Refused)?;
        let mut bo = Self::create(vm, label, size)?;
        bo.bind(va, map)?;
        Ok(bo)
    }

    fn create(vm: &'v Vm<'v>, label: &str, size: u64) -> Result<Bo<'v>> {
        if size == 0 {
            return Err(Error::Refused(format!("{label}: a zero-byte buffer")));
        }
        let mut c = DrmPanthorBoCreate { size, flags: 0, exclusive_vm_id: vm.id, ..Default::default() };
        vm.dev
            .ioctl(DRM_IOCTL_PANTHOR_BO_CREATE, &mut c, || format!("DRM_IOCTL_PANTHOR_BO_CREATE {label} {size:#x}"))?;
        let mut bo = Bo { vm, handle: c.handle, size: c.size, cpu: core::ptr::null_mut(), va: 0, label: label.into() };
        bo.cpu = mmap_handle(vm, bo.handle, 0, bo.size, label)?;
        Ok(bo)
    }

    fn bind(&mut self, va: u64, map: Map) -> Result<()> {
        // The kernel rounds the buffer up to pages; the range is the BO's.
        bind_handle(self.vm, self.handle, 0, va, self.size, map, &self.label)?;
        self.va = va;
        Ok(())
    }

    /// A second mapping of `size` bytes of this buffer from `offset` (both
    /// page-aligned): bound at the next free VA and mapped for the CPU, each
    /// of its own. The kernel gives each the buffer's pages for as long as
    /// it lasts, whatever happens to this `Bo`: a VM mapping holds a
    /// reference to the object and keeps its pages pinned
    /// (`drm_gpuvm_bo_create`, Linux v6.18 drm_gpuvm.c:1555;
    /// panthor_mmu.c:1268-1297), and so does a CPU mapping
    /// (`drm_gem_mmap_obj`, drm_gem.c). A second mapping in the same VM is
    /// the kernel's to allow: the exclusive-VM check compares the VM's root
    /// object (panthor_mmu.c:1234-1236).
    pub fn alias(&self, label: &str, offset: u64, size: u64, map: Map) -> Result<Alias<'v>> {
        let page = 4096;
        if !offset.is_multiple_of(page) || !size.is_multiple_of(page) || size == 0 {
            return Err(Error::Refused(format!(
                "{label}: offset {offset:#x} or size {size:#x} is not a whole number of pages (panthor_mmu.c:2454)"
            )));
        }
        if offset.checked_add(size).is_none_or(|e| e > self.size) {
            return Err(Error::Refused(format!(
                "{label}: [{offset:#x}, +{size:#x}) is outside {} ({:#x} bytes)",
                self.label, self.size
            )));
        }
        let vm = self.vm;
        let va = vm.alloc_va(size, page)?;
        let cpu = mmap_handle(vm, self.handle, offset, size, label)?;
        let mut a = Alias { vm, va: 0, size, cpu, label: label.into() };
        bind_handle(vm, self.handle, offset, va, size, map, label)?;
        a.va = va;
        Ok(a)
    }

    /// The GPU VA the buffer is bound at.
    pub fn va(&self) -> u64 {
        self.va
    }

    /// The CPU view. The CPU mapping is write-combined on this
    /// non-coherent SoC (Linux v6.18 panthor_gem.c:247, `map_wc = !ptdev->coherent`); reads
    /// after a fence see what the GPU wrote once its caches were cleaned.
    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: `cpu` maps `size` bytes for as long as `self` lives; the
        // &mut borrow makes this the only CPU view.
        unsafe { core::slice::from_raw_parts_mut(self.cpu, self.size as usize) }
    }

    /// Write `data` at `off`, refusing a range past the end.
    pub fn write(&mut self, off: usize, data: &[u8]) -> Result<()> {
        let end = off.checked_add(data.len()).filter(|&e| e as u64 <= self.size);
        if end.is_none() {
            return Err(Error::Refused(format!(
                "{}: write {off:#x}+{:#x} past {:#x}",
                self.label,
                data.len(),
                self.size
            )));
        }
        self.bytes()[off..off + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// A volatile 64-bit read at an 8-aligned offset.
    pub fn read_u64(&self, off: usize) -> Result<u64> {
        if !off.is_multiple_of(8) || off as u64 + 8 > self.size {
            return Err(Error::Refused(format!("{}: u64 read at {off:#x}", self.label)));
        }
        // SAFETY: in bounds and aligned (checked above); volatile because the
        // GPU writes this memory behind the compiler's back.
        Ok(unsafe { core::ptr::read_volatile(self.cpu.add(off) as *const u64) })
    }

    /// A volatile 64-bit write at an 8-aligned offset.
    pub fn write_u64(&mut self, off: usize, v: u64) -> Result<()> {
        if !off.is_multiple_of(8) || off as u64 + 8 > self.size {
            return Err(Error::Refused(format!("{}: u64 write at {off:#x}", self.label)));
        }
        // SAFETY: as in read_u64.
        unsafe { core::ptr::write_volatile(self.cpu.add(off) as *mut u64, v) };
        Ok(())
    }
}

/// `size` bytes of buffer `handle` from `offset`, CPU-mapped (shared, on
/// our DRM fd); unmapped by the owner's Drop.
fn mmap_handle(vm: &Vm, handle: u32, offset: u64, size: u64, label: &str) -> Result<*mut u8> {
    let mut m = DrmPanthorBoMmapOffset { handle, ..Default::default() };
    vm.dev.ioctl(DRM_IOCTL_PANTHOR_BO_MMAP_OFFSET, &mut m, || format!("DRM_IOCTL_PANTHOR_BO_MMAP_OFFSET {label}"))?;
    // SAFETY: a fresh shared mapping of the BO's mmap offset on our DRM fd,
    // inside the size the kernel allocated (the callers check the range).
    let p = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            size as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            vm.dev.fd.as_raw_fd(),
            (m.offset + offset) as libc::off_t,
        )
    };
    if p == libc::MAP_FAILED {
        return Err(Error::Os { what: format!("mmap {label}"), err: io::Error::last_os_error() });
    }
    Ok(p as *mut u8)
}

/// `VM_BIND` map of `size` bytes of buffer `handle` from `bo_offset` at
/// `va`, refused first if it overlaps a bound range; counted in the VM's
/// binds and live bytes.
fn bind_handle(vm: &Vm, handle: u32, bo_offset: u64, va: u64, size: u64, map: Map, label: &str) -> Result<()> {
    let op = DrmPanthorVmBindOp {
        flags: map.flags(),
        bo_handle: handle,
        bo_offset,
        va,
        size,
        syncs: DrmPanthorObjArray::default(),
    };
    vm.va.borrow().check_free(label, va, size).map_err(Error::Refused)?;
    vm.bind_op(op, || format!("DRM_IOCTL_PANTHOR_VM_BIND map {label} at {va:#x} size {size:#x}"))?;
    vm.va.borrow_mut().claim(label, va, size).map_err(Error::Refused)?;
    vm.binds.set(vm.binds.get() + 1);
    vm.live.set(vm.live.get() + 1);
    vm.live_bytes.set(vm.live_bytes.get() + size);
    vm.live_bytes_high_water.set(vm.live_bytes_high_water.get().max(vm.live_bytes.get()));
    Ok(())
}

/// `VM_BIND` unmap of `[va, va + size)`, released from the VM's ranges and
/// counts; an unmap the kernel refuses is printed and the range kept.
fn unbind(vm: &Vm, va: u64, size: u64, label: &str) {
    let op = DrmPanthorVmBindOp { flags: DRM_PANTHOR_VM_BIND_OP_TYPE_UNMAP, va, size, ..Default::default() };
    match vm.bind_op(op, || format!("DRM_IOCTL_PANTHOR_VM_BIND unmap {label}")) {
        Ok(()) => {
            vm.va.borrow_mut().release(va);
            vm.live.set(vm.live.get() - 1);
            vm.live_bytes.set(vm.live_bytes.get() - size);
        }
        Err(e) => eprintln!("{e}"),
    }
}

/// A second mapping of a buffer's pages ([`Bo::alias`]): a VA and a CPU
/// address of its own, each holding the pages for as long as it lasts.
pub struct Alias<'v> {
    vm: &'v Vm<'v>,
    va: u64,
    pub size: u64,
    cpu: *mut u8,
    pub label: String,
}

impl Alias<'_> {
    /// The GPU VA of the mapping.
    pub fn va(&self) -> u64 {
        self.va
    }

    /// The CPU address of the mapping (write-combined, as the buffer's).
    pub fn cpu(&self) -> *mut u8 {
        self.cpu
    }
}

impl Drop for Alias<'_> {
    fn drop(&mut self) {
        if self.va != 0 {
            unbind(self.vm, self.va, self.size, &self.label);
        }
        // SAFETY: the mapping made in `Bo::alias`, of this size.
        unsafe { libc::munmap(self.cpu as *mut libc::c_void, self.size as usize) };
    }
}

impl Drop for Bo<'_> {
    fn drop(&mut self) {
        let vm = self.vm;
        if self.va != 0 {
            unbind(vm, self.va, self.size, &self.label);
        }
        if !self.cpu.is_null() {
            // SAFETY: the mapping made in `create`, of this size.
            unsafe { libc::munmap(self.cpu as *mut libc::c_void, self.size as usize) };
        }
        let mut c = DrmGemClose { handle: self.handle, pad: 0 };
        if let Err(e) = vm.dev.ioctl(DRM_IOCTL_GEM_CLOSE, &mut c, || "DRM_IOCTL_GEM_CLOSE".into()) {
            eprintln!("{}: {e}", self.label);
        }
    }
}

/// A scheduling group with one queue.
pub struct Group<'v> {
    vm: &'v Vm<'v>,
    pub handle: u32,
    pub ringbuf_size: u32,
    submits: Cell<u32>,
}

/// The group's state after a submit (`DRM_IOCTL_PANTHOR_GROUP_GET_STATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupState {
    pub state: u32,
    pub fatal_queues: u32,
}

impl<'v> Group<'v> {
    /// One queue of priority 1 with a `ringbuf_size` ring, every shader core
    /// for compute and fragment, the one tiler, medium priority: Mesa's
    /// context group (pan_csf.c:1823-1838; its core masks default to
    /// `shader_present`).
    pub fn new(vm: &'v Vm<'v>, ringbuf_size: u32) -> Result<Group<'v>> {
        let cores = vm.dev.gpu.shader_present;
        let queues = [DrmPanthorQueueCreate { priority: 1, pad: [0; 3], ringbuf_size }];
        let mut c = DrmPanthorGroupCreate {
            queues: DrmPanthorObjArray::of(&queues),
            max_compute_cores: cores.count_ones() as u8,
            max_fragment_cores: cores.count_ones() as u8,
            max_tiler_cores: 1,
            priority: PANTHOR_GROUP_PRIORITY_MEDIUM,
            compute_core_mask: cores,
            fragment_core_mask: cores,
            tiler_core_mask: 1,
            vm_id: vm.id,
            ..Default::default()
        };
        vm.dev.ioctl(DRM_IOCTL_PANTHOR_GROUP_CREATE, &mut c, || "DRM_IOCTL_PANTHOR_GROUP_CREATE".into())?;
        Ok(Group { vm, handle: c.group_handle, ringbuf_size, submits: Cell::new(0) })
    }

    /// Submit `size` bytes of command stream at `stream_va` to queue 0 and
    /// return the syncobj the kernel signals when the job is done
    /// (`DRM_PANTHOR_SYNC_OP_SIGNAL`, as `csf_submit_batch` attaches one;
    /// pan_csf.c:921-939). `latest_flush` 0: the kernel cleans and
    /// invalidates the GPU caches before the stream unconditionally
    /// (panthor_drm.h:856-867).
    pub fn submit(&self, stream_va: u64, size: u32) -> Result<Fence<'v>> {
        if !stream_va.is_multiple_of(64) || !size.is_multiple_of(8) || size == 0 {
            return Err(Error::Refused(format!(
                "stream at {stream_va:#x} size {size:#x}: the address must be 64-byte aligned and the size a non-zero multiple of 8"
            )));
        }
        let syncobj = self.vm.dev.syncobj()?;
        let syncs = [DrmPanthorSyncOp {
            flags: DRM_PANTHOR_SYNC_OP_SIGNAL | DRM_PANTHOR_SYNC_OP_HANDLE_TYPE_SYNCOBJ,
            handle: syncobj.handle,
            timeline_value: 0,
        }];
        let qs = [DrmPanthorQueueSubmit {
            queue_index: 0,
            stream_size: size,
            stream_addr: stream_va,
            latest_flush: 0,
            pad: 0,
            syncs: DrmPanthorObjArray::of(&syncs),
        }];
        let mut g =
            DrmPanthorGroupSubmit { group_handle: self.handle, pad: 0, queue_submits: DrmPanthorObjArray::of(&qs) };
        let t0 = Instant::now();
        self.vm.dev.ioctl(DRM_IOCTL_PANTHOR_GROUP_SUBMIT, &mut g, || {
            format!("DRM_IOCTL_PANTHOR_GROUP_SUBMIT stream {stream_va:#x} size {size:#x}")
        })?;
        let ioctl_time = t0.elapsed();
        self.submits.set(self.submits.get() + 1);
        Ok(Fence { syncobj, submitted: t0, ioctl_time })
    }

    /// The group's state flags.
    pub fn state(&self) -> Result<GroupState> {
        let mut s = DrmPanthorGroupGetState { group_handle: self.handle, ..Default::default() };
        self.vm.dev.ioctl(DRM_IOCTL_PANTHOR_GROUP_GET_STATE, &mut s, || "DRM_IOCTL_PANTHOR_GROUP_GET_STATE".into())?;
        Ok(GroupState { state: s.state, fatal_queues: s.fatal_queues })
    }

    /// Wait for `fence`, then read the group's state: a timeout or a fatal
    /// fault is an error even though the fence signalled (the kernel
    /// signals the fences of a faulted job too).
    pub fn wait(&self, fence: &Fence<'_>, timeout: Duration) -> Result<(Waited, GroupState)> {
        let w = fence.syncobj.wait(timeout)?;
        let s = self.state()?;
        if s.state & (DRM_PANTHOR_GROUP_STATE_TIMEDOUT | DRM_PANTHOR_GROUP_STATE_FATAL_FAULT) != 0 {
            return Err(Error::GroupFault { state: s.state, fatal_queues: s.fatal_queues });
        }
        Ok((w, s))
    }
}

impl Drop for Group<'_> {
    fn drop(&mut self) {
        println!("group {}: {} submits", self.handle, self.submits.get());
        let mut d = DrmPanthorGroupDestroy { group_handle: self.handle, pad: 0 };
        if let Err(e) =
            self.vm.dev.ioctl(DRM_IOCTL_PANTHOR_GROUP_DESTROY, &mut d, || "DRM_IOCTL_PANTHOR_GROUP_DESTROY".into())
        {
            eprintln!("group {}: {e}", self.handle);
        }
    }
}

/// A submitted job: the syncobj the kernel signals, and when it went in.
pub struct Fence<'d> {
    pub syncobj: Syncobj<'d>,
    pub submitted: Instant,
    /// How long the submit ioctl took.
    pub ioctl_time: Duration,
}

/// A tiler heap: the kernel allocates its chunks and its context.
pub struct TilerHeap<'v> {
    vm: &'v Vm<'v>,
    pub handle: u32,
    pub ctx_va: u64,
    pub first_chunk_va: u64,
    pub chunk_size: u32,
    pub initial_chunks: u32,
    pub max_chunks: u32,
}

impl<'v> TilerHeap<'v> {
    /// `DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE`, refusing what the header says
    /// the kernel refuses (panthor_drm.h:948-962). Mesa's defaults are 2 MiB
    /// chunks, 5 initial, 64 at most, 65535 in flight
    /// (driinfo_panfrost.h:16-21, pan_csf.c:1854-1860).
    pub fn new(vm: &'v Vm<'v>, chunk_size: u32, initial: u32, max: u32, in_flight: u32) -> Result<TilerHeap<'v>> {
        if !chunk_size.is_multiple_of(4096) || !(128 << 10..=8 << 20).contains(&chunk_size) {
            return Err(Error::Refused(format!(
                "tiler heap chunk size {chunk_size:#x}: page-aligned in [128 KiB, 8 MiB]"
            )));
        }
        if initial == 0 || max < initial {
            return Err(Error::Refused(format!("tiler heap chunks: initial {initial}, max {max}")));
        }
        let mut c = DrmPanthorTilerHeapCreate {
            vm_id: vm.id,
            initial_chunk_count: initial,
            chunk_size,
            max_chunks: max,
            target_in_flight: in_flight,
            ..Default::default()
        };
        vm.dev.ioctl(DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE, &mut c, || "DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE".into())?;
        Ok(TilerHeap {
            vm,
            handle: c.handle,
            ctx_va: c.tiler_heap_ctx_gpu_va,
            first_chunk_va: c.first_heap_chunk_gpu_va,
            chunk_size,
            initial_chunks: initial,
            max_chunks: max,
        })
    }
}

impl Drop for TilerHeap<'_> {
    fn drop(&mut self) {
        let mut d = DrmPanthorTilerHeapDestroy { handle: self.handle, pad: 0 };
        if let Err(e) = self
            .vm
            .dev
            .ioctl(DRM_IOCTL_PANTHOR_TILER_HEAP_DESTROY, &mut d, || "DRM_IOCTL_PANTHOR_TILER_HEAP_DESTROY".into())
        {
            eprintln!("tiler heap {}: {e}", self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bind_over_a_bound_range_is_refused_naming_both() {
        let mut m = VaMap::new();
        m.claim("a", 0x2000_0000, 0x4000).unwrap();
        // Same range, inside, straddling the start, straddling the end.
        for (va, size) in [(0x2000_0000, 0x4000), (0x2000_1000, 0x1000), (0x1fff_f000, 0x2000), (0x2000_3000, 0x2000)] {
            let e = m.check_free("b", va, size).unwrap_err();
            assert!(e.contains("b [") && e.contains("a [0x20000000, 0x20004000)"), "{e}");
        }
        // Touching ranges do not overlap.
        m.check_free("b", 0x2000_4000, 0x1000).unwrap();
        m.check_free("b", 0x1fff_f000, 0x1000).unwrap();
        // claim refuses too, and leaves the set unchanged.
        assert!(m.claim("c", 0x2000_2000, 0x1000).is_err());
        assert_eq!(m.bound.len(), 1);
        // A released range is free again.
        m.release(0x2000_0000);
        m.claim("c", 0x2000_2000, 0x1000).unwrap();
    }

    #[test]
    fn the_bump_allocator_steps_over_a_range_bound_at_a_chosen_address() {
        let mut m = VaMap::new();
        let limit = 1 << 40;
        let first = m.alloc(0x1000, 0x1000, limit).unwrap();
        assert_eq!(first, VA_BASE);
        // A buffer bound where the bump would go next.
        m.claim("fixed", VA_BASE + 0x1000, 0x3000).unwrap();
        let next = m.alloc(0x2000, 0x1000, limit).unwrap();
        assert_eq!(next, VA_BASE + 0x4000, "handed out inside the fixed range");
        assert!(m.check_free("x", next, 0x2000).is_ok(), "alloc only reserves; the bind claims");
        // Two fixed ranges in a row are both stepped over.
        m.claim("fixed2", VA_BASE + 0x6000, 0x1000).unwrap();
        assert_eq!(m.alloc(0x1000, 0x1000, limit).unwrap(), VA_BASE + 0x7000);
        // The limit still holds.
        assert_eq!(m.alloc(0x1000, 0x1000, VA_BASE + 0x7000), None);
    }
}
