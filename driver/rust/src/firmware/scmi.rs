// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An SCMI codec and client (Arm DEN0056, "System Control and Management
//! Interface", sections 4.1 to 4.8 for the protocols used here).
//!
//! A message is little-endian: a 32-bit header, then the payload; a response
//! is the header, a 32-bit signed status, then the return values. The
//! transport (an SMC mailbox, OP-TEE's message protocol) moves the bytes and
//! is the [`Transport`] trait; this module knows nothing of it, nor of any
//! SoC's domain numbers.
//!
//! Header (DEN0056 4.1.2): message id in bits 7:0, message type in 9:8
//! (0 command), protocol id in 17:10, token in 27:18.

/// Protocol ids (DEN0056 table 1).
pub const PROTO_BASE: u8 = 0x10;
pub const PROTO_POWER: u8 = 0x11;
pub const PROTO_PERF: u8 = 0x13;
pub const PROTO_CLOCK: u8 = 0x14;
pub const PROTO_RESET: u8 = 0x16;
pub const PROTO_VOLTAGE: u8 = 0x17;

/// Message ids every protocol has (DEN0056 4.1.2).
pub const PROTOCOL_VERSION: u8 = 0x0;
pub const PROTOCOL_ATTRIBUTES: u8 = 0x1;

/// BASE (4.2.2).
pub const BASE_DISCOVER_VENDOR: u8 = 0x3;

/// POWER (4.3.2).
pub const POWER_DOMAIN_ATTRIBUTES: u8 = 0x3;
pub const POWER_STATE_SET: u8 = 0x4;
pub const POWER_STATE_GET: u8 = 0x5;

/// PERFORMANCE (4.5.2).
pub const PERF_DOMAIN_ATTRIBUTES: u8 = 0x3;
pub const PERF_DESCRIBE_LEVELS: u8 = 0x4;
pub const PERF_LIMITS_SET: u8 = 0x5;
pub const PERF_LIMITS_GET: u8 = 0x6;
pub const PERF_LEVEL_SET: u8 = 0x7;
pub const PERF_LEVEL_GET: u8 = 0x8;

/// CLOCK (4.6.2).
pub const CLOCK_ATTRIBUTES: u8 = 0x3;
pub const CLOCK_DESCRIBE_RATES: u8 = 0x4;
pub const CLOCK_RATE_SET: u8 = 0x5;
pub const CLOCK_RATE_GET: u8 = 0x6;
pub const CLOCK_CONFIG_SET: u8 = 0x7;

/// RESET (4.7.2).
pub const RESET_DOMAIN_ATTRIBUTES: u8 = 0x3;
pub const RESET_RESET: u8 = 0x4;

/// VOLTAGE DOMAIN (4.8.2). OP-TEE 4.0.0-stm32mp-r3.1 answers these in
/// core/drivers/scmi-msg/voltage_domain.c (message ids at voltage_domain.h:20-25,
/// handlers at :129 attributes, :270 config_set, :297 config_get, :325 level_set,
/// :350 level_get); STM32MP's regulators are behind regulator_consumer.c.
pub const VOLTAGE_DOMAIN_ATTRIBUTES: u8 = 0x3;
pub const VOLTAGE_DESCRIBE_LEVELS: u8 = 0x4;
pub const VOLTAGE_CONFIG_SET: u8 = 0x5;
pub const VOLTAGE_CONFIG_GET: u8 = 0x6;
pub const VOLTAGE_LEVEL_SET: u8 = 0x7;
pub const VOLTAGE_LEVEL_GET: u8 = 0x8;

/// VOLTAGE_CONFIG_SET/GET mode, bits 3:0 (4.8.2.6; OP-TEE scmi-msg.h:21-22):
/// 0 off, 7 architecturally on. regulator_consumer.c:196-216 rejects any other.
pub const VOLTAGE_CONFIG_OFF: u32 = 0x0;
pub const VOLTAGE_CONFIG_ON: u32 = 0x7;

/// CLOCK_RATE_SET flags (4.6.2.6; OP-TEE clock.h:111-118): bit 0 asynchronous,
/// bit 1 no delayed response, bit 2 round up, bit 3 platform chooses.
pub const CLOCK_RATE_ASYNC: u32 = 1 << 0;
pub const CLOCK_RATE_NO_DELAYED_RESPONSE: u32 = 1 << 1;
pub const CLOCK_RATE_ROUND_UP: u32 = 1 << 2;
pub const CLOCK_RATE_ROUND_AUTO: u32 = 1 << 3;

