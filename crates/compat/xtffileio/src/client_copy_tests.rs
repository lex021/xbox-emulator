use crate::client::*;
use crate::client_copy::{CopyCallbackVtbl, PERF_READ_PENDING, PERF_SIZE, PERF_WRITE_PENDING};
use crate::client_tests::{open_client, Client, Dispatcher};
use crate::com::*;
use crate::tests::{fresh, guard, host};
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Start { root: String, name: String, dest: String, size: u64 },
    Progress { src: String, dst: String, size: u64, done: u64 },
    End { dest: String, hr: HRESULT },
}

#[repr(C)]
struct CopyCb {
    vtbl: *const CopyCallbackVtbl,
    events: Mutex<Vec<Ev>>,
    fail_progress_after: Option<u64>,
}

unsafe fn s(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

unsafe extern "system" fn qi(_: *mut c_void, _: *const GUID, _: *mut *mut c_void) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn rf(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn on_start(this: *mut c_void, root: *const u16, _pattern: *const u16, info: *const XtfFileInfo, dest: *const u16) -> HRESULT {
    let me = &*(this as *const CopyCb);
    me.events.lock().unwrap().push(Ev::Start { root: s(root), name: s((*info).file_name), dest: s(dest), size: (*info).size });
    S_OK
}

unsafe extern "system" fn on_progress(this: *mut c_void, src: *const u16, dst: *const u16, size: u64, done: u64) -> HRESULT {
    let me = &*(this as *const CopyCb);
    me.events.lock().unwrap().push(Ev::Progress { src: s(src), dst: s(dst), size, done });
    match me.fail_progress_after {
        Some(n) if done >= n => E_ABORT,
        _ => S_OK,
    }
}

unsafe extern "system" fn on_end(this: *mut c_void, _root: *const u16, _pattern: *const u16, _info: *const XtfFileInfo, dest: *const u16, hr: HRESULT) -> HRESULT {
    let me = &*(this as *const CopyCb);
    me.events.lock().unwrap().push(Ev::End { dest: s(dest), hr });
    S_OK
}

const E_ABORT: HRESULT = 0x8000_4004u32 as i32;
const CONSOLE_STALE_MORE: HRESULT = 0x8c11_0203u32 as i32;

static CB_VTBL: CopyCallbackVtbl = CopyCallbackVtbl {
    base: UnknownVtbl { query_interface: qi, add_ref: rf, release: rf },
    on_start_file_copy: on_start,
    on_file_copy_progress: on_progress,
    on_end_file_copy: on_end,
};

impl CopyCb {
    fn new(fail_progress_after: Option<u64>) -> Box<CopyCb> {
        Box::new(CopyCb { vtbl: &CB_VTBL, events: Mutex::new(Vec::new()), fail_progress_after })
    }
    fn ptr(&self) -> *mut c_void {
        self as *const CopyCb as *mut c_void
    }
    fn events(&self) -> Vec<Ev> {
        self.events.lock().unwrap().clone()
    }
}

fn w(s: &str) -> Vec<u16> {
    wide(s)
}

fn pattern_bytes(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u32).wrapping_mul(2654435761).wrapping_add(seed as u32).to_le_bytes()[1]).collect()
}

const SIZES: [usize; 4] = [0, 0x1234, 0x80005, 0x1a0033];

fn populate(base: &str) {
    std::fs::create_dir_all(host(&format!("{base}\\sub"))).unwrap();
    for (i, n) in SIZES.iter().enumerate() {
        std::fs::write(host(&format!("{base}\\f{i}.bin")), pattern_bytes(*n, i as u8)).unwrap();
    }
    std::fs::write(host(&format!("{base}\\sub\\deep.txt")), b"nested").unwrap();
    let ro = host(&format!("{base}\\f1.bin"));
    let mut p = std::fs::metadata(&ro).unwrap().permissions();
    p.set_readonly(true);
    std::fs::set_permissions(&ro, p).unwrap();
}

fn clear_readonly(base: &str) {
    for f in ["f1.bin"] {
        let h = host(&format!("{base}\\{f}"));
        if let Ok(m) = std::fs::metadata(&h) {
            let mut p = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            let _ = std::fs::set_permissions(&h, p);
        }
    }
}

