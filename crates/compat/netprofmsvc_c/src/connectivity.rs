use console_sandbox::wnf;

pub const WNF_NLM_4: u64 = 0x418b_0223_a3bc_2075;
pub const WNF_CNET_3: u64 = 0x1583_002e_a3bc_1875;
pub const WNF_CNET_9: u64 = 0x1583_002e_a3bc_4875;
pub const WNF_CNET_13: u64 = 0x1583_002e_a3bc_6875;
pub const WNF_CNET_14: u64 = 0x1583_002e_a3bc_7075;
pub const WNF_DUSM_2: u64 = 0x0c95_1b29_a3bc_1075;
pub const WNF_SEB_4: u64 = 0x4184_0b3e_a3bc_2075;

pub const CONNECTED: u32 = 0x1;
pub const DISCONNECTED: u32 = 0x2;
pub const IPV4_LOCAL: u32 = 0x4;
pub const IPV4_INTERNET: u32 = 0x8;
pub const IPV6_LOCAL: u32 = 0x10;
pub const IPV6_INTERNET: u32 = 0x20;
pub const IPV4_WEB_HIJACK: u32 = 0x40;
pub const IPV6_WEB_HIJACK: u32 = 0x80;
pub const IPV4_AUTH_PROXY: u32 = 0x100;
pub const IPV6_AUTH_PROXY: u32 = 0x200;

pub const LEVEL_NONE: u32 = 0;
pub const LEVEL_LOCAL: u32 = 1;
pub const LEVEL_INTERNET: u32 = 2;
pub const DETAIL_WEB_HIJACK: u32 = 0x1;
pub const DETAIL_AUTH_PROXY: u32 = 0x80;

pub const COST_UNRESTRICTED: u32 = 0x1;
pub const COST_FIXED: u32 = 0x2;
pub const COST_VARIABLE: u32 = 0x4;
pub const COST_OVER_DATA_LIMIT: u32 = 0x1_0000;
pub const COST_CONGESTED: u32 = 0x2_0000;
pub const COST_ROAMING: u32 = 0x4_0000;
pub const COST_APPROACHING_DATA_LIMIT: u32 = 0x8_0000;

pub const SEB_NETWORK_EVENT: [u8; 16] = [0x36, 0x03, 0x2e, 0xb0, 0x13, 0x9e, 0x74, 0x4e, 0x8a, 0x0a, 0x7f, 0x04, 0xa2, 0x73, 0xad, 0xe8];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Family {
    pub level: u32,
    pub timestamp: u32,
    pub detail: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    pub connected: bool,
    pub identified: bool,
    pub hidden: bool,
    pub v4: Family,
    pub v6: Family,
    pub cost: Option<u32>,
}

