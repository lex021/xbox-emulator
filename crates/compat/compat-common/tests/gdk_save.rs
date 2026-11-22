use compat_common::gdk::{self, save, Guid};
use core::ffi::c_void;

unsafe fn method(clsid: Guid, iid: Guid, slot: usize) -> (*mut c_void, usize) {
    let mut obj: *mut c_void = core::ptr::null_mut();
    assert_eq!(gdk::query_api_impl(&clsid, &iid, &mut obj), 0);
    (obj, *(*(obj as *const *const usize)).add(slot))
}

fn tmp(n: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tombox_save_{n}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

type Write = unsafe extern "system" fn(*mut c_void, usize, *const u8, *const u8, usize) -> i32;

unsafe fn update(p: *mut save::Provider, container: &[u8], blobs: &[(&[u8], usize)], deletes: &[&[u8]]) -> i32 {
    let c = save::CLSID;
    let (o, create) = method(c, c, 16);
    let create: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *const u8, *mut usize) -> i32 = core::mem::transmute(create);
    let mut cont = 0usize;
    assert_eq!(create(o, p, container.as_ptr(), &mut cont), 0);
    let (_, upd) = method(c, c, 23);
    let upd: unsafe extern "system" fn(*mut c_void, usize, *const u8, *mut usize) -> i32 = core::mem::transmute(upd);
    let mut u = 0usize;
    assert_eq!(upd(o, cont, core::ptr::null(), &mut u), 0);
    let (_, write) = method(c, c, 25);
    let write: Write = core::mem::transmute(write);
    for (name, len) in blobs {
        let data = vec![0x5au8; *len];
        assert_eq!(write(o, u, name.as_ptr(), data.as_ptr(), *len), 0);
    }
    let (_, del) = method(c, c, 26);
    let del: unsafe extern "system" fn(*mut c_void, usize, *const u8) -> i32 = core::mem::transmute(del);
    for name in deletes {
        assert_eq!(del(o, u, name.as_ptr()), 0);
    }
    let (_, submit) = method(c, c, 27);
    let submit: unsafe extern "system" fn(*mut c_void, usize) -> i32 = core::mem::transmute(submit);
    let hr = submit(o, u);
    let (_, close_u) = method(c, c, 24);
    let close_u: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close_u);
    close_u(o, u);
    let (_, close_c) = method(c, c, 17);
    let close_c: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close_c);
    close_c(o, cont);
    hr
}

#[test]
fn interrupted_commit_is_all_or_nothing() {
    let root = tmp("journal");
    let s = save::Store::open(root.clone()).unwrap();
    s.commit("Slot1", Some("First"), &[("a".into(), b"old-a".to_vec()), ("b".into(), b"old-b".to_vec())], &[]).unwrap();
    let d = root.join("c_Slot1");
    std::fs::write(d.join("b_a.blob.tmp"), b"half").unwrap();
    std::fs::write(d.join("b_c.blob.tmp"), b"half").unwrap();
    drop(s);
    let s = save::Store::open(root.clone()).unwrap();
    assert_eq!(s.read_blob("Slot1", "a").unwrap(), b"old-a");
    assert_eq!(s.blobs("Slot1").len(), 2);
    assert!(!d.join("b_a.blob.tmp").exists() && !d.join("b_c.blob.tmp").exists());

    std::fs::write(d.join("b_a.blob.tmp"), b"new-a").unwrap();
    std::fs::write(d.join("b_c.blob.tmp"), b"new-c").unwrap();
    std::fs::write(d.join(".journal"), "W a\nW c\nD b\n").unwrap();
    drop(s);
    let s = save::Store::open(root.clone()).unwrap();
    assert_eq!(s.read_blob("Slot1", "a").unwrap(), b"new-a");
    assert_eq!(s.read_blob("Slot1", "c").unwrap(), b"new-c");
    assert!(s.read_blob("Slot1", "b").is_none());
    assert!(!d.join(".journal").exists());
    let info = s.container_info("Slot1").unwrap();
    assert_eq!((info.display.as_str(), info.blob_count, info.total_size), ("First", 2, 10));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quota_and_update_size_limits_match_console() {
    std::env::set_var("TOMBOX_SAVE_QUOTA_MB", "1");
    unsafe {
        let root = tmp("quota");
        let p = Box::into_raw(Box::new(save::Provider { store: save::Store::open(root.clone()).unwrap() }));
        assert_eq!(update(p, b"A\0", &[(b"x\0", 600 << 10)], &[]), 0);
        assert_eq!(update(p, b"B\0", &[(b"y\0", 600 << 10)], &[]), save::E_GS_QUOTA_EXCEEDED);
        assert!((*p).store.container_info("B").map(|c| c.blob_count).unwrap_or(0) == 0);
        assert_eq!(update(p, b"A\0", &[(b"x\0", 900 << 10)], &[]), 0);
        assert_eq!(update(p, b"A\0", &[(b"z\0", 50 << 10)], &[b"x\0"]), 0);
        assert_eq!(update(p, b"B\0", &[(b"y\0", 900 << 10)], &[]), 0);

        let (o, quota) = method(save::CLSID, save::CLSID, 7);
        let quota: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *mut i64) -> i32 = core::mem::transmute(quota);
        let mut left = 0i64;
        assert_eq!(quota(o, p, &mut left), 0);
        assert_eq!(left, (1 << 20) - (950 << 10));

        std::env::set_var("TOMBOX_SAVE_QUOTA_MB", "64");
        assert_eq!(update(p, b"C\0", &[(b"big\0", 17 << 20)], &[]), save::E_GS_UPDATE_TOO_BIG);
        assert_eq!(update(p, b"C\0", &[(b"big\0", 15 << 20)], &[]), 0);
        std::env::remove_var("TOMBOX_SAVE_QUOTA_MB");
        let _ = std::fs::remove_dir_all(root);
    }
}

