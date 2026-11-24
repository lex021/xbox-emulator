use core::ffi::c_void;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptOpenAlgorithmProvider(alg: *mut *mut c_void, id: *const u16, implementation: *const u16, flags: u32) -> i32;
    fn BCryptGenerateKeyPair(alg: *mut c_void, key: *mut *mut c_void, length: u32, flags: u32) -> i32;
    fn BCryptFinalizeKeyPair(key: *mut c_void, flags: u32) -> i32;
    fn BCryptHash(alg: *mut c_void, secret: *const u8, secret_len: u32, input: *const u8, input_len: u32, out: *mut u8, out_len: u32) -> i32;
    fn BCryptSignHash(key: *mut c_void, padding: *const c_void, hash: *const u8, hash_len: u32, sig: *mut u8, sig_len: u32, result: *mut u32, flags: u32) -> i32;
}

pub const VERSION: u32 = 1;
pub const MAX_BODY_BYTES: usize = 8192;

struct Keys {
    sha: usize,
    key: usize,
}

static KEYS: OnceLock<Result<Keys, i32>> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn hr_from_nt(st: i32) -> i32 {
    if st >= 0 { st } else { ((st as u32 & 0x0fff_ffff) | 0x1000_0000 | 0x8000_0000) as i32 }
}

fn keys() -> Result<&'static Keys, i32> {
    KEYS.get_or_init(|| unsafe {
        let mut sha = core::ptr::null_mut();
        let st = BCryptOpenAlgorithmProvider(&mut sha, wide("SHA256").as_ptr(), core::ptr::null(), 0);
        if st < 0 {
            return Err(hr_from_nt(st));
        }
        let mut ec = core::ptr::null_mut();
        let st = BCryptOpenAlgorithmProvider(&mut ec, wide("ECDSA_P256").as_ptr(), core::ptr::null(), 0);
        if st < 0 {
            return Err(hr_from_nt(st));
        }
        let mut key = core::ptr::null_mut();
        let st = BCryptGenerateKeyPair(ec, &mut key, 256, 0);
        if st < 0 {
            return Err(hr_from_nt(st));
        }
        let st = BCryptFinalizeKeyPair(key, 0);
        if st < 0 {
            return Err(hr_from_nt(st));
        }
        Ok(Keys { sha: sha as usize, key: key as usize })
    })
    .as_ref()
    .map_err(|e| *e)
}

pub fn filetime_now() -> u64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(d.subsec_nanos() / 100)
}

pub fn path_and_query(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split('#').next().unwrap_or("");
    match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => rest[i..].to_string(),
        Some(i) => format!("/{}", &rest[i..]),
        None => "/".to_string(),
    }
}

pub fn signed_payload(timestamp: u64, method: &str, url: &str, authorization: &str, body: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(64 + url.len() + authorization.len() + body.len().min(MAX_BODY_BYTES));
    p.extend_from_slice(&VERSION.to_be_bytes());
    p.push(0);
    p.extend_from_slice(&timestamp.to_be_bytes());
    p.push(0);
    p.extend_from_slice(method.to_ascii_uppercase().as_bytes());
    p.push(0);
    p.extend_from_slice(path_and_query(url).as_bytes());
    p.push(0);
    p.extend_from_slice(authorization.as_bytes());
    p.push(0);
    p.extend_from_slice(&body[..body.len().min(MAX_BODY_BYTES)]);
    p.push(0);
    p
}

pub fn sign_at(timestamp: u64, method: &str, url: &str, authorization: &str, body: &[u8]) -> Result<String, i32> {
    let k = keys()?;
    let payload = signed_payload(timestamp, method, url, authorization, body);
    let mut hash = [0u8; 32];
    let mut sig = [0u8; 64];
    let mut got = 0u32;
    unsafe {
        let st = BCryptHash(k.sha as *mut c_void, core::ptr::null(), 0, payload.as_ptr(), payload.len() as u32, hash.as_mut_ptr(), 32);
        if st < 0 {
            return Err(hr_from_nt(st));
        }
        let st = BCryptSignHash(k.key as *mut c_void, core::ptr::null(), hash.as_ptr(), 32, sig.as_mut_ptr(), 64, &mut got, 0);
        if st < 0 {
            return Err(hr_from_nt(st));
        }
    }
    let mut header = Vec::with_capacity(76);
    header.extend_from_slice(&VERSION.to_be_bytes());
    header.extend_from_slice(&timestamp.to_be_bytes());
    header.extend_from_slice(&sig[..got as usize]);
    Ok(crate::online::util::b64(&header))
}

pub fn sign(method: &str, url: &str, authorization: &str, body: &[u8]) -> Result<String, i32> {
    sign_at(filetime_now(), method, url, authorization, body)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_and_query_strips_scheme_and_host() {
        assert_eq!(path_and_query("https://social.xboxlive.com/users/me/people?startIndex=0&maxItems=1000"), "/users/me/people?startIndex=0&maxItems=1000");
        assert_eq!(path_and_query("wss://rta.xboxlive.com/connect"), "/connect");
        assert_eq!(path_and_query("https://userstats.xboxlive.com"), "/");
        assert_eq!(path_and_query("https://h.example?x=1"), "/?x=1");
    }

    #[test]
    fn payload_matches_xsts_layout() {
        let p = signed_payload(0x0102030405060708, "post", "https://a.b/c?d=1", "XBL3.0 x=1;t", b"{}");
        let mut want = vec![0, 0, 0, 1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0];
        want.extend_from_slice(b"POST\0/c?d=1\0XBL3.0 x=1;t\0{}\0");
        assert_eq!(p, want);
    }

    #[test]
    fn signature_header_is_version_time_and_p256_signature() {
        let s = sign_at(0x01d9_0000_0000_0000, "GET", "https://social.xboxlive.com/users/me/people", "XBL3.0 x=1;t", b"").unwrap();
        let raw = crate::online::util::b64_decode(&s);
        assert_eq!(raw.len(), 76);
        assert_eq!(&raw[..4], &[0, 0, 0, 1]);
        assert_eq!(&raw[4..12], &0x01d9_0000_0000_0000u64.to_be_bytes());
        assert!(raw[12..].iter().any(|b| *b != 0));
    }
}
