use core::ffi::c_void;
use pixrt10::etw::*;

const DUMP_22000: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\pixrt10.dll";
const DUMP_22621: &str = r"D:\xbox-pc-port\games\Trackmania\extracted\Ubisoft.TrackmaniaNext_1.6.2.0_neutral__ngz4m417e0mpw\EmbeddedXvd\Windows\System32\pixrt10.dll";

extern "system" {
    fn GetCurrentProcess() -> usize;
    fn OpenProcessToken(p: usize, access: u32, t: *mut usize) -> i32;
    fn GetTokenInformation(t: usize, class: u32, out: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn CloseHandle(h: usize) -> i32;
}

fn elevated() -> bool {
    unsafe {
        let mut t = 0usize;
        assert_ne!(OpenProcessToken(GetCurrentProcess(), 8, &mut t), 0);
        let mut e = 0u32;
        let mut n = 0u32;
        assert_ne!(GetTokenInformation(t, 20, &mut e as *mut u32 as *mut c_void, 4, &mut n), 0);
        CloseHandle(t);
        e != 0
    }
}

fn setup() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pixrt10_etw_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::set_var("TOMBOX_CONSOLE_OSBUILD", "22000");
}

struct Pe {
    img: Vec<u8>,
    secs: Vec<(u32, u32, u32)>,
}

impl Pe {
    fn load(p: &str) -> Option<Pe> {
        let img = std::fs::read(p).ok()?;
        let pe = u32::from_le_bytes(img[0x3c..0x40].try_into().unwrap()) as usize;
        let n = u16::from_le_bytes(img[pe + 6..pe + 8].try_into().unwrap()) as usize;
        let opt = u16::from_le_bytes(img[pe + 0x14..pe + 0x16].try_into().unwrap()) as usize;
        let mut secs = Vec::new();
        for i in 0..n {
            let s = pe + 0x18 + opt + i * 0x28;
            let rd = |o: usize| u32::from_le_bytes(img[s + o..s + o + 4].try_into().unwrap());
            secs.push((rd(0xc), rd(0x8).max(rd(0x10)), rd(0x14)));
        }
        Some(Pe { img, secs })
    }

    fn at(&self, rva: u32, n: usize) -> &[u8] {
        let (va, _, raw) = *self.secs.iter().find(|(va, sz, _)| rva >= *va && rva < va + sz).unwrap();
        let o = (rva - va + raw) as usize;
        &self.img[o..o + n]
    }

