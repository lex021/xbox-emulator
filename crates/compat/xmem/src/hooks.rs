use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::OnceLock;

use crate::iat;
use crate::sys::*;
use compat_common::record;

const M: &str = "xmem";

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const u8) -> usize;
    fn GetProcAddress(h: usize, name: *const u8) -> usize;
    fn TlsAlloc() -> u32;
    fn TlsGetValue(index: u32) -> usize;
    fn TlsSetValue(index: u32, value: usize) -> i32;
    fn GetProcessId(process: usize) -> u32;
    fn GetCurrentProcessId() -> u32;
}

const TLS_OUT_OF_INDEXES: u32 = 0xffff_ffff;
const SLOTS: usize = 4;

static TLS: AtomicU32 = AtomicU32::new(TLS_OUT_OF_INDEXES);

fn tls_index() -> u32 {
    let i = TLS.load(Ordering::Acquire);
    if i != TLS_OUT_OF_INDEXES {
        return i;
    }
    let n = unsafe { TlsAlloc() };
    match TLS.compare_exchange(TLS_OUT_OF_INDEXES, n, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => n,
        Err(prev) => prev,
    }
}

unsafe fn guarded(f: impl FnOnce()) {
    let err = GetLastError();
    let i = tls_index();
    if i != TLS_OUT_OF_INDEXES && TlsGetValue(i) == 0 {
        TlsSetValue(i, 1);
        f();
        TlsSetValue(i, 0);
    }
    SetLastError(err);
}

pub static TRACKING_HOOKS: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
pub static REALLOC_REPORTS_FREE: AtomicU32 = AtomicU32::new(0);

static ALLOC_ORIG: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static REALLOC_ORIG: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static FREE_ORIG: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

type AllocFn = unsafe extern "system" fn(usize, u32, usize) -> usize;
type ReallocFn = unsafe extern "system" fn(usize, u32, usize, usize) -> usize;
type FreeFn = unsafe extern "system" fn(usize, u32, usize) -> usize;
type AllocHook = unsafe extern "system" fn(usize, u32, usize, usize);
type ReallocHook = unsafe extern "system" fn(usize, u32, usize, usize, usize);
type FreeHook = unsafe extern "system" fn(usize, u32, usize);

unsafe fn report_free(heap: usize, flags: u32, ptr: usize) {
    let h = TRACKING_HOOKS[2].load(Ordering::Acquire);
    if h != 0 {
        guarded(|| core::mem::transmute::<usize, FreeHook>(h)(heap, flags, ptr));
    }
}

unsafe extern "system" fn alloc_w<const I: usize>(heap: usize, flags: u32, size: usize) -> usize {
    let r = core::mem::transmute::<usize, AllocFn>(ALLOC_ORIG[I].load(Ordering::Acquire))(heap, flags, size);
    if r != 0 {
        if let Some(a) = crate::heap_acct::slot(heap) {
            a.add(size);
        }
    }
    let h = TRACKING_HOOKS[0].load(Ordering::Acquire);
    if h != 0 {
        guarded(|| core::mem::transmute::<usize, AllocHook>(h)(heap, flags, size, r));
    }
    r
}

unsafe extern "system" fn realloc_w<const I: usize>(heap: usize, flags: u32, old: usize, size: usize) -> usize {
    if REALLOC_REPORTS_FREE.load(Ordering::Acquire) != 0 {
        report_free(heap, flags, old);
    }
    let acct = crate::heap_acct::slot(heap);
    let old_size = if acct.is_some() { crate::heap_acct::block_size(heap, flags, old) } else { None };
    let r = core::mem::transmute::<usize, ReallocFn>(REALLOC_ORIG[I].load(Ordering::Acquire))(heap, flags, old, size);
    if let (Some(a), true) = (acct, r != 0) {
        if let Some(o) = old_size {
            a.sub(o);
        }
        a.add(size);
    }
    let h = TRACKING_HOOKS[1].load(Ordering::Acquire);
    if h != 0 {
        guarded(|| core::mem::transmute::<usize, ReallocHook>(h)(heap, flags, size, old, r));
    }
    r
}

unsafe extern "system" fn free_w<const I: usize>(heap: usize, flags: u32, ptr: usize) -> usize {
    let acct = crate::heap_acct::slot(heap);
    let old_size = if acct.is_some() { crate::heap_acct::block_size(heap, flags, ptr) } else { None };
    let r = core::mem::transmute::<usize, FreeFn>(FREE_ORIG[I].load(Ordering::Acquire))(heap, flags, ptr);
    if let (Some(a), Some(o), true) = (acct, old_size, r != 0) {
        a.sub(o);
    }
    report_free(heap, flags, ptr);
    r
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Alloc,
    Realloc,
    Free,
}

