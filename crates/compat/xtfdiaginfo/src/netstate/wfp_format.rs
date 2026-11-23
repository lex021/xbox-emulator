use super::guids::guid_text;
use super::wfp::G_FORMAT;
use super::wfp_tables as t;
use super::*;
use compat_common::netview::wfp as store;
use store::Value;

pub const SP4: &str = "    ";
pub const SP5: &str = "     ";
pub const SP8: &str = "        ";
pub const SP12: &str = "            ";
pub const SP16: &str = "                ";
pub const SP21: &str = "                     ";
pub const BUF: usize = 0x400;

#[link(name = "advapi32")]
extern "system" {
    fn ConvertSidToStringSidW(sid: *const u8, out: *mut *mut u16) -> BOOL;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(sd: *const u8, rev: u32, info: u32, out: *mut *mut u16, len: *mut u32) -> BOOL;
    fn IsValidSid(sid: *const u8) -> BOOL;
    fn IsValidSecurityDescriptor(sd: *const u8) -> BOOL;
    fn GetSecurityDescriptorLength(sd: *const u8) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

pub fn gs(s: &str) -> [u8; 16] {
    let h: Vec<&str> = s.split('-').collect();
    let d1 = u32::from_str_radix(h[0], 16).unwrap_or(0);
    let d2 = u16::from_str_radix(h[1], 16).unwrap_or(0);
    let d3 = u16::from_str_radix(h[2], 16).unwrap_or(0);
    let tail = format!("{}{}", h[3], h[4]);
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&d1.to_le_bytes());
    b[4..6].copy_from_slice(&d2.to_le_bytes());
    b[6..8].copy_from_slice(&d3.to_le_bytes());
    for i in 0..8 {
        b[8 + i] = u8::from_str_radix(&tail[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    b
}

pub const COND_IP_LOCAL_ADDRESS: &str = "d9ee00de-c1ef-4617-bfe3-ffd8f5a08957";
pub const COND_IP_REMOTE_ADDRESS: &str = "b235ae9a-1d64-49b8-a44c-5ff3d9095045";
pub const COND_IP_SOURCE_ADDRESS: &str = "ae96897e-2e94-4bc9-b313-b27ee80e574d";
pub const COND_IP_DESTINATION_ADDRESS: &str = "2d79133b-b390-45c6-8699-acaceaafed33";
pub const COND_IP_LOCAL_ADDRESS_TYPE: &str = "6ec7f6c4-376b-45d7-9e9c-d337cedcd237";
pub const COND_ARRIVAL_INTERFACE_TYPE: &str = "89f990de-e798-4e6d-ab76-7c9558292e6f";
pub const COND_ARRIVAL_TUNNEL_TYPE: &str = "511166dc-7a8c-4aa7-b533-95ab59fb0340";
pub const COND_FLAGS: &str = "632ce23b-5167-435c-86d7-e903684aa80c";
pub const COND_ALE_SIO_FIREWALL_SYSTEM_PORT: &str = "b9f4e088-cb98-4efb-a2c7-ad07332643db";
pub const CALLOUT_OUTBOUND_TRANSPORT: [&str; 2] = ["4b46bf0a-4523-4e57-aa38-a87987c910d9", "38d87722-ad83-4f11-a91f-df0fb077225b"];
pub const CALLOUT_INBOUND_TRANSPORT: [&str; 2] = ["5132900d-5e84-4b5f-80e4-01741e81ff10", "49d3ac92-2a6c-4dcf-955f-1c3be009dd99"];
pub const CALLOUT_CONNECT_SECURE: [&str; 4] = ["6ac141fc-f75d-4203-b9c8-48e6149c2712", "4c0dda05-e31f-4666-90b0-b3dfad34129a", "7dff309b-ba7d-4aba-91aa-ae5c6640c944", "a9a0d6d9-c58c-474e-8aeb-3cfe99d6d53d"];

fn is(k: &[u8; 16], s: &str) -> bool {
    *k == gs(s)
}

pub fn cap(s: String, count: usize) -> String {
    let w: Vec<u16> = s.encode_utf16().collect();
    if count == 0 {
        return String::new();
    }
    if w.len() < count {
        return s;
    }
    String::from_utf16_lossy(&w[..count - 1])
}

pub fn lookup(v: u32, table: &[(u32, &str)]) -> String {
    match table.iter().find(|(k, _)| *k == v) {
        Some((_, n)) => n.to_string(),
        None => format!("??? {}/0x{:08x}", v as i32, v),
    }
}

pub fn flags(v: u64, is32: bool, table: &[(u32, u64, &str)]) -> String {
    let v = if is32 { v & 0xffff_ffff } else { v };
    let mut s = if is32 { format!("0x{:08x} (", v) } else { format!("0x{:016x} (", v) };
    let mut rest = v;
    for (kind, mask, name) in table.iter() {
        if *kind != 0 {
            if mask & rest == *mask {
                s.push_str(name);
                s.push(' ');
                rest &= !mask;
            }
        } else if v == 0 {
            s.push_str(name);
            s.push(' ');
        }
    }
    if rest != 0 {
        if is32 {
            s.push_str(&format!("unrecognized_0x{:08x}", rest as u32));
        } else {
            s.push_str(&format!("unrecognized_0x{:016x}", rest));
        }
    } else if s.ends_with(' ') {
        s.pop();
    }
    s.push(')');
    s
}

pub fn ipv4(x: u32) -> String {
    format!("{}.{}.{}.{}", x >> 24, (x >> 16) & 0xff, (x >> 8) & 0xff, x & 0xff)
}

pub fn v4_of(b: &[u8; 16]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub fn ipv6(b: &[u8; 16]) -> String {
    let mut s = String::new();
    for i in 0..8 {
        if i > 0 {
            s.push(':');
        }
        s.push_str(&format!("{:02X}{:02X}", b[i * 2], b[i * 2 + 1]));
    }
    s
}

fn ptr(nonnull: bool) -> String {
    format!("{:016X}", if nonnull { REDACTED } else { 0 })
}

fn ffmt(x: f64) -> String {
    if x.is_nan() {
        "nan".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "inf".to_string() } else { "-inf".to_string() }
    } else {
        format!("{:.6}", x)
    }
}

unsafe fn take_local(p: *mut u16) -> String {
    let mut n = 0usize;
    while *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
    LocalFree(p as *mut c_void);
    s
}

fn sid_text(b: &[u8]) -> Option<String> {
    if b.len() < 8 || b.len() < 8 + 4 * b[1] as usize {
        return None;
    }
    unsafe {
        if IsValidSid(b.as_ptr()) == 0 {
            return None;
        }
        let mut out: *mut u16 = core::ptr::null_mut();
        if ConvertSidToStringSidW(b.as_ptr(), &mut out) == 0 || out.is_null() {
            return None;
        }
        Some(take_local(out))
    }
}

fn sd_text(b: &[u8]) -> Option<String> {
    if b.len() < 20 {
        return None;
    }
    unsafe {
        if IsValidSecurityDescriptor(b.as_ptr()) == 0 || GetSecurityDescriptorLength(b.as_ptr()) as usize > b.len() {
            return None;
        }
        let mut out: *mut u16 = core::ptr::null_mut();
        if ConvertSecurityDescriptorToStringSecurityDescriptorW(b.as_ptr(), 1, 0xf, &mut out, core::ptr::null_mut()) == 0 || out.is_null() {
            return None;
        }
        Some(take_local(out))
    }
}

pub fn value_text(v: &Value, field: Option<&[u8; 16]>, count: usize) -> String {
    let fk = |s: &str| field.map(|k| is(k, s)).unwrap_or(false);
    let s = match v {
        Value::Empty => "FWP_EMPTY".to_string(),
        Value::U8(x) => {
            if fk(COND_IP_LOCAL_ADDRESS_TYPE) && count > 29 {
                format!("FWP_UINT8 local address type {}", cap(lookup(*x as u32, &t::NL_ADDRESS_TYPE), count - 29))
            } else {
                format!("FWP_UINT8 uint8 0x{:02x}/{}", x, x)
            }
        }
        Value::U16(x) => format!("FWP_UINT16 uint16 0x{:04x}/{}", x, x),
        Value::U32(x) => {
            let x = *x;
            if (fk(COND_IP_LOCAL_ADDRESS) || fk(COND_IP_REMOTE_ADDRESS) || fk(COND_IP_SOURCE_ADDRESS) || fk(COND_IP_DESTINATION_ADDRESS)) && count > 24 {
                format!("FWP_UINT32 IPv4 address {}", cap(ipv4(x), count - 24))
            } else if fk(COND_FLAGS) && count > 27 {
                format!("FWP_UINT32 condition flags {}", cap(flags(x as u64, true, &t::CONDITION_FLAGS), count - 27))
            } else if fk(COND_ARRIVAL_INTERFACE_TYPE) && count > 34 {
                format!("FWP_UINT32 arrival interface type {}", cap(lookup(x, &t::IF_TYPE), count - 34))
            } else if fk(COND_ARRIVAL_TUNNEL_TYPE) && count > 31 {
                format!("FWP_UINT32 arrival tunnel type {}", cap(lookup(x, &t::TUNNEL_TYPE), count - 31))
            } else if fk(COND_ALE_SIO_FIREWALL_SYSTEM_PORT) && count > 33 {
                format!("FWP_UINT32 socket property flags {}", cap(flags(x as u64, true, &t::SIO_FIREWALL_FLAGS), count - 33))
            } else {
                format!("FWP_UINT32 uint32 0x{:08x}/{}", x, x)
            }
        }
        Value::U64(y) => format!("FWP_UINT64 uint64 0x{:016x}/{}", y, y),
        Value::I8(x) => format!("FWP_INT8 int8 0x{:02x}/{}", *x as i32 as u32, x),
        Value::I16(x) => format!("FWP_INT16 int16 0x{:04x}/{}", *x as i32 as u32, x),
        Value::I32(x) => format!("FWP_INT32 int32 0x{:08x}/{}", *x as u32, x),
        Value::I64(y) => format!("FWP_INT64 int64 0x{:016x}/{}", *y as u64, y),
        Value::F32(x) => format!("FWP_FLOAT float32 {}", ffmt(*x as f64)),
        Value::F64(x) => format!("FWP_DOUBLE double64 {}", ffmt(*x)),
        Value::Bytes16(b) => {
            if (fk(COND_IP_LOCAL_ADDRESS) || fk(COND_IP_REMOTE_ADDRESS)) && count > 35 {
                format!("FWP_BYTE_ARRAY16_TYPE IPv6 address {}", cap(ipv6(b), count - 35))
            } else {
                let parts: Vec<String> = b.iter().map(|x| format!("0x{:02X}", x)).collect();
                format!("FWP_BYTE_ARRAY16_TYPE byteArray16 0x{} = {{ {} }}", ptr(true), parts.join(", "))
            }
        }
        Value::Blob(b) => format!("FWP_BYTE_BLOB_TYPE byteBlob 0x{} {{ size {}, ptr 0x{} }}", ptr(true), b.len(), ptr(!b.is_empty())),
        Value::Sid(b) => match sid_text(b) {
            Some(x) => format!("FWP_SID sid \"{}\"", x),
            None => format!("FWP_SID sid 0x{}", ptr(!b.is_empty())),
        },
        Value::SecurityDescriptor(b) => match sd_text(b) {
            Some(x) => format!("FWP_SECURITY_DESCRIPTOR_TYPE sd \"{}\"", x),
            None => format!("FWP_SECURITY_DESCRIPTOR_TYPE sd 0x{}", ptr(true)),
        },
        Value::TokenInformation(p) => format!("FWP_TOKEN_INFORMATION_TYPE tokenInformation 0x{}", ptr(*p)),
        Value::TokenAccessInformation(p) => format!("FWP_TOKEN_ACCESS_INFORMATION_TYPE tokenAccessInformation 0x{}", ptr(*p)),
        Value::Unicode(s) => format!("FWP_UNICODE_STRING_TYPE unicodeString \"{}\"", s.as_ref().map(|x| String::from_utf16_lossy(x)).unwrap_or_else(|| "(null)".to_string())),
        Value::Bytes6(b) => {
            let parts: Vec<String> = b.iter().map(|x| format!("0x{:02X}", x)).collect();
            format!("FWP_BYTE_ARRAY6_TYPE byteArray6 0x{} = {{ {} }}", ptr(true), parts.join(", "))
        }
        Value::V4AddrMask(a, m) => format!("FWP_V4_ADDR_MASK v4AddrMask 0x{} {{addr {}, mask {})", ptr(true), cap(ipv4(*a), 0x80), cap(ipv4(*m), 0x80)),
        Value::V6AddrMask(a, l) => format!("FWP_V6_ADDR_MASK v6AddrMask 0x{} {{addr {}, prefixLength {})", ptr(true), cap(ipv6(a), 0x80), l),
        Value::Range(lo, hi) => format!("FWP_RANGE_TYPE rangeValue 0x{} {{ valueLow {}, valueHigh {}}}", ptr(true), value_text(lo, field, 0x100), value_text(hi, field, 0x100)),
        Value::Other(ty) => format!("??? type {} ???", *ty as i32),
    };
    cap(s, count)
}

unsafe fn f(id: u16, a: &[Arg]) {
    st(&G_FORMAT, id, a)
}

fn red(nonnull: bool) -> Arg<'static> {
    Arg::U64(if nonnull { REDACTED } else { 0 })
}

unsafe fn pg(id: u16, p: &str, text: &str) {
    let (pw, tw) = (ws(p), ws(text));
    f(id, &[Z, w(&pw), w(&tw)]);
}

unsafe fn pgo(id: u16, p: &str, text: Option<&[u16]>) {
    let pw = ws(p);
    f(id, &[Z, w(&pw), Arg::W(text)]);
}

unsafe fn pr(id: u16, p: &str, nonnull: bool) {
    let pw = ws(p);
    f(id, &[Z, w(&pw), red(nonnull)]);
}

unsafe fn pq(id: u16, p: &str, v: u64) {
    let pw = ws(p);
    f(id, &[Z, w(&pw), Arg::U64(v)]);
}

unsafe fn pu(id: u16, p: &str, v: u32) {
    let pw = ws(p);
    f(id, &[Z, w(&pw), u(v)]);
}

unsafe fn puu(id: u16, p: &str, a: u32, b: u32) {
    let pw = ws(p);
    f(id, &[Z, w(&pw), u(a), u(b)]);
}

unsafe fn put(id: u16, p: &str, v: u32, text: &str) {
    let (pw, tw) = (ws(p), ws(text));
    f(id, &[Z, w(&pw), u(v), w(&tw)]);
}

unsafe fn pir(id: u16, p: &str, i: u32, nonnull: bool) {
    let pw = ws(p);
    f(id, &[Z, w(&pw), u(i), red(nonnull)]);
}

unsafe fn po(id: u16, p: &str) {
    let pw = ws(p);
    f(id, &[Z, w(&pw)]);
}

unsafe fn nq(id: u16, v: u64) {
    f(id, &[Z, Arg::U64(v)]);
}

unsafe fn nr(id: u16, nonnull: bool) {
    f(id, &[Z, red(nonnull)]);
}

unsafe fn nu(id: u16, v: u32) {
    f(id, &[Z, u(v)]);
}

unsafe fn nt(id: u16, text: &str) {
    let tw = ws(text);
    f(id, &[Z, w(&tw)]);
}

unsafe fn nut(id: u16, v: u32, text: &str) {
    let tw = ws(text);
    f(id, &[Z, u(v), w(&tw)]);
}

fn cat(a: &str, b: &str) -> String {
    format!("{}{}", a, b)
}

pub unsafe fn display(name: &Option<Vec<u16>>, desc: &Option<Vec<u16>>) {
    match name {
        Some(n) => f(0xb, &[Z, w(n)]),
        None => f(0xc, &[Z]),
    }
    match desc {
        Some(n) => f(0xd, &[Z, w(n)]),
        None => f(0xe, &[Z]),
    }
}

pub fn hex_lines(data: &[u8]) -> Vec<(u32, String, String)> {
    let mut out = Vec::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        let mut hex = String::new();
        let mut asc = String::new();
        for b in chunk {
            hex.push_str(&format!("{:02X} ", b));
            asc.push(if (0x20..=0x7e).contains(b) { *b as char } else { '.' });
        }
        while hex.len() < 48 {
            hex.push(' ');
        }
        out.push(((i * 16) as u32, hex, asc));
    }
    out
}

pub unsafe fn hexdump(prefix: &str, indent: &str, data: &[u8]) {
    let (pw, iw) = (ws(prefix), ws(indent));
    for (off, hex, asc) in hex_lines(data) {
        f(0xa, &[Z, w(&pw), w(&iw), u(off), Arg::A(Some(hex.as_bytes())), Arg::A(Some(asc.as_bytes()))]);
    }
}

pub unsafe fn blob(prefix: &str, data: &[u8]) {
    pu(0x17, prefix, data.len() as u32);
    if !data.is_empty() {
        pr(0x18, prefix, true);
        hexdump(prefix, SP8, data);
    } else {
        po(0x19, prefix);
    }
}

pub unsafe fn format_provider(v: &store::Provider) {
    nt(0x1a, &guid_text(Some(&v.key)));
    nr(0x1b, true);
    display(&v.name, &v.description);
    nt(0x1c, &flags(v.flags as u64, true, &t::PROVIDER_FLAGS));
    nr(0x1d, true);
    blob(SP4, &v.data);
    match &v.service_name {
        Some(n) => f(0x1e, &[Z, w(n)]),
        None => f(0x1f, &[Z]),
    }
}

pub unsafe fn format_sublayer(v: &store::SubLayer) {
    nt(0x39, &guid_text(Some(&v.key)));
    nr(0x3a, true);
    display(&v.name, &v.description);
    nt(0x3b, &flags(v.flags as u64, true, &t::SUBLAYER_FLAGS));
    let pk = ws(&guid_text(v.provider_key.as_ref()));
    f(0x3c, &[Z, red(v.provider_key.is_some()), w(&pk)]);
    nr(0x3d, true);
    blob(SP4, &v.data);
    nu(0x3e, v.weight as u32);
}

pub unsafe fn format_callout(v: &store::Callout) {
    let p = SP4;
    pg(0x3f, p, &guid_text(Some(&v.key)));
    pr(0x40, p, true);
    display(&v.name, &v.description);
    pg(0x41, p, &flags(v.flags as u64, true, &t::CALLOUT_FLAGS));
    pg(0x42, p, &guid_text(v.provider_key.as_ref()));
    pr(0x43, p, true);
    blob(SP8, &v.data);
    pg(0x44, p, &guid_text(Some(&v.layer)));
    pu(0x45, p, v.id);
}

pub unsafe fn format_filter(p: &str, v: &store::Filter) {
    pg(0x46, p, &guid_text(Some(&v.key)));
    pr(0x47, p, true);
    display(&v.name, &v.description);
    pg(0x48, p, &flags(v.flags as u64, true, &t::FILTER_FLAGS));
    pg(0x49, p, &guid_text(v.provider_key.as_ref()));
    pr(0x4a, p, true);
    blob(&cap(cat(p, SP4), BUF), &v.data);
    pg(0x4b, p, &guid_text(Some(&v.layer)));
    pg(0x4c, p, &guid_text(Some(&v.sublayer)));
    pg(0x4d, p, &value_text(&v.weight, None, BUF));
    pu(0x4e, p, v.conditions.len() as u32);
    pr(0x4f, p, !v.conditions.is_empty());
    for (i, c) in v.conditions.iter().enumerate() {
        pir(0x50, p, i as u32, true);
        pg(0x51, p, &guid_text(Some(&c.field)));
        put(0x52, p, c.match_type, &lookup(c.match_type, &t::MATCH_TYPE));
        pg(0x53, p, &value_text(&c.value, Some(&c.field), BUF));
        if let Value::Blob(b) = &c.value {
            hexdump(p, SP21, b);
        }
    }
    pr(0x54, p, true);
    put(0x55, p, v.action_type, &lookup(v.action_type, &t::ACTION_TYPE));
    let callout = v.action_type & store::FWP_ACTION_FLAG_CALLOUT != 0;
    pg(if callout { 0x56 } else { 0x57 }, p, &guid_text(Some(&v.action_key)));
    let raw = u64::from_le_bytes(v.context[0..8].try_into().unwrap());
    if v.flags & store::FWPM_FILTER_FLAG_HAS_PROVIDER_CONTEXT != 0 {
        pg(0x58, p, &guid_text(Some(&v.context)));
    } else if callout {
        let k = &v.action_key;
        if CALLOUT_OUTBOUND_TRANSPORT.iter().any(|g| is(k, g)) {
            pg(0x59, p, &flags(raw, false, &t::OUTBOUND_TRANSPORT_FLAGS));
        } else if CALLOUT_INBOUND_TRANSPORT.iter().any(|g| is(k, g)) {
            pg(0x5a, p, &flags(raw, false, &t::INBOUND_TRANSPORT_FLAGS));
        } else if CALLOUT_CONNECT_SECURE.iter().any(|g| is(k, g)) {
            pg(0x5b, p, &flags(raw, false, &t::ALE_CONNECT_FLAGS));
        } else {
            pq(0x5c, p, raw);
        }
    } else {
        pq(0x5d, p, raw);
    }
    pg(0x5e, p, &guid_text(v.reserved.as_ref()));
    let pw = ws(p);
    f(0x5f, &[Z, w(&pw), Arg::U64(v.id), Arg::U64(v.id)]);
    pg(0x60, p, &value_text(&v.effective_weight, None, BUF));
}

pub unsafe fn format_classify_options(o: &[store::ClassifyOption]) {
    let p = SP4;
    pu(0x0f, p, o.len() as u32);
    let base = if o.is_empty() { 0u64 } else { o.as_ptr() as u64 };
    pq(0x10, p, base);
    for (i, c) in o.iter().enumerate() {
        let pw = ws(p);
        f(0x11, &[Z, w(&pw), u(i as u32), Arg::U64(c as *const store::ClassifyOption as u64)]);
        put(0x12, p, c.kind, &lookup(c.kind, &t::CLASSIFY_OPTION));
        match (c.kind, &c.value) {
            (0, Value::U32(x)) => put(0x13, p, *x, &lookup(*x, &t::MULTICAST_STATE)),
            (1, Value::U32(x)) => put(0x14, p, *x, &lookup(*x, &t::LOOSE_SOURCE)),
            (7, Value::U32(x)) => put(0x15, p, *x, &lookup(*x, &t::LOCAL_ONLY_MAPPING)),
            _ => pg(0x16, p, &value_text(&c.value, None, BUF)),
        }
    }
}

pub unsafe fn format_auth_transform(p: &str, a: &store::AuthTransform) {
    pr(0xb5, p, true);
    put(0xb6, p, a.auth_type, &lookup(a.auth_type, &t::AUTH_TYPE));
    put(0xb7, p, a.auth_config as u32, &lookup(a.auth_config as u32, &t::AUTH_CONFIG));
    pg(0xb8, p, &guid_text(a.crypto_module.as_ref()));
}

pub unsafe fn format_cipher_transform(p: &str, c: &store::CipherTransform) {
    pr(0xb9, p, true);
    put(0xba, p, c.cipher_type, &lookup(c.cipher_type, &t::CIPHER_TYPE));
    put(0xbb, p, c.cipher_config as u32, &lookup(c.cipher_config as u32, &t::CIPHER_CONFIG));
    pg(0xbc, p, &guid_text(c.crypto_module.as_ref()));
}

pub unsafe fn format_sa_transform(p: &str, x: &store::SaTransform) {
    put(0xbd, p, x.kind, &lookup(x.kind, &t::TRANSFORM_TYPE));
    match x.kind {
        1 => {
            pr(0xbe, p, true);
            format_auth_transform(&cat(p, SP4), &x.auth);
        }
        2 => {
            pr(0xbf, p, true);
            format_auth_transform(&cat(p, SP5), &x.auth);
        }
        3 => {
            pr(0xc0, p, true);
            format_cipher_transform(&cat(p, SP4), &x.cipher);
        }
        4 => {
            pr(0xc1, p, true);
            pr(0xc2, p, true);
            format_auth_transform(&cat(p, SP8), &x.auth);
            pr(0xc3, p, true);
            format_cipher_transform(&cat(p, SP8), &x.cipher);
        }
        _ => {
            pr(0xc4, p, true);
            format_auth_transform(&cat(p, SP4), &x.auth);
        }
    }
}

pub unsafe fn format_ipsec_proposal(p: &str, x: &store::IpsecProposal) {
    pr(0xc6, p, true);
    pu(0xc7, p, x.transforms.len() as u32);
    pr(0xc8, p, true);
    let child = cat(p, SP8);
    for (i, s) in x.transforms.iter().enumerate() {
        pir(0xc9, p, i as u32, true);
        format_sa_transform(&child, s);
    }
    put(0xca, p, x.pfs_group, &lookup(x.pfs_group, &t::PFS_GROUP));
}

pub unsafe fn format_transport_policy(x: &store::TransportPolicy) {
    let p = SP4;
    pu(0xcb, p, x.proposals.len() as u32);
    pr(0xcc, p, !x.proposals.is_empty());
    let child = cat(p, SP8);
    for (i, pr_) in x.proposals.iter().enumerate() {
        pir(0xcd, p, i as u32, true);
        format_ipsec_proposal(&child, pr_);
    }
    pg(0xce, p, &flags(x.flags as u64, true, &t::POLICY_FLAGS));
    pu(0xcf, p, x.nd_allow_clear_timeout);
    pr(0xd0, p, true);
    pu(0xd1, p, x.idle_timeout);
    pu(0xd2, p, x.idle_timeout_failover);
    match &x.em_policy {
        Some(em) => {
            pr(0xd3, p, true);
            pu(0xd4, p, em.methods.len() as u32);
            pr(0xd5, p, !em.methods.is_empty());
            let child = cat(p, SP12);
            for (j, m) in em.methods.iter().enumerate() {
                pir(0xd6, p, j as u32, true);
                format_auth_method(&child, m);
            }
            put(0xd7, p, em.impersonation, &lookup(em.impersonation, &t::IMPERSONATION));
        }
        None => po(0xd8, p),
    }
}

pub unsafe fn format_ike_cipher(p: &str, x: &store::IkeProposal) {
    put(0xea, p, x.cipher, &lookup(x.cipher, &t::IKE_CIPHER));
    pu(0xeb, p, x.key_len);
    pu(0xec, p, x.rounds);
}

pub unsafe fn format_ike_integrity(p: &str, x: &store::IkeProposal) {
    put(0xed, p, x.integrity, &lookup(x.integrity, &t::IKE_INTEGRITY));
}

pub unsafe fn format_ike_proposal(p: &str, q: &store::IkeProposal) {
    pr(0xee, p, true);
    let child = cat(p, SP8);
    format_ike_cipher(&child, q);
    pr(0xef, p, true);
    format_ike_integrity(&child, q);
    pu(0xf0, p, q.max_lifetime);
    put(0xf1, p, q.dh_group, &lookup(q.dh_group, &t::DH_GROUP));
    pu(0xf2, p, q.qm_limit);
}

pub unsafe fn format_cert_criteria(p: &str, k: &store::CertCriteria) {
    pr(0xf5, p, true);
    let child = cat(p, SP8);
    blob(&child, &k.data);
    pr(0xf6, p, true);
    blob(&child, &k.hash);
    match &k.eku {
        Some(e) => {
            pr(0xf7, p, true);
            pu(0xf8, p, e.as_ref().map(|v| v.len() as u32).unwrap_or(0));
            match e {
                Some(list) => {
                    pr(0xf9, p, true);
                    let pw = ws(p);
                    for (i, s) in list.iter().enumerate() {
                        f(0xfa, &[Z, w(&pw), u(i as u32), Arg::A(Some(s))]);
                    }
                }
                None => po(0xfb, p),
            }
        }
        None => po(0xfc, p),
    }
    match &k.name {
        Some(n) => {
            pr(0xfd, p, true);
            put(0xfe, p, n.kind, &lookup(n.kind, &t::CERT_CRITERIA_NAME));
            match &n.name {
                Some(s) => pgo(0xff, p, Some(s)),
                None => po(0x100, p),
            }
        }
        None => po(0x101, p),
    }
    pr(0x102, p, k.name.is_some());
    pu(0x103, p, k.flags);
}

unsafe fn cert_side(p: &str, ty: u32, list: &[store::CertCriteria], ids: [[u16; 3]; 3]) {
    if ty > 2 {
        return;
    }
    let [a, b, c] = ids[ty as usize];
    pu(a, p, list.len() as u32);
    pr(b, p, !list.is_empty());
    let child = cat(p, SP12);
    for (i, k) in list.iter().enumerate() {
        pir(c, p, i as u32, true);
        format_cert_criteria(&child, k);
    }
}

pub unsafe fn format_cert_auth(p: &str, c: &store::CertAuth) {
    put(0x104, p, c.inbound_type, &lookup(c.inbound_type, &t::CERT_CONFIG));
    cert_side(p, c.inbound_type, &c.inbound, [[0x108, 0x109, 0x10a], [0x10d, 0x10e, 0x10f], [0x112, 0x113, 0x114]]);
    put(0x115, p, c.outbound_type, &lookup(c.outbound_type, &t::CERT_CONFIG));
    cert_side(p, c.outbound_type, &c.outbound, [[0x119, 0x11a, 0x11b], [0x11e, 0x11f, 0x120], [0x123, 0x124, 0x125]]);
    pg(0x126, p, &flags(c.flags as u64, true, &t::CERT_AUTH_FLAGS));
    pr(0x127, p, true);
    blob(&cat(p, SP8), &c.url);
}

pub unsafe fn format_auth_method(p: &str, m: &store::AuthMethod) {
    put(0x128, p, m.kind, &lookup(m.kind, &t::AUTH_METHOD));
    match m.kind as i32 {
        0 => {
            pr(0x129, p, true);
            pr(0x12a, p, true);
            pg(0x12b, p, &flags(m.psk_flags as u64, true, &t::PSK_FLAGS));
        }
        1 | 7 | 8 => {
            pr(0x12c, p, true);
            format_cert_auth(&cat(p, SP8), &m.cert);
        }
        4 | 9 | 10 => {
            pr(0x12e, p, true);
            format_cert_auth(&cat(p, SP8), &m.cert);
        }
        2 => pr(0x12d, p, true),
        5 => pr(0x12f, p, true),
        6 => pr(0x130, p, true),
        11 => pr(0x131, p, true),
        12 => pr(0x132, p, true),
        _ => {}
    }
}

pub unsafe fn format_ike_policy(k: &store::IkePolicy) {
    let p = SP4;
    pu(0x134, p, k.soft_expiration);
    pu(0x135, p, k.methods.len() as u32);
    pr(0x136, p, !k.methods.is_empty());
    let child = cat(p, SP8);
    for (i, m) in k.methods.iter().enumerate() {
        pir(0x137, p, i as u32, true);
        format_auth_method(&child, m);
    }
    put(0x138, p, k.impersonation, &lookup(k.impersonation, &t::IMPERSONATION));
    pu(0x139, p, k.proposals.len() as u32);
    pr(0x13a, p, !k.proposals.is_empty());
    for (j, q) in k.proposals.iter().enumerate() {
        pir(0x13b, p, j as u32, true);
        format_ike_proposal(&child, q);
    }
    pu(0x13c, p, k.flags);
    pu(0x13d, p, k.max_dynamic_filters);
    pu(0x13e, p, k.retransmit_secs);
}

pub unsafe fn format_provider_context(c: &store::ProviderContext) {
    nt(0x20, &guid_text(Some(&c.key)));
    nr(0x21, true);
    display(&c.name, &c.description);
    nt(0x22, &flags(c.flags as u64, true, &t::CONTEXT_FLAGS));
    nt(0x23, &guid_text(c.provider_key.as_ref()));
    nr(0x24, true);
    blob(SP4, &c.data);
    nut(0x25, c.kind, &lookup(c.kind, &t::CONTEXT_TYPE));
    let present = !matches!(c.body, store::ContextBody::Null);
    match (c.kind, &c.body) {
        (0, store::ContextBody::Keying { key_mods, flags: kf }) => {
            nr(0x26, true);
            nu(0x27, key_mods.len() as u32);
            nr(0x28, !key_mods.is_empty());
            for (i, g) in key_mods.iter().enumerate() {
                let tw = ws(&guid_text(Some(g)));
                f(0x29, &[Z, u(i as u32), w(&tw)]);
            }
            nu(0x2a, *kf);
        }
        (1 | 3 | 12, body) => {
            nr(match c.kind { 1 => 0x2b, 3 => 0x2d, _ => 0x37 }, present);
            if let store::ContextBody::Transport(tp) = body {
                format_transport_policy(tp);
            }
        }
        (5 | 6 | 10, body) => {
            nr(match c.kind { 5 => 0x2f, 6 => 0x30, _ => 0x34 }, present);
            if let store::ContextBody::MainMode(mm) = body {
                format_ike_policy(mm);
            }
        }
        (2 | 4 | 9 | 11, _) => nr(match c.kind { 2 => 0x2c, 4 => 0x2e, 9 => 0x33, _ => 0x35 }, present),
        (7, body) => {
            nr(0x31, present);
            if let store::ContextBody::ClassifyOptions(o) = body {
                format_classify_options(o);
            }
        }
        (8, body) => {
            nr(0x32, present);
            if let store::ContextBody::General(g) = body {
                blob(SP4, g);
            }
        }
        _ => {}
    }
    nq(0x38, c.id);
}

pub unsafe fn format_ale(e: &store::AleEndpoint) {
    nq(0x16f, e.id);
    nut(0x170, e.ip_version, &lookup(e.ip_version, &t::IP_VERSION));
    match e.ip_version {
        0 => {
            nt(0x171, &ipv4(v4_of(&e.local)));
            nt(0x172, &ipv4(v4_of(&e.remote)));
        }
        1 => {
            nt(0x173, &ipv6(&e.local));
            nt(0x174, &ipv6(&e.remote));
        }
        _ => {}
    }
    nu(0x175, e.protocol as u32);
    nu(0x176, e.local_port as u32);
    nu(0x177, e.remote_port as u32);
    nq(0x178, e.token_id);
    nq(0x179, e.mm_sa);
    nq(0x17a, e.qm_sa);
    f(0x17b, &[Z, u(e.ipsec_status), u(e.ipsec_status)]);
    nu(0x17c, e.flags);
    nr(0x17d, true);
    blob(SP4, &e.app_id);
}

pub unsafe fn format_sa(p: &str, s: &store::Sa) {
    pu(0x61, p, s.spi);
    put(0x62, p, s.transform, &lookup(s.transform, &t::TRANSFORM_TYPE));
    let child = cat(p, SP8);
    let auth = |ids: [u16; 3]| {
        pr(ids[0], p, true);
        pr(ids[1], p, true);
        format_auth_transform(&child, &s.auth);
        pr(ids[2], p, true);
        blob(&child, &s.auth_key);
    };
    match s.transform {
        1 => auth([0x63, 0x64, 0x65]),
        2 => auth([0x66, 0x67, 0x68]),
        3 => {
            pr(0x69, p, true);
            pr(0x6a, p, true);
            format_cipher_transform(&child, &s.cipher);
            pr(0x6b, p, true);
            blob(&child, &s.cipher_key);
        }
        4 => {
            pr(0x6c, p, true);
            pr(0x6d, p, true);
            pr(0x6e, p, true);
            format_auth_transform(&child, &s.auth);
            pr(0x6f, p, true);
            blob(&cat(p, SP12), &s.auth_key);
            pr(0x70, p, true);
            pr(0x71, p, true);
            format_cipher_transform(&child, &s.cipher);
            pr(0x72, p, true);
            blob(&child, &s.cipher_key);
        }
        _ => auth([0x73, 0x74, 0x75]),
    }
}

pub unsafe fn format_sa_details(p: &str, d: &store::SaDetails) {
    put(0x7a, p, d.ip_version, &lookup(d.ip_version, &t::IP_VERSION));
    put(0x7b, p, d.direction, &lookup(d.direction, &t::DIRECTION));
    pr(0x7c, p, true);
    let tr_ = &d.traffic;
    put(0x7d, p, tr_.ip_version, &lookup(tr_.ip_version, &t::IP_VERSION));
    match tr_.ip_version {
        0 => {
            pg(0x7e, p, &ipv4(v4_of(&tr_.local)));
            pg(0x7f, p, &ipv4(v4_of(&tr_.remote)));
        }
        1 => {
            pg(0x80, p, &ipv6(&tr_.local));
            pg(0x81, p, &ipv6(&tr_.remote));
        }
        _ => {}
    }
    put(0x82, p, tr_.traffic_type, &lookup(tr_.traffic_type, &t::TRAFFIC_TYPE));
    match tr_.traffic_type {
        0 => pq(0x83, p, tr_.filter_or_policy_id),
        1 => pq(0x84, p, tr_.filter_or_policy_id),
        _ => {}
    }
    pu(0x85, p, tr_.remote_port as u32);
    pu(0x86, p, tr_.local_port as u32);
    puu(0x87, p, tr_.protocol as u32, tr_.protocol as u32);
    pq(0x88, p, tr_.local_if_luid);
    pu(0x89, p, tr_.real_if_profile_id);
    let b = &d.bundle;
    pr(0x8a, p, true);
    pg(0x8b, p, &flags(b.flags as u64, true, &t::SA_BUNDLE_FLAGS));
    pr(0x8c, p, true);
    pu(0x8d, p, b.lifetime[0]);
    pu(0x8e, p, b.lifetime[1]);
    pu(0x8f, p, b.lifetime[2]);
    pu(0x90, p, b.idle_timeout);
    pu(0x91, p, b.nd_allow_clear_timeout);
    pr(0x92, p, b.ipsec_id.is_some());
    if let Some(id) = &b.ipsec_id {
        match &id.mm_target {
            Some(n) => pgo(0x93, p, Some(n)),
            None => po(0x94, p),
        }
        match &id.em_target {
            Some(n) => pgo(0x95, p, Some(n)),
            None => po(0x96, p),
        }
        pu(0x97, p, id.tokens.len() as u32);
        pr(0x98, p, !id.tokens.is_empty());
        for (i, k) in id.tokens.iter().enumerate() {
            pir(0x99, p, i as u32, true);
            put(0x9a, p, k.kind, &lookup(k.kind, &t::TOKEN_TYPE));
            put(0x9b, p, k.mode, &lookup(k.mode, &t::TOKEN_MODE));
            put(0x9c, p, k.principal, &lookup(k.principal, &t::TOKEN_PRINCIPAL));
            pq(0x9d, p, k.token);
        }
        pq(0x9e, p, id.explicit_credentials);
        pq(0x9f, p, id.logon_id);
    }
    pu(0xa0, p, b.nap_context);
    pu(0xa1, p, b.qm_sa_id);
    pu(0xa2, p, b.sas.len() as u32);
    pr(0xa3, p, !b.sas.is_empty());
    let child = cat(p, SP16);
    for (i, s) in b.sas.iter().enumerate() {
        pir(0xa4, p, i as u32, true);
        format_sa(&child, s);
    }
    match &b.key_module {
        Some(km) => {
            pr(0xa5, p, true);
            pg(0xa6, p, &guid_text(Some(&km.key)));
            pr(0xa7, p, true);
            blob(&child, &km.state);
        }
        None => po(0xa8, p),
    }
    put(0xa9, p, b.ip_version, &lookup(b.ip_version, &t::IP_VERSION));
    if b.ip_version == 0 {
        pg(0xaa, p, &ipv4(b.peer_v4_private));
    }
    pq(0xab, p, b.mm_sa_id);
    put(0xac, p, b.pfs_group, &lookup(b.pfs_group, &t::PFS_GROUP));
    pg(0xad, p, &guid_text(Some(&b.sa_lookup_context)));
    pq(0xae, p, b.qm_filter_id);
    if d.ip_version == 0 {
        match d.udp_encap {
            Some((l, r)) => {
                pr(0xaf, p, true);
                pu(0xb0, p, l as u32);
                pu(0xb1, p, r as u32);
            }
            None => po(0xb2, p),
        }
    }
    match &d.transport_filter {
        Some(flt) => {
            pr(0xb3, p, true);
            format_filter(&cat(p, SP4), flt);
        }
        None => po(0xb4, p),
    }
}

pub unsafe fn format_sa_context(c: &store::SaContext) {
    let p = SP4;
    let child = cat(p, SP8);
    pq(0x77, p, c.id);
    pr(0x78, p, true);
    format_sa_details(&child, &c.inbound);
    pr(0x79, p, true);
    format_sa_details(&child, &c.outbound);
}

pub unsafe fn format_credential(c: &store::Credential) {
    let q = SP16;
    put(0xde, q, c.kind, &lookup(c.kind, &t::AUTH_METHOD));
    put(0xdf, q, c.impersonation, &lookup(c.impersonation, &t::IMPERSONATION));
    let child = cat(q, SP8);
    match c.kind {
        0 => match &c.body {
            store::CredentialBody::PresharedKey { key, flags: kf } => {
                pr(0xe0, q, true);
                po(0xe1, q);
                blob(&child, key);
                pu(0xe2, q, *kf);
            }
            _ => po(0xe3, q),
        },
        1 | 4 | 6..=10 => {
            let empty: Vec<u8> = Vec::new();
            let (subject, hash, cf, cert, present) = match &c.body {
                store::CredentialBody::Certificate { subject, hash, flags: cf, certificate } => (subject, hash, *cf, certificate, true),
                _ => (&empty, &empty, 0, &empty, false),
            };
            pr(0xe4, q, present);
            po(0xe5, q);
            blob(&child, subject);
            po(0xe6, q);
            blob(&child, hash);
            pu(0xe7, q, cf);
            blob(&child, cert);
        }
        2 | 5 => {
            let name = match &c.body {
                store::CredentialBody::Name(n) => n.as_deref(),
                _ => None,
            };
            pr(0xe8, q, matches!(c.body, store::CredentialBody::Name(_)));
            pgo(0xe9, q, name);
        }
        _ => {}
    }
}

pub unsafe fn format_ike_sa(d: &store::IkeSa) {
    nq(0x13f, d.id);
    nut(0x140, d.key_module, &lookup(d.key_module, &t::KEY_MODULE));
    nut(0x141, d.ip_version, &lookup(d.ip_version, &t::IP_VERSION));
    if d.ip_version == 0 {
        match d.udp_encap {
            Some((l, r)) => {
                nr(0x142, true);
                nu(0x143, l as u32);
                nu(0x144, r as u32);
            }
            None => f(0x145, &[Z]),
        }
    }
    nr(0x146, true);
    nut(0x147, d.traffic_ip_version, &lookup(d.traffic_ip_version, &t::IP_VERSION));
    match d.traffic_ip_version {
        0 => {
            nt(0x148, &ipv4(v4_of(&d.local)));
            nt(0x149, &ipv4(v4_of(&d.remote)));
        }
        1 => {
            nt(0x14a, &ipv6(&d.local));
            nt(0x14b, &ipv6(&d.remote));
        }
        _ => {}
    }
    nq(0x14c, d.auth_ip_filter_id);
    nr(0x14d, true);
    nr(0x14e, true);
    format_ike_cipher(SP12, &d.proposal);
    nr(0x14f, true);
    format_ike_integrity(SP12, &d.proposal);
    nu(0x150, d.proposal.max_lifetime);
    nut(0x151, d.proposal.dh_group, &lookup(d.proposal.dh_group, &t::DH_GROUP));
    nu(0x152, d.proposal.qm_limit);
    nr(0x153, true);
    nq(0x154, d.cookie_initiator);
    nq(0x155, d.cookie_responder);
    nr(0x156, true);
    nu(0x157, d.credentials.len() as u32);
    nr(0x158, !d.credentials.is_empty());
    for (i, (l, r)) in d.credentials.iter().enumerate() {
        f(0x159, &[Z, u(i as u32), red(true)]);
        f(0x15a, &[Z]);
        format_credential(l);
        f(0x15b, &[Z]);
        format_credential(r);
    }
    nt(0x15c, &guid_text(Some(&d.policy_key)));
    nq(0x15d, d.virtual_if_tunnel_id);
    nr(0x15e, true);
    blob(SP4, &d.correlation_key);
}

pub unsafe fn format_object(o: &store::Object) {
    match o {
        store::Object::Provider(v) => format_provider(v),
        store::Object::SubLayer(v) => format_sublayer(v),
        store::Object::Callout(v) => format_callout(v),
        store::Object::Filter(v) => format_filter("", v),
        store::Object::ProviderContext(v) => format_provider_context(v),
        store::Object::AleEndpoint(v) => format_ale(v),
        store::Object::SaContext(v) => format_sa_context(v),
        store::Object::IpsecSa(v) => format_sa_details(SP4, v),
        store::Object::IkeSa(v) => format_ike_sa(v),
    }
}
