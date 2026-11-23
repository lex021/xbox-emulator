use core::ffi::c_void;
use pixrt10::runtime::{bind_xbtp_traffic, E_INVALIDARG, S_FALSE};
use pixrt10::session::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const DUMP_22000: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\pixrt10.dll";

extern "system" {
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn GetThreadDescription(h: usize, out: *mut *mut u16) -> i32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn GetActiveProcessorCount(group: u16) -> u32;
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

static SERIAL: Mutex<()> = Mutex::new(());

fn setup() -> std::sync::MutexGuard<'static, ()> {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pixrt10_session_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::set_var("TOMBOX_CONSOLE_OSBUILD", "22000");
    bind_xbtp_traffic(xbtp::xbtp2_sys::XbtpGetTcpNetworkTrafficStatistics);
    g
}

fn description(h: usize) -> String {
    let mut p: *mut u16 = core::ptr::null_mut();
    assert!(unsafe { GetThreadDescription(h, &mut p) } >= 0);
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p, n) });
    unsafe { LocalFree(p as *mut c_void) };
    s
}

fn wait_until(f: impl Fn() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn at(img: &[u8], rva: usize, n: usize) -> &[u8] {
    let pe = u32::from_le_bytes(img[0x3c..0x40].try_into().unwrap()) as usize;
    let cnt = u16::from_le_bytes(img[pe + 6..pe + 8].try_into().unwrap()) as usize;
    let opt = u16::from_le_bytes(img[pe + 0x14..pe + 0x16].try_into().unwrap()) as usize;
    for i in 0..cnt {
        let s = pe + 0x18 + opt + i * 0x28;
        let rd = |o: usize| u32::from_le_bytes(img[s + o..s + o + 4].try_into().unwrap()) as usize;
        let (va, sz, raw) = (rd(0xc), rd(0x8).max(rd(0x10)), rd(0x14));
        if rva >= va && rva < va + sz {
            return &img[rva - va + raw..rva - va + raw + n];
        }
    }
    panic!("rva {rva:#x}");
}

#[test]
fn constants_match_22000_dump() {
    let Ok(img) = std::fs::read(DUMP_22000) else { return };
    let checks: &[(usize, &[u8])] = &[
        (0x1a9a4, &[0xb9, 0x20, 0x02, 0x00, 0x00]),
        (0x3299d, &[0xba, 0x8e, 0x00, 0x00, 0x00]),
        (0x3337c, &[0xb9, 0x10, 0x08, 0x00, 0x00]),
        (0x333d8, &[0xb9, 0xe8, 0x00, 0x00, 0x00]),
        (0x33488, &[0xc7, 0x00, 0x62, 0x00, 0x00, 0x80]),
        (0x3348e, &[0xc7, 0x40, 0x04, 0x0c, 0x00, 0x00, 0x00]),
        (0x33495, &[0xc7, 0x40, 0x08, 0x54, 0x45, 0x4e, 0x58]),
        (0x3349c, &[0xc7, 0x40, 0x0c, 0x20, 0x00, 0x00, 0x00]),
        (0x34062, &[0x41, 0xbf, 0x62, 0x00, 0x00, 0x80]),
        (0x3406d, &[0xc7, 0x44, 0x24, 0x4c, 0x32, 0x00, 0x00, 0x00]),
        (0x34075, &[0xc7, 0x44, 0x24, 0x50, 0x55, 0x50, 0x43, 0x44]),
        (0x3407d, &[0xc7, 0x44, 0x24, 0x54, 0xb8, 0x00, 0x00, 0x00]),
        (0x34095, &[0xbb, 0x38, 0x02, 0x00, 0x00]),
        (0x340c5, &[0xc7, 0x45, 0x24, 0x92, 0x00, 0x00, 0x00]),
        (0x340cc, &[0xc7, 0x45, 0x28, 0x49, 0x50, 0x53, 0x44]),
        (0x340e6, &[0xb9, 0x64, 0x00, 0x00, 0x00]),
        (0x34bc2, &[0xbe, 0x58, 0x01, 0x00, 0x00]),
        (0x34bde, &[0x41, 0xbe, 0xa0, 0x02, 0x00, 0x00]),
        (0x34c13, &[0x41, 0x8d, 0x7d, 0x40]),
        (0x34fb7, &[0x41, 0xbf, 0x0e, 0x00, 0x00, 0x00]),
        (0x350ce, &[0xb9, 0x03, 0x00, 0x00, 0x20]),
        (0x45336, &[0xba, 0x4c, 0x20, 0xff, 0x8f]),
    ];
    for (rva, bytes) in checks {
        assert_eq!(at(&img, *rva, bytes.len()), *bytes, "rva {rva:#x}");
    }
    assert_eq!(f64::from_le_bytes(at(&img, 0x55548, 8).try_into().unwrap()), PERCENT);
    assert_eq!(tenx_header(), [PACKET_TAG, 0xc, TAG_TENX, 0x20, 1]);
    assert_eq!(upcd_header(), [PACKET_TAG, 0x32, TAG_UPCD, 0xb8, 2]);
    assert_eq!(ipsd_header(), [PACKET_TAG, 0x92, TAG_IPSD, 0x238, 0x402]);
    assert_eq!(START_FLAGS, F_SUMMARY | F_IPSD | F_PMC | F_NET);
}

fn cpu(idle: i64, kernel: i64, user: i64, dpc: i64, interrupt: i64) -> CpuTimes {
    CpuTimes { idle, kernel, user, dpc, interrupt }
}

#[test]
fn per_core_shares_follow_0x34fbd() {
    let prev = cpu(100, 300, 50, 10, 20);
    let cur = cpu(150, 500, 100, 30, 25);
    assert_eq!(cpu_shares(cur, prev), (10, 2));
    assert_eq!(cpu_shares(cpu(100, 100, 0, 9, 9), cpu(0, 0, 0, 0, 0)), (0, 0));
    assert_eq!(cpu_shares(cpu(0, 3, 0, 1, 2), cpu(0, 0, 0, 0, 0)), (33, 66));
    assert_eq!(cpu_shares(cpu(0, 1, 0, 0, -3), cpu(0, 0, 0, 0, 0)), (0, (-300i32) as u16));
    assert_eq!(cvtt32(f64::NAN), i32::MIN);
    assert_eq!(cvtt32(3.0e9), i32::MIN);
    assert_eq!(cvtt32(-2.9), -2);
    let mut b = [0u8; 0x60];
    for (i, v) in [1i64, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].iter().enumerate() {
        b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    assert_eq!(CpuTimes::read(&b, 1), cpu(7, 8, 9, 10, 11));
}

#[test]
fn nt_status_map_follows_0x34b40() {
    assert_eq!(nt_hresult(0), 0);
    for s in [0xC000_0003u32, 0xC000_0004, 0xC000_000B] {
        assert_eq!(nt_hresult(s as i32), E_INVALIDARG);
    }
    assert_eq!(nt_hresult(0xC000_0005u32 as i32), 0x8007_0005u32 as i32);
    assert_eq!(nt_hresult(0xC000_009Au32 as i32), 0x8000_FFFFu32 as i32);
    assert_eq!(nt_hresult(0x103), 0x8000_4005u32 as i32);
    assert_eq!(nt_hresult(0xC000_0001u32 as i32), 0x8000_4005u32 as i32);
}

#[test]
fn ipsd_sample_reads_the_host_like_the_console() {
    let _g = setup();
    let mut k = SampleState::new();
    assert_eq!(&k.packet()[..0x14], &[0x62, 0, 0, 0x80, 0x92, 0, 0, 0, 0x49, 0x50, 0x53, 0x44, 0x38, 2, 0, 0, 2, 4, 0, 0]);
    let mut perf = [0u8; SYSTEM_PERFORMANCE_BYTES as usize];
    let mut ret = 0u32;
    let perf_st = unsafe { NtQuerySystemInformation(2, perf.as_mut_ptr().cast(), SYSTEM_PERFORMANCE_BYTES, &mut ret) };
    let mut cpu_buf = [0u8; SYSTEM_PROCESSOR_BYTES as usize];
    let cpu_st = unsafe { NtQuerySystemInformation(8, cpu_buf.as_mut_ptr().cast(), SYSTEM_PROCESSOR_BYTES, &mut ret) };
    let cpus = (unsafe { GetActiveProcessorCount(0xffff) } as usize).min(CPU_SLOTS);
    let hr = unsafe { ipsd_sample(&mut k) };
    assert_eq!(k.samples(), 1);
    assert_eq!(k.get32(0x20), 0x1000);
    if perf_st != 0 {
        assert_eq!(hr, nt_hresult(perf_st));
        assert!(k.get64(0x18) != 0);
        return;
    }
    assert!(k.get64(0x18) != 0);
    assert!(k.0[0x28..0x180].iter().any(|b| *b != 0));
    if cpu_st != 0 {
        assert_eq!(hr, nt_hresult(cpu_st));
        assert_eq!(k.get64(0x1b8), 0);
        assert!(k.0[0x248..0x4e8].iter().all(|b| *b == 0));
        let hr2 = unsafe { ipsd_sample(&mut k) };
        assert_eq!(hr2, nt_hresult(cpu_st));
        assert_eq!(k.samples(), 2);
        assert!(k.0[0x180..0x1b8].iter().all(|b| *b == 0));
    } else {
        assert_eq!(hr, 0);
        assert!(k.0[0x248..0x4e8].iter().all(|b| *b == 0));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(unsafe { ipsd_sample(&mut k) }, 0);
        assert_eq!(k.samples(), 2);
        assert!(k.0[0x248..0x248 + cpus * CPU_ENTRY].iter().any(|b| *b != 0));
        assert!(k.get64(0x1b8) != 0 || k.get64(0x1c0) != 0 || k.get64(0x240) != 0 || k.get64(0x208) != 0);
    }
}

#[test]
fn session_runs_the_console_sampler_and_stops_on_zero_flags() {
    let _g = setup();
    unsafe {
        assert_eq!(ensure(START_FLAGS), 0);
        let s = session().unwrap();
        assert_eq!(s.flags(), START_FLAGS & !F_PMC);
        for i in 0..4 {
            assert!(s.block(i) != 0);
        }
        let t = s.tenx();
        assert_eq!(&t[..0x14], &[0x62, 0, 0, 0x80, 0xc, 0, 0, 0, 0x54, 0x45, 0x4e, 0x58, 0x20, 0, 0, 0, 1, 0, 0, 0]);
        let h = s.thread_handle();
        assert!(h != 0);
        assert_eq!(WaitForSingleObject(h, 0), WAIT_TIMEOUT);
        assert_eq!(description(h), THREAD_NAME);
        assert!(s.half_qpf() > 0);
        assert!(wait_until(|| u64::from_le_bytes(s.tenx()[0x18..0x20].try_into().unwrap()) != 0));
        assert!(wait_until(|| s.ticks() >= 3));
        assert_eq!(ensure(START_FLAGS), 0);
        assert_eq!(s.flags(), START_FLAGS & !F_PMC);
        assert_eq!(s.thread_handle(), h);
        let mut keep = 7u32;
        assert_eq!(s.set_flags(START_FLAGS & !F_PMC, Some(&mut keep)), S_FALSE);
        assert_eq!(keep, 7);
        assert_eq!(end(), 0);
        assert_eq!(s.flags(), 0);
        assert_eq!(WaitForSingleObject(h, 0), 0);
        assert_eq!(ensure(F_IPSD), 0);
        assert!(core::ptr::eq(session().unwrap(), s));
        assert_eq!(s.flags(), F_IPSD);
        let h2 = s.thread_handle();
        assert!(h2 != 0);
        assert_eq!(WaitForSingleObject(h2, 0), WAIT_TIMEOUT);
        let mut out = 0u32;
        assert_eq!(s.set_flags(F_IPSD | F_NET, Some(&mut out)), 0);
        assert_eq!(out, F_IPSD | F_NET);
        assert_eq!(s.thread_handle(), h2);
        destroy();
        assert!(session().is_none());
        assert_eq!(ensure(START_FLAGS), 0);
        assert_eq!(session().unwrap().flags(), START_FLAGS & !F_PMC);
        destroy();
    }
}
