use super::*;
use crate::trace::WFP_GUID;
use compat_common::netview::wfp as store;

pub const G_SESSION: GUID = GUID::new(0x17a750b7, 0x9768, 0x3aae, [0x05, 0xb5, 0xab, 0xc3, 0x47, 0xed, 0x0f, 0x89]);
pub const G_UUID: GUID = GUID::new(0x3cd2cec2, 0x1779, 0x3016, [0x5e, 0x85, 0xe9, 0xab, 0x7b, 0x75, 0x10, 0x5f]);
pub const G_ENUM: GUID = GUID::new(0x6f12514b, 0x4528, 0x3464, [0x1f, 0x30, 0x9f, 0x05, 0x4f, 0xbc, 0x04, 0x04]);
pub const G_FORMAT: GUID = GUID::new(0x0357c792, 0x4bcb, 0x3d45, [0x2c, 0xe8, 0x1f, 0x38, 0xa0, 0x61, 0xa6, 0xa9]);
pub const G_HOLDER: GUID = GUID::new(0xaba9cbc2, 0x99a9, 0x3a51, [0xb4, 0x74, 0x69, 0x0b, 0xe7, 0x5c, 0x0b, 0x4e]);
pub const G_SA_CONTEXT_HOLDER: GUID = GUID::new(0x1713ef11, 0xc257, 0x336f, [0x57, 0xb1, 0x04, 0xd8, 0xcc, 0xc9, 0xe3, 0x48]);
pub const G_IKE_HOLDER: GUID = GUID::new(0x65383675, 0xddcd, 0x3365, [0x8b, 0x06, 0x96, 0x2f, 0xe3, 0x9e, 0xe5, 0x87]);
pub const G_SA_HOLDER: GUID = GUID::new(0xd7b79d12, 0xe848, 0x33fb, [0xf5, 0x5f, 0xd9, 0xe4, 0x0d, 0xcc, 0xf9, 0x30]);
pub const G_SA_INBOUND_HOLDER: GUID = GUID::new(0x20df95b3, 0x0dbf, 0x38c1, [0x52, 0x19, 0xc4, 0xbf, 0x3b, 0xa8, 0x43, 0xd2]);
pub const G_SA_OUTBOUND_HOLDER: GUID = GUID::new(0x3b106749, 0x7d66, 0x303e, [0x6f, 0x42, 0x60, 0x63, 0xf9, 0x14, 0x5a, 0x9a]);

pub const PROVIDER_NAME: &str = "XboxNetApiDiag WFP Provider";
pub const FWP_E_IKE_NOT_RUNNING: u32 = 0x8032_0044;
pub const ENUM_BATCH: u32 = 0x40;

#[repr(C)]
#[derive(Default)]
pub struct Session {
    vtable: usize,
    mode: u32,
    engine: u64,
    key: [u8; 16],
    existed: bool,
    txn: bool,
}

impl Session {
    fn addr(&self) -> usize {
        self as *const Session as usize
    }
}

#[repr(C)]
#[derive(Default)]
struct Enumerator {
    session: usize,
    open: bool,
    handle: u64,
    holder: [u64; 2],
    layer: [u8; 16],
    idx: u32,
    num: u32,
    entries: Vec<store::Object>,
    filter_layer: Option<[u8; 16]>,
}

impl Enumerator {
    fn addr(&self) -> usize {
        self as *const Enumerator as usize
    }
    fn entries_ptr(&self) -> usize {
        if self.entries.is_empty() {
            0
        } else {
            self.entries.as_ptr() as usize
        }
    }
}

#[derive(Clone, Copy)]
struct Spec {
    kind: store::Kind,
    create: [u16; 3],
    next: [u16; 3],
    destroy: [u16; 3],
    destroy_by_value: bool,
}

#[derive(Clone, Copy)]
struct Section {
    begin: [u16; 2],
    create_failed: u16,
    item: [u16; 2],
    end: u16,
}

fn guid_fields(g: &[u8; 16]) -> Vec<Arg<'static>> {
    let mut v = vec![u(u32::from_le_bytes(g[0..4].try_into().unwrap())), u(u16::from_le_bytes(g[4..6].try_into().unwrap()) as u32), u(u16::from_le_bytes(g[6..8].try_into().unwrap()) as u32)];
    for b in g[8..].iter() {
        v.push(u(*b as u32));
    }
    v
}