fn wrappers(k: Kind) -> [usize; SLOTS] {
    match k {
        Kind::Alloc => [alloc_w::<0> as *const () as usize, alloc_w::<1> as *const () as usize, alloc_w::<2> as *const () as usize, alloc_w::<3> as *const () as usize],
        Kind::Realloc => [realloc_w::<0> as *const () as usize, realloc_w::<1> as *const () as usize, realloc_w::<2> as *const () as usize, realloc_w::<3> as *const () as usize],
        Kind::Free => [free_w::<0> as *const () as usize, free_w::<1> as *const () as usize, free_w::<2> as *const () as usize, free_w::<3> as *const () as usize],
    }
}

fn origs(k: Kind) -> &'static [AtomicUsize; SLOTS] {
    match k {
        Kind::Alloc => &ALLOC_ORIG,
        Kind::Realloc => &REALLOC_ORIG,
        Kind::Free => &FREE_ORIG,
    }
}

fn claim(table: &[AtomicUsize; SLOTS], value: usize) -> Option<usize> {
    for (i, s) in table.iter().enumerate() {
        match s.compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(i),
            Err(v) if v == value => return Some(i),
            Err(_) => {}
        }
    }
    None
}

fn heap_kind_by_name(n: &[u8]) -> Option<Kind> {
    match n {
        b"HeapAlloc" | b"RtlAllocateHeap" => Some(Kind::Alloc),
        b"HeapReAlloc" | b"RtlReAllocateHeap" => Some(Kind::Realloc),
        b"HeapFree" | b"RtlFreeHeap" => Some(Kind::Free),
        _ => None,
    }
}

fn resolve(dlls: &[&[u8]], names: &[&[u8]]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for d in dlls {
        let h = unsafe { GetModuleHandleA(d.as_ptr()) };
        if h == 0 {
            continue;
        }
        for (i, n) in names.iter().enumerate() {
            let f = unsafe { GetProcAddress(h, n.as_ptr()) };
            if f != 0 {
                out.push((f, i));
            }
        }
    }
    out
}

fn heap_known() -> &'static Vec<(usize, Kind)> {
    static K: OnceLock<Vec<(usize, Kind)>> = OnceLock::new();
    K.get_or_init(|| {
        let names: [&[u8]; 6] = [b"HeapAlloc\0", b"RtlAllocateHeap\0", b"HeapReAlloc\0", b"RtlReAllocateHeap\0", b"HeapFree\0", b"RtlFreeHeap\0"];
        let kinds = [Kind::Alloc, Kind::Alloc, Kind::Realloc, Kind::Realloc, Kind::Free, Kind::Free];
        resolve(&[b"ntdll.dll\0", b"kernel32.dll\0", b"kernelbase.dll\0"], &names).into_iter().map(|(f, i)| (f, kinds[i])).collect()
    })
}

fn is_ours(v: usize) -> bool {
    [Kind::Alloc, Kind::Realloc, Kind::Free].iter().any(|&k| wrappers(k).contains(&v))
        || winhttp_wrappers().contains(&v)
        || [vfree_w as *const () as usize, valloc_w as *const () as usize, valloc2_w as *const () as usize, map3_w as *const () as usize].contains(&v)
}

pub fn heap_rule(_dll: &[u8], name: Option<&[u8]>, value: usize) -> Option<usize> {
    if is_ours(value) {
        return None;
    }
    let k = name.and_then(heap_kind_by_name).or_else(|| heap_known().iter().find(|(f, _)| *f == value).map(|(_, k)| *k))?;
    let i = claim(origs(k), value)?;
    Some(wrappers(k)[i])
}

pub fn install_heap_accounting() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| iat::patch_all(&heap_rule))
}

pub fn install_heap_hooks() {
    let n = iat::patch_all(&heap_rule);
    record(M, "XMemSetWin32HeapTrackingHooks", &format!("heap imports redirected through tracking wrappers: {n} slots"));
}

pub const WINHTTP_NAMES: [&[u8]; 7] = [
    b"WinHttpOpen",
    b"WinHttpConnect",
    b"WinHttpOpenRequest",
    b"WinHttpWebSocketCompleteUpgrade",
    b"WinHttpCloseHandle",
    b"WinHttpReceiveResponse",
    b"WinHttpSetOption",
];

pub static WINHTTP_HOOKS: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
static WINHTTP_ORIG: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];