fn check_copy(src: &str, dst: &str) {
    for (i, n) in SIZES.iter().enumerate() {
        let a = host(&format!("{src}\\f{i}.bin"));
        let b = host(&format!("{dst}\\f{i}.bin"));
        let got = std::fs::read(&b).unwrap_or_else(|e| panic!("{b:?}: {e}"));
        assert_eq!(got.len(), *n, "{b:?}");
        assert!(got == pattern_bytes(*n, i as u8), "content {b:?}");
        let ma = std::fs::metadata(&a).unwrap();
        let mb = std::fs::metadata(&b).unwrap();
        assert_eq!(ma.modified().unwrap(), mb.modified().unwrap(), "write time {b:?}");
        assert_eq!(mb.permissions().readonly(), i == 1, "readonly {b:?}");
    }
    assert_eq!(std::fs::read(host(&format!("{dst}\\sub\\deep.txt"))).unwrap(), b"nested");
}

unsafe fn copy(c: &Client, pattern: &str, recursion: u32, dst: &str, cb: &CopyCb) -> HRESULT {
    (c.vt().copy_files)(c.0, w(pattern).as_ptr(), 0, 0, recursion, w(dst).as_ptr(), 0, cb.ptr())
}

unsafe fn round_trip(c: &Client, tag: &str) {
    let src = fresh(&format!("{tag}_src"));
    let mid = fresh(&format!("{tag}_mid"));
    let back = fresh(&format!("{tag}_back"));
    populate(&src);
    let cb = CopyCb::new(None);
    assert_eq!(copy(c, &format!("{src}\\*"), u32::MAX, &format!("X{mid}\\"), &cb), S_OK, "{tag} to console");
    check_copy(&src, &mid);
    let ev = cb.events();
    let starts = ev.iter().filter(|e| matches!(e, Ev::Start { .. })).count();
    let ends: Vec<&Ev> = ev.iter().filter(|e| matches!(e, Ev::End { .. })).collect();
    assert_eq!(starts, SIZES.len() + 1, "{tag} {ev:?}");
    assert_eq!(ends.len(), SIZES.len() + 1, "{tag}");
    assert!(ends.iter().all(|e| matches!(e, Ev::End { hr: 0, .. })));
    let big = format!("{mid}\\f3.bin");
    let last = ev.iter().filter_map(|e| match e {
        Ev::Progress { dst, size, done, .. } if *dst == big => Some((*size, *done)),
        _ => None,
    });
    let v: Vec<(u64, u64)> = last.collect();
    assert!(v.len() >= 3, "{tag} progress {v:?}");
    assert_eq!(v.first().unwrap().1, 0);
    assert_eq!(*v.last().unwrap(), (SIZES[3] as u64, SIZES[3] as u64));
    assert!(v.windows(2).all(|p| p[0].1 <= p[1].1));
    let cb2 = CopyCb::new(None);
    assert_eq!(copy(c, &format!("X{mid}\\*"), u32::MAX, &format!("{back}\\"), &cb2), S_OK, "{tag} from console");
    check_copy(&src, &back);
    assert_eq!(cb2.events().iter().filter(|e| matches!(e, Ev::End { hr: 0, .. })).count(), SIZES.len() + 1);
    let single = format!("{back}\\renamed.bin");
    assert_eq!(copy(c, &format!("X{mid}\\f2.bin"), 0, &single, &CopyCb::new(None)), S_OK, "{tag} single from console");
    assert!(std::fs::read(host(&single)).unwrap() == pattern_bytes(SIZES[2], 2));
    let up = format!("X{mid}\\up.bin");
    assert_eq!(copy(c, &format!("{src}\\f0.bin"), 0, &up, &CopyCb::new(None)), S_OK, "{tag} single to console");
    assert_eq!(std::fs::read(host(&format!("{mid}\\up.bin"))).unwrap().len(), 0);
    for b in [&src, &mid, &back] {
        clear_readonly(b);
    }
}

fn legacy_ops() -> Vec<(u32, OperationFn)> {
    crate::operation_table().into_iter().filter(|(id, _)| *id != OP_TRANSFER_FILES_SUPPORTED).collect()
}

#[test]
fn copy_files_legacy_protocol_both_directions() {
    let _g = guard();
    let port = 47948;
    let _d = unsafe { Dispatcher::custom(port, &legacy_ops()) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 0);
        round_trip(&c, "cpl");
    }
}

#[cfg(fileio_b)]
unsafe extern "system" fn level_one(_session: usize, _request: *mut c_void, response: *mut c_void) -> HRESULT {
    let w = wide("{\"Result\":0}");
    (vt::<MessageVtbl>(response).set_json_body_w)(response, w.as_ptr())
}

#[cfg(fileio_b)]
#[test]
fn copy_files_direct_protocol_level_one_both_directions() {
    let _g = guard();
    let port = 47949;
    let mut ops = legacy_ops();
    ops.push((OP_TRANSFER_FILES_SUPPORTED, level_one as OperationFn));
    let _d = unsafe { Dispatcher::custom(port, &ops) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 1);
        round_trip(&c, "cd1");
    }
}