fn guid_bytes(g: &GUID) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.d1.to_le_bytes());
    b[4..6].copy_from_slice(&g.d2.to_le_bytes());
    b[6..8].copy_from_slice(&g.d3.to_le_bytes());
    b[8..].copy_from_slice(&g.d4);
    b
}

fn args<'a>(head: &[Arg<'a>], mid: Vec<Arg<'a>>, tail: &[Arg<'a>]) -> Vec<Arg<'a>> {
    let mut v = head.to_vec();
    v.extend(mid);
    v.extend_from_slice(tail);
    v
}

unsafe fn sw(id: u16, a: &[Arg]) {
    st(&WFP_GUID, id, a)
}

unsafe fn ew(id: u16, a: &[Arg]) {
    er(&WFP_GUID, id, a)
}

unsafe fn construct(e: &Enumerator) {
    let a = e.addr();
    st(&G_ENUM, 0xa, &[p(a)]);
    st(&G_ENUM, 0xb, &[p(a), p(a)]);
    st(&G_ENUM, 0xd, &[p(a)]);
}

unsafe fn destruct(e: &mut Enumerator, s: &Session, spec: &Spec) {
    let a = e.addr();
    st(&G_ENUM, 0xe, &[p(a)]);
    st(&G_ENUM, 0xf, &[p(a), p(a)]);
    close(e, s, spec);
    st(&G_ENUM, 0x10, &[p(a)]);
}

unsafe fn release_entries(e: &mut Enumerator) {
    e.entries = Vec::new();
    e.idx = 0;
    e.num = 0;
}

unsafe fn close(e: &mut Enumerator, s: &Session, spec: &Spec) {
    let a = e.addr();
    let o = s.addr();
    st(&G_ENUM, 0x11, &[p(a)]);
    if e.open {
        if !e.entries.is_empty() {
            st(&G_ENUM, 0x12, &[p(a), p(e.entries_ptr())]);
            release_entries(e);
        }
        st(&G_ENUM, 0x14, &[p(a), Arg::U64(e.handle)]);
        let h = if spec.destroy_by_value { Arg::U64(e.handle) } else { p(&e.handle as *const u64 as usize) };
        tr(&G_SESSION, spec.destroy[0], &[p(o), h]);
        let r = store::destroy_enum_handle(s.engine, e.handle);
        if r != 0 {
            er(&G_SESSION, spec.destroy[1], &[p(o), u(r), u(r), Arg::A(None)]);
        }
        tr(&G_SESSION, spec.destroy[2], &[p(o), u(hr_map(r))]);
        e.open = false;
        e.handle = 0;
    }
    st(&G_ENUM, 0x18, &[p(a)]);
}

unsafe fn create(e: &mut Enumerator, s: &Session, tmpl: usize, spec: &Spec) -> u32 {
    let a = e.addr();
    let o = s.addr();
    e.session = o;
    st(&G_ENUM, 0x2b, &[p(a), p(o), p(tmpl)]);
    close(e, s, spec);
    st(&G_ENUM, 0x2d, &[p(a)]);
    tr(&G_SESSION, spec.create[0], &[p(o), p(tmpl), p(&e.handle as *const u64 as usize)]);
    let made = if spec.kind == store::Kind::Filter { store::create_filter_enum_handle(s.engine, e.filter_layer) } else { store::create_enum_handle(s.engine, spec.kind) };
    let hr = match made {
        Ok(h) => {
            e.handle = h;
            0
        }
        Err(r) => {
            er(&G_SESSION, spec.create[1], &[p(o), u(r), u(r), Arg::A(None)]);
            hr_map(r)
        }
    };
    tr(&G_SESSION, spec.create[2], &[p(o), u(hr)]);
    if hr != 0 {
        er(&G_ENUM, 0x2f, &[p(a)]);
    } else {
        e.open = true;
        st(&G_ENUM, 0x31, &[p(a), Arg::U64(e.handle)]);
    }
    st(&G_ENUM, 0x34, &[p(a), u(hr)]);
    hr
}