unsafe fn pick(i: usize) -> usize {
    let h = WINHTTP_HOOKS[i].load(Ordering::Acquire);
    if h != 0 { h } else { WINHTTP_ORIG[i].load(Ordering::Acquire) }
}

unsafe extern "system" fn wh_open(a: usize, b: u32, c: usize, d: usize, e: u32) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, u32, usize, usize, u32) -> usize>(pick(0))(a, b, c, d, e)
}

unsafe extern "system" fn wh_connect(a: usize, b: usize, c: u16, d: u32) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, u16, u32) -> usize>(pick(1))(a, b, c, d)
}

unsafe extern "system" fn wh_open_request(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize, g: u32) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, u32) -> usize>(pick(2))(a, b, c, d, e, f, g)
}

unsafe extern "system" fn wh_ws_upgrade(a: usize, b: usize) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize) -> usize>(pick(3))(a, b)
}

unsafe extern "system" fn wh_close(a: usize) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> usize>(pick(4))(a)
}

unsafe extern "system" fn wh_receive(a: usize, b: usize) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize) -> usize>(pick(5))(a, b)
}

unsafe extern "system" fn wh_set_option(a: usize, b: u32, c: usize, d: u32) -> usize {
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, u32, usize, u32) -> usize>(pick(6))(a, b, c, d)
}

fn winhttp_wrappers() -> [usize; 7] {
    [
        wh_open as *const () as usize,
        wh_connect as *const () as usize,
        wh_open_request as *const () as usize,
        wh_ws_upgrade as *const () as usize,
        wh_close as *const () as usize,
        wh_receive as *const () as usize,
        wh_set_option as *const () as usize,
    ]
}

pub fn winhttp_rule(dll: &[u8], name: Option<&[u8]>, value: usize) -> Option<usize> {
    if is_ours(value) {
        return None;
    }
    let i = match name {
        Some(n) if dll.to_ascii_lowercase().starts_with(b"winhttp") => WINHTTP_NAMES.iter().position(|w| *w == n)?,
        _ => {
            let names: Vec<Vec<u8>> = WINHTTP_NAMES.iter().map(|n| [*n, b"\0"].concat()).collect();
            let refs: Vec<&[u8]> = names.iter().map(|v| v.as_slice()).collect();
            resolve(&[b"winhttp.dll\0"], &refs).into_iter().find(|(f, _)| *f == value)?.1
        }
    };
    match WINHTTP_ORIG[i].compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(winhttp_wrappers()[i]),
        Err(v) if v == value => Some(winhttp_wrappers()[i]),
        Err(_) => None,
    }
}

pub fn install_winhttp_hooks() {
    let n = iat::patch_all(&winhttp_rule);
    record(M, "XMemSetWinHttpHooks", &format!("WinHttp imports redirected through hook dispatch: {n} slots"));
}

static VFREE_ORIG: AtomicUsize = AtomicUsize::new(0);
static VALLOC2_ORIG: AtomicUsize = AtomicUsize::new(0);
static MAP3_ORIG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn vfree_w(addr: usize, size: usize, kind: u32) -> i32 {
    let (lo, hi) = crate::epoch::bump_free(addr, size);
    let r = vfree_inner(addr, size, kind);
    crate::epoch::bump_range(lo, hi);
    r
}

unsafe fn vfree_inner(addr: usize, size: usize, kind: u32) -> i32 {
    if kind & MEM_RELEASE != 0 {
        if let Some(r) = crate::phys::release_region(addr, size) {
            return r;
        }
        if crate::phys::owns(addr) {
            SetLastError(487);
            return 0;
        }
    } else if kind & 0x4000 != 0 {
        if let Some(r) = crate::phys::decommit_owned(addr, size) {
            return r;
        }
    }
    let f = VFREE_ORIG.load(Ordering::Acquire);
    let f = if f != 0 { f } else { VirtualFree as *const () as usize };
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, u32) -> i32>(f)(addr, size, kind)
}

static VALLOC_ORIG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn valloc_w(addr: usize, size: usize, kind: u32, protect: u32) -> usize {
    if addr != 0 && kind & MEM_COMMIT != 0 {
        if let Some(r) = crate::phys::commit_owned(addr, size, protect) {
            crate::write_note::committed(r, size);
            return r;
        }
    }
    let f = VALLOC_ORIG.load(Ordering::Acquire);
    let f = if f != 0 { f } else { VirtualAlloc as *const () as usize };
    let r = core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, u32, u32) -> usize>(f)(addr, size, kind, protect);
    if r != 0 && kind & MEM_COMMIT != 0 {
        crate::write_note::committed(r, size);
    }
    r
}

