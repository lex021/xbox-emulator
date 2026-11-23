use core::ffi::c_void;
use std::path::PathBuf;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn FreeLibrary(h: *mut c_void) -> i32;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn EnumerateTraceGuidsEx(class: u32, inb: *const c_void, insz: u32, out: *mut c_void, outsz: u32, ret: *mut u32) -> u32;
}

const NETWORK_PROFILE: [u8; 16] = [0x72, 0x59, 0x79, 0xed, 0xe8, 0x60, 0x15, 0x48, 0x86, 0x34, 0xcf, 0xaa, 0x21, 0xa8, 0x9d, 0xe7];
const AGGREGATE: [u8; 16] = [0x13, 0xcc, 0x3f, 0x70, 0x6f, 0xb6, 0x68, 0x58, 0xdd, 0xd9, 0xe2, 0xdb, 0x7f, 0x38, 0x1f, 0xfb];

fn built() -> PathBuf {
    xone_exports::build_cdylib(env!("CARGO"), env!("CARGO_MANIFEST_DIR"), "netprofmsvc_c", "netprofmsvc_c.dll")
}

fn rows() -> Vec<xone_exports::Export> {
    xone_exports::parse(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap())
}

fn instances(guid: &[u8; 16]) -> u32 {
    let mut buf = vec![0u8; 0x10000];
    let mut ret = 0u32;
    let st = unsafe { EnumerateTraceGuidsEx(1, guid.as_ptr() as *const c_void, 16, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut ret) };
    if st == 4200 {
        return 0;
    }
    assert_eq!(st, 0);
    let n = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let pid = unsafe { GetCurrentProcessId() };
    let (mut off, mut mine) = (8usize, 0);
    for _ in 0..n {
        let next = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        if u32::from_le_bytes(buf[off + 8..off + 12].try_into().unwrap()) == pid {
            mine += 1;
        }
        if next == 0 {
            break;
        }
        off += next;
    }
    mine
}

#[test]
fn export_table_matches_exports_txt_without_missing_rows() {
    let rows = rows();
    let ords: Vec<(u16, String)> = rows.iter().map(|e| (e.ordinal, e.name.clone())).collect();
    for p in ["Fortnite", "XDefiant"] {
        let c = PathBuf::from(format!(r"D:\xbox-pc-port\games\{p}\extracted\EmbeddedXvd\Windows\System32\netprofmsvc_c.dll"));
        if c.is_file() {
            assert_eq!(xone_exports::pe_exports(&std::fs::read(&c).unwrap()).unwrap(), ords, "{}", c.display());
        }
    }
    let got = xone_exports::pe_exports(&std::fs::read(built()).unwrap()).unwrap();
    let want: Vec<(u16, String)> = rows.into_iter().filter(|e| e.status != "missing").map(|e| (e.ordinal, e.name)).collect();
    assert_eq!(got, want);
}

#[test]
fn loading_the_dll_runs_dll_main_and_unloading_tears_it_down() {
    let p = built();
    let before = (instances(&NETWORK_PROFILE), instances(&AGGREGATE));
    let w: Vec<u16> = p.to_str().unwrap().encode_utf16().chain(Some(0)).collect();
    let h = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!h.is_null());
    assert_eq!((instances(&NETWORK_PROFILE), instances(&AGGREGATE)), (before.0 + 1, before.1 + 1));
    unsafe {
        assert!(!GetProcAddress(h, b"DllMain\0".as_ptr()).is_null());
        assert!(!GetProcAddress(h, b"SvchostPushServiceGlobalsEx\0".as_ptr()).is_null());
        assert!(GetProcAddress(h, b"ServiceMain\0".as_ptr()).is_null());
        assert_ne!(FreeLibrary(h), 0);
    }
    assert_eq!((instances(&NETWORK_PROFILE), instances(&AGGREGATE)), before);
}

fn import_dlls(pe: &[u8]) -> Vec<String> {
    let rd16 = |o: usize| u16::from_le_bytes([pe[o], pe[o + 1]]) as usize;
    let rd32 = |o: usize| u32::from_le_bytes(pe[o..o + 4].try_into().unwrap()) as usize;
    let nt = rd32(0x3c);
    let sections = rd16(nt + 6);
    let opt = nt + 24;
    let sec0 = opt + rd16(nt + 20);
    let to_off = |rva: usize| {
        (0..sections).map(|i| sec0 + i * 40).find(|&s| rva >= rd32(s + 12) && rva < rd32(s + 12) + rd32(s + 8).max(rd32(s + 16))).map(|s| rva - rd32(s + 12) + rd32(s + 20))
    };
    let imp = rd32(opt + 112 + 8);
    let mut out = Vec::new();
    let Some(mut d) = to_off(imp) else { return out };
    while rd32(d + 12) != 0 {
        let n = to_off(rd32(d + 12)).unwrap();
        let end = pe[n..].iter().position(|b| *b == 0).unwrap();
        out.push(String::from_utf8_lossy(&pe[n..n + end]).into_owned());
        d += 20;
    }
    out
}

#[test]
fn service_build_dll_main_registers_only_its_own_providers_and_unloads() {
    let p = xone_exports::build_cdylib_with_features(env!("CARGO"), env!("CARGO_MANIFEST_DIR"), "netprofmsvc_c", "netprofmsvc_c.dll", "service");
    let bytes = std::fs::read(&p).unwrap();
    let rows: Vec<(u16, String)> = rows().into_iter().filter(|e| e.status != "missing").map(|e| (e.ordinal, e.name)).collect();
    assert_eq!(xone_exports::pe_exports(&bytes).unwrap(), rows);
    let deps = import_dlls(&bytes);
    assert!(deps.iter().any(|d| d.eq_ignore_ascii_case("WINHTTP.dll")));
    let mut held = Vec::new();
    for d in &deps {
        let w: Vec<u16> = d.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { LoadLibraryW(w.as_ptr()) };
        assert!(!h.is_null(), "{d}");
        held.push(h);
    }
    let owner_before = std::env::var("TOMBOX_GDK_OWNER").ok();
    let before = (instances(&NETWORK_PROFILE), instances(&AGGREGATE));
    let w: Vec<u16> = p.to_str().unwrap().encode_utf16().chain(Some(0)).collect();
    let h = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!h.is_null());
    assert_eq!((instances(&NETWORK_PROFILE), instances(&AGGREGATE)), (before.0 + 1, before.1 + 1));
    assert_eq!(std::env::var("TOMBOX_GDK_OWNER").ok(), owner_before);
    unsafe {
        assert!(GetProcAddress(h, b"ServiceMain\0".as_ptr()).is_null());
        assert_ne!(FreeLibrary(h), 0);
    }
    assert_eq!((instances(&NETWORK_PROFILE), instances(&AGGREGATE)), before);
    for h in held {
        unsafe { FreeLibrary(h) };
    }
}
