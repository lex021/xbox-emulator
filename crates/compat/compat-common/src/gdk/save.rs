use super::xasync::{self, Payload};
use super::{cstr, log, Guid, Iface, This, E_INVALIDARG, S_OK};
use core::ffi::c_void;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

fn first_call(slot: u32, name: &str) {
    static SEEN: AtomicU64 = AtomicU64::new(0);
    let bit = 1u64 << slot;
    if SEEN.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        log(name, &format!("first call (slot {slot})"));
    }
}

pub const CLSID: Guid = Guid::parse("704c3f58-e629-4cc2-b197-30511b996fe2");

pub const E_GS_INVALID_CONTAINER_NAME: i32 = 0x8083_0001u32 as i32;
pub const E_GS_NO_ACCESS: i32 = 0x8083_0002u32 as i32;
pub const E_GS_OUT_OF_LOCAL_STORAGE: i32 = 0x8083_0003u32 as i32;
pub const E_GS_PROVIDED_BUFFER_TOO_SMALL: i32 = 0x8083_0007u32 as i32;
pub const E_GS_BLOB_NOT_FOUND: i32 = 0x8083_0008u32 as i32;
pub const E_GS_UPDATE_TOO_BIG: i32 = 0x8083_0005u32 as i32;
pub const E_GS_QUOTA_EXCEEDED: i32 = 0x8083_0006u32 as i32;
pub const MAX_UPDATE_BYTES: u64 = 16 << 20;

pub fn encode_name(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    for (i, &c) in b.iter().enumerate() {
        let plain = c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || (c == b'.' && i + 1 != b.len());
        if plain {
            out.push(c as char);
        } else {
            out.push_str(&format!("~{c:02X}"));
        }
    }
    out
}