unsafe fn next(e: &mut Enumerator, s: &Session, spec: &Spec, out: usize) -> (u32, Option<store::Object>) {
    let a = e.addr();
    let o = s.addr();
    st(&G_ENUM, 0x19, &[p(a), p(out)]);
    let mut hr = 0u32;
    if !e.open {
        er(&G_ENUM, 0x1a, &[p(a)]);
        hr = HR_NO_DATA;
    } else if e.idx >= e.num {
        if !e.entries.is_empty() {
            st(&G_ENUM, 0x1c, &[p(a), p(e.entries_ptr())]);
            release_entries(e);
        }
        st(&G_ENUM, 0x21, &[p(a), u(ENUM_BATCH)]);
        let entries_at = &e.entries as *const Vec<store::Object> as usize;
        let num_at = &e.num as *const u32 as usize;
        tr(&G_SESSION, spec.next[0], &[p(o), Arg::U64(e.handle), u(ENUM_BATCH), p(entries_at), p(num_at)]);
        match store::enum_next(s.engine, e.handle, ENUM_BATCH) {
            Ok(v) => {
                e.num = v.len() as u32;
                e.entries = v;
            }
            Err(r) => {
                er(&G_SESSION, spec.next[1], &[p(o), u(r), u(r), Arg::A(None)]);
                hr = hr_map(r);
            }
        }
        tr(&G_SESSION, spec.next[2], &[p(o), u(hr)]);
        if hr != 0 {
            er(&G_ENUM, 0x23, &[p(a)]);
        } else if e.num == 0 {
            st(&G_ENUM, 0x24, &[p(a)]);
            hr = HR_NO_MORE_ITEMS;
        } else {
            st(&G_ENUM, 0x25, &[p(a), u(e.num), p(e.entries_ptr())]);
            e.idx = 0;
        }
    }
    let mut item = None;
    if hr == 0 {
        st(&G_ENUM, 0x28, &[p(a), u(e.idx), u(e.num)]);
        item = Some(e.entries[e.idx as usize].clone());
        e.idx += 1;
    }
    st(&G_ENUM, 0x29, &[p(a), u(hr)]);
    (hr, item)
}

pub use super::wfp_format::{format_object, format_provider};

unsafe fn run_simple(s: &Session, sec: Section, spec: Spec, tmpl: usize) -> u32 {
    let mut e = Enumerator::default();
    construct(&e);
    sw(sec.begin[0], &[Z]);
    sw(sec.begin[1], &[Z]);
    let mut hr = create(&mut e, s, tmpl, &spec);
    if hr != 0 {
        ew(sec.create_failed, &[Z]);
    } else {
        let mut i = 0u32;
        loop {
            let out = 0usize;
            let (h, item) = next(&mut e, s, &spec, &out as *const usize as usize);
            hr = h;
            if hr == 0 {
                sw(sec.item[0], &[Z]);
                sw(sec.item[1], &[Z, u(i)]);
                if let Some(it) = item.as_ref() {
                    format_object(it);
                }
                i += 1;
                continue;
            }
            if hr == HR_NO_MORE_ITEMS {
                close(&mut e, s, &spec);
                hr = 0;
            }
            break;
        }
    }
    sw(sec.end, &[Z, u(hr)]);
    destruct(&mut e, s, &spec);
    hr
}