#[cfg(fileio_b)]
#[test]
fn copy_files_direct_protocol_level_two_both_directions_with_perf() {
    let _g = guard();
    let port = 47950;
    let _d = unsafe { Dispatcher::fileio(port) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 2);
        round_trip(&c, "cd2");
        let src = fresh("cd2p_src");
        let dst = fresh("cd2p_dst");
        std::fs::write(host(&format!("{src}\\p.bin")), pattern_bytes(0x90000, 9)).unwrap();
        let mut i3 = null_mut();
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT3, &mut i3), S_OK);
        let v3 = &**(i3 as *const *const Client3Vtbl);
        let mut perf = [0xAAu8; PERF_SIZE];
        let hr = (v3.copy_files_perf)(i3, w(&format!("{src}\\p.bin")).as_ptr(), 0, 0, 0, w(&format!("X{dst}\\p.bin")).as_ptr(), 0, perf.as_mut_ptr() as *mut c_void, null_mut());
        assert_eq!(hr, S_OK);
        assert_eq!(perf[PERF_READ_PENDING], 1);
        assert_eq!(perf[PERF_WRITE_PENDING], 0);
        let f = |o: usize| f64::from_le_bytes(perf[o..o + 8].try_into().unwrap());
        assert!(f(0x78) >= 0.0 && f(0xa8) >= 0.0);
        let pct: f64 = (0..5).map(|i| f(0x20 + i * 0x10)).sum();
        assert!((pct - 100.0).abs() < 1e-6 || pct == 0.0, "{pct}");
        assert_eq!(perf[0], 0);
        let mut perf2 = [0xAAu8; PERF_SIZE];
        let back = fresh("cd2p_back");
        let hr = (v3.copy_files_perf)(i3, w(&format!("X{dst}\\p.bin")).as_ptr(), 0, 0, 0, w(&format!("{back}\\p.bin")).as_ptr(), 0, perf2.as_mut_ptr() as *mut c_void, null_mut());
        assert_eq!(hr, S_OK);
        assert_eq!(perf2[PERF_READ_PENDING], 0);
        assert_eq!(perf2[PERF_WRITE_PENDING], 1);
        assert!(std::fs::read(host(&format!("{back}\\p.bin"))).unwrap() == pattern_bytes(0x90000, 9));
        assert_eq!((v3.copy_files_direct_list)(i3, null_mut(), 0, null_mut(), null_mut(), 0, null_mut(), null_mut()), E_INVALIDARG);
        assert_eq!((v3.copy_files_direct_list)(i3, perf.as_mut_ptr() as *mut c_void, 1, null_mut(), null_mut(), 0, null_mut(), null_mut()), E_INVALIDARG);
        release(i3);
    }
}

#[test]
fn copy_files_client2_copies_each_named_file() {
    let _g = guard();
    let port = 47951;
    let _d = unsafe { Dispatcher::fileio(port) };
    let c = open_client(port, "127.0.0.1").unwrap();
    let src = fresh("cc2_src");
    let dst = fresh("cc2_dst");
    std::fs::write(host(&format!("{src}\\a.txt")), b"alpha").unwrap();
    std::fs::write(host(&format!("{src}\\b.txt")), b"beta").unwrap();
    unsafe {
        let mut i2 = null_mut();
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT2, &mut i2), S_OK);
        let v2 = &**(i2 as *const *const Client2Vtbl);
        let a = w("a.txt");
        let b = w("b.txt");
        let names = [a.as_ptr(), b.as_ptr()];
        let cb = CopyCb::new(None);
        assert_eq!((v2.copy_files)(i2, w(&src).as_ptr(), names.as_ptr(), 2, w(&format!("X{dst}\\")).as_ptr(), 0, cb.ptr()), S_OK);
        assert_eq!(std::fs::read(host(&format!("{dst}\\a.txt"))).unwrap(), b"alpha");
        assert_eq!(std::fs::read(host(&format!("{dst}\\b.txt"))).unwrap(), b"beta");
        assert_eq!(cb.events().iter().filter(|e| matches!(e, Ev::End { hr: 0, .. })).count(), 2);
        let missing = w("zzz.txt");
        let names = [a.as_ptr(), missing.as_ptr()];
        assert_eq!((v2.copy_files)(i2, w(&src).as_ptr(), names.as_ptr(), 2, w(&format!("X{dst}\\")).as_ptr(), 0, null_mut()), HR_FILE_NOT_FOUND);
        assert_eq!((v2.copy_files)(i2, w(&src).as_ptr(), core::ptr::null(), 1, w(&format!("X{dst}\\")).as_ptr(), 0, null_mut()), E_INVALIDARG);
        release(i2);
    }
}