pub fn decode_name(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'~' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub const BLOB_ENC_MAX: usize = 244;
pub const CONTAINER_ENC_MAX: usize = 253;

fn fnv(s: &str, seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn file_enc(s: &str, max: usize) -> String {
    let e = encode_name(s);
    if e.len() <= max {
        e
    } else {
        let f = s.to_lowercase();
        format!("~H{:016x}{:016x}", fnv(&f, 0), fnv(&f, 0x9e37_79b9_7f4a_7c15))
    }
}

fn is_hashed(enc: &str) -> bool {
    enc.starts_with("~H")
}

fn name_from(enc: &str, sidecar: &Path, max: usize) -> Option<String> {
    if is_hashed(enc) {
        let n = std::fs::read_to_string(sidecar).ok()?;
        file_enc(&n, max).eq_ignore_ascii_case(enc).then_some(n)
    } else {
        decode_name(enc)
    }
}

pub fn valid_name(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains('\0')
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerInfo {
    pub name: String,
    pub display: String,
    pub blob_count: u32,
    pub total_size: u64,
    pub modified: i64,
}

pub struct Store {
    pub root: PathBuf,
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Store {
    pub fn open(root: PathBuf) -> io::Result<Store> {
        std::fs::create_dir_all(&root)?;
        let s = Store { root };
        s.recover()?;
        Ok(s)
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn recover(&self) -> io::Result<()> {
        let _g = Self::lock();
        let Ok(rd) = std::fs::read_dir(&self.root) else { return Ok(()) };
        for e in rd.flatten() {
            let d = e.path();
            if !d.is_dir() {
                continue;
            }
            let j = d.join(".journal");
            if let Ok(text) = std::fs::read_to_string(&j) {
                Self::apply(&d, &text)?;
                std::fs::remove_file(&j)?;
            }
            if let Ok(files) = std::fs::read_dir(&d) {
                for f in files.flatten() {
                    if f.file_name().to_string_lossy().ends_with(".tmp") {
                        let _ = std::fs::remove_file(f.path());
                    }
                }
            }
        }
        Ok(())
    }

    fn apply(d: &Path, journal: &str) -> io::Result<()> {
        for line in journal.lines() {
            if let Some(enc) = line.strip_prefix("W ") {
                let tmp = d.join(format!("b_{enc}.blob.tmp"));
                if tmp.exists() {
                    std::fs::rename(&tmp, d.join(format!("b_{enc}.blob")))?;
                }
            } else if let Some(enc) = line.strip_prefix("D ") {
                match std::fs::remove_file(d.join(format!("b_{enc}.blob"))) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
                if is_hashed(enc) {
                    match std::fs::remove_file(d.join(format!("n_{enc}"))) {
                        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                        _ => {}
                    }
                }
            } else if line == "N" {
                let tmp = d.join(".display.tmp");
                if tmp.exists() {
                    std::fs::rename(&tmp, d.join(".display"))?;
                }
            }
        }
        Ok(())
    }

    pub fn used_bytes(&self) -> u64 {
        self.containers().iter().map(|c| c.total_size).sum()
    }

    fn cdir(&self, name: &str) -> PathBuf {
        self.root.join(format!("c_{}", file_enc(name, CONTAINER_ENC_MAX)))
    }

    pub fn blobs(&self, container: &str) -> Vec<(String, u64)> {
        let mut v = Vec::new();
        let d = self.cdir(container);
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().into_owned();
                if let Some(enc) = f.strip_prefix("b_").and_then(|x| x.strip_suffix(".blob")) {
                    if let Some(n) = name_from(enc, &d.join(format!("n_{enc}")), BLOB_ENC_MAX) {
                        v.push((n, e.metadata().map(|m| m.len()).unwrap_or(0)));
                    }
                }
            }
        }
        v.sort();
        v
    }

    pub fn container_info(&self, name: &str) -> Option<ContainerInfo> {
        let d = self.cdir(name);
        if !d.is_dir() {
            return None;
        }
        let blobs = self.blobs(name);
        let display = std::fs::read_to_string(d.join(".display")).unwrap_or_default();
        let mut modified = mtime(&d);
        for (b, _) in &blobs {
            modified = modified.max(mtime(&d.join(format!("b_{}.blob", file_enc(b, BLOB_ENC_MAX)))));
        }
        Some(ContainerInfo {
            name: name.to_string(),
            display,
            blob_count: blobs.len() as u32,
            total_size: blobs.iter().map(|b| b.1).sum(),
            modified,
        })
    }

    pub fn containers(&self) -> Vec<ContainerInfo> {
        let mut names = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.root) {
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().into_owned();
                if let Some(n) = f.strip_prefix("c_").and_then(|enc| name_from(enc, &e.path().join(".name"), CONTAINER_ENC_MAX)) {
                    if e.path().is_dir() {
                        names.push(n);
                    }
                }
            }
        }
        names.sort();
        names.iter().filter_map(|n| self.container_info(n)).collect()
    }

    pub fn read_blob(&self, container: &str, blob: &str) -> Option<Vec<u8>> {
        std::fs::read(self.cdir(container).join(format!("b_{}.blob", file_enc(blob, BLOB_ENC_MAX)))).ok()
    }

    pub fn commit(&self, container: &str, display: Option<&str>, writes: &[(String, Vec<u8>)], deletes: &[String]) -> io::Result<()> {
        let _g = Self::lock();
        let d = self.cdir(container);
        std::fs::create_dir_all(&d)?;
        if is_hashed(&file_enc(container, CONTAINER_ENC_MAX)) && std::fs::read_to_string(d.join(".name")).ok().as_deref() != Some(container) {
            std::fs::write(d.join(".name"), container)?;
        }
        let mut journal = String::new();
        for (name, data) in writes {
            let enc = file_enc(name, BLOB_ENC_MAX);
            if is_hashed(&enc) {
                std::fs::write(d.join(format!("n_{enc}")), name)?;
            }
            let tmp = d.join(format!("b_{enc}.blob.tmp"));
            let mut f = std::fs::File::create(&tmp)?;
            io::Write::write_all(&mut f, data)?;
            f.sync_all()?;
            journal.push_str(&format!("W {enc}\n"));
        }
        for name in deletes {
            journal.push_str(&format!("D {}\n", file_enc(name, BLOB_ENC_MAX)));
        }
        if let Some(disp) = display {
            std::fs::write(d.join(".display.tmp"), disp)?;
            journal.push_str("N\n");
        }
        let jt = d.join(".journal.tmp");
        let mut f = std::fs::File::create(&jt)?;
        io::Write::write_all(&mut f, journal.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&jt, d.join(".journal"))?;
        Self::apply(&d, &journal)?;
        std::fs::remove_file(d.join(".journal"))?;
        Ok(())
    }

    pub fn delete_container(&self, name: &str) -> io::Result<bool> {
        let d = self.cdir(name);
        if !d.is_dir() {
            return Ok(false);
        }
        std::fs::remove_dir_all(d)?;
        Ok(true)
    }
}

pub fn provider_dir(title_key: &str, xuid: u64, scid: &str) -> PathBuf {
    let scid = if scid.is_empty() { "default".to_string() } else { encode_name(scid) };
    super::data_root().join("saves").join(super::sanitize(title_key)).join(xuid.to_string()).join(scid)
}

pub struct Provider {
    pub store: Store,
}

pub struct Container {
    provider: *const Provider,
    name: String,
}

pub struct Update {
    container: *const Container,
    display: Option<String>,
    writes: Vec<(String, Vec<u8>)>,
    deletes: Vec<String>,
}

fn intern(s: &str) -> *const u8 {
    static M: OnceLock<Mutex<HashMap<String, &'static CStr>>> = OnceLock::new();
    let mut m = M.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = m.get(s) {
        return c.as_ptr() as *const u8;
    }
    let c: &'static CStr = Box::leak(CString::new(s.replace('\0', "")).unwrap_or_default().into_boxed_c_str());
    m.insert(s.to_string(), c);
    c.as_ptr() as *const u8
}

#[repr(C)]
struct ContainerInfoC {
    name: *const u8,
    display: *const u8,
    blob_count: u32,
    total_size: u64,
    modified: i64,
    needs_sync: bool,
}

#[repr(C)]
struct BlobInfoC {
    name: *const u8,
    size: u32,
}

type ContainerCb = unsafe extern "system" fn(*const ContainerInfoC, *mut c_void) -> bool;
type BlobCb = unsafe extern "system" fn(*const BlobInfoC, *mut c_void) -> bool;

static OP_INIT: &[u8] = b"XGameSaveInitializeProviderAsync\0";

unsafe extern "system" fn init_provider_async(_t: This, user: usize, scid: *const u8, sync_on_demand: bool, block: *mut c_void) -> i32 {
    first_call(4, "XGameSaveInitializeProviderAsync");
    let scid = cstr(scid).unwrap_or_default();
    let dir = provider_dir(&super::title().key, super::user::user().xuid, &scid);
    let payload = match Store::open(dir.clone()) {
        Ok(store) => Payload::value(Box::into_raw(Box::new(Provider { store })) as usize),
        Err(e) => {
            log("XGameSaveInitializeProviderAsync", &format!("cannot create {}: {e}", dir.display()));
            Payload::status(E_GS_NO_ACCESS)
        }
    };
    log("XGameSaveInitializeProviderAsync", &format!("user={user:#x} scid='{scid}' syncOnDemand={sync_on_demand} -> {}", dir.display()));
    xasync::run(block, OP_INIT, payload)
}

unsafe extern "system" fn init_provider_result(_t: This, block: *mut c_void, out: *mut usize) -> i32 {
    first_call(5, "XGameSaveInitializeProviderResult");
    xasync::value(block, OP_INIT, out)
}

unsafe extern "system" fn close_provider(_t: This, p: *mut Provider) {
    first_call(6, "XGameSaveCloseProvider");
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

unsafe extern "system" fn delete_container(_t: This, p: *const Provider, name: *const u8) -> i32 {
    first_call(10, "XGameSaveDeleteContainer");
    let (Some(p), Some(n)) = (p.as_ref(), cstr(name)) else { return E_INVALIDARG };
    match p.store.delete_container(&n) {
        Ok(_) => S_OK,
        Err(e) => {
            log("XGameSaveDeleteContainer", &format!("'{n}': {e}"));
            E_GS_NO_ACCESS
        }
    }
}

unsafe fn report_container(info: &ContainerInfo, ctx: *mut c_void, cb: ContainerCb) -> bool {
    let name = CString::new(info.name.clone()).unwrap_or_default();
    let disp = CString::new(info.display.clone()).unwrap_or_default();
    let c = ContainerInfoC {
        name: name.as_ptr() as *const u8,
        display: disp.as_ptr() as *const u8,
        blob_count: info.blob_count,
        total_size: info.total_size,
        modified: info.modified,
        needs_sync: false,
    };
    cb(&c, ctx)
}

unsafe extern "system" fn get_container_info(_t: This, p: *const Provider, name: *const u8, ctx: *mut c_void, cb: Option<ContainerCb>) -> i32 {
    first_call(13, "XGameSaveGetContainerInfo");
    let (Some(p), Some(n), Some(cb)) = (p.as_ref(), cstr(name), cb) else { return E_INVALIDARG };
    if let Some(info) = p.store.container_info(&n) {
        report_container(&info, ctx, cb);
    }
    S_OK
}

unsafe extern "system" fn enumerate_containers(_t: This, p: *const Provider, ctx: *mut c_void, cb: Option<ContainerCb>) -> i32 {
    first_call(14, "XGameSaveEnumerateContainerInfo");
    let (Some(p), Some(cb)) = (p.as_ref(), cb) else { return E_INVALIDARG };
    for info in p.store.containers() {
        if !report_container(&info, ctx, cb) {
            break;
        }
    }
    S_OK
}

unsafe extern "system" fn create_container(_t: This, p: *const Provider, name: *const u8, out: *mut *mut Container) -> i32 {
    first_call(16, "XGameSaveCreateContainer");
    if p.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    let Some(n) = cstr(name).filter(|n| valid_name(n, 256)) else { return E_GS_INVALID_CONTAINER_NAME };
    *out = Box::into_raw(Box::new(Container { provider: p, name: n }));
    S_OK
}

unsafe extern "system" fn close_container(_t: This, c: *mut Container) {
    first_call(17, "XGameSaveCloseContainer");
    if !c.is_null() {
        drop(Box::from_raw(c));
    }
}

unsafe extern "system" fn enumerate_blobs(_t: This, c: *const Container, ctx: *mut c_void, cb: Option<BlobCb>) -> i32 {
    first_call(18, "XGameSaveEnumerateBlobInfo");
    let (Some(c), Some(cb)) = (c.as_ref(), cb) else { return E_INVALIDARG };
    for (name, size) in (*c.provider).store.blobs(&c.name) {
        let n = CString::new(name).unwrap_or_default();
        let info = BlobInfoC { name: n.as_ptr() as *const u8, size: size as u32 };
        if !cb(&info, ctx) {
            break;
        }
    }
    S_OK
}

unsafe extern "system" fn read_blob_data(_t: This, c: *const Container, names: *const *const u8, count: *mut u32, size: usize, out: *mut u8) -> i32 {
    first_call(20, "XGameSaveReadBlobData");
    let Some(c) = c.as_ref() else { return E_INVALIDARG };
    if count.is_null() {
        return E_INVALIDARG;
    }
    let store = &(*c.provider).store;
    let wanted: Vec<String> = if names.is_null() || *count == 0 {
        store.blobs(&c.name).into_iter().map(|b| b.0).collect()
    } else {
        (0..*count as usize).filter_map(|i| cstr(*names.add(i))).collect()
    };
    let mut data = Vec::with_capacity(wanted.len());
    for n in &wanted {
        match store.read_blob(&c.name, n) {
            Some(d) => data.push(d),
            None => {
                log("XGameSaveReadBlobData", &format!("'{}' blob '{n}' not found", c.name));
                return E_GS_BLOB_NOT_FOUND;
            }
        }
    }
    let head = wanted.len() * 24;
    let need = head + data.iter().map(|d| d.len()).sum::<usize>();
    if out.is_null() || size < need {
        log("XGameSaveReadBlobData", &format!("buffer {size} < needed {need}"));
        return E_GS_PROVIDED_BUFFER_TOO_SMALL;
    }
    let mut off = head;
    for (i, (n, d)) in wanted.iter().zip(&data).enumerate() {
        let e = out.add(i * 24);
        core::ptr::write_unaligned(e as *mut usize, intern(n) as usize);
        core::ptr::write_unaligned(e.add(8) as *mut u64, d.len() as u64);
        core::ptr::write_unaligned(e.add(16) as *mut usize, out.add(off) as usize);
        core::ptr::copy_nonoverlapping(d.as_ptr(), out.add(off), d.len());
        off += d.len();
    }
    *count = wanted.len() as u32;
    S_OK
}

unsafe extern "system" fn create_update(_t: This, c: *const Container, display: *const u8, out: *mut *mut Update) -> i32 {
    first_call(23, "XGameSaveCreateUpdate");
    if c.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    *out = Box::into_raw(Box::new(Update { container: c, display: cstr(display), writes: Vec::new(), deletes: Vec::new() }));
    S_OK
}

unsafe extern "system" fn close_update(_t: This, u: *mut Update) {
    first_call(24, "XGameSaveCloseUpdate");
    if !u.is_null() {
        drop(Box::from_raw(u));
    }
}

unsafe extern "system" fn submit_blob_write(_t: This, u: *mut Update, name: *const u8, data: *const u8, len: usize) -> i32 {
    first_call(25, "XGameSaveSubmitBlobWrite");
    let Some(u) = u.as_mut() else { return E_INVALIDARG };
    let Some(n) = cstr(name).filter(|n| valid_name(n, 256)) else { return E_GS_INVALID_CONTAINER_NAME };
    if data.is_null() && len > 0 {
        return E_INVALIDARG;
    }
    let bytes = if len == 0 { Vec::new() } else { core::slice::from_raw_parts(data, len).to_vec() };
    u.deletes.retain(|d| d != &n);
    u.writes.retain(|w| w.0 != n);
    u.writes.push((n, bytes));
    S_OK
}

unsafe extern "system" fn submit_update(_t: This, u: *const Update) -> i32 {
    first_call(27, "XGameSaveSubmitUpdate");
    let Some(u) = u.as_ref() else { return E_INVALIDARG };
    let c = &*u.container;
    let store = &(*c.provider).store;
    let size: u64 = u.writes.iter().map(|w| w.1.len() as u64 + w.0.len() as u64).sum();
    if size > MAX_UPDATE_BYTES {
        log("XGameSaveSubmitUpdate", &format!("'{}': {size} bytes in one update (max {MAX_UPDATE_BYTES})", c.name));
        return E_GS_UPDATE_TOO_BIG;
    }
    let existing = store.blobs(&c.name);
    let freed: u64 = existing.iter().filter(|(n, _)| u.writes.iter().any(|w| &w.0 == n) || u.deletes.contains(n)).map(|b| b.1).sum();
    let added: u64 = u.writes.iter().map(|w| w.1.len() as u64).sum();
    let after = store.used_bytes() - freed + added;
    if after as i64 > quota_bytes() {
        log("XGameSaveSubmitUpdate", &format!("'{}': would use {after} of {} quota bytes", c.name, quota_bytes()));
        return E_GS_QUOTA_EXCEEDED;
    }
    match store.commit(&c.name, u.display.as_deref(), &u.writes, &u.deletes) {
        Ok(()) => {
            log("XGameSaveSubmitUpdate", &format!("'{}': {} written, {} deleted", c.name, u.writes.len(), u.deletes.len()));
            S_OK
        }
        Err(e) => {
            log("XGameSaveSubmitUpdate", &format!("'{}': {e}", c.name));
            E_GS_OUT_OF_LOCAL_STORAGE
        }
    }
}

pub const IID_V2: Guid = Guid::parse("704c3f58-e629-4cc2-b197-30511b996ee2");

pub fn quota_bytes() -> i64 {
    super::setting("save_quota_mb").and_then(|v| v.parse::<i64>().ok()).unwrap_or(256) << 20
}

fn remaining(p: &Provider) -> i64 {
    let used: u64 = p.store.containers().iter().map(|c| c.total_size).sum();
    (quota_bytes() - used as i64).max(0)
}

unsafe fn open_provider(user: usize, scid: *const u8) -> Result<Box<Provider>, i32> {
    if user == 0 {
        return Err(E_INVALIDARG);
    }
    let scid = cstr(scid).unwrap_or_default();
    let dir = provider_dir(&super::title().key, super::user::user().xuid, &scid);
    match Store::open(dir.clone()) {
        Ok(store) => Ok(Box::new(Provider { store })),
        Err(e) => {
            log("XGameSaveInitializeProvider", &format!("cannot create {}: {e}", dir.display()));
            Err(E_GS_NO_ACCESS)
        }
    }
}

unsafe extern "system" fn init_provider(_t: This, user: usize, scid: *const u8, _sync: bool, out: *mut *mut Provider) -> i32 {
    first_call(3, "XGameSaveInitializeProvider");
    if out.is_null() {
        return E_INVALIDARG;
    }
    match open_provider(user, scid) {
        Ok(p) => {
            *out = Box::into_raw(p);
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn get_quota(_t: This, p: *const Provider, out: *mut i64) -> i32 {
    first_call(7, "XGameSaveGetRemainingQuota");
    let Some(p) = p.as_ref() else { return E_INVALIDARG };
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = remaining(p);
    S_OK
}

static OP_QUOTA: &[u8] = b"XGameSaveGetRemainingQuotaAsync\0";
static OP_DELETE: &[u8] = b"XGameSaveDeleteContainerAsync\0";
static OP_READ: &[u8] = b"XGameSaveReadBlobDataAsync\0";
static OP_SUBMIT: &[u8] = b"XGameSaveSubmitUpdateAsync\0";

unsafe extern "system" fn get_quota_async(_t: This, p: *const Provider, block: *mut c_void) -> i32 {
    first_call(8, "XGameSaveGetRemainingQuotaAsync");
    let Some(p) = p.as_ref() else { return E_INVALIDARG };
    xasync::run(block, OP_QUOTA, xasync::Payload::value(remaining(p)))
}

unsafe extern "system" fn get_quota_result(_t: This, block: *mut c_void, out: *mut i64) -> i32 {
    first_call(9, "XGameSaveGetRemainingQuotaResult");
    xasync::value(block, OP_QUOTA, out)
}

unsafe extern "system" fn delete_container_async(t: This, p: *const Provider, name: *const u8, block: *mut c_void) -> i32 {
    first_call(11, "XGameSaveDeleteContainerAsync");
    if p.is_null() || cstr(name).is_none() {
        return E_INVALIDARG;
    }
    let hr = delete_container(t, p, name);
    xasync::run(block, OP_DELETE, xasync::Payload::status(hr))
}

unsafe extern "system" fn status_result(_t: This, block: *mut c_void) -> i32 {
    first_call(12, "XGameSaveDeleteContainerResult or XGameSaveSubmitUpdateResult");
    xasync::status(block)
}

unsafe extern "system" fn enumerate_containers_by_name(_t: This, p: *const Provider, prefix: *const u8, ctx: *mut c_void, cb: Option<ContainerCb>) -> i32 {
    first_call(15, "XGameSaveEnumerateContainerInfoByName");
    let (Some(p), Some(cb)) = (p.as_ref(), cb) else { return E_INVALIDARG };
    let pre = cstr(prefix).unwrap_or_default();
    for info in p.store.containers() {
        if info.name.starts_with(&pre) && !report_container(&info, ctx, cb) {
            break;
        }
    }
    S_OK
}

unsafe extern "system" fn enumerate_blobs_by_name(_t: This, c: *const Container, prefix: *const u8, ctx: *mut c_void, cb: Option<BlobCb>) -> i32 {
    first_call(19, "XGameSaveEnumerateBlobInfoByName");
    let (Some(c), Some(cb)) = (c.as_ref(), cb) else { return E_INVALIDARG };
    let pre = cstr(prefix).unwrap_or_default();
    for (name, size) in (*c.provider).store.blobs(&c.name) {
        if !name.starts_with(&pre) {
            continue;
        }
        let n = CString::new(name).unwrap_or_default();
        let info = BlobInfoC { name: n.as_ptr() as *const u8, size: size as u32 };
        if !cb(&info, ctx) {
            break;
        }
    }
    S_OK
}

unsafe fn wanted_names(c: &Container, names: *const *const u8, count: u32) -> Vec<String> {
    if names.is_null() || count == 0 {
        (*c.provider).store.blobs(&c.name).into_iter().map(|b| b.0).collect()
    } else {
        (0..count as usize).filter_map(|i| cstr(*names.add(i))).collect()
    }
}

unsafe extern "system" fn read_blob_data_async(_t: This, c: *const Container, names: *const *const u8, count: u32, block: *mut c_void) -> i32 {
    first_call(21, "XGameSaveReadBlobDataAsync");
    let Some(c) = c.as_ref() else { return E_INVALIDARG };
    let store = &(*c.provider).store;
    let wanted = wanted_names(c, names, count);
    let mut data = Vec::new();
    for n in &wanted {
        match store.read_blob(&c.name, n) {
            Some(d) => data.push(d),
            None => return xasync::run(block, OP_READ, xasync::Payload::status(E_GS_BLOB_NOT_FOUND)),
        }
    }
    let head = wanted.len() * 24;
    let mut b = vec![0u8; head];
    let mut fixups = Vec::new();
    for (i, (n, d)) in wanted.iter().zip(&data).enumerate() {
        let off = b.len();
        b.extend_from_slice(d);
        let e = i * 24;
        b[e..e + 8].copy_from_slice(&(intern(n) as u64).to_le_bytes());
        b[e + 8..e + 16].copy_from_slice(&(d.len() as u64).to_le_bytes());
        b[e + 16..e + 24].copy_from_slice(&(off as u64).to_le_bytes());
        fixups.push(e + 16);
    }
    xasync::run(block, OP_READ, xasync::Payload { hr: S_OK, bytes: b, fixups })
}

unsafe extern "system" fn read_blob_data_result(_t: This, block: *mut c_void, size: usize, out: *mut u8, count: *mut u32) -> i32 {
    first_call(22, "XGameSaveReadBlobDataResult");
    if count.is_null() {
        return E_INVALIDARG;
    }
    let mut need = 0usize;
    let hr = xasync::size(block, &mut need);
    if hr < 0 {
        return hr;
    }
    if need == 0 {
        *count = 0;
        return xasync::bytes(block, OP_READ, size, out as *mut c_void, core::ptr::null_mut());
    }
    if out.is_null() || size < need {
        return E_GS_PROVIDED_BUFFER_TOO_SMALL;
    }
    let hr = xasync::bytes(block, OP_READ, need, out as *mut c_void, core::ptr::null_mut());
    if hr < 0 {
        return hr;
    }
    let first = core::ptr::read_unaligned(out.add(16) as *const u64);
    *count = ((first - out as u64) / 24) as u32;
    S_OK
}

unsafe extern "system" fn submit_blob_delete(_t: This, u: *mut Update, name: *const u8) -> i32 {
    first_call(26, "XGameSaveSubmitBlobDelete");
    let Some(u) = u.as_mut() else { return E_INVALIDARG };
    let Some(n) = cstr(name).filter(|n| valid_name(n, 256)) else { return E_GS_INVALID_CONTAINER_NAME };
    u.writes.retain(|w| w.0 != n);
    if !u.deletes.contains(&n) {
        u.deletes.push(n);
    }
    S_OK
}

unsafe extern "system" fn submit_update_async(t: This, u: *const Update, block: *mut c_void) -> i32 {
    first_call(28, "XGameSaveSubmitUpdateAsync");
    if u.is_null() {
        return E_INVALIDARG;
    }
    let hr = submit_update(t, u);
    xasync::run(block, OP_SUBMIT, xasync::Payload::status(hr))
}

fn slots() -> Vec<(usize, usize)> {
    vec![
        (3, init_provider as usize),
        (4, init_provider_async as usize),
        (5, init_provider_result as usize),
        (6, close_provider as usize),
        (7, get_quota as usize),
        (8, get_quota_async as usize),
        (9, get_quota_result as usize),
        (10, delete_container as usize),
        (11, delete_container_async as usize),
        (12, status_result as usize),
        (13, get_container_info as usize),
        (14, enumerate_containers as usize),
        (15, enumerate_containers_by_name as usize),
        (16, create_container as usize),
        (17, close_container as usize),
        (18, enumerate_blobs as usize),
        (19, enumerate_blobs_by_name as usize),
        (20, read_blob_data as usize),
        (21, read_blob_data_async as usize),
        (22, read_blob_data_result as usize),
        (23, create_update as usize),
        (24, close_update as usize),
        (25, submit_blob_write as usize),
        (26, submit_blob_delete as usize),
        (27, submit_update as usize),
        (28, submit_update_async as usize),
        (29, status_result as usize),
    ]
}

pub fn interfaces() -> Vec<Iface> {
    vec![
        Iface { clsid: CLSID, iid: CLSID, name: "XGameSave(704c3f58)", slots: slots() },
        Iface { clsid: CLSID, iid: IID_V2, name: "XGameSave(704c3f58..6ee2)", slots: slots() },
    ]
}