const PROVIDERS: (Section, Spec) = (
    Section { begin: [0xa, 0xb], create_failed: 0xc, item: [0xd, 0xe], end: 0xf },
    Spec { kind: store::Kind::Provider, create: [0x5a, 0x5b, 0x5c], next: [0x5d, 0x5e, 0x5f], destroy: [0x60, 0x61, 0x62], destroy_by_value: true },
);
const PROVIDER_CONTEXTS: (Section, Spec) = (
    Section { begin: [0x10, 0x11], create_failed: 0x12, item: [0x13, 0x14], end: 0x15 },
    Spec { kind: store::Kind::ProviderContext, create: [0x7d, 0x7e, 0x7f], next: [0x80, 0x83, 0x84], destroy: [0x85, 0x86, 0x87], destroy_by_value: false },
);
const SUBLAYERS: (Section, Spec) = (
    Section { begin: [0x16, 0x17], create_failed: 0x18, item: [0x19, 0x1a], end: 0x1b },
    Spec { kind: store::Kind::SubLayer, create: [0x98, 0x99, 0x9a], next: [0x9b, 0x9c, 0x9d], destroy: [0x9e, 0x9f, 0xa0], destroy_by_value: false },
);
const CALLOUTS: (Section, Spec) = (
    Section { begin: [0x1c, 0x1d], create_failed: 0x1e, item: [0x1f, 0x20], end: 0x21 },
    Spec { kind: store::Kind::Callout, create: [0xa1, 0xa2, 0xa3], next: [0xa4, 0xa5, 0xa6], destroy: [0xa7, 0xa8, 0xa9], destroy_by_value: false },
);
const ALE_ENDPOINTS: (Section, Spec) = (
    Section { begin: [0x96, 0x97], create_failed: 0x98, item: [0x99, 0x9a], end: 0x9b },
    Spec { kind: store::Kind::AleEndpoint, create: [0x13c, 0x13d, 0x13e], next: [0x13f, 0x140, 0x141], destroy: [0x142, 0x143, 0x144], destroy_by_value: false },
);
const FILTERS: Spec = Spec { kind: store::Kind::Filter, create: [0xc3, 0xc4, 0xc5], next: [0xc6, 0xc7, 0xc8], destroy: [0xc9, 0xca, 0xcb], destroy_by_value: false };
const SA_CONTEXTS: Spec = Spec { kind: store::Kind::IpsecSaContext, create: [0xf7, 0xf8, 0xf9], next: [0xfa, 0xfb, 0xfc], destroy: [0xfd, 0xfe, 0xff], destroy_by_value: false };
const IPSEC_SAS: Spec = Spec { kind: store::Kind::IpsecSa, create: [0x109, 0x10a, 0x10b], next: [0x10c, 0x10d, 0x10e], destroy: [0x10f, 0x110, 0x111], destroy_by_value: false };
const IKE_SAS: Spec = Spec { kind: store::Kind::IkeSa, create: [0x128, 0x129, 0x12a], next: [0x12b, 0x12e, 0x12f], destroy: [0x130, 0x131, 0x132], destroy_by_value: false };

pub const FILTER_LAYERS: [(u16, Option<GUID>); 17] = [
    (0x23, None),
    (0x25, Some(GUID::new(0xe1cd9fe7, 0xf4b5, 0x4273, [0x96, 0xc0, 0x59, 0x2e, 0x48, 0x7b, 0x86, 0x50]))),
    (0x2d, Some(GUID::new(0xa3b42c97, 0x9f04, 0x4672, [0xb8, 0x7e, 0xce, 0xe9, 0xc4, 0x83, 0x25, 0x7f]))),
    (0x31, Some(GUID::new(0xb14b7bdb, 0xdbbd, 0x473e, [0xbe, 0xd4, 0x8b, 0x47, 0x08, 0xd4, 0xf2, 0x70]))),
    (0x35, Some(GUID::new(0xb64786b3, 0xf687, 0x4eb9, [0x89, 0xd2, 0x8e, 0xf3, 0x2a, 0xcd, 0xab, 0xe2]))),
    (0x39, Some(GUID::new(0xeda65c74, 0x610d, 0x4bc5, [0x94, 0x8f, 0x3c, 0x4f, 0x89, 0x55, 0x68, 0x67]))),
    (0x3f, Some(GUID::new(0x13c48442, 0x8d87, 0x4261, [0x9a, 0x29, 0x59, 0xd2, 0xab, 0xc3, 0x48, 0xb4]))),
    (0x43, Some(GUID::new(0x09e61aea, 0xd214, 0x46e2, [0x9b, 0x21, 0xb2, 0x6b, 0x0b, 0x2f, 0x28, 0xc8]))),
    (0x49, Some(GUID::new(0xe1735bde, 0x013f, 0x4655, [0xb3, 0x51, 0xa4, 0x9e, 0x15, 0x76, 0x2d, 0xf0]))),
    (0x4f, Some(GUID::new(0xc38d57d1, 0x05a7, 0x4c33, [0x90, 0x4f, 0x7f, 0xbc, 0xee, 0xe6, 0x0e, 0x82]))),
    (0x59, Some(GUID::new(0x4a72393b, 0x319f, 0x44bc, [0x84, 0xc3, 0xba, 0x54, 0xdc, 0xb3, 0xb6, 0xb4]))),
    (0x63, Some(GUID::new(0xf02b1526, 0xa459, 0x4a51, [0xb9, 0xe3, 0x75, 0x9d, 0xe5, 0x2b, 0x9d, 0x2c]))),
    (0x69, Some(GUID::new(0x2f755cf6, 0x2fd4, 0x4e88, [0xb3, 0xe4, 0xa9, 0x1b, 0xca, 0x49, 0x52, 0x35]))),
    (0x6b, Some(GUID::new(0x5926dfc8, 0xe3cf, 0x4426, [0xa2, 0x83, 0xdc, 0x39, 0x3f, 0x5d, 0x0f, 0x9d]))),
    (0x73, Some(GUID::new(0x634a869f, 0xfc23, 0x4b90, [0xb0, 0xc1, 0xbf, 0x62, 0x0a, 0x36, 0xae, 0x6f]))),
    (0x77, Some(GUID::new(0x1247d66d, 0x0b60, 0x4a15, [0x8d, 0x44, 0x71, 0x55, 0xd0, 0xf5, 0x3a, 0x0c]))),
    (0x79, Some(GUID::new(0x55a650e1, 0x5f0a, 0x4eca, [0xa6, 0x53, 0x88, 0xf5, 0x3b, 0x26, 0xaa, 0x8c]))),
];

