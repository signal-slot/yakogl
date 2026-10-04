// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The cluster's application layer, ported to `no_std` from
//! signal-slot/agl-slint-cluster at 2b9e00d4 (MIT): `rust/src/telemetry.rs`
//! (the `Telemetry` frame), `rust/src/sim.rs` (the built-in drive cycle) and
//! `apply()` from `rust/src/main.rs` (the only place a frame becomes widget
//! properties). The cluster's README says this layer is per target and the
//! `.slint` design is shared; the design is not copied here, it is compiled
//! from the cluster's own `ui/` (see ../build.rs).
//!
//! What changed in the port, and nothing else:
//! - `core` + `alloc` instead of `std`; `expf` from the `libm` crate;
//! - the wall clock (`chrono::Local::now()`) is the drive cycle's own clock,
//!   `CLOCK_START_MIN` plus the elapsed time, so a frame is the same picture on
//!   every run and on the host reference;
//! - the sources other than the simulation (CAN, Kuksa, OBD-II) are not ported.
//!
//! The file is compiled twice: into the board's staticlib (lib.rs) and into
//! the host reference render (../../host/ref), so both draw the same state.

use alloc::format;
use alloc::string::{String, ToString};

use crate::AppWindow;

/// Headlight state.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)] // the sim uses a subset
pub enum Beam {
    Off,
    Low,
    High,
}

/// A warning lamp with the usual two escalation colours.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub enum Warn {
    Off,
    Amber,
    Red,
}

/// Cruise-control lamp.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub enum Cruise {
    Off,
    Set,
    Active,
}

/// Lane-keeping lamp.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub enum Lane {
    Off,
    Tracking,
    Warn,
    Depart,
}

/// One frame of vehicle signals, in physical / semantic units.
#[derive(Clone, Debug)]
pub struct Telemetry {
    pub speed_kmh: f32,
    pub power: f32,
    pub efficiency_percent: f32,
    pub avg_efficiency_kwh: f32,
    pub range_km: f32,
    pub battery_level: f32,
    pub fuel_level: f32,
    pub outside_temp_c: i32,
    pub odo_km: f32,
    pub trip_seconds: u32,
    pub turn_distance_km: f32,
    pub turn_street: String,
    pub cruise_speed_kmh: f32,
    pub lta_on: bool,
    pub lca_on: bool,
    pub gear: char,
    pub ready: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub headlights: Beam,
    pub engine: Warn,
    pub oil: Warn,
    pub battery_warn: Warn,
    pub abs: Warn,
    pub parking_brake: bool,
    pub seatbelt: bool,
    pub door_open: bool,
    pub cruise: Cruise,
    pub lane: Lane,
}

/// Length of one drive cycle, seconds.
const CYCLE: f32 = 66.0;

/// The built-in demo source: one repeating drive cycle from which every
/// reading is derived (sim.rs).
pub struct SimSource {
    t: f32,
    speed: f32,
    trip_km: f32,
    odo_km: f32,
}

impl SimSource {
    pub const fn new() -> Self {
        Self { t: 0.0, speed: 0.0, trip_km: 0.0, odo_km: 12_345.0 }
    }

    /// Seconds since the cycle started.
    pub fn time(&self) -> f32 {
        self.t
    }

    fn target_speed(t: f32) -> f32 {
        let t = t % CYCLE;
        const TRACE: &[(f32, f32)] = &[
            (0.0, 0.0),
            (11.0, 62.0),
            (22.0, 68.0),
            (28.0, 34.0),
            (34.0, 30.0),
            (45.0, 118.0),
            (52.0, 104.0),
            (61.0, 0.0),
            (CYCLE, 0.0),
        ];
        let mut prev = TRACE[0];
        for &pt in &TRACE[1..] {
            if t <= pt.0 {
                let span = pt.0 - prev.0;
                let k = if span > 0.0 { (t - prev.0) / span } else { 0.0 };
                let k = k * k * (3.0 - 2.0 * k);
                return prev.1 + (pt.1 - prev.1) * k;
            }
            prev = pt;
        }
        0.0
    }

    fn accel(&self) -> f32 {
        (Self::target_speed(self.t + 0.25) - Self::target_speed(self.t - 0.25)) / 0.5
    }