/// Bytes of a domain or clock name in a response: NUL-padded (clock.h:59,
/// voltage_domain.h:40).
pub const NAME_LEN: usize = 16;

/// Most rates one CLOCK_DESCRIBE_RATES reply holds in [`MAX_MSG`].
pub const MAX_DESCRIBED_RATES: usize = (MAX_MSG - 12) / 8;

/// RESET flags (4.7.2.5): bit 0 asynchronous, bit 1 explicit signal.
pub const RESET_ASYNC: u32 = 1 << 0;
pub const RESET_EXPLICIT: u32 = 1 << 1;

/// POWER_STATE_SET states (4.3.2.5): 0 on, bit 30 off (the generic types).
pub const POWER_ON: u32 = 0;
pub const POWER_OFF: u32 = 1 << 30;

/// CLOCK_CONFIG_SET attributes (4.6.2.7): bits 1:0 the enable state.
pub const CLOCK_DISABLE: u32 = 0;
pub const CLOCK_ENABLE: u32 = 1;

/// Largest message the codec builds or accepts, header included.
pub const MAX_MSG: usize = 128;

/// Status codes (DEN0056 table 6).
pub const STATUS_SUCCESS: i32 = 0;
pub const STATUS_NOT_SUPPORTED: i32 = -1;
pub const STATUS_INVALID_PARAMETERS: i32 = -2;
pub const STATUS_DENIED: i32 = -3;
pub const STATUS_NOT_FOUND: i32 = -4;
pub const STATUS_OUT_OF_RANGE: i32 = -5;
pub const STATUS_BUSY: i32 = -6;
pub const STATUS_COMMS_ERROR: i32 = -7;
pub const STATUS_GENERIC_ERROR: i32 = -8;
pub const STATUS_HARDWARE_ERROR: i32 = -9;
pub const STATUS_PROTOCOL_ERROR: i32 = -10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The transport failed (the code is the transport's own).
    Transport(i32),
    /// The agent answered with a non-zero status.
    Status(i32),
    /// The response is shorter than header + status (+ the values asked for),
    /// or its header does not answer the command sent.
    Malformed,
    /// The payload does not fit [`MAX_MSG`].
    TooLong,
}

/// Moves one command to the agent and brings back its response; `rx` is the
/// buffer for the response, the return value its length in bytes.
pub trait Transport {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32>;
}

/// The header of a command (type 0).
pub const fn header(proto: u8, msg: u8, token: u16) -> u32 {
    (msg as u32) | ((proto as u32) << 10) | (((token & 0x3ff) as u32) << 18)
}

/// Build a command: header, then the payload words. Returns the length.
pub fn encode(buf: &mut [u8; MAX_MSG], hdr: u32, words: &[u32]) -> Result<usize, Error> {
    let len = 4 + 4 * words.len();
    if len > MAX_MSG {
        return Err(Error::TooLong);
    }
    buf[..4].copy_from_slice(&hdr.to_le_bytes());
    for (i, w) in words.iter().enumerate() {
        buf[4 + 4 * i..8 + 4 * i].copy_from_slice(&w.to_le_bytes());
    }
    Ok(len)
}

/// Word `i` of `buf` (little-endian).
pub fn word(buf: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([buf[4 * i], buf[4 * i + 1], buf[4 * i + 2], buf[4 * i + 3]])
}

/// The first CLOCK_DESCRIBE_RATES reply (4.6.2.4; clock.c:455-542).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockRates {
    /// True: `rates[..3]` are lowest, highest and step (clock.h:203 format 1).
    /// False: `rates[..count]` is a list.
    pub range: bool,
    /// Rates returned in this reply (`rates[..count]`).
    pub count: usize,
    /// Rates the agent still holds beyond this reply (bits 31:16).
    pub remaining: u32,
    pub rates: [u64; MAX_DESCRIBED_RATES],
}

/// A client on one channel. Tokens count up, 10 bits.
pub struct Client<T: Transport> {
    pub transport: T,
    token: u16,
}

/// The values of a response: `len()` words after the status.
pub struct Reply {
    buf: [u8; MAX_MSG],
    n: usize,
}