#[repr(C)]
struct FilterTemplate {
    provider_key: usize,
    layer_key: [u8; 16],
    enum_type: u32,
    flags: u32,
    provider_context: usize,
    condition_count: u32,
    conditions: usize,
    action_mask: u32,
    callout_key: usize,
}

unsafe fn holder_set(at: usize, item: usize) {
    tr(&G_HOLDER, 0xa, &[p(at), p(item)]);
    tr(&G_HOLDER, 0xb, &[p(at)]);
}

unsafe fn filter_next(e: &mut Enumerator, s: &Session) -> (u32, Option<store::Object>) {
    let out = 0usize;
    let out_at = &out as *const usize as usize;
    st(&G_ENUM, 0x3d, &[p(e.addr()), p(out_at)]);
    let local = 0usize;
    let (hr, item) = next(e, s, &FILTERS, &local as *const usize as usize);
    let item_at = item.as_ref().map(|i| i as *const store::Object as usize).unwrap_or(0);
    holder_set(e.addr() + 0x20, item_at);
    if hr != 0 {
        st(&G_ENUM, 0x40, &[p(e.addr()), u(hr)]);
    }
    (hr, item)
}

unsafe fn run_filters(s: &Session) -> u32 {
    let mut e = Enumerator::default();
    construct(&e);
    sw(0x22, &[Z]);
    let mut hr = 0u32;
    'cases: for (pre, layer) in FILTER_LAYERS.iter() {
        sw(*pre, &[Z]);
        let a = e.addr();
        let o = s.addr();
        let c = match layer {
            None => {
                st(&G_ENUM, 0x35, &[p(a), p(o)]);
                e.filter_layer = None;
                let h = create(&mut e, s, 0, &FILTERS);
                st(&G_ENUM, 0x36, &[p(a), u(h)]);
                h
            }
            Some(g) => {
                let key = guid_bytes(g);
                st(&G_ENUM, 0x3a, &[p(a), p(o), p(g as *const GUID as usize), Arg::U64(0), u(0), Arg::U64(0), u(0)]);
                let tmpl = FilterTemplate { provider_key: 0, layer_key: key, enum_type: 1, flags: 2, provider_context: 0, condition_count: 0, conditions: 0, action_mask: 0xffff_ffff, callout_key: 0 };
                e.layer = [0; 16];
                e.filter_layer = Some(key);
                let mut head = vec![p(a), u(0)];
                head.extend((0..10).map(|_| u(0)));
                st(&G_ENUM, 0x3b, &args(&head, guid_fields(&key), &[u(0), u(0)]));
                let h = create(&mut e, s, &tmpl as *const FilterTemplate as usize, &FILTERS);
                st(&G_ENUM, 0x3c, &[p(a), u(h)]);
                h
            }
        };
        if c != 0 {
            ew(pre + 1, &[Z]);
            hr = c;
            break;
        }
        let mut i = 0u32;
        loop {
            let (h, item) = filter_next(&mut e, s);
            if h == 0 {
                let n = i + 1;
                if n & 0x3f == 0 {
                    Sleep(0x10);
                }
                sw(0x7b, &[Z]);
                sw(0x7c, &[Z, u(i)]);
                if let Some(it) = item.as_ref() {
                    format_object(it);
                }
                i = n;
                continue;
            }
            if h != HR_NO_MORE_ITEMS {
                hr = h;
                break 'cases;
            }
            close(&mut e, s, &FILTERS);
            sw(0x7d, &[Z]);
            sw(0x7e, &[Z, u(i)]);
            break;
        }
    }
    sw(0x7f, &[Z, u(hr)]);
    destruct(&mut e, s, &FILTERS);
    hr
}