unsafe fn write_blobs(p: *mut save::Provider, container: &[u8], blobs: &[(&[u8], &[u8])]) -> i32 {
    let c = save::CLSID;
    let (o, create) = method(c, c, 16);
    let create: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *const u8, *mut usize) -> i32 = core::mem::transmute(create);
    let mut cont = 0usize;
    assert_eq!(create(o, p, container.as_ptr(), &mut cont), 0);
    let (_, upd) = method(c, c, 23);
    let upd: unsafe extern "system" fn(*mut c_void, usize, *const u8, *mut usize) -> i32 = core::mem::transmute(upd);
    let mut u = 0usize;
    assert_eq!(upd(o, cont, core::ptr::null(), &mut u), 0);
    let (_, write) = method(c, c, 25);
    let write: Write = core::mem::transmute(write);
    for (name, data) in blobs {
        assert_eq!(write(o, u, name.as_ptr(), data.as_ptr(), data.len()), 0);
    }
    let (_, submit) = method(c, c, 28);
    let submit: unsafe extern "system" fn(*mut c_void, usize, *mut c_void) -> i32 = core::mem::transmute(submit);
    let mut b = gdk::rt::Block { queue: 0, context: core::ptr::null_mut(), callback: None, internal: [0; 32] };
    let blk = &mut b as *mut gdk::rt::Block;
    assert_eq!(submit(o, u, blk as *mut c_void), 0);
    gdk::rt::get_status(blk, true);
    let (_, status) = method(c, c, 29);
    let status: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = core::mem::transmute(status);
    let hr = status(o, blk as *mut c_void);
    let (_, close_u) = method(c, c, 24);
    let close_u: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close_u);
    close_u(o, u);
    let (_, close_c) = method(c, c, 17);
    let close_c: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close_c);
    close_c(o, cont);
    hr
}

struct Read {
    hr: i32,
    size: usize,
    buf: Vec<u8>,
    count: u32,
}

unsafe fn read_async(p: *mut save::Provider, container: &[u8], names: &[&[u8]]) -> Read {
    let c = save::CLSID;
    let (o, create) = method(c, c, 16);
    let create: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *const u8, *mut usize) -> i32 = core::mem::transmute(create);
    let mut cont = 0usize;
    assert_eq!(create(o, p, container.as_ptr(), &mut cont), 0);
    let ptrs: Vec<*const u8> = names.iter().map(|n| n.as_ptr()).collect();
    let (_, start) = method(c, c, 21);
    let start: unsafe extern "system" fn(*mut c_void, usize, *const *const u8, u32, *mut c_void) -> i32 = core::mem::transmute(start);
    let mut b = gdk::rt::Block { queue: 0, context: core::ptr::null_mut(), callback: None, internal: [0; 32] };
    let blk = &mut b as *mut gdk::rt::Block;
    let np = if ptrs.is_empty() { core::ptr::null() } else { ptrs.as_ptr() };
    assert_eq!(start(o, cont, np, ptrs.len() as u32, blk as *mut c_void), 0);
    let st = gdk::rt::get_status(blk, true);
    let mut size = 0usize;
    let mut buf = Vec::new();
    let mut count = 0xffff_ffffu32;
    let mut hr = st;
    if st >= 0 {
        assert_eq!(gdk::rt::get_result_size(blk, &mut size), 0);
        buf = vec![0xeeu8; size];
        let (_, result) = method(c, c, 22);
        let result: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, *mut u8, *mut u32) -> i32 = core::mem::transmute(result);
        hr = result(o, blk as *mut c_void, size, buf.as_mut_ptr(), &mut count);
    }
    let (_, close_c) = method(c, c, 17);
    let close_c: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close_c);
    close_c(o, cont);
    Read { hr, size, buf, count }
}