#[test]
fn copy_files_rejects_bad_arguments_and_honours_callback_abort() {
    let _g = guard();
    let port = 47952;
    let _d = unsafe { Dispatcher::custom(port, &legacy_ops()) };
    let c = open_client(port, "127.0.0.1").unwrap();
    let src = fresh("cce_src");
    let dst = fresh("cce_dst");
    std::fs::write(host(&format!("{src}\\big.bin")), pattern_bytes(0x300000, 3)).unwrap();
    std::fs::write(host(&format!("{src}\\two.bin")), b"2").unwrap();
    unsafe {
        let none = CopyCb::new(None);
        assert_eq!(copy(&c, &format!("{src}\\big.bin"), 0, &format!("{dst}\\x.bin"), &none), E_INVALIDARG);
        assert_eq!(copy(&c, &format!("X{src}\\big.bin"), 0, &format!("X{dst}\\x.bin"), &none), E_INVALIDARG);
        assert_eq!(copy(&c, &format!("{src}\\nothing*.bin"), 0, &format!("X{dst}\\"), &none), HR_FILE_NOT_FOUND);
        std::fs::write(host(&format!("{dst}\\file")), b"f").unwrap();
        assert_eq!(copy(&c, &format!("{src}\\*.bin"), 0, &format!("X{dst}\\file"), &none), E_INVALIDARG);
        assert_eq!((c.vt().copy_files)(c.0, core::ptr::null(), 0, 0, 0, w("XD:\\a").as_ptr(), 0, null_mut()), E_INVALIDARG);
        let ok = CopyCb::new(None);
        assert_eq!(copy(&c, &format!("{src}\\two.bin"), 0, &format!("X{dst}\\two.bin"), &ok), S_OK);
        assert_eq!(std::fs::read(host(&format!("{dst}\\two.bin"))).unwrap(), b"2");
        match &ok.events()[0] {
            Ev::Start { root, name, dest, size } => {
                assert_eq!(root, &src);
                assert_eq!(name, &format!("{src}\\two.bin"));
                assert_eq!(dest, &format!("{dst}\\two.bin"));
                assert_eq!(*size, 1);
            }
            e => panic!("{e:?}"),
        }
        let abort = CopyCb::new(Some(1));
        assert_eq!(copy(&c, &format!("{src}\\big.bin"), 0, &format!("X{dst}\\big.bin"), &abort), E_ABORT);
        assert!(!abort.events().iter().any(|e| matches!(e, Ev::End { .. })));
        assert!(!host(&format!("{dst}\\big.bin")).exists());
        let after = CopyCb::new(None);
        let h = copy(&c, &format!("{src}\\two.bin"), 0, &format!("X{dst}\\two2.bin"), &after);
        assert_eq!(h, S_OK, "0x{:08X}", h as u32);
        assert_eq!(std::fs::read(host(&format!("{dst}\\two2.bin"))).unwrap(), b"2");
        let abort2 = CopyCb::new(Some(1));
        assert_eq!(copy(&c, &format!("X{src}\\big.bin"), 0, &format!("{dst}\\down.bin"), &abort2), E_ABORT);
        let after2 = CopyCb::new(None);
        let h = copy(&c, &format!("{src}\\two.bin"), 0, &format!("X{dst}\\two3.bin"), &after2);
        assert_eq!(h, CONSOLE_STALE_MORE, "0x{:08X}", h as u32);
        assert!(host(&format!("{dst}\\down.bin")).exists());
    }
}