unsafe fn empty_pair(guid: &GUID, at: usize) {
    tr(guid, 0xa, &[p(at), Arg::U64(0)]);
    tr(guid, 0xb, &[p(at)]);
}

unsafe fn empty_sa(at: usize) {
    tr(&G_SA_HOLDER, 0xa, &[p(at), Arg::U64(0)]);
    tr(&G_SA_HOLDER, 0xc, &[p(at)]);
    empty_pair(&G_SA_INBOUND_HOLDER, at + 8);
    empty_pair(&G_SA_OUTBOUND_HOLDER, at + 0x10);
    empty_pair(&G_HOLDER, at + 0x28);
    tr(&G_SA_HOLDER, 0xd, &[p(at)]);
}

unsafe fn sa_context_holder(item: Option<&store::Object>) {
    let h = [0u64; 0x10];
    let at = h.as_ptr() as usize;
    tr(&G_SA_CONTEXT_HOLDER, 0xa, &[p(at), p(item.map(|i| i as *const store::Object as usize).unwrap_or(0))]);
    tr(&G_SA_CONTEXT_HOLDER, 0xc, &[p(at)]);
    empty_sa(at + 8);
    empty_sa(at + 0x38);
    tr(&G_SA_CONTEXT_HOLDER, 0xd, &[p(at)]);
}

unsafe fn ike_holder(item: Option<&store::Object>) {
    let h = [0u64; 4];
    let at = h.as_ptr() as usize;
    tr(&G_IKE_HOLDER, 0xa, &[p(at), p(item.map(|i| i as *const store::Object as usize).unwrap_or(0))]);
    tr(&G_IKE_HOLDER, 0xf, &[p(at)]);
    tr(&G_IKE_HOLDER, 0x10, &[p(at)]);
}