    pub fn poll(&mut self, dt: f32) -> Telemetry {
        self.t += dt;
        let target = Self::target_speed(self.t);
        self.speed += (target - self.speed) * (1.0 - libm::expf(-dt * 2.5));
        let km = self.speed * dt / 3600.0;
        self.trip_km += km;
        self.odo_km += km;

        let a = self.accel();
        let used = self.trip_km / 560.0;
        let eff = 85.0 - a.max(0.0) * 1.6 + (-a).max(0.0) * 1.4;
        let to_turn = (2.4 - (self.t % CYCLE) / CYCLE * 2.4).max(0.0);
        let cruising = self.speed > 55.0 && a.abs() < 3.0;

        Telemetry {
            speed_kmh: self.speed,
            power: (a / 9.0).clamp(-1.0, 1.0),
            efficiency_percent: eff.clamp(40.0, 99.0),
            avg_efficiency_kwh: (4.8 - a.max(0.0) * 0.06).max(2.0),
            range_km: (560.0 - self.trip_km).max(0.0),
            battery_level: (0.85 - used * 0.6).clamp(0.05, 1.0),
            fuel_level: (0.55 - used * 0.4).clamp(0.02, 1.0),
            outside_temp_c: 21,
            odo_km: self.odo_km,
            trip_seconds: self.t as u32,
            turn_distance_km: to_turn,
            turn_street: "Minatomirai Blvd".into(),
            cruise_speed_kmh: 80.0,
            lta_on: true,
            lca_on: true,
            gear: 'D',
            ready: true,
            turn_left: false,
            turn_right: to_turn > 0.05 && to_turn < 1.2,
            headlights: Beam::Low,
            engine: Warn::Off,
            oil: Warn::Off,
            battery_warn: Warn::Off,
            abs: Warn::Off,
            parking_brake: false,
            seatbelt: false,
            door_open: false,
            cruise: if cruising { Cruise::Active } else { Cruise::Off },
            lane: Lane::Tracking,
        }
    }
}

/// The telemetry period, as the cluster's timer (main.rs: 20 Hz).
pub const TICK_S: f32 = 0.05;

/// The clock the cluster shows: this many minutes past midnight at the start
/// of the drive cycle (the cluster's own default text is "12:34").
pub const CLOCK_START_MIN: u32 = 12 * 60 + 34;

/// Push one telemetry frame into the UI (main.rs `apply`). `ticks` is how many
/// telemetry periods have run (the turn-signal blink, ~1.25 Hz, is resolved
/// from it as the cluster does), `clock_s` the seconds since the cycle began.
pub fn apply(ui: &AppWindow, t: &Telemetry, ticks: u64, clock_s: u32) {
    let blink = (ticks / 8) % 2 == 0;
    ui.set_speed(t.speed_kmh);
    ui.set_charge_angle((-6.0 + t.power * 118.0).clamp(-138.0, 138.0));
    ui.set_efficiency_percent(t.efficiency_percent);
    ui.set_avg_efficiency(format!("{:.1}", t.avg_efficiency_kwh).into());
    ui.set_range_km(format!("{}", t.range_km as u32).into());
    ui.set_battery_level(t.battery_level);
    ui.set_fuel_level(t.fuel_level);
    ui.set_outside_temp(format!("{}", t.outside_temp_c).into());
    ui.set_odo(group_thousands(t.odo_km as u32).into());
    ui.set_drive_time(format!("{:02}:{:02}", t.trip_seconds / 60, t.trip_seconds % 60).into());
    ui.set_turn_distance(format!("{:.1}", t.turn_distance_km).into());
    ui.set_turn_street(t.turn_street.clone().into());
    ui.set_cruise_speed(format!("{}", t.cruise_speed_kmh as u32).into());
    ui.set_lta_on(t.lta_on);
    ui.set_lca_on(t.lca_on);
    ui.set_gear(t.gear.to_string().into());
    ui.set_ready(t.ready);

    ui.set_turn_left_on(t.turn_left && blink);
    ui.set_turn_right_on(t.turn_right && blink);
    ui.set_tt_lights(match t.headlights {
        Beam::Off => 0,
        Beam::Low => 1,
        Beam::High => 2,
    });
    ui.set_tt_engine(warn_level(t.engine));
    ui.set_tt_oil(warn_level(t.oil));
    ui.set_tt_battery(warn_level(t.battery_warn));
    ui.set_tt_abs(warn_level(t.abs));
    ui.set_tt_parking_brake(t.parking_brake as i32);
    ui.set_tt_seatbelt(t.seatbelt as i32);
    ui.set_tt_door(t.door_open as i32);
    ui.set_tt_cruise(match t.cruise {
        Cruise::Off => 0,
        Cruise::Set => 1,
        Cruise::Active => 2,
    });
    ui.set_tt_lane(match t.lane {
        Lane::Off => 0,
        Lane::Tracking => 1,
        Lane::Warn => 2,
        Lane::Depart => 3,
    });

    let min = (CLOCK_START_MIN + clock_s / 60) % (24 * 60);
    ui.set_time(format!("{:02}:{:02}", min / 60, min % 60).into());
}

fn warn_level(w: Warn) -> i32 {
    match w {
        Warn::Off => 0,
        Warn::Amber => 1,
        Warn::Red => 2,
    }
}

fn group_thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The drive cycle and its tick count, as the cluster's timer closure holds
/// them.
pub struct Drive {
    pub sim: SimSource,
    pub ticks: u64,
    pub last: Option<Telemetry>,
}

impl Drive {
    pub const fn new() -> Self {
        Self { sim: SimSource::new(), ticks: 0, last: None }
    }

    /// Run `n` telemetry periods (the last frame is kept) and push the last
    /// frame into `ui`.
    pub fn advance(&mut self, ui: &AppWindow, n: u32) {
        for _ in 0..n {
            self.last = Some(self.sim.poll(TICK_S));
            self.ticks += 1;
        }
        if let Some(t) = self.last.as_ref() {
            apply(ui, t, self.ticks.saturating_sub(1), self.sim.time() as u32);
        }
    }
}