#[cfg(fileio_b)]
#[test]
fn copy_files_direct_abort_reports_server_token_error() {
    let _g = guard();
    let port = 47953;
    let _d = unsafe { Dispatcher::fileio(port) };
    let c = open_client(port, "127.0.0.1").unwrap();
    let src = fresh("cda_src");
    let dst = fresh("cda_dst");
    std::fs::write(host(&format!("{src}\\big.bin")), pattern_bytes(0x300000, 4)).unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 2);
        std::fs::write(host(&format!("{src}\\small.bin")), b"small").unwrap();
        let before = crate::fileio_b::error_token_count();
        let abort = CopyCb::new(Some(1));
        let hr = copy(&c, &format!("{src}\\big.bin"), 0, &format!("X{dst}\\big.bin"), &abort);
        assert!(hr == crate::fileio_b::HR_CANCELLED || hr == E_ABORT, "0x{:08X}", hr as u32);
        assert!(!abort.events().iter().any(|e| matches!(e, Ev::End { .. })));
        let mut waited = 0;
        while crate::fileio_b::error_token_count() <= before && hr == E_ABORT && waited < 100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            waited += 1;
        }
        assert!(crate::fileio_b::error_token_count() > before || hr == crate::fileio_b::HR_CANCELLED);
        assert!(!host(&format!("{dst}\\big.bin")).exists());
        let h = copy(&c, &format!("{src}\\small.bin"), 0, &format!("X{dst}\\small.bin"), &CopyCb::new(None));
        assert!(h < 0, "0x{:08X}", h as u32);
        let c2 = open_client(port, "127.0.0.1").unwrap();
        let abort2 = CopyCb::new(Some(1));
        let h = copy(&c2, &format!("X{src}\\big.bin"), 0, &format!("{dst}\\down.bin"), &abort2);
        assert_eq!(h, E_ABORT, "0x{:08X}", h as u32);
        assert!(!abort2.events().iter().any(|e| matches!(e, Ev::End { .. })));
        let h = copy(&c2, &format!("{src}\\small.bin"), 0, &format!("X{dst}\\small2.bin"), &CopyCb::new(None));
        assert_eq!(h, CONSOLE_STALE_MORE, "0x{:08X}", h as u32);
    }
}

#[cfg(fileio_b)]
mod incremental_tests {
    use super::*;
    use crate::client_copy::{FileListInnerVtbl, FileListVtbl, E_DIRECT_PROTOCOL_REQUIRED, INCREMENTAL_STATS_SIZE};
    use crate::fileio_b::diff::{find, md5, normalize_key, DeltaTable};

    #[repr(C)]
    struct TestEntry {
        pad: [u8; 0x60],
        info: XtfFileInfo,
        name: Vec<u16>,
    }

    #[repr(C)]
    struct TestList {
        vtbl: *const FileListVtbl,
        pad: usize,
        inner: *const FileListInnerVtbl,
        entries: Mutex<Vec<Box<TestEntry>>>,
    }