unsafe fn run_ipsec(s: &Session) -> u32 {
    sw(0x87, &[Z, u(1), u(1)]);
    let hr = 'run: {
        let mut e1 = Enumerator::default();
        construct(&e1);
        sw(0x88, &[Z]);
        let h = create(&mut e1, s, 0, &SA_CONTEXTS);
        if h != 0 {
            ew(0x89, &[Z]);
            destruct(&mut e1, s, &SA_CONTEXTS);
            break 'run h;
        }
        let mut i = 0u32;
        loop {
            let local = 0usize;
            st(&G_ENUM, 0x49, &[p(e1.addr()), p(&local as *const usize as usize)]);
            let (h, item) = next(&mut e1, s, &SA_CONTEXTS, &local as *const usize as usize);
            sa_context_holder(item.as_ref());
            st(&G_ENUM, 0x4a, &[p(e1.addr()), u(h)]);
            if h == 0 {
                sw(0x8a, &[Z, u(i)]);
                if let Some(it) = item.as_ref() {
                    format_object(it);
                }
                i += 1;
                continue;
            }
            if h != HR_NO_MORE_ITEMS {
                destruct(&mut e1, s, &SA_CONTEXTS);
                break 'run h;
            }
            close(&mut e1, s, &SA_CONTEXTS);
            destruct(&mut e1, s, &SA_CONTEXTS);
            break;
        }
        let mut e2 = Enumerator::default();
        construct(&e2);
        sw(0x8b, &[Z]);
        let h = create(&mut e2, s, 0, &IPSEC_SAS);
        if h != 0 {
            ew(0x8c, &[Z]);
            destruct(&mut e2, s, &IPSEC_SAS);
            break 'run h;
        }
        let mut i = 0u32;
        loop {
            let local = 0usize;
            let (h, item) = next(&mut e2, s, &IPSEC_SAS, &local as *const usize as usize);
            if h == 0 {
                sw(0x8d, &[Z, u(i)]);
                if let Some(it) = item.as_ref() {
                    format_object(it);
                }
                i += 1;
                continue;
            }
            if h != HR_NO_MORE_ITEMS {
                destruct(&mut e2, s, &IPSEC_SAS);
                break 'run h;
            }
            close(&mut e2, s, &IPSEC_SAS);
            destruct(&mut e2, s, &IPSEC_SAS);
            break;
        }
        let mut e3 = Enumerator::default();
        construct(&e3);
        sw(0x91, &[Z]);
        let h = create(&mut e3, s, 0, &IKE_SAS);
        if h != 0 {
            if h == FWP_E_IKE_NOT_RUNNING {
                sw(0x93, &[Z]);
                destruct(&mut e3, s, &IKE_SAS);
                break 'run 0;
            }
            ew(0x92, &[Z]);
            destruct(&mut e3, s, &IKE_SAS);
            break 'run h;
        }
        let mut i = 0u32;
        loop {
            let local = 0usize;
            st(&G_ENUM, 0x53, &[p(e3.addr()), p(&local as *const usize as usize)]);
            let (h, item) = next(&mut e3, s, &IKE_SAS, &local as *const usize as usize);
            ike_holder(item.as_ref());
            st(&G_ENUM, 0x54, &[p(e3.addr()), u(h)]);
            if h == 0 {
                sw(0x94, &[Z, u(i)]);
                if let Some(it) = item.as_ref() {
                    format_object(it);
                }
                i += 1;
                continue;
            }
            if h != HR_NO_MORE_ITEMS {
                destruct(&mut e3, s, &IKE_SAS);
                break 'run h;
            }
            close(&mut e3, s, &IKE_SAS);
            destruct(&mut e3, s, &IKE_SAS);
            break 'run 0;
        }
    };
    sw(0x95, &[Z, u(hr)]);
    hr
}

unsafe fn cleanup(s: &mut Session) {
    let o = s.addr();
    tr(&G_SESSION, 0x27, &[p(o), u(0)]);
    tr(&G_SESSION, 0x49, &[p(o)]);
    if s.txn {
        tr(&G_SESSION, 0x4a, &[p(o)]);
        let r = store::transaction_abort(s.engine);
        if r != 0 {
            er(&G_SESSION, 0x4b, &[p(o), u(r), u(r), Arg::A(None)]);
        } else {
            s.txn = false;
        }
    } else {
        tr(&G_SESSION, 0x4d, &[p(o)]);
    }
    tr(&G_SESSION, 0x4e, &[p(o)]);
    if s.key.iter().any(|b| *b != 0) {
        let key = s.key;
        let tail = [u(s.mode), u(0), u(0)];
        if s.existed {
            tr(&G_SESSION, 0x2f, &args(&[p(o)], guid_fields(&key), &tail));
        } else {
            tr(&G_SESSION, 0x2d, &args(&[p(o)], guid_fields(&key), &tail));
            let r = store::provider_delete_by_key(s.engine, &key);
            if r != 0 {
                er(&G_SESSION, 0x2e, &args(&[p(o)], guid_fields(&key), &[u(r), u(r), Arg::A(None)]));
            }
        }
        s.key = [0; 16];
    }
    if s.engine != 0 {
        tr(&G_SESSION, 0x35, &[p(o)]);
        store::engine_close(s.engine);
        s.engine = 0;
    }
    tr(&G_SESSION, 0x36, &[p(o)]);
}