impl Reply {
    /// Return value `i` (the word after the status and `i` earlier ones).
    pub fn get(&self, i: usize) -> u32 {
        word(&self.buf, 2 + i)
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// The name field at return value `first` of `r` (four words, NUL-padded).
fn name_at(r: &Reply, first: usize) -> [u8; NAME_LEN] {
    let mut n = [0u8; NAME_LEN];
    for i in 0..NAME_LEN / 4 {
        n[4 * i..4 * i + 4].copy_from_slice(&r.get(first + i).to_le_bytes());
    }
    n
}

impl<T: Transport> Client<T> {
    pub const fn new(transport: T) -> Self {
        Client { transport, token: 0 }
    }

    /// One command; `want` return values at least are required on success.
    pub fn call(&mut self, proto: u8, msg: u8, words: &[u32], want: usize) -> Result<Reply, Error> {
        let hdr = header(proto, msg, self.token);
        self.token = (self.token + 1) & 0x3ff;
        let mut tx = [0u8; MAX_MSG];
        let len = encode(&mut tx, hdr, words)?;
        let mut rx = [0u8; MAX_MSG];
        let got = self.transport.transfer(&tx[..len], &mut rx).map_err(Error::Transport)?;
        if !(8..=MAX_MSG).contains(&got) || word(&rx, 0) != hdr {
            return Err(Error::Malformed);
        }
        let status = word(&rx, 1) as i32;
        if status != STATUS_SUCCESS {
            return Err(Error::Status(status));
        }
        let n = (got - 8) / 4;
        if n < want {
            return Err(Error::Malformed);
        }
        Ok(Reply { buf: rx, n })
    }

    /// PROTOCOL_VERSION of `proto`: major in bits 31:16, minor in 15:0.
    pub fn version(&mut self, proto: u8) -> Result<u32, Error> {
        Ok(self.call(proto, PROTOCOL_VERSION, &[], 1)?.get(0))
    }

    /// POWER_STATE_SET, synchronous (flags 0).
    pub fn power_set(&mut self, domain: u32, state: u32) -> Result<(), Error> {
        self.call(PROTO_POWER, POWER_STATE_SET, &[0, domain, state], 0).map(|_| ())
    }

    pub fn power_get(&mut self, domain: u32) -> Result<u32, Error> {
        Ok(self.call(PROTO_POWER, POWER_STATE_GET, &[domain], 1)?.get(0))
    }

    /// PERFORMANCE_LIMITS_SET: the domain, then the maximum and the minimum level.
    pub fn perf_limits_set(&mut self, domain: u32, max: u32, min: u32) -> Result<(), Error> {
        self.call(PROTO_PERF, PERF_LIMITS_SET, &[domain, max, min], 0).map(|_| ())
    }

    /// PERFORMANCE_LIMITS_GET: (maximum, minimum).
    pub fn perf_limits_get(&mut self, domain: u32) -> Result<(u32, u32), Error> {
        let r = self.call(PROTO_PERF, PERF_LIMITS_GET, &[domain], 2)?;
        Ok((r.get(0), r.get(1)))
    }

    pub fn perf_level_set(&mut self, domain: u32, level: u32) -> Result<(), Error> {
        self.call(PROTO_PERF, PERF_LEVEL_SET, &[domain, level], 0).map(|_| ())
    }

    pub fn perf_level_get(&mut self, domain: u32) -> Result<u32, Error> {
        Ok(self.call(PROTO_PERF, PERF_LEVEL_GET, &[domain], 1)?.get(0))
    }

    /// CLOCK_RATE_GET in Hz.
    pub fn clock_rate_get(&mut self, clock: u32) -> Result<u64, Error> {
        let r = self.call(PROTO_CLOCK, CLOCK_RATE_GET, &[clock], 2)?;
        Ok(r.get(0) as u64 | (r.get(1) as u64) << 32)
    }

    pub fn clock_config_set(&mut self, clock: u32, attributes: u32) -> Result<(), Error> {
        self.call(PROTO_CLOCK, CLOCK_CONFIG_SET, &[clock, attributes], 0).map(|_| ())
    }

    /// CLOCK_ATTRIBUTES (clock.c:132-166): (enabled, name). Bit 0 of the
    /// attributes is the enable state; the other bits are not decoded.
    pub fn clock_attributes(&mut self, clock: u32) -> Result<(bool, [u8; NAME_LEN]), Error> {
        let r = self.call(PROTO_CLOCK, CLOCK_ATTRIBUTES, &[clock], 5)?;
        Ok((r.get(0) & 1 != 0, name_at(&r, 1)))
    }

    /// CLOCK_RATE_SET (clock.c:228-255): the flags (0: synchronous, rounded
    /// by the agent), the clock, then the rate as low and high words.
    pub fn clock_rate_set(&mut self, clock: u32, hz: u64) -> Result<(), Error> {
        self.call(PROTO_CLOCK, CLOCK_RATE_SET, &[0, clock, hz as u32, (hz >> 32) as u32], 0).map(|_| ())
    }

    /// CLOCK_DESCRIBE_RATES from rate index 0: the first reply only.
    pub fn clock_describe_rates(&mut self, clock: u32) -> Result<ClockRates, Error> {
        let r = self.call(PROTO_CLOCK, CLOCK_DESCRIBE_RATES, &[clock, 0], 1)?;
        let flags = r.get(0);
        let count = (flags & 0xfff) as usize;
        if count > MAX_DESCRIBED_RATES || r.len() < 1 + 2 * count {
            return Err(Error::Malformed);
        }
        let range = flags >> 12 & 1 != 0;
        if range && count < 3 {
            return Err(Error::Malformed);
        }
        let mut rates = [0u64; MAX_DESCRIBED_RATES];
        for (i, rate) in rates.iter_mut().enumerate().take(count) {
            *rate = r.get(1 + 2 * i) as u64 | (r.get(2 + 2 * i) as u64) << 32;
        }
        Ok(ClockRates { range, count, remaining: flags >> 16, rates })
    }

    /// PROTOCOL_ATTRIBUTES of VOLTAGE (voltage_domain.c:89-105): the number
    /// of domains, bits 15:0.
    pub fn voltage_domain_count(&mut self) -> Result<u32, Error> {
        Ok(self.call(PROTO_VOLTAGE, PROTOCOL_ATTRIBUTES, &[], 1)?.get(0) & 0xffff)
    }

    /// VOLTAGE_DOMAIN_ATTRIBUTES (voltage_domain.c:129-160): the name.
    pub fn voltage_domain_name(&mut self, domain: u32) -> Result<[u8; NAME_LEN], Error> {
        let r = self.call(PROTO_VOLTAGE, VOLTAGE_DOMAIN_ATTRIBUTES, &[domain], 5)?;
        Ok(name_at(&r, 1))
    }

    /// VOLTAGE_CONFIG_SET (voltage_domain.c:270): [`VOLTAGE_CONFIG_ON`] or
    /// [`VOLTAGE_CONFIG_OFF`].
    pub fn voltage_config_set(&mut self, domain: u32, config: u32) -> Result<(), Error> {
        self.call(PROTO_VOLTAGE, VOLTAGE_CONFIG_SET, &[domain, config], 0).map(|_| ())
    }

    /// VOLTAGE_CONFIG_GET (voltage_domain.c:297): the mode, bits 3:0.
    pub fn voltage_config_get(&mut self, domain: u32) -> Result<u32, Error> {
        Ok(self.call(PROTO_VOLTAGE, VOLTAGE_CONFIG_GET, &[domain], 1)?.get(0) & 0xf)
    }

    /// VOLTAGE_LEVEL_SET (voltage_domain.c:325): microvolts, synchronous
    /// (the flags word is 0; the agent reads the level from the third word, :344).
    pub fn voltage_level_set(&mut self, domain: u32, microvolt: i32) -> Result<(), Error> {
        self.call(PROTO_VOLTAGE, VOLTAGE_LEVEL_SET, &[domain, 0, microvolt as u32], 0).map(|_| ())
    }

    /// VOLTAGE_LEVEL_GET (voltage_domain.c:350): microvolts.
    pub fn voltage_level_get(&mut self, domain: u32) -> Result<i32, Error> {
        Ok(self.call(PROTO_VOLTAGE, VOLTAGE_LEVEL_GET, &[domain], 1)?.get(0) as i32)
    }

    /// RESET: the domain, the flags ([`RESET_ASYNC`], [`RESET_EXPLICIT`]) and
    /// the reset state (0 for an architectural reset).
    pub fn reset(&mut self, domain: u32, flags: u32, state: u32) -> Result<(), Error> {
        self.call(PROTO_RESET, RESET_RESET, &[domain, flags, state], 0).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records the command and answers with a canned response, whose header
    /// is the command's own.
    struct Fake {
        sent: [u8; MAX_MSG],
        sent_len: usize,
        status: i32,
        values: [u32; 16],
        nvalues: usize,
        fail: Option<i32>,
    }

    fn fake(status: i32, values: &[u32]) -> Fake {
        let mut v = [0; 16];
        v[..values.len()].copy_from_slice(values);
        Fake { sent: [0; MAX_MSG], sent_len: 0, status, values: v, nvalues: values.len(), fail: None }
    }

    impl Transport for Fake {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            if let Some(e) = self.fail {
                return Err(e);
            }
            self.sent[..tx.len()].copy_from_slice(tx);
            self.sent_len = tx.len();
            rx[..4].copy_from_slice(&tx[..4]);
            rx[4..8].copy_from_slice(&self.status.to_le_bytes());
            for i in 0..self.nvalues {
                rx[8 + 4 * i..12 + 4 * i].copy_from_slice(&self.values[i].to_le_bytes());
            }
            Ok(8 + 4 * self.nvalues)
        }
    }

    fn sent(c: &Client<Fake>) -> &[u8] {
        &c.transport.sent[..c.transport.sent_len]
    }

    #[test]
    fn header_fields() {
        // id 7 | proto 0x13 << 10 | token 5 << 18
        assert_eq!(header(PROTO_PERF, 7, 5), 0x0014_4C07);
        assert_eq!(header(PROTO_BASE, 3, 0), 0x4003);
        // the token is 10 bits
        assert_eq!(header(PROTO_BASE, 3, 0x401), 0x0004_4003);
    }

    #[test]
    fn spec_ids() {
        // DEN0056 message ids and flag bits, literal.
        assert_eq!(PERF_DESCRIBE_LEVELS, 4);
        assert_eq!(PERF_LIMITS_SET, 5);
        assert_eq!(PERF_LIMITS_GET, 6);
        assert_eq!(PERF_LEVEL_SET, 7);
        assert_eq!(PERF_LEVEL_GET, 8);
        assert_eq!(RESET_ASYNC, 1);
        assert_eq!(RESET_EXPLICIT, 2);
    }

    #[test]
    fn version_bytes() {
        let mut c = Client::new(fake(0, &[0x0003_0000]));
        assert_eq!(c.version(PROTO_PERF), Ok(0x0003_0000));
        assert_eq!(sent(&c), [0x00, 0x4C, 0x00, 0x00]);
    }

    #[test]
    fn power_set_bytes() {
        let mut c = Client::new(fake(0, &[]));
        c.power_set(0, POWER_ON).unwrap();
        assert_eq!(sent(&c), [0x04, 0x44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        c.power_set(2, POWER_OFF).unwrap();
        // token 1 now
        assert_eq!(sent(&c), [0x04, 0x44, 0x04, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0x40]);
    }

    #[test]
    fn perf_limits_bytes() {
        let mut c = Client::new(fake(0, &[900, 100]));
        c.perf_limits_set(1, 900, 100).unwrap();
        assert_eq!(sent(&c), [0x05, 0x4C, 0, 0, 1, 0, 0, 0, 0x84, 0x03, 0, 0, 100, 0, 0, 0]);
        assert_eq!(c.perf_limits_get(1), Ok((900, 100)));
        assert_eq!(sent(&c), [0x06, 0x4C, 0x04, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn perf_level_bytes() {
        let mut c = Client::new(fake(0, &[7]));
        c.perf_level_set(1, 3).unwrap();
        assert_eq!(sent(&c), [0x07, 0x4C, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(c.perf_level_get(1), Ok(7));
        assert_eq!(sent(&c), [0x08, 0x4C, 0x04, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn clock_bytes() {
        let mut c = Client::new(fake(0, &[0x1000_0000, 1]));
        assert_eq!(c.clock_rate_get(9), Ok(0x1_1000_0000));
        assert_eq!(sent(&c), [0x06, 0x50, 0, 0, 9, 0, 0, 0]);
        c.clock_config_set(9, CLOCK_ENABLE).unwrap();
        assert_eq!(sent(&c), [0x07, 0x50, 0x04, 0, 9, 0, 0, 0, 1, 0, 0, 0]);
    }

    fn name_words(s: &str) -> [u32; 4] {
        let mut b = [0u8; 16];
        b[..s.len()].copy_from_slice(s.as_bytes());
        [0, 4, 8, 12].map(|i| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]))
    }

    #[test]
    fn voltage_and_clock_ids() {
        // DEN0056 4.8 / 4.6, and OP-TEE voltage_domain.h:20-25, clock.h:111-118.
        assert_eq!(PROTO_VOLTAGE, 0x17);
        assert_eq!(VOLTAGE_DOMAIN_ATTRIBUTES, 3);
        assert_eq!(VOLTAGE_DESCRIBE_LEVELS, 4);
        assert_eq!(VOLTAGE_CONFIG_SET, 5);
        assert_eq!(VOLTAGE_CONFIG_GET, 6);
        assert_eq!(VOLTAGE_LEVEL_SET, 7);
        assert_eq!(VOLTAGE_LEVEL_GET, 8);
        assert_eq!(VOLTAGE_CONFIG_ON, 7);
        assert_eq!(VOLTAGE_CONFIG_OFF, 0);
        assert_eq!(CLOCK_RATE_ASYNC, 1);
        assert_eq!(CLOCK_RATE_ROUND_UP, 4);
        // proto 0x17 << 10 = 0x5C00
        assert_eq!(header(PROTO_VOLTAGE, VOLTAGE_LEVEL_GET, 0), 0x5C08);
    }

    #[test]
    fn voltage_protocol_attributes_bytes() {
        let mut c = Client::new(fake(0, &[0x0001_0005]));
        assert_eq!(c.voltage_domain_count(), Ok(5));
        assert_eq!(sent(&c), [0x01, 0x5C, 0x00, 0x00]);
    }

    #[test]
    fn voltage_name_bytes() {
        let w = name_words("regu2");
        let mut c = Client::new(fake(0, &[0, w[0], w[1], w[2], w[3]]));
        assert_eq!(c.voltage_domain_name(2), Ok(*b"regu2\0\0\0\0\0\0\0\0\0\0\0"));
        assert_eq!(sent(&c), [0x03, 0x5C, 0, 0, 2, 0, 0, 0]);
        // a reply without the 16 name bytes is malformed
        let mut c = Client::new(fake(0, &[0, w[0]]));
        assert_eq!(c.voltage_domain_name(2), Err(Error::Malformed));
    }

    #[test]
    fn voltage_config_bytes() {
        let mut c = Client::new(fake(0, &[7]));
        c.voltage_config_set(1, VOLTAGE_CONFIG_ON).unwrap();
        assert_eq!(sent(&c), [0x05, 0x5C, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(c.voltage_config_get(1), Ok(VOLTAGE_CONFIG_ON));
        // token 1 now
        assert_eq!(sent(&c), [0x06, 0x5C, 0x04, 0, 1, 0, 0, 0]);
        c.voltage_config_set(1, VOLTAGE_CONFIG_OFF).unwrap();
        assert_eq!(sent(&c)[8..12], [0, 0, 0, 0]);
        // only the low four bits are the mode
        let mut c = Client::new(fake(0, &[0xf0]));
        assert_eq!(c.voltage_config_get(1), Ok(0));
    }

    #[test]
    fn voltage_level_bytes() {
        let mut c = Client::new(fake(0, &[1_800_000]));
        c.voltage_level_set(3, 1_800_000).unwrap();
        // domain, flags 0, 1800000 = 0x001B7740
        assert_eq!(sent(&c), [0x07, 0x5C, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0x40, 0x77, 0x1B, 0]);
        assert_eq!(c.voltage_level_get(3), Ok(1_800_000));
        assert_eq!(sent(&c), [0x08, 0x5C, 0x04, 0, 3, 0, 0, 0]);
    }

    #[test]
    fn voltage_errors() {
        let mut c = Client::new(fake(STATUS_INVALID_PARAMETERS, &[]));
        assert_eq!(c.voltage_config_set(9, VOLTAGE_CONFIG_ON), Err(Error::Status(-2)));
        // regulator_consumer.c:171 answers DENIED for a domain with no regulator
        let mut c = Client::new(fake(STATUS_DENIED, &[]));
        assert_eq!(c.voltage_level_set(0, 1), Err(Error::Status(-3)));
        let mut c = Client::new(fake(STATUS_NOT_FOUND, &[]));
        assert_eq!(c.voltage_domain_name(0), Err(Error::Status(-4)));
        let mut c = Client::new(fake(0, &[]));
        assert_eq!(c.voltage_level_get(0), Err(Error::Malformed));
    }

    #[test]
    fn clock_attributes_bytes() {
        let w = name_words("ck_icn_p_ltdc");
        let mut c = Client::new(fake(0, &[1, w[0], w[1], w[2], w[3]]));
        let (on, name) = c.clock_attributes(7).unwrap();
        assert!(on);
        assert_eq!(&name[..13], b"ck_icn_p_ltdc");
        assert_eq!(name[13..], [0, 0, 0]);
        assert_eq!(sent(&c), [0x03, 0x50, 0, 0, 7, 0, 0, 0]);
        let mut c = Client::new(fake(0, &[0, w[0], w[1], w[2], w[3]]));
        assert!(!c.clock_attributes(7).unwrap().0);
        let mut c = Client::new(fake(STATUS_INVALID_PARAMETERS, &[]));
        assert_eq!(c.clock_attributes(999), Err(Error::Status(-2)));
    }

    #[test]
    fn clock_rate_set_bytes() {
        let mut c = Client::new(fake(0, &[]));
        // 0x1_2345_6789 Hz: low word first; flags 0 = synchronous
        c.clock_rate_set(9, 0x1_2345_6789).unwrap();
        assert_eq!(sent(&c), [0x05, 0x50, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0x89, 0x67, 0x45, 0x23, 1, 0, 0, 0]);
        let mut c = Client::new(fake(STATUS_DENIED, &[]));
        assert_eq!(c.clock_rate_set(9, 148_500_000), Err(Error::Status(-3)));
    }

    #[test]
    fn clock_describe_rates_bytes() {
        // a list: 2 rates in this reply, 3 remaining, format 0
        let flags = 2 | 3 << 16;
        let mut c = Client::new(fake(0, &[flags, 25_000_000, 0, 0x1000_0000, 1]));
        let r = c.clock_describe_rates(4).unwrap();
        assert_eq!(sent(&c), [0x04, 0x50, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0]);
        assert!(!r.range);
        assert_eq!((r.count, r.remaining), (2, 3));
        assert_eq!(r.rates[..2], [25_000_000, 0x1_1000_0000]);
        // a range: min, max, step; format bit 12
        let flags = 3 | 1 << 12;
        let mut c = Client::new(fake(0, &[flags, 1000, 0, 2000, 0, 10, 0]));
        let r = c.clock_describe_rates(4).unwrap();
        assert!(r.range);
        assert_eq!((r.count, r.remaining), (3, 0));
        assert_eq!(r.rates[..3], [1000, 2000, 10]);
        // a count the reply does not carry, and a range of fewer than three
        let mut c = Client::new(fake(0, &[2, 1, 0]));
        assert_eq!(c.clock_describe_rates(4), Err(Error::Malformed));
        let mut c = Client::new(fake(0, &[1 | 1 << 12, 1, 0]));
        assert_eq!(c.clock_describe_rates(4), Err(Error::Malformed));
        let mut c = Client::new(fake(STATUS_NOT_SUPPORTED, &[]));
        assert_eq!(c.clock_describe_rates(4), Err(Error::Status(-1)));
    }

    #[test]
    fn reset_bytes() {
        let mut c = Client::new(fake(0, &[]));
        c.reset(75, RESET_EXPLICIT, 0).unwrap();
        // flags word is 2: explicit assert, synchronous
        assert_eq!(sent(&c), [0x04, 0x58, 0, 0, 75, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        c.reset(75, RESET_ASYNC, 0).unwrap();
        assert_eq!(sent(&c)[8..12], [1, 0, 0, 0]);
    }

    #[test]
    fn errors() {
        let mut c = Client::new(fake(STATUS_DENIED, &[]));
        assert_eq!(c.perf_level_set(1, 1), Err(Error::Status(-3)));
        let mut c = Client::new(fake(0, &[]));
        assert_eq!(c.perf_level_get(1), Err(Error::Malformed));
        let mut c = Client::new(fake(0, &[]));
        c.transport.fail = Some(-5);
        assert_eq!(c.power_get(0), Err(Error::Transport(-5)));
    }

    #[test]
    fn wrong_header_is_malformed() {
        struct Bad;
        impl Transport for Bad {
            fn transfer(&mut self, _: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
                rx[..8].fill(0xff);
                Ok(8)
            }
        }
        assert_eq!(Client::new(Bad).power_set(0, 0), Err(Error::Malformed));
    }

    #[test]
    fn too_long() {
        let mut b = [0u8; MAX_MSG];
        assert_eq!(encode(&mut b, 0, &[0; 32]), Err(Error::TooLong));
        assert_eq!(encode(&mut b, 0, &[0; 31]), Ok(128));
    }
}