impl Interface {
    fn counted(&self) -> bool {
        self.connected && self.identified && !self.hidden
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aggregate {
    pub connected: bool,
    pub v4_level: u32,
    pub v6_level: u32,
    pub v4_detail: u32,
    pub v6_detail: u32,
    pub cost_low: u32,
    pub cost_high: u32,
}

pub fn accumulate(ifaces: &[Interface]) -> Aggregate {
    let mut a = Aggregate::default();
    for i in ifaces {
        if !i.counted() {
            continue;
        }
        a.connected = true;
        a.v4_level = a.v4_level.max(i.v4.level);
        a.v6_level = a.v6_level.max(i.v6.level);
        a.v4_detail |= i.v4.detail;
        a.v6_detail |= i.v6.detail;
        if let Some(c) = i.cost {
            a.cost_low = a.cost_low.max(c & 0xffff);
            a.cost_high |= c & 0xffff_0000;
        }
    }
    a
}

pub fn flags(a: &Aggregate) -> u32 {
    let mut f = if a.connected { CONNECTED } else { DISCONNECTED };
    match a.v4_level {
        LEVEL_INTERNET => f |= IPV4_INTERNET,
        LEVEL_LOCAL => f |= IPV4_LOCAL,
        _ => {}
    }
    if a.v4_detail & DETAIL_WEB_HIJACK != 0 {
        f |= IPV4_WEB_HIJACK;
    }
    if a.v4_detail & DETAIL_AUTH_PROXY != 0 {
        f |= IPV4_AUTH_PROXY;
    }
    match a.v6_level {
        LEVEL_INTERNET => f |= IPV6_INTERNET,
        LEVEL_LOCAL => f |= IPV6_LOCAL,
        _ => {}
    }
    if a.v6_detail & DETAIL_WEB_HIJACK != 0 {
        f |= IPV6_WEB_HIJACK;
    }
    if a.v6_detail & DETAIL_AUTH_PROXY != 0 {
        f |= IPV6_AUTH_PROXY;
    }
    f
}

pub fn cost(a: &Aggregate) -> u32 {
    a.cost_low | a.cost_high
}

pub fn flag_names(f: u32) -> String {
    let names = ["Connected", "Disconnected", "IPv4Local", "IPv4Internet", "IPv6Local", "IPv6Internet", "IPv4WebHijack", "IPv6WebHijack", "IPv4AuthProx", "IPv6AuthProx"];
    let s: Vec<&str> = names.iter().enumerate().filter(|(i, _)| f & (1 << i) != 0).map(|(_, n)| *n).collect();
    if s.is_empty() {
        "Unknown".into()
    } else {
        s.join(",")
    }
}

pub fn cost_names(c: u32) -> String {
    let names = [(COST_UNRESTRICTED, "Unrestricted"), (COST_FIXED, "Fixed"), (COST_VARIABLE, "Variable"), (COST_OVER_DATA_LIMIT, "OverDataLimit"), (COST_CONGESTED, "Congested"), (COST_ROAMING, "Roaming"), (COST_APPROACHING_DATA_LIMIT, "ApproachingDataLimit")];
    let s: Vec<&str> = names.iter().filter(|(b, _)| c & b != 0).map(|(_, n)| *n).collect();
    if s.is_empty() {
        "Unknown".into()
    } else {
        s.join(",")
    }
}

pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 0x17;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub key: u32,
    pub identified: bool,
    pub hidden: bool,
    pub v4: Family,
    pub v6: Family,
}

pub fn select_families(records: &[Record], order: &[(u32, u16)]) -> (Family, Family) {
    let end = order.len();
    let (mut b4, mut b6) = (end, end);
    let (mut v4, mut v6) = (Family::default(), Family::default());
    for r in records {
        if !r.identified || r.hidden {
            continue;
        }
        let pos = order.iter().position(|(k, _)| *k == r.key).unwrap_or(end);
        let fam = if pos < end { order[pos].1 } else { 0 };
        let mut take4 = false;
        if pos < end && fam == AF_INET {
            if pos < b4 {
                b4 = pos;
                take4 = true;
            } else if pos == b4 {
                take4 = true;
            }
        } else {
            if pos < end && fam == AF_INET6 && pos < b6 {
                b6 = pos;
            }
            take4 = pos == b4 || (b4 == end && r.v4.level > v4.level);
        }
        if take4 {
            v4 = r.v4;
        }
        if pos == b6 || (b6 == end && r.v6.level > v6.level) {
            v6 = r.v6;
        }
    }
    (v4, v6)
}

pub fn connectivity_timestamp(v4: &Family, v6: &Family) -> u32 {
    let (a, b) = (v4.timestamp, v6.timestamp);
    if a != 0 && b != 0 {
        a.min(b)
    } else {
        a.max(b)
    }
}

pub fn query(agg: Option<&Aggregator>) -> (u32, u32) {
    match agg {
        Some(a) => a.read(),
        None => (DISCONNECTED, 0),
    }
}

pub struct Aggregator {
    pub flags: u32,
    pub cost: u32,
    pub publishing: bool,
    pub detail: [u8; 0x38],
}

impl Default for Aggregator {
    fn default() -> Self {
        Aggregator { flags: 0, cost: 0, publishing: false, detail: [0; 0x38] }
    }
}

pub fn publish_nlm_state(flags: u32, cost: u32) -> i32 {
    let mut d = [0u8; 8];
    d[..4].copy_from_slice(&flags.to_le_bytes());
    d[4..].copy_from_slice(&cost.to_le_bytes());
    wnf::update(WNF_NLM_4, &d, None, None)
}

impl Aggregator {
    pub fn read(&self) -> (u32, u32) {
        (self.flags, self.cost)
    }

    pub fn update(&mut self, flags: u32, cost: u32, detail: [u8; 0x38]) -> Option<i32> {
        let changed = flags != self.flags || cost != self.cost;
        let r = if changed {
            self.flags = flags;
            self.cost = cost;
            if self.publishing {
                Some(publish_nlm_state(flags, cost))
            } else {
                None
            }
        } else {
            None
        };
        self.detail = detail;
        r
    }

    pub fn set_publishing(&mut self, on: bool) -> Option<i32> {
        if on == self.publishing {
            return None;
        }
        self.publishing = on;
        Some(if on { publish_nlm_state(self.flags, self.cost) } else { publish_nlm_state(0, 0) })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostNetworkState {
    pub cnet_13: u8,
    pub cnet_9: u8,
    pub cnet_14: u8,
    pub cnet_3: u32,
    pub dusm_2: u32,
}

impl HostNetworkState {
    pub fn from_bytes(b: &[u8; 12]) -> Self {
        HostNetworkState { cnet_13: b[0], cnet_9: b[1], cnet_14: b[2], cnet_3: u32::from_le_bytes(b[4..8].try_into().unwrap()), dusm_2: u32::from_le_bytes(b[8..12].try_into().unwrap()) }
    }
}

pub fn publish_host_state(s: &HostNetworkState) -> [i32; 5] {
    [
        wnf::update(WNF_CNET_9, &[s.cnet_9], None, None),
        wnf::update(WNF_CNET_13, &[s.cnet_13], None, None),
        wnf::update(WNF_CNET_14, &[s.cnet_14], None, None),
        wnf::update(WNF_CNET_3, &s.cnet_3.to_le_bytes(), None, None),
        wnf::update(WNF_DUSM_2, &s.dusm_2.to_le_bytes(), None, None),
    ]
}

pub fn seb_payload(event: &[u8; 16], value: u32) -> [u8; 0x1c] {
    let mut p = [0u8; 0x1c];
    p[0..4].copy_from_slice(&0x50u32.to_le_bytes());
    p[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    p[8..24].copy_from_slice(event);
    p[24..28].copy_from_slice(&value.to_le_bytes());
    p
}

pub fn signal_network_event() -> i32 {
    wnf::update(WNF_SEB_4, &seb_payload(&SEB_NETWORK_EVENT, u32::MAX), None, None)
}