unsafe fn open(s: &mut Session, key: &[u8; 16], key_at: usize) -> u32 {
    let o = s.addr();
    let name = ws(PROVIDER_NAME);
    tr(&G_SESSION, 0x12, &[p(o), p(key_at), u(0), w(&name)]);
    s.mode = 0;
    tr(&G_SESSION, 0x16, &[p(o), u(0)]);
    let hr = 'body: {
        match store::engine_open(store::FWPM_SESSION_FLAG_DYNAMIC) {
            Ok(h) => s.engine = h,
            Err(r) => {
                er(&G_SESSION, 0x17, &[p(o), u(r), u(r), Arg::A(None)]);
                let hr = hr_map(r);
                cleanup(s);
                break 'body hr;
            }
        }
        let existed = match store::provider_get_by_key(s.engine, key) {
            Ok(found) => {
                let fname = found.name.clone().unwrap_or_default();
                tr(&G_SESSION, 0x1c, &args(&[p(o)], guid_fields(key), &[w(&fname)]));
                let persistent = found.flags & store::FWPM_PROVIDER_FLAG_PERSISTENT != 0;
                if s.mode == 2 && !persistent {
                    er(&G_SESSION, 0x1d, &[p(o)]);
                } else if s.mode != 2 && persistent {
                    er(&G_SESSION, 0x1e, &[p(o)]);
                }
                true
            }
            Err(r) => {
                tr(&G_SESSION, 0x19, &args(&[p(o)], guid_fields(key), &[u(r), u(r), Arg::A(None)]));
                let provider = store::Provider { key: *key, name: Some(name.clone()), description: None, flags: if s.mode == 2 { store::FWPM_PROVIDER_FLAG_PERSISTENT } else { 0 }, data: Vec::new(), service_name: None };
                let r = store::provider_add(s.engine, provider);
                if r != 0 {
                    er(&G_SESSION, 0x1a, &[p(o), u(r), u(r), Arg::A(None)]);
                    let hr = hr_map(r);
                    cleanup(s);
                    break 'body hr;
                }
                tr(&G_SESSION, 0x1b, &args(&[p(o)], guid_fields(key), &[w(&name)]));
                false
            }
        };
        s.key = *key;
        s.existed = existed;
        0
    };
    tr(&G_SESSION, 0x26, &[p(o), u(hr)]);
    hr
}

#[link(name = "rpcrt4")]
extern "system" {
    fn UuidCreate(uuid: *mut [u8; 16]) -> i32;
}

pub unsafe fn log_wfp() -> u32 {
    let mut s = Session::default();
    let o = s.addr();
    tr(&G_SESSION, 0xa, &[p(o)]);
    tr(&G_SESSION, 0xd, &[p(o), p(o)]);
    tr(&G_SESSION, 0xe, &[p(o)]);
    sw(0x9c, &[Z, u(1), u(1), u(1)]);
    let mut key = [0u8; 16];
    let status = UuidCreate(&mut key) as u32;
    let hr = 'run: {
        if status != 0 {
            er(&G_UUID, 0xa, &[Z, u(status)]);
            key = [0; 16];
            let hr = hr_map(status);
            if hr != 0 {
                ew(0x9d, &[Z]);
                break 'run hr;
            }
        }
        let key_at = &key as *const [u8; 16] as usize;
        let hr = open(&mut s, &key, key_at);
        if hr != 0 {
            ew(0x9e, &[Z]);
            break 'run hr;
        }
        let mut sublayer_template = [0usize; 1];
        let steps: [(u16, (Section, Spec), usize); 4] = [(0x9f, PROVIDERS, 0), (0xa0, PROVIDER_CONTEXTS, 0), (0xa1, SUBLAYERS, sublayer_template.as_mut_ptr() as usize), (0xa2, CALLOUTS, 0)];
        for (fail, (sec, spec), tmpl) in steps.iter() {
            let hr = run_simple(&s, *sec, *spec, *tmpl);
            if hr != 0 {
                ew(*fail, &[Z]);
                break 'run hr;
            }
        }
        let hr = run_filters(&s);
        if hr != 0 {
            ew(0xa3, &[Z]);
            break 'run hr;
        }
        let hr = run_ipsec(&s);
        if hr != 0 {
            ew(0xa5, &[Z]);
            break 'run hr;
        }
        let hr = run_simple(&s, ALE_ENDPOINTS.0, ALE_ENDPOINTS.1, 0);
        if hr != 0 {
            ew(0xa6, &[Z]);
            break 'run hr;
        }
        0
    };
    cleanup(&mut s);
    sw(0xa7, &[Z, u(hr)]);
    tr(&G_SESSION, 0xf, &[p(o)]);
    tr(&G_SESSION, 0x10, &[p(o), p(o)]);
    cleanup(&mut s);
    tr(&G_SESSION, 0x11, &[p(o)]);
    hr
}