pub fn valloc_wrapper() -> usize {
    valloc_w as *const () as usize
}

const PARAM_PARTITION_HANDLE: u64 = 3;

fn strip_partition(params: usize, count: u32) -> Vec<[u64; 2]> {
    let mut out = Vec::new();
    if params == 0 {
        return out;
    }
    let me = unsafe { GetCurrentProcessId() };
    for i in 0..count as usize {
        let p = unsafe { core::ptr::read_unaligned((params + i * 16) as *const [u64; 2]) };
        if p[0] & 0xff == PARAM_PARTITION_HANDLE && unsafe { GetProcessId(p[1] as usize) } == me {
            continue;
        }
        out.push(p);
    }
    out
}

unsafe extern "system" fn valloc2_w(process: usize, base: usize, size: usize, kind: u32, protect: u32, params: usize, count: u32) -> usize {
    let v = strip_partition(params, count);
    let (pp, pc) = if v.is_empty() { (0, 0) } else { (v.as_ptr() as usize, v.len() as u32) };
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, u32, u32, usize, u32) -> usize>(VALLOC2_ORIG.load(Ordering::Acquire))(process, base, size, kind, protect, pp, pc)
}

unsafe extern "system" fn map3_w(file: usize, process: usize, base: usize, offset: u64, size: usize, kind: u32, protect: u32, params: usize, count: u32) -> usize {
    let v = strip_partition(params, count);
    let (pp, pc) = if v.is_empty() { (0, 0) } else { (v.as_ptr() as usize, v.len() as u32) };
    core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, u64, usize, u32, u32, usize, u32) -> usize>(MAP3_ORIG.load(Ordering::Acquire))(file, process, base, offset, size, kind, protect, pp, pc)
}

pub(crate) fn single_rule(names: &[&[u8]], orig: &AtomicUsize, wrapper: usize, name: Option<&[u8]>, value: usize) -> Option<usize> {
    if is_ours(value) {
        return None;
    }
    let by_name = name.map(|n| names.contains(&n)).unwrap_or(false);
    let by_value = !by_name && {
        let z: Vec<Vec<u8>> = names.iter().map(|n| [*n, b"\0"].concat()).collect();
        let refs: Vec<&[u8]> = z.iter().map(|v| v.as_slice()).collect();
        resolve(&[b"kernel32.dll\0", b"kernelbase.dll\0"], &refs).iter().any(|(f, _)| *f == value)
    };
    if !by_name && !by_value {
        return None;
    }
    match orig.compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(wrapper),
        Err(v) if v == value => Some(wrapper),
        Err(_) => None,
    }
}

pub fn vfree_rule(_dll: &[u8], name: Option<&[u8]>, value: usize) -> Option<usize> {
    single_rule(&[b"VirtualFree"], &VFREE_ORIG, vfree_w as *const () as usize, name, value)
        .or_else(|| single_rule(&[b"VirtualAlloc"], &VALLOC_ORIG, valloc_w as *const () as usize, name, value))
}

pub fn partition_rule(_dll: &[u8], name: Option<&[u8]>, value: usize) -> Option<usize> {
    single_rule(&[b"VirtualAlloc2"], &VALLOC2_ORIG, valloc2_w as *const () as usize, name, value)
        .or_else(|| single_rule(&[b"MapViewOfFile3"], &MAP3_ORIG, map3_w as *const () as usize, name, value))
}

pub fn install_vfree_hook() {
    static DONE: AtomicU32 = AtomicU32::new(0);
    if DONE.swap(1, Ordering::AcqRel) == 0 {
        let n = iat::patch_all(&vfree_rule);
        record(M, "XMemMapPhysicalPages", &format!("VirtualAlloc/VirtualFree imports redirected for xmem-owned physical-page regions: {n} slots"));
    }
}

pub fn install_partition_hooks() {
    static DONE: AtomicU32 = AtomicU32::new(0);
    if DONE.swap(1, Ordering::AcqRel) == 0 {
        let n = iat::patch_all(&partition_rule);
        record(M, "XMemGetPartitionHandleAndId", &format!("VirtualAlloc2/MapViewOfFile3 imports strip the stand-in partition handle: {n} slots"));
    }
}

pub fn vfree_wrapper() -> usize {
    vfree_w as *const () as usize
}

#[cfg(test)]
pub fn strip_for_test(params: &[[u64; 2]]) -> Vec<[u64; 2]> {
    strip_partition(params.as_ptr() as usize, params.len() as u32)
}