    unsafe fn units_of(p: *const u16) -> Vec<u16> {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n).to_vec()
    }

    unsafe extern "system" fn l_add(this: *mut c_void, info: *const XtfFileInfo, _r: *mut c_void) -> HRESULT {
        let l = &*(this as *const TestList);
        let mut name = units_of((*info).file_name);
        name.push(0);
        let mut e = Box::new(TestEntry { pad: [0; 0x60], info: *info, name });
        e.info.file_name = e.name.as_ptr();
        l.entries.lock().unwrap().push(e);
        S_OK
    }

    unsafe extern "system" fn l_max(this: *mut c_void) -> u64 {
        let l = &*(this as *const TestList);
        l.entries.lock().unwrap().iter().map(|e| e.info.size).max().unwrap_or(0)
    }

    unsafe extern "system" fn l_count(inner: *mut c_void) -> u32 {
        let l = &*((inner as *mut u8).sub(0x10) as *const TestList);
        l.entries.lock().unwrap().len() as u32
    }

    unsafe extern "system" fn l_next(inner: *mut c_void, prev: *mut c_void) -> *mut c_void {
        let l = &*((inner as *mut u8).sub(0x10) as *const TestList);
        let v = l.entries.lock().unwrap();
        let i = if prev.is_null() { 0 } else { v.iter().position(|e| &**e as *const TestEntry as *mut c_void == prev).map_or(v.len(), |i| i + 1) };
        v.get(i).map_or(null_mut(), |e| &**e as *const TestEntry as *mut c_void)
    }

    static LIST_VTBL: FileListVtbl = FileListVtbl {
        base: UnknownVtbl { query_interface: qi, add_ref: rf, release: rf },
        slot3: 0,
        add: l_add,
        slot5: 0,
        slot6: 0,
        max_file_size: l_max,
        slots8_11: [0; 4],
    };

    static INNER_VTBL: FileListInnerVtbl = FileListInnerVtbl { slots0_5: [0; 6], count: l_count, slots7_25: [0; 19], next: l_next, slots27_29: [0; 3] };

    impl TestList {
        fn new() -> Box<TestList> {
            Box::new(TestList { vtbl: &LIST_VTBL, pad: 0, inner: &INNER_VTBL, entries: Mutex::new(Vec::new()) })
        }
        fn ptr(&self) -> *mut c_void {
            self as *const TestList as *mut c_void
        }
        fn names(&self) -> Vec<String> {
            let mut v: Vec<String> = self.entries.lock().unwrap().iter().map(|e| String::from_utf16_lossy(&e.name[..e.name.len() - 1])).collect();
            v.sort();
            v
        }
    }

    fn key(s: &str) -> Vec<u16> {
        normalize_key(&s.encode_utf16().collect::<Vec<u16>>())
    }

    fn hash_of(k: &[u16]) -> [u8; 16] {
        let b: Vec<u8> = k.iter().flat_map(|u| u.to_le_bytes()).collect();
        md5(&b)
    }

    #[test]
    fn delta_table_tree_and_collisions_follow_console_lookup() {
        let mut t = DeltaTable::default();
        let names = ["", "\\a.txt", "\\Sub", "\\sub\\B.txt", "\\z"];
        for n in names {
            t.add(&n.encode_utf16().collect::<Vec<u16>>());
        }
        for (i, n) in names.iter().enumerate() {
            let k = key(n);
            assert_eq!(find(&t.entries, &t.collisions, &hash_of(&k), &k), Some(i), "{n}");
        }
        let m = key("\\missing");
        assert_eq!(find(&t.entries, &t.collisions, &hash_of(&m), &m), None);
        assert!(t.collisions.is_empty());
        let mut c = DeltaTable::default();
        let h = [7u8; 16];
        c.add_hashed([1u8; 16], &key(""));
        let first = c.add_hashed(h, &key("\\one"));
        let second = c.add_hashed(h, &key("\\two"));
        let third = c.add_hashed([9u8; 16], &key("\\three"));
        assert_eq!(c.collision_count, 1);
        let mut want: Vec<u8> = key("\\two").iter().flat_map(|u| u.to_le_bytes()).collect();
        want.extend_from_slice(&[0, 0]);
        want.extend_from_slice(&(second as u32).to_le_bytes());
        assert_eq!(c.collisions, want);
        assert_eq!(c.entries[first].greater as usize, second);
        assert_eq!(find(&c.entries, &c.collisions, &h, &key("\\two")), Some(second));
        assert_eq!(find(&c.entries, &c.collisions, &h, &key("\\one")), None);
        assert_eq!(find(&c.entries, &c.collisions, &[9u8; 16], &key("\\three")), Some(third));
    }

    #[test]
    fn calculate_incremental_list_and_direct_list_copy_only_dirty_files() {
        let _g = guard();
        let port = 47954;
        let _d = unsafe { Dispatcher::fileio(port) };
        let c = open_client(port, "127.0.0.1").unwrap();
        let src = fresh("cil_src");
        let dst = fresh("cil_dst");
        std::fs::create_dir_all(host(&format!("{src}\\sub\\deep"))).unwrap();
        std::fs::write(host(&format!("{src}\\a.txt")), b"same").unwrap();
        std::fs::write(host(&format!("{src}\\sub\\b.txt")), b"new").unwrap();
        std::fs::write(host(&format!("{src}\\sub\\deep\\c.bin")), pattern_bytes(0x90000, 5)).unwrap();
        std::fs::write(host(&format!("{src}\\stale.txt")), b"fresh").unwrap();
        std::fs::create_dir_all(host(&format!("{dst}\\old_dir"))).unwrap();
        std::fs::write(host(&format!("{dst}\\x.old")), b"orphan").unwrap();
        std::fs::write(host(&format!("{dst}\\stale.txt")), b"old").unwrap();
        std::fs::OpenOptions::new().write(true).open(host(&format!("{dst}\\stale.txt"))).unwrap().set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)).unwrap();
        unsafe {
            assert_eq!(copy(&c, &format!("{src}\\a.txt"), 0, &format!("X{dst}\\a.txt"), &CopyCb::new(None)), S_OK);
            let mut i3 = null_mut();
            assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT3, &mut i3), S_OK);
            let v3 = &**(i3 as *const *const Client3Vtbl);
            let list = TestList::new();
            let mut stats = [0xAAu8; INCREMENTAL_STATS_SIZE];
            let hr = (v3.calculate_incremental_list)(i3, w(&src).as_ptr(), w(&format!("X{dst}")).as_ptr(), 1, stats.as_mut_ptr() as *mut c_void, list.ptr(), w("guid-1").as_ptr(), null_mut());
            assert_eq!(hr, S_OK, "0x{:08X}", hr as u32);
            let mut want = vec![format!("{src}\\stale.txt"), format!("{src}\\sub\\b.txt"), format!("{src}\\sub\\deep\\c.bin")];
            want.sort();
            assert_eq!(list.names(), want);
            assert!(!host(&format!("{dst}\\x.old")).exists());
            assert!(!host(&format!("{dst}\\old_dir")).exists());
            assert!(host(&format!("{dst}\\a.txt")).exists());
            let f = |o: usize| f64::from_le_bytes(stats[o..o + 8].try_into().unwrap());
            assert_eq!(f(0x98), 1.0);
            assert_eq!(f(0xa0), 1.0);
            assert_eq!(f(0x78), (7 * 32) as f64);
            assert_eq!(f(0x80), 0.0);
            assert_eq!(f(0x90), 8.0);
            let cb = CopyCb::new(None);
            let mut perf = [0u8; PERF_SIZE];
            let hr = (v3.copy_files_direct_list)(i3, list.ptr(), 0, w(&src).as_ptr(), w(&format!("X{dst}\\")).as_ptr(), 0, perf.as_mut_ptr() as *mut c_void, cb.ptr());
            assert_eq!(hr, S_OK, "0x{:08X}", hr as u32);
            assert_eq!(std::fs::read(host(&format!("{dst}\\sub\\b.txt"))).unwrap(), b"new");
            assert_eq!(std::fs::read(host(&format!("{dst}\\stale.txt"))).unwrap(), b"fresh");
            assert!(std::fs::read(host(&format!("{dst}\\sub\\deep\\c.bin"))).unwrap() == pattern_bytes(0x90000, 5));
            assert_eq!(cb.events().iter().filter(|e| matches!(e, Ev::End { hr: 0, .. })).count(), 3);
            let again = TestList::new();
            assert_eq!((v3.calculate_incremental_list)(i3, w(&src).as_ptr(), w(&format!("X{dst}")).as_ptr(), 0, null_mut(), again.ptr(), core::ptr::null(), null_mut()), S_OK);
            assert!(again.names().is_empty(), "{:?}", again.names());
            assert_eq!((v3.copy_files_direct_list)(i3, again.ptr(), 0, w(&src).as_ptr(), w(&format!("X{dst}\\")).as_ptr(), 0, null_mut(), null_mut()), HR_FILE_NOT_FOUND);
            assert_eq!((v3.calculate_incremental_list)(i3, core::ptr::null(), w(&format!("X{dst}")).as_ptr(), 0, null_mut(), again.ptr(), core::ptr::null(), null_mut()), E_INVALIDARG);
            assert_eq!((v3.calculate_incremental_list)(i3, w(&src).as_ptr(), w(&format!("X{dst}")).as_ptr(), 0, null_mut(), null_mut(), core::ptr::null(), null_mut()), E_INVALIDARG);
            assert_eq!((v3.calculate_incremental_list)(i3, w(&format!("{src}\\nope")).as_ptr(), w(&format!("X{dst}")).as_ptr(), 0, null_mut(), again.ptr(), core::ptr::null(), null_mut()), HR_FILE_NOT_FOUND);
            release(i3);
        }
        let port2 = 47955;
        let _d2 = unsafe { Dispatcher::custom(port2, &legacy_ops()) };
        let c2 = open_client(port2, "127.0.0.1").unwrap();
        unsafe {
            let mut i3 = null_mut();
            assert_eq!((c2.vt().query_interface)(c2.0, &IID_XTF_FILEIO_CLIENT3, &mut i3), S_OK);
            let v3 = &**(i3 as *const *const Client3Vtbl);
            let list = TestList::new();
            assert_eq!((v3.calculate_incremental_list)(i3, w(&src).as_ptr(), w(&format!("X{dst}")).as_ptr(), 0, null_mut(), list.ptr(), core::ptr::null(), null_mut()), E_DIRECT_PROTOCOL_REQUIRED);
            release(i3);
        }
    }

    use crate::client_copy::incremental::{StatusCallbackVtbl, E_XTFAPP_INVALID_SYMLINK, E_XTFAPP_PATH_TOO_LONG};

    #[repr(C)]
    struct StatusCb {
        vtbl: *const StatusCallbackVtbl,
        ret: HRESULT,
        calls: Mutex<Vec<(HRESULT, String)>>,
    }

    unsafe extern "system" fn s_update(this: *mut c_void, code: HRESULT, message: *const u16) -> HRESULT {
        let s = &*(this as *const StatusCb);
        s.calls.lock().unwrap().push((code, String::from_utf16_lossy(&units_of(message))));
        s.ret
    }

    static STATUS_VTBL: StatusCallbackVtbl = StatusCallbackVtbl { base: UnknownVtbl { query_interface: qi, add_ref: rf, release: rf }, on_update_status: s_update };

    impl StatusCb {
        fn new(ret: HRESULT) -> Box<StatusCb> {
            Box::new(StatusCb { vtbl: &STATUS_VTBL, ret, calls: Mutex::new(Vec::new()) })
        }
        fn ptr(&self) -> *mut c_void {
            self as *const StatusCb as *mut c_void
        }
        fn calls(&self) -> Vec<(HRESULT, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    const LONG_PREFIX: &str = "Source file path detected which is longer than 260 characters, which could cause issues while testing or building a package from this content: ";

    unsafe fn incremental(i3: *mut c_void, src: &str, dst: &str, status: *mut c_void) -> (HRESULT, Box<TestList>) {
        let v3 = &**(i3 as *const *const Client3Vtbl);
        let list = TestList::new();
        let hr = (v3.calculate_incremental_list)(i3, w(src).as_ptr(), w(&format!("X{dst}")).as_ptr(), 1, null_mut(), list.ptr(), core::ptr::null(), status);
        (hr, list)
    }

    #[test]
    fn calculate_incremental_list_status_callback_follows_console_sites() {
        let _g = guard();
        let port = 47956;
        let _d = unsafe { Dispatcher::fileio(port) };
        let c = open_client(port, "127.0.0.1").unwrap();
        let src = fresh("cis_src");
        let dst = fresh("cis_dst");
        let deep = format!("{src}\\{}\\{}", "d".repeat(120), "e".repeat(130));
        std::fs::create_dir_all(host(&deep)).unwrap();
        let long = format!("{deep}\\long_file_name.txt");
        assert!(long.encode_utf16().count() > 260);
        std::fs::write(host(&long), b"L").unwrap();
        std::fs::write(host(&format!("{src}\\short.txt")), b"S").unwrap();
        std::fs::write(host(&format!("{dst}\\orphan.txt")), b"O").unwrap();
        unsafe {
            let mut i3 = null_mut();
            assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT3, &mut i3), S_OK);
            let stop = StatusCb::new(E_ABORT);
            let (hr, list) = incremental(i3, &src, &dst, stop.ptr());
            assert_eq!(hr, E_ABORT);
            assert!(list.names().is_empty());
            assert_eq!(stop.calls(), vec![(E_XTFAPP_PATH_TOO_LONG, format!("{LONG_PREFIX}{long}"))]);
            assert!(host(&format!("{dst}\\orphan.txt")).exists());
            let ok = StatusCb::new(S_OK);
            let (hr, list) = incremental(i3, &src, &dst, ok.ptr());
            assert_eq!(hr, S_OK, "0x{:08X}", hr as u32);
            let mut want = vec![long.clone(), format!("{src}\\short.txt")];
            want.sort();
            assert_eq!(list.names(), want);
            assert_eq!(ok.calls(), vec![(E_XTFAPP_PATH_TOO_LONG, format!("{LONG_PREFIX}{long}"))]);
            assert!(!host(&format!("{dst}\\orphan.txt")).exists());
            let (hr, list) = incremental(i3, &src, &dst, null_mut());
            assert_eq!(hr, S_OK);
            assert_eq!(list.names(), want);
            std::fs::remove_dir_all(host(&format!("{src}\\{}", "d".repeat(120)))).unwrap();
            let target = fresh("cis_target");
            std::fs::write(host(&format!("{target}\\inside.txt")), b"I").unwrap();
            let link = format!("{src}\\link");
            let made = std::process::Command::new("cmd").arg("/c").arg("mklink").arg("/J").arg(host(&link)).arg(host(&target)).output().unwrap();
            assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
            let follow = StatusCb::new(S_OK);
            let (hr, list) = incremental(i3, &src, &dst, follow.ptr());
            assert_eq!(hr, S_OK, "0x{:08X}", hr as u32);
            let mut want = vec![format!("{link}\\inside.txt"), format!("{src}\\short.txt")];
            want.sort();
            assert_eq!(list.names(), want);
            assert!(follow.calls().is_empty());
            std::fs::remove_dir_all(host(&target)).unwrap();
            let skip = StatusCb::new(S_OK);
            let (hr, list) = incremental(i3, &src, &dst, skip.ptr());
            assert_eq!(hr, S_OK, "0x{:08X}", hr as u32);
            assert_eq!(list.names(), vec![format!("{src}\\short.txt")]);
            assert_eq!(skip.calls(), vec![(E_XTFAPP_INVALID_SYMLINK, format!("Invalid symlink to directory at \"{link}\""))]);
            let stop = StatusCb::new(E_ABORT);
            let (hr, _) = incremental(i3, &src, &dst, stop.ptr());
            assert_eq!(hr, E_ABORT);
            assert_eq!(stop.calls().len(), 1);
            let (hr, _) = incremental(i3, &src, &dst, null_mut());
            assert_eq!(hr, HR_FILE_NOT_FOUND, "0x{:08X}", hr as u32);
            std::fs::remove_dir(host(&link)).unwrap();
            release(i3);
        }
    }
}