unsafe fn blob(r: &Read, i: usize) -> (String, &[u8]) {
    let base = r.buf.as_ptr();
    let e = base.add(i * 24);
    let name = std::ffi::CStr::from_ptr(core::ptr::read_unaligned(e as *const *const i8)).to_string_lossy().into_owned();
    let size = core::ptr::read_unaligned(e.add(8) as *const u32) as usize;
    let data = core::ptr::read_unaligned(e.add(16) as *const *const u8);
    let off = data as usize - base as usize;
    assert!(off + size <= r.size);
    (name, &r.buf[off..off + size])
}

fn json(lines: usize) -> Vec<u8> {
    let mut s = String::from("{\r\n");
    for i in 0..lines {
        s.push_str(&format!("\t\"Key{i}\" : {i},\r\n"));
    }
    s.push_str("\t\"Last\" : 1\r\n}");
    s.into_bytes()
}

#[test]
fn async_read_result_ends_exactly_at_blob_data_after_shorter_overwrite() {
    unsafe {
        let root = tmp("readback");
        let p = Box::into_raw(Box::new(save::Provider { store: save::Store::open(root.clone()).unwrap() }));
        let long = json(400);
        let short = json(3);
        assert_eq!(write_blobs(p, b"Profile\0", &[(b"Default.json\0", &long)]), 0);
        assert_eq!(write_blobs(p, b"Profile\0", &[(b"Default.json\0", &short)]), 0);
        assert_eq!(std::fs::metadata(root.join("c_Profile").join("b_Default.json.blob")).unwrap().len(), short.len() as u64);

        let r = read_async(p, b"Profile\0", &[b"Default.json\0"]);
        assert_eq!(r.hr, 0);
        assert_eq!(r.count, 1);
        assert_eq!(r.size, 24 + short.len());
        let (name, data) = blob(&r, 0);
        assert_eq!(name, "Default.json");
        assert_eq!(data, &short[..]);
        let data_ptr = core::ptr::read_unaligned(r.buf.as_ptr().add(16) as *const *const u8);
        assert_eq!(data_ptr.add(short.len()), r.buf.as_ptr().add(r.size));
        let off = data_ptr as usize - r.buf.as_ptr() as usize;
        assert_eq!(&r.buf[off..r.size], &short[..]);

        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn async_read_all_blobs_and_empty_container() {
    unsafe {
        let root = tmp("readall");
        let p = Box::into_raw(Box::new(save::Provider { store: save::Store::open(root.clone()).unwrap() }));
        let a = json(10);
        let gbx: Vec<u8> = (0..1039u32).map(|i| (i * 7 + 1) as u8).collect();
        assert_eq!(write_blobs(p, b"Profile\0", &[(b"Default.json\0", &a), (b"User.Profile.Gbx\0", &gbx)]), 0);
        let shorter = json(2);
        assert_eq!(write_blobs(p, b"Profile\0", &[(b"Default.json\0", &shorter)]), 0);

        let r = read_async(p, b"Profile\0", &[]);
        assert_eq!(r.hr, 0);
        assert_eq!(r.count, 2);
        assert_eq!(r.size, 48 + shorter.len() + gbx.len());
        let mut got: Vec<(String, Vec<u8>)> = (0..2).map(|i| { let (n, d) = blob(&r, i); (n, d.to_vec()) }).collect();
        got.sort();
        assert_eq!(got, vec![("Default.json".to_string(), shorter.clone()), ("User.Profile.Gbx".to_string(), gbx.clone())]);

        let one = read_async(p, b"Profile\0", &[b"User.Profile.Gbx\0"]);
        assert_eq!((one.hr, one.count, one.size), (0, 1, 24 + gbx.len()));
        assert_eq!(blob(&one, 0).1, &gbx[..]);

        let missing = read_async(p, b"Profile\0", &[b"Nope\0"]);
        assert_eq!(missing.hr, save::E_GS_BLOB_NOT_FOUND);

        let empty = read_async(p, b"Empty\0", &[]);
        assert_eq!((empty.hr, empty.count, empty.size), (0, 0, 0));

        let _ = std::fs::remove_dir_all(root);
    }
}