    fn has(&self, needle: &[u8]) -> bool {
        self.img.windows(needle.len()).any(|w| w == needle)
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(Some(0)).flat_map(|u| u.to_le_bytes()).collect()
}

#[test]
fn constants_match_22000_dump() {
    let Some(pe) = Pe::load(DUMP_22000) else { return };
    let guids: [(u32, [u8; 16]); 11] = [
        (0x52248, SYSTEM_GUID),
        (0x52258, GUID_52258),
        (0x52268, GUID_52268),
        (0x52288, GUID_52288),
        (0x522a8, GUID_522A8),
        (0x522b8, GUID_522B8),
        (0x527a0, GUID_527A0),
        (0x527b0, GUID_527B0),
        (0x52820, GUID_52820),
        (0x52848, GUID_52848),
        (0x52858, GUID_52858),
    ];
    for (rva, g) in guids {
        assert_eq!(pe.at(rva, 16), &g, "guid at {rva:#x}");
    }
    for (i, (bit, v)) in GROUP_MASK_TABLE.iter().enumerate() {
        let e = pe.at(0x52f40 + i as u32 * 8, 8);
        assert_eq!(u32::from_le_bytes(e[0..4].try_into().unwrap()), *bit);
        assert_eq!(u32::from_le_bytes(e[4..8].try_into().unwrap()), *v);
    }
    assert_eq!(pe.at(0x527c0, SESSION_NAME.len() * 2 + 2), wide(SESSION_NAME).as_slice());
    let props = pe.at(0x362a8, 0xde);
    for pat in [
        &[0xc7, 0x06, 0xfc, 0x02, 0x00, 0x00][..],
        &[0xc7, 0x46, 0x2c, 0x00, 0x00, 0x02, 0x00],
        &[0xc7, 0x46, 0x74, 0x78, 0x00, 0x00, 0x00],
        &[0xc7, 0x46, 0x48, 0x00, 0x00, 0x00, 0x80],
        &[0xc6, 0x46, 0x4a, 0xff],
        &[0x25, 0x0e, 0x01, 0x00, 0x00],
        &[0x05, 0x02, 0x00, 0x00, 0x02],
        &[0xb8, 0xc0, 0x02, 0x00, 0x00],
        &[0xc7, 0x86, 0xc0, 0x02, 0x00, 0x00, 0x0f, 0x00, 0x03, 0x00],
        &[0xc7, 0x86, 0xc4, 0x02, 0x00, 0x00, 0x09, 0x00, 0x01, 0x00],
    ] {
        assert!(contains(props, pat), "0x362a8 lacks {pat:02x?}");
    }
    let cfg = pe.at(0x36388, 0x12a);
    for pat in [&[0xc7, 0x43, 0x44, 0xc8, 0x00, 0x00, 0x00][..], &[0xc7, 0x43, 0x3c, 0x00, 0x02, 0x00, 0x00], &[0xc7, 0x43, 0x70, 0xb8, 0x00, 0x00, 0x00], &[0xb9, 0x06, 0x00, 0x00, 0x00]] {
        assert!(contains(cfg, pat), "0x36388 lacks {pat:02x?}");
    }
    let base = pe.at(0x2afcc, 0x60);
    for pat in [&[0xc7, 0x44, 0x24, 0x20, 0x0a, 0x05, 0x00, 0x00][..], &[0x41, 0xb9, 0x00, 0x04, 0x00, 0x00], &[0x41, 0xb8, 0x40, 0x00, 0x00, 0x00]] {
        assert!(contains(base, pat), "0x2afcc lacks {pat:02x?}");
    }
    assert!(contains(pe.at(0x2460c, 0x50), &[0x48, 0xc7, 0x46, 0x4c, 0xe8, 0x03, 0x00, 0x00]));
    assert_eq!(f64::from_le_bytes(pe.at(0x55568, 8).try_into().unwrap()), PROFILE_CLOCK);
    let inline = pe.at(0x2b537, 0x1c);
    for (i, w) in GUID_INLINE_2B537.chunks(4).enumerate() {
        assert_eq!(&inline[i * 7 + 3..i * 7 + 7], w);
    }
    let start = pe.at(0x3678c, 0x2ca);
    assert!(contains(start, &[0x48, 0x83, 0xf8, 0x20]));
    assert!(contains(start, &[0xf7, 0x47, 0x50, 0x00, 0x00, 0x00, 0x01]));
    assert!(contains(start, &[0xb9, 0x02, 0x80, 0x00, 0x20]));
    assert!(contains(start, &[0x48, 0x3d, 0x00, 0x00, 0x00, 0x08]));
    let enable = pe.at(0x364b4, 0x2d5);
    assert!(contains(enable, &[0xc7, 0x45, 0x9c, 0x00, 0x40, 0x00, 0x80]));
    assert!(contains(enable, &[0x41, 0x80, 0xf9, 0x05]));
    let rundown = pe.at(0x36a58, 0x170);
    assert!(contains(rundown, &[0x41, 0xb8, 0x02, 0x00, 0x00, 0x00]));
    assert!(contains(rundown, &[0x40, 0x38, 0x6b, 0x14]));
}

#[test]
fn tables_present_in_22621_dump() {
    let Some(pe) = Pe::load(DUMP_22621) else { return };
    for g in [SYSTEM_GUID, GUID_52258, GUID_52268, GUID_52288, GUID_522A8, GUID_522B8, GUID_527A0, GUID_527B0, GUID_52820, GUID_52848, GUID_52858] {
        assert!(pe.has(&g), "22621 lacks {g:02x?}");
    }
    let table: Vec<u8> = GROUP_MASK_TABLE.iter().flat_map(|(a, b)| a.to_le_bytes().into_iter().chain(b.to_le_bytes())).collect();
    assert!(pe.has(&table));
    assert!(pe.has(&wide(SESSION_NAME)));
    assert!(pe.has(&[0xc7, 0x06, 0xfc, 0x02, 0x00, 0x00]));
    assert!(pe.has(&[0xc7, 0x86, 0xc0, 0x02, 0x00, 0x00, 0x0f, 0x00, 0x03, 0x00]));
}

fn desc() -> TimingDesc {
    TimingDesc { name: core::ptr::null(), cap_mb: 0, reserved: 0, f10: 0, f14: 0, f18: 0, f1c: 0, f20: 0, f24: 0, f28: 0, f2c: 0, f30: 0 }
}

#[test]
fn timing_config_from_descriptor() {
    let d = desc();
    assert_eq!(timing_config(&d), [1, 1, 1, 0, 0, 1, 1, 0, 0xa, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    let d = TimingDesc { f10: 7, f14: 1, f18: 1, f1c: 0x3e8, f20: 1, f24: 1, f28: 1, f2c: 1, f30: 1, ..desc() };
    assert_eq!(timing_config(&d), [1, 2, 2, 0, 7, 1, 1, 0, 0x3e8, 1, 2, 0, 0, 2, 2, 2, 2, 0]);
    assert_eq!(capture_options(&d).kind, 2);
    assert_eq!(capture_options(&desc()).kind, 1);
}

#[test]
fn kernel_session_properties() {
    let t = timing_config(&desc());
    let c = kernel_config(&t, &capture_options(&desc()));
    assert_eq!(c.name, SESSION_NAME);
    assert_eq!((c.buffer_kb, c.min_buffers, c.max_buffers, c.rate, c.mask, c.pid), (0x40, 0x400, 0x50a, 0xa, MASK_SAMPLED_PROFILE, 0));
    assert_eq!(c.stack_events, vec![StackEvent { guid: GUID_52258, kind: 0x2f, reserved: [0; 7] }]);
    assert!(c.providers.is_empty());
    assert_eq!(group_mask(&c), [0, 0x2, 0, 0, 0, 0, 0, 0]);
    let p = properties(&c, 0);
    assert_eq!(p.get::<u32>(0), 0x2fc);
    assert_eq!(p.get::<u32>(0x28), 3);
    assert_eq!(p.get::<u32>(0x2c), 0x2_0000);
    assert_eq!((p.get::<u32>(0x30), p.get::<u32>(0x34), p.get::<u32>(0x38)), (0x40, 0x400, 0x50a));
    assert_eq!(p.get::<u32>(0x40), 0x200_0110);
    assert_eq!(p.get::<u32>(0x44), 0xc8);
    assert_eq!(p.get::<u32>(0x48), 0x80ff_02c0);
    assert_eq!(p.get::<u32>(0x70), 0);
    assert_eq!(p.get::<u32>(0x74), 0x78);
    assert_eq!(p.get::<[u8; 16]>(0x18), [0; 16]);
    assert_eq!((p.get::<u32>(0x2c0), p.get::<u32>(0x2c4)), (0x3_000f, 0x1_0009));
    assert_eq!(p.get::<u32>(0x2cc), 0x2);
    assert_eq!((p.get::<u16>(0x2e8), p.get::<u16>(0x2ea), p.get::<u64>(0x2ec)), (3, 0, 0));
    assert_eq!((p.get::<u16>(0x2f4), p.get::<u16>(0x2f6), p.get::<u32>(0x2f8)), (2, 0, 0));
    let d = TimingDesc { f30: 0, f28: 1, ..desc() };
    let k = kernel_config(&timing_config(&d), &capture_options(&d));
    assert_ne!(k.pid, 0);
    let p = properties(&k, 0x1234);
    assert_eq!(p.get::<[u8; 16]>(0x18), SYSTEM_GUID);
    assert_eq!((p.get::<u16>(0x2f6), p.get::<u32>(0x2f8)), (2, k.pid));
    assert_eq!((p.get::<u16>(0x2ea), p.get::<u64>(0x2ec)), (6, 0x1234));
    let mut f = k.clone();
    f.log_file = "x".into();
    let p = properties(&f, 0);
    assert_eq!((p.get::<u32>(0x40), p.get::<u32>(0x44), p.get::<u32>(0x3c), p.get::<u32>(0x70), p.get::<u16>(0xb8)), (0x200_0002, 0, 0x200, 0xb8, b'x' as u16));
    f.profile_sources.push(ProfileSource { source: 1, interval: 2 });
    assert_eq!(group_mask(&f)[1], 0x2 | 0x400);
}

#[test]
fn user_session_providers() {
    let t = timing_config(&desc());
    let c = user_config(&t, &capture_options(&desc()));
    assert_eq!(c.mask, 7 | 0x10 | 0x800 | 0x100_0000);
    assert_eq!(c.providers.iter().map(|p| p.guid).collect::<Vec<_>>(), vec![GUID_52820, GUID_527A0]);
    let p = c.providers[1];
    assert_eq!((p.enable, p.rundown, p.has_level, p.level, p.timeout), (1, 1, 1, 3, u32::MAX));
    assert_eq!(c.stack_events, vec![StackEvent { guid: GUID_52258, kind: 0x2f, reserved: [0; 7] }]);
    assert_eq!(group_mask(&c), [0x7, 0x1004 | 0x200 | 0x2, 0, 0, 0, 0, 0, 0]);
    let d = TimingDesc { f14: 1, f18: 1, f1c: 1000, f20: 1, f24: 1, f28: 1, f2c: 1, f30: 1, ..desc() };
    let c = user_config(&timing_config(&d), &capture_options(&d));
    assert_eq!(c.providers.iter().map(|p| p.guid).collect::<Vec<_>>(), vec![GUID_52820, GUID_527A0, GUID_INLINE_2B537, GUID_52848]);
    assert_eq!(c.providers[1].level, 4);
    let q = c.providers[2];
    assert_eq!((q.rundown, q.any, q.stack, q.filter, q.filter_level, q.filter_any, q.level), (1, 6 | 4 | 9, 1, 1, 5, 1, 5));
    assert_eq!(c.providers[3].stack, 1);
    assert_ne!(c.pid, 0);
    let kinds: Vec<u8> = c.stack_events.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec![0x24, 0x32, 0x2e, 0x43, 0x44, 0x62, 0x63, 0x20, 0x21, 0x2f]);
    assert_eq!(c.mask, 7 | 0x10 | 0x800 | 0x100_0000 | 0x600_0200 | 0x4000);
}

#[test]
fn profile_interval_conversion() {
    assert_eq!(profile_interval(10), 1_000_000);
    assert_eq!(profile_interval(0x3e8), 10_000);
    assert_eq!(profile_interval(0), 0);
    assert_eq!(cvttsd2si(f64::INFINITY), i64::MIN);
}

fn win32_hr(e: u32) -> i32 {
    ((e & 0xffff) | 0x8007_0000) as i32
}

#[test]
fn kernel_session_start_takes_console_failure_path() {
    setup();
    if elevated() {
        return;
    }
    let c = kernel_config(&timing_config(&desc()), &capture_options(&desc()));
    assert_eq!(unsafe { start_session(&c) }, Err(win32_hr(1314)));
}

#[test]
fn start_trace_without_profile_interval_reaches_start_trace() {
    setup();
    let mut c = SessionConfig::timing();
    c.name = "TomBoxPixStartProbe".into();
    let r = unsafe { start_session(&c) };
    if elevated() {
        if r.is_ok() {
            assert_eq!(unsafe { stop_by_name(&c.name, None, None) }, 0);
        }
        return;
    }
    assert_eq!(r, Err(win32_hr(5)));
}

#[test]
fn session_name_limit() {
    let mut c = SessionConfig::timing();
    c.name = "N".repeat(0x20);
    assert_eq!(unsafe { start_session(&c) }, Err(E_INVALIDARG));
    assert_eq!(unsafe { capture_state(&c) }, E_INVALIDARG);
    assert_eq!(unsafe { stop_by_name(&c.name, None, None) }, E_INVALIDARG);
    c.name = "N".repeat(0x1f);
    assert_ne!(unsafe { stop_by_name(&c.name, None, None) }, E_INVALIDARG);
}

#[test]
fn stop_and_rundown_without_session() {
    setup();
    let mut lost = 7u64;
    assert_eq!(unsafe { stop_by_name("TomBoxPixNoSuchSession", Some(&mut lost), None) }, 0);
    assert_eq!(lost, 7);
    let mut c = user_config(&timing_config(&desc()), &capture_options(&desc()));
    c.name = "TomBoxPixNoSuchSession".into();
    assert_eq!(unsafe { capture_state(&c) }, win32_hr(4201));
}

#[test]
fn enable_providers_on_dead_handle() {
    setup();
    let c = user_config(&timing_config(&desc()), &capture_options(&desc()));
    assert_eq!(unsafe { enable_providers(&c, 0) }, E_INVALIDARG);
    let mut bad = c.clone();
    bad.providers[0].has_level = 1;
    bad.providers[0].level = 6;
    assert_eq!(unsafe { enable_providers(&bad, 0) }, E_INVALIDARG);
}
