use core::ffi::c_void;
use console_sandbox::crossvm;
use XVMAudioService::*;

const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const STEP: &str = "TOMBOX_VERIFY_CROSSVM_STEP";
const CLSID_XVM_EVENT: Guid = Guid { data1: 0x148EC3BF, data2: 0x349C, data3: 0x4EAC, data4: [0xB3, 0x73, 0x9F, 0xD5, 0x7C, 0xB4, 0x6D, 0x88] };
const IID_XVM_EVENT_OPENER: Guid = Guid { data1: 0xC4E70434, data2: 0x407D, data3: 0x416B, data4: [0x94, 0xBE, 0x97, 0x17, 0xB7, 0x90, 0x65, 0xFB] };
const PKEY_DEVICE_FRIENDLY_NAME: ([u8; 16], u32) = ([78, 37, 92, 164, 28, 223, 253, 78, 128, 32, 103, 209, 70, 168, 80, 224], 14);
const PKEY_AUDIO_ENGINE_DEVICE_FORMAT: ([u8; 16], u32) = ([77, 6, 159, 241, 44, 8, 39, 78, 188, 115, 104, 130, 161, 187, 142, 76], 0);

#[link(name = "kernel32")]
extern "system" {
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn ResetEvent(h: isize) -> i32;
    fn QueryPerformanceCounter(v: *mut i64) -> i32;
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
    fn VirtualQuery(p: *const c_void, b: *mut Mbi, n: usize) -> usize;
}

#[repr(C)]
#[derive(Default)]
struct Mbi {
    base: usize,
    alloc: usize,
    alloc_protect: u32,
    partition: u32,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
    pad: u32,
}

fn pc_dcas_blocks() -> Vec<usize> {
    let mut out = Vec::new();
    let mut a = 0usize;
    let mut m = Mbi::default();
    while unsafe { VirtualQuery(a as *const c_void, &mut m, core::mem::size_of::<Mbi>()) } != 0 {
        let readable = matches!(m.protect & 0xff, 0x02 | 0x04 | 0x08 | 0x20 | 0x40) && m.protect & 0x100 == 0;
        if m.state == 0x1000 && m.kind == 0x40000 && readable && m.size >= 0x340 {
            let head = unsafe { core::slice::from_raw_parts(m.base as *const u8, 8) };
            if head == [0x26, 0x03, 0, 0, b'D', b'C', b'A', b'S'] {
                out.push(m.base);
            }
        }
        a = m.base + m.size;
        if m.size == 0 || a >= 0x7fff_ffff_0000 {
            break;
        }
    }
    out
}

fn qpc_100ns() -> i64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    (c as i128 * 10_000_000 / f as i128) as i64
}

unsafe fn vslot<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

unsafe fn release(obj: *mut c_void) -> u32 {
    vslot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj)
}

fn console() -> Option<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("XVMAudioService.dll")).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

fn envelope(id: u32, object: u32) -> Vec<u8> {
    let mut e = vec![0u8; crossvm::ENVELOPE];
    e[8..12].copy_from_slice(&id.to_le_bytes());
    e[0xc..0x10].copy_from_slice(&object.to_le_bytes());
    e
}

fn send(e: &mut [u8]) -> i32 {
    let id = u32::from_le_bytes(e[8..12].try_into().unwrap());
    assert_eq!(unsafe { crossvm::call(e.len() as i32, e.as_mut_ptr()) }, 0);
    assert_eq!(u32::from_le_bytes(e[8..12].try_into().unwrap()), id, "the host echoes the call id");
    i32::from_le_bytes(e[4..8].try_into().unwrap())
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn wide_at(b: &[u8], o: usize, chars: usize) -> String {
    let w: Vec<u16> = (0..chars).map(|i| u16::from_le_bytes([b[o + 2 * i], b[o + 2 * i + 1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&w)
}

fn create(kind: u32, flag: u32, parent: u32, method: u32, args: &[u8]) -> (i32, Vec<u8>) {
    let mut e = envelope(crossvm::CREATE_OBJECT, 0);
    e[0x10..0x14].copy_from_slice(&kind.to_le_bytes());
    e[0x14..0x18].copy_from_slice(&flag.to_le_bytes());
    e[0x18..0x1c].copy_from_slice(&parent.to_le_bytes());
    e[0x20..0x24].copy_from_slice(&method.to_le_bytes());
    e[0x24..0x24 + args.len()].copy_from_slice(args);
    let hr = send(&mut e);
    (hr, e)
}

static PC_AUDIO: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_step(step: &str) {
    let _pc = PC_AUDIO.lock().unwrap_or_else(|e| e.into_inner());
    if console().is_none() {
        eprintln!("no console XVMAudioService.dll, skipped");
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe).args(["--exact", "inner", "--nocapture", "--test-threads=1"]).env(STEP, step).env("TOMBOX_AUDIO_MUTE", "1").env("TOMBOX_AUDIO_CAPTURE", "1").output().unwrap();
    let text = format!("exit={:?}\n{}{}", out.status.code().map(|c| c as u32), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    println!("{text}");
    assert!(out.status.success() && (text.contains("STEP OK") || text.contains("NO PC AUDIO ENDPOINT")), "{text}");
}

#[test]
fn crossvm_audio_renders_the_guest_ring_through_wasapi() {
    run_step("render");
}

#[test]
fn crossvm_audio_devices_sessions_and_volumes_follow_the_guest_layouts() {
    run_step("objects");
}

#[test]
fn crossvm_audio_guest_get_position_reads_wasapi_clock_records() {
    run_step("position");
}

#[test]
fn crossvm_audio_shared_stream_and_clock_adjustment_follow_the_guest_layouts() {
    run_step("shared");
}

#[test]
fn crossvm_audio_endpoint_and_spatial_queries_match_the_pc() {
    run_step("endpoint");
}

#[test]
fn crossvm_audio_client_items_and_property_vectors_match_the_pc() {
    run_step("items");
}

#[test]
fn crossvm_audio_capture_endpoint_posts_wasapi_packets_to_the_guest_queue() {
    run_step("capture");
}

#[test]
fn crossvm_audio_pc_events_become_guest_notification_frames() {
    run_step("notify");
}

#[test]
fn crossvm_audio_media_manager_calls_reach_the_pc_playback_manager() {
    run_step("media");
}

#[test]
fn crossvm_audio_property_values_over_0x104_bytes_follow_the_console_memcpy() {
    let data: Vec<u8> = (0..0x300u32).map(|i| (i * 7 + 3) as u8).collect();
    let mut e = envelope(crossvm::PROPERTY_GET_VALUE, 0);
    let room = e.len() - crossvm::PAYLOAD;
    let r = unsafe { e.as_mut_ptr().add(crossvm::PAYLOAD) };
    assert_eq!(unsafe { crossvm::put_property(r, room, 3, &data) }, 0, "MMDevAPI 5f240 memcpys the whole serialised value (no size check) and returns S_OK");
    let p = &e[crossvm::PAYLOAD..];
    assert_eq!(u32_at(p, crossvm::PROP_TAG), 3);
    assert_eq!(u64::from_le_bytes(p[crossvm::PROP_LEN..crossvm::PROP_LEN + 8].try_into().unwrap()), 0x300, "len is stored at R+0x11c after the copy");
    assert_eq!(&p[crossvm::PROP_BYTES..crossvm::PROP_LEN], &data[..crossvm::PROP_DATA], "the first 0x104 bytes land in the data field");
    assert_eq!(&p[crossvm::PROP_LEN + 8..crossvm::PROP_BYTES + 0x300], &data[crossvm::PROP_DATA + 8..], "the bytes after the len field are the rest of the value");
    let max = room - crossvm::PROP_BYTES;
    let big = vec![0x5au8; max];
    assert_eq!(unsafe { crossvm::put_property(r, room, 3, &big) }, 0, "a value filling the envelope payload still fits");
    let over = vec![0x5au8; max + 1];
    assert_eq!(unsafe { crossvm::put_property(r, room, 3, &over) }, 0x8007000Eu32 as i32, "past the payload the console memcpy overruns its 0x810 heap block; the host refuses");
    let small = [1u8, 2, 3, 4];
    assert_eq!(unsafe { crossvm::put_property(r, room, 4, &small) }, 0);
    let p = &e[crossvm::PAYLOAD..];
    assert_eq!((u32_at(p, crossvm::PROP_TAG), &p[crossvm::PROP_BYTES..crossvm::PROP_BYTES + 4], u64::from_le_bytes(p[crossvm::PROP_LEN..crossvm::PROP_LEN + 8].try_into().unwrap())), (4, &small[..], 4));
}

#[test]
fn crossvm_audio_spatial_stream_creators_check_args_and_activate_the_pc_stream() {
    run_step("spatial_stream");
}

#[test]
fn crossvm_audio_dcas_control_block_follows_the_console_layout() {
    use console_sandbox::crossvm::dcas;
    let mut store = vec![0u8; 0x8000 + 0x40];
    let off = (0x20 - (store.as_ptr() as usize & 0x1f)) & 0x1f;
    let b = &mut store[off..off + 0x8000];
    let mut fmt = vec![0u8; 0x12];
    fmt[0..2].copy_from_slice(&3u16.to_le_bytes());
    fmt[2..4].copy_from_slice(&1u16.to_le_bytes());
    fmt[4..8].copy_from_slice(&48000u32.to_le_bytes());
    fmt[8..12].copy_from_slice(&192000u32.to_le_bytes());
    fmt[12..14].copy_from_slice(&4u16.to_le_bytes());
    fmt[14..16].copy_from_slice(&32u16.to_le_bytes());
    let (frames, st, dy, props, meta) = (481u32, 2u32, 3u32, 0x28u32, 0x10u32);
    let shape = dcas::write(b, "TomBox spatial", frames, 0x1e0, st, dy, props, meta, &fmt).unwrap();
    let n = st + dy;
    let frame = (frames * 4 + 0x1f) & !0x1f;
    let data = ((0x1c + 2 * n * props + 2 * n * meta + 0x1f) & !0x1f) + 2 * n * frame;
    assert_eq!(shape, dcas::Shape { objects: n, properties: props, metadata: meta, frame_bytes: frame }, "fd8a4 / fcd60: objects = static + dynamic, per-object frame bytes align32(frames * 4)");
    assert_eq!((u32_at(b, 0), u32_at(b, 4), u32_at(b, 0x30)), (0x326, 0x5341_4344, 6), "fd8a4 header: 0x326, 'DCAS', control state 6");
    assert_eq!((u32_at(b, 0x34), u32_at(b, 0x48)), (0x1e0, 1), "PC oracle at activation: frames per packet 0x1e0, flags = valid (Start / Stop / Reset need (flags & 0x81) == 1)");
    assert!(b[8..0x30].iter().chain(&b[0x38..0x48]).chain(&b[0x4c..0x54]).all(|&x| x == 0), "fd8a4 zeroes the rest of the 0x4c control bytes");
    assert_eq!(dcas::total(535, 2, 0, 0x20, 0).unwrap(), 0x2220 + 0x340, "PC oracle: 535 frames, 2 static, 0x20 property bytes -> data 0x2220");
    assert_eq!((u32_at(b, 0x204), u32_at(b, 0x208), u32_at(b, 0x20c), u32_at(b, 0x210)), (frames, st, dy, data + 0x340), "fd8a4 endpoint block at +0x204");
    assert_eq!(wide_at(b, 0x184, 0x40), "TomBox spatial");
    assert_eq!(&b[0x214..0x214 + 0x12], &fmt[..]);
    let d = &b[0x340..];
    assert_eq!((u32_at(d, 0), u32_at(d, 4), u32_at(d, 8), u32_at(d, 0xc), u32_at(d, 0x10), u32_at(d, 0x14), u32_at(d, 0x18)), (0x1c, data, n, props, meta, frame, 0x5341_4244), "fcd60 data header 'DBAS'");
    assert_eq!(dcas::validate(b), 0, "fe6c0 accepts the host header");
    let (ps, r) = dcas::parse(b).unwrap();
    assert_eq!(ps, shape, "fd780 recomputes the same data size");
    let p1 = 0x1c + (props * n) as usize;
    let m0 = p1 + (props * n) as usize;
    let m1 = m0 + (meta * n) as usize;
    let s0 = (m1 + (meta * n) as usize + 0x1f) & !0x1f;
    assert_eq!(r, dcas::Regions { properties: [0x1c, p1], metadata: [m0, m1], samples: [s0, s0 + (frame * n) as usize] }, "fe608 ping-pong regions");
    assert_eq!(r.samples[1] + (frame * n) as usize, data as usize, "the second sample set ends at the data size");
    let mut bad = b[..0x400].to_vec();
    bad[4] ^= 1;
    assert_eq!(dcas::validate(&bad), 0x887c_0045u32 as i32, "fe6c0: wrong magic");
    let mut bad = b[..0x400].to_vec();
    bad[0..4].copy_from_slice(&0x325u32.to_le_bytes());
    assert_eq!(dcas::validate(&bad), 0x887c_0045u32 as i32, "fe6c0: wrong header size");
    let mut bad = b[..0x400].to_vec();
    bad[0x208..0x210].fill(0);
    assert_eq!(dcas::validate(&bad), 0x887c_0045u32 as i32, "fe6c0: static + dynamic == 0");
    let mut bad = b[..0x400].to_vec();
    bad[0x210..0x214].copy_from_slice(&0x33fu32.to_le_bytes());
    assert_eq!(dcas::validate(&bad), 0x887c_0045u32 as i32, "fe6c0: total below 0x340");
    let mut bad = b.to_vec();
    bad[0x340 + 0xc..0x340 + 0x10].copy_from_slice(&(props + 0x20).to_le_bytes());
    assert_eq!(dcas::parse(&bad).unwrap_err(), 0x887c_0045u32 as i32, "fd780: data size no longer matches the shape");
    assert_eq!(dcas::total(0x4000_0000, 1, 0, 0, 0).unwrap_err(), 0x8007_0216u32 as i32, "fd1fc: overflow is ERROR_ARITHMETIC_OVERFLOW");
    let mut ext = fmt.clone();
    ext[0x10..0x12].copy_from_slice(&0x101u16.to_le_bytes());
    ext.resize(0x12 + 0x101, 0);
    assert_eq!(dcas::write(b, "x", frames, 0x1e0, st, dy, props, meta, &ext).unwrap_err(), 0x8007_0057u32 as i32, "fd8a4: cbSize above 0x100 is E_INVALIDARG");
    assert_eq!(dcas::write(&mut store[off + 1..off + 1 + 0x7000], "x", frames, 0x1e0, st, dy, props, meta, &fmt).unwrap_err(), 0x887c_0054u32 as i32, "fcd60: data base must be 32-byte aligned");
    println!("dcas: {n} objects, data {data:#x} bytes, samples at {s0:#x}");
}

#[test]
fn inner() {
    let Ok(step) = std::env::var(STEP) else { return };
    let dir = std::env::temp_dir().join(format!("xvmaudio_crossvm_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::set_var("TOMBOX_XVMAUDIOSERVICE_CONSOLE", console().unwrap());
    let mut e = envelope(crossvm::GET_MIX_FORMAT, 0);
    let hr = send(&mut e);
    if hr < 0 {
        println!("NO PC AUDIO ENDPOINT: GetMixFormat {:#010x}", hr as u32);
        return;
    }
    let mix_len = 0x12 + u16::from_le_bytes([e[0x20], e[0x21]]) as usize;
    let mix = e[0x10..0x10 + mix_len].to_vec();
    match step.as_str() {
        "render" => render(&mix),
        "position" => position(&mix),
        "shared" => shared(&mix),
        "endpoint" => endpoint(&mix),
        "items" => items(&mix),
        "capture" => capture(),
        "notify" => notify(),
        "media" => media(),
        "spatial_stream" => spatial_stream(),
        "spatial_pump" => spatial_pump(),
        "spatial_variants" => spatial_variants(),
        _ => objects(&mix),
    }
    println!("STEP OK");
}

fn render(mix: &[u8]) {
    let mix_len = mix.len();
    let align = u16::from_le_bytes([mix[0xc], mix[0xd]]) as usize;
    let rate = u32_at(mix, 4);
    println!("PC mix format {mix_len:#x} bytes, {rate} Hz, block {align}");
    assert!(mix_len <= 0x48 && align > 0);

    let mut e = envelope(crossvm::GET_DEVICE_PERIOD, 0);
    e[0x10] = 1;
    e[0x11] = 1;
    assert_eq!(send(&mut e), 0);
    let period = i64::from_le_bytes(e[0x12..0x1a].try_into().unwrap());
    let min = i64::from_le_bytes(e[0x1a..0x22].try_into().unwrap());
    println!("GetDevicePeriod default {period} min {min} (hns, R+2 / R+0xa)");
    assert!(period > 0 && min > 0 && min <= period, "GetDevicePeriod writes default at R+2 and min at R+0xa");

    let (hr, e) = create(2, 1, 0, 0x2f, &[]);
    assert_eq!(hr, 0);
    let object = u32_at(&e, 0x1c);
    assert_ne!(object, 0);

    let mut e = envelope(crossvm::GET_STREAM_LATENCY, object);
    assert_eq!(send(&mut e), 0x8889_0001u32 as i32, "GetStreamLatency before Initialize is AUDCLNT_E_NOT_INITIALIZED, like WASAPI");

    let mut e = envelope(crossvm::INITIALIZE, object);
    let a = 0x10;
    e[a + 8..a + 16].copy_from_slice(&1_000_000i64.to_le_bytes());
    e[a + 0x18..a + 0x18 + mix_len].copy_from_slice(mix);
    assert_eq!(send(&mut e), 0, "Initialize with the PC mix format");
    let r = e[a..].to_vec();
    assert_eq!(u32_at(&r, crossvm::INIT_MEMORY), crossvm::KIND_CROSS_VM_MEMORY);
    assert_eq!(u32_at(&r, crossvm::INIT_EVENT1), crossvm::KIND_NONE, "desc1 (guest wrote) is kind 0, so ReleaseBuffer does not block on the handshake");
    assert_eq!(u32_at(&r, crossvm::INIT_EVENT2), crossvm::KIND_NONE, "desc2 (host consumed) is kind 0");
    assert_eq!(u32_at(&r, crossvm::INIT_EVENT3), crossvm::KIND_CROSS_VM_EVENT, "desc3 carries the period event the guest forwards to SetEventHandle");
    assert_eq!(u32_at(&r, crossvm::INIT_EVENT3 + 0x24), crossvm::EVENT_KIND_GAMECORE, "X = 2 selects the full-access Gamecore event");
    for o in [0x80usize, 0x84, 0x88, 0x8c, 0x90, 0x400, 0x528, 0x52c, 0x530] {
        assert_eq!(u32_at(&r, o), 0, "Initialize reply field {o:#x} is 0");
    }
    let index = u32_at(&r, crossvm::INIT_MEMORY + 0x24);
    let (base, pool_index, frames, event_name, _) = crossvm::stream_info(object).expect("stream");
    assert_eq!(index, pool_index as u32);
    assert_eq!(base, console_sandbox::logan::pool() + index as usize * console_sandbox::logan::PAGE);

    let mut e = envelope(crossvm::GET_STREAM_LATENCY, object);
    assert_eq!(send(&mut e), 0);
    println!("stream latency {} hns", i64::from_le_bytes(e[0x10..0x18].try_into().unwrap()));

    let m = console_module();
    assert_ne!(m, 0);
    let theirs: unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllGetClassObject").unwrap()) };
    let mut f = core::ptr::null_mut();
    assert_eq!(unsafe { theirs(&CLSID_LOGAN_BOUND, &IID_ICLASSFACTORY, &mut f) }, S_OK);
    let mut o = core::ptr::null_mut();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(f, 3)(f, core::ptr::null_mut(), &IID_IUNKNOWN, &mut o) }, S_OK);
    let mut desc = [0u8; 0x128];
    desc[0..4].copy_from_slice(&r[crossvm::INIT_MEMORY..crossvm::INIT_MEMORY + 4]);
    desc[8..0x2c].copy_from_slice(&r[crossvm::INIT_MEMORY + 4..crossvm::INIT_MEMORY + 0x28]);
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *const u8) -> i32>(o, 3)(o, desc.as_ptr()) }, S_OK, "the console XvmBuffer accepts the memory descriptor from the Initialize reply");
    let mut size = 0u32;
    let mut va = core::ptr::null_mut::<c_void>();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32>(o, 4)(o, &mut size, &mut va) }, S_OK);
    println!("guest XvmBuffer va {va:?} size {size:#x}, host block {base:#x}, {frames} frames, event {event_name}");
    assert_eq!(va as usize, base, "the guest maps the same pool run the host renders from");

    let mut ef = core::ptr::null_mut();
    assert_eq!(unsafe { theirs(&CLSID_XVM_EVENT, &IID_ICLASSFACTORY, &mut ef) }, S_OK);
    let mut eo = core::ptr::null_mut();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(ef, 3)(ef, core::ptr::null_mut(), &IID_XVM_EVENT_OPENER, &mut eo) }, S_OK, "console event class 148ec3bf");
    let mut edesc = [0u8; 0x128];
    edesc[0..4].copy_from_slice(&r[crossvm::INIT_EVENT3..crossvm::INIT_EVENT3 + 4]);
    edesc[8..0x2e].copy_from_slice(&r[crossvm::INIT_EVENT3 + 4..crossvm::INIT_EVENT3 + 0x2a]);
    let mut guest_event: isize = 0;
    let ehr = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *const u8, *mut isize) -> i32>(eo, 5)(eo, edesc.as_ptr(), &mut guest_event) };
    let opened = console_sandbox::vab::opened_names();
    println!("console GetHandle {:#010x} handle {guest_event:#x}, OpenEventW names {opened:?}", ehr as u32);
    assert_eq!(ehr, S_OK, "the console opens the host period event from desc3");
    assert!(event_name.starts_with(console_sandbox::vab::GAMECORE_BASE), "{event_name:?} vs {:?}", console_sandbox::vab::GAMECORE_BASE);
    assert!(opened.contains(&format!("{event_name}{}", console_sandbox::vab::SUFFIX)), "the console derives the same name the host created");
    assert_ne!(guest_event, 0);
    assert_eq!(u32::from_le_bytes(edesc[0..4].try_into().unwrap()), 0, "the console takes ownership of the 0x128-byte descriptor and clears its kind");
    let ev_obj = guest_event as *mut c_void;
    let guest_handle = unsafe { vslot::<unsafe extern "system" fn(*mut c_void) -> isize>(ev_obj, 3)(ev_obj) };
    println!("guest event object {ev_obj:?} -> handle {guest_handle:#x}");
    assert_ne!(guest_handle, 0);
    assert_ne!(unsafe { ResetEvent(guest_handle) }, 0, "X = 2 opens the event with EVENT_MODIFY_STATE, so the guest forwarder thread can ResetEvent it");

    let cb = va as *mut u8;
    let rd = |o: usize| unsafe { (cb.add(o) as *const u32).read_unaligned() };
    assert_eq!(rd(crossvm::CB_MAGIC_AT), crossvm::CB_MAGIC);
    assert_eq!(rd(crossvm::CB_FLAGS), crossvm::FLAG_POSITION_IS_READ, "flags bit 2 set, bit 1 clear: guest GetPosition returns READ");
    let c = crossvm::CB_CONFIG;
    let start = rd(c + crossvm::CFG_DATA_START) as usize;
    let end = rd(c + crossvm::CFG_DATA_END) as usize;
    let total = rd(c + crossvm::CFG_TOTAL) as usize;
    assert!(start < end && end <= total && total <= size as usize, "IsValidControlData rule start < end <= total within the mapped run");
    assert_eq!(rd(c) as usize, 0xc8 + mix_len - 0x12);
    assert_eq!((end - start) % align, 0);
    assert_eq!(unsafe { core::slice::from_raw_parts(cb.add(c + crossvm::CFG_FORMAT), mix_len) }, mix);

    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 1, "Stop on a stopped stream is S_FALSE, like WASAPI");
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0x8889_0005u32 as i32, "Start while running is AUDCLNT_E_NOT_STOPPED");
    let s0 = crossvm::stream_signals(object).unwrap();
    assert_eq!(unsafe { WaitForSingleObject(guest_handle, 1000) }, 0, "the host signals the period event with an empty ring, so an event-driven client waiting before its first write wakes");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let s1 = crossvm::stream_signals(object).unwrap();
    let per = (period as f64 / 10_000.0).max(1.0);
    println!("period signals in 200 ms: {} (engine period {per} ms)", s1 - s0);
    assert!(s1 - s0 >= 2, "one signal per engine period while running");
    assert_eq!(crossvm::stream_info(object).unwrap().4, 0, "nothing rendered from an empty ring");
    let mut e = envelope(crossvm::RESET, object);
    assert_eq!(send(&mut e), 0x8889_0005u32 as i32, "Reset while running is AUDCLNT_E_NOT_STOPPED");

    let ring = end - start;
    let bytes = (ring / 2 / align) * align;
    let pattern: Vec<u8> = (0..bytes).map(|i| (i * 7 % 251) as u8).collect();
    unsafe {
        core::ptr::copy_nonoverlapping(pattern.as_ptr(), cb.add(start), bytes);
        let w = &*(cb.add(crossvm::CB_WRITE) as *const std::sync::atomic::AtomicI64);
        w.fetch_add(bytes as i64, std::sync::atomic::Ordering::SeqCst);
    }
    let t = std::time::Instant::now();
    loop {
        let read = unsafe { (cb.add(crossvm::CB_READ) as *const i64).read_volatile() };
        if read >= bytes as i64 || t.elapsed().as_secs() > 5 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    let read = unsafe { (cb.add(crossvm::CB_READ) as *const i64).read_volatile() };
    let cap = crossvm::take_capture(object).expect("capture enabled");
    println!("host READ {read} of {bytes} written, captured {} bytes from the WASAPI render buffer", cap.len());
    assert_eq!(read, bytes as i64, "the host consumed exactly what the guest released");
    assert_eq!(&cap[..bytes], &pattern[..], "the bytes in the WASAPI render buffer are the guest's samples");
    let mut e = envelope(crossvm::RESET, object);
    assert_eq!(send(&mut e), 0, "Reset on a stopped stream succeeds");

    let (hr, e) = create(9, 0, object, 0, &[]);
    assert_eq!(hr, 0);
    let child = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::SET_SIMPLE_VOLUME, child);
    e[0x10..0x14].copy_from_slice(&0.25f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::GET_SIMPLE_VOLUME, child);
    assert_eq!(send(&mut e), 0);
    assert_eq!(f32_at(&e, 0x10), 0.25, "ISimpleAudioVolume f32 at A+0 / R+0");
    let mut e = envelope(crossvm::SET_SIMPLE_MUTE, child);
    e[0x10] = 1;
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::GET_SIMPLE_MUTE, child);
    assert_eq!(send(&mut e), 0);
    assert_eq!(e[0x10], 1, "mute u8 at A+0 / R+0");
    assert_eq!(crossvm::stream_gain(object).unwrap().0, Some(0.0), "mute reaches the PC stream gain");
    let mut e = envelope(crossvm::SET_SIMPLE_MUTE, child);
    e[0x10] = 0;
    assert_eq!(send(&mut e), 0);
    assert_eq!(crossvm::stream_gain(object).unwrap().0, Some(0.25));
    let mut e = envelope(crossvm::STREAM_GET_CHANNEL_COUNT, child);
    assert_eq!(send(&mut e), 0);
    let ch = u32_at(&e, 0x10);
    assert_eq!(ch, u16::from_le_bytes([mix[2], mix[3]]) as u32, "IAudioStreamVolume channel count is the stream's");
    let mut e = envelope(crossvm::STREAM_SET_CHANNEL_VOLUME, child);
    e[0x14..0x18].copy_from_slice(&0.5f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::STREAM_GET_ALL_VOLUMES, child);
    e[0x10..0x14].copy_from_slice(&ch.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    assert_eq!(u32_at(&e, 0x10), ch);
    assert_eq!(f32_at(&e, 0x14), 0.5);
    assert_eq!(crossvm::stream_gain(object).unwrap().1.unwrap()[0], 0.5, "channel volume reaches the PC IAudioStreamVolume");
    let mut e = envelope(crossvm::STREAM_SET_CHANNEL_VOLUME, child);
    e[0x10..0x14].copy_from_slice(&ch.to_le_bytes());
    e[0x14..0x18].copy_from_slice(&0.5f32.to_le_bytes());
    assert_eq!(send(&mut e), 0x8007_0057u32 as i32, "channel index out of range is E_INVALIDARG");

    unsafe { release(ev_obj) };
    unsafe { release(eo) };
    unsafe { release(ef) };
    unsafe { release(o) };
    unsafe { release(f) };
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);
    assert!(crossvm::stream_info(object).is_none());
}

fn objects(mix: &[u8]) {
    crossvm::set_hello(2);
    let (hr, _) = create(0x19, 1, 0, 0x7b, &[]);
    assert_eq!(hr, 0x8000_4005u32 as i32, "type 0x19 is accepted after the 0x00020008 hello and reaches the 0x7b instance-id parse, which fails on an empty key");
    crossvm::set_hello(1);
    let (hr, _) = create(0x19, 1, 0, 0x7b, &[]);
    assert_eq!(hr, 0x8007_0057u32 as i32, "19041 guests only take types below 0x17");
    crossvm::set_hello(2);

    let (hr, e) = create(1, 1, 0, 0, &[]);
    assert_eq!(hr, 0);
    let enumerator = u32_at(&e, 0x1c);
    let mut args = vec![0u8; 8];
    args[0..4].copy_from_slice(&0u32.to_le_bytes());
    args[4..8].copy_from_slice(&0u32.to_le_bytes());
    let (hr, e) = create(5, 0, enumerator, crossvm::METHOD_DEFAULT_ENDPOINT, &args);
    assert_eq!(hr, 0);
    let dev = u32_at(&e, 0x1c);
    assert_eq!(u32_at(&e, 0x10), 5, "the reply echoes the blob type");
    assert_eq!(u32_at(&e, 0x20), crossvm::METHOD_DEFAULT_ENDPOINT, "the reply echoes the method id the guest re-validates");
    let id = wide_at(&e, 0x10 + crossvm::BLOB_ID, crossvm::BLOB_ID_CHARS + 1);
    println!("method 0x80 default render endpoint id {id}");
    assert_eq!(id, console_sandbox::wasapi::default_id(0, 0).unwrap(), "the id at +0x1c is the PC default render endpoint");
    assert_eq!(u16::from_le_bytes([e[0x10 + 0x8a], e[0x10 + 0x8b]]), 0, "NUL by +0x8a");
    assert_eq!(crossvm::object_device(dev), Some(id.clone()));

    let mut a = vec![0u8; 8 + 0x70];
    a[8..8 + id.len() * 2].copy_from_slice(&id.encode_utf16().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>());
    let (hr, e) = create(6, 0, enumerator, crossvm::METHOD_ENDPOINT, &a);
    assert_eq!(hr, 0, "method 0x81 opens the endpoint by id");
    let ep = u32_at(&e, 0x1c);
    assert_eq!(crossvm::object_device(ep), Some(id.clone()));
    let mut bad = vec![0u8; 8 + 0x70];
    bad[8..8 + 14].copy_from_slice(&"{nope}".encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>());
    let (hr, _) = create(6, 0, enumerator, crossvm::METHOD_ENDPOINT, &bad);
    assert!(hr < 0, "an unknown endpoint id fails with the PC error {hr:#010x}");

    let mut e = envelope(crossvm::DEVICE_GET_STATE, dev);
    assert_eq!(send(&mut e), 0);
    assert_eq!(u32_at(&e, 0x10), 1, "DEVICE_STATE_ACTIVE");
    let mut e = envelope(crossvm::DEVICE_GET_ID, dev);
    assert_eq!(send(&mut e), 0);
    assert_eq!(wide_at(&e, 0x10, crossvm::DEVICE_ID_CHARS), id);
    assert_eq!(u64::from_le_bytes(e[0x10 + crossvm::DEVICE_ID_COUNT..0x18 + crossvm::DEVICE_ID_COUNT].try_into().unwrap()) as usize, (id.len() + 1) * 2);

    let mut e = envelope(crossvm::ENDPOINT_LIST, enumerator);
    let mut seen = Vec::new();
    for page in 0.. {
        e[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
        e[0x14..0x18].copy_from_slice(&0xfu32.to_le_bytes());
        e[0x18..0x20].fill(0);
        assert_eq!(send(&mut e), 0);
        let n = u32_at(&e, 0x18) as usize;
        let last = u32_at(&e, 0x1c);
        assert!(n <= 8 && last <= 1);
        for i in 0..n {
            let at = 0x20 + i * crossvm::LIST_ENTRY;
            assert_eq!(u16::from_le_bytes([e[at + 0x6e], e[at + 0x6f]]), 0, "NUL at entry+0x6e");
            let st = u32_at(&e, at + 0x70);
            assert!(st < 0xf);
            seen.push((wide_at(&e, at, crossvm::BLOB_ID_CHARS), st));
        }
        if last == 1 {
            println!("endpoint list: {} endpoints in {} pages", seen.len(), page + 1);
            break;
        }
        assert!(page < 64, "the host reaches the last page");
    }
    let pc = console_sandbox::wasapi::endpoints(2, 0xf).unwrap();
    assert_eq!(seen, pc, "0x70 pages through every PC endpoint, the same request each time");
    assert!(seen.iter().any(|(s, _)| *s == id));

    let mut e = envelope(crossvm::DEFAULT_ENDPOINT_TABLE, enumerator);
    assert_eq!(send(&mut e), 0);
    let n = u32_at(&e, 0x10) as usize;
    assert_eq!(u32_at(&e, 0x14), 1, "single page");
    let mut roles = Vec::new();
    for i in 0..n {
        let at = 0x18 + i * crossvm::TABLE_ENTRY;
        let (flow, slot) = (u32_at(&e, at), u32_at(&e, at + 4));
        assert!(flow < 2 && slot < 9);
        assert_eq!(u16::from_le_bytes([e[at + 0x76], e[at + 0x77]]), 0);
        let s = wide_at(&e, at + 8, crossvm::BLOB_ID_CHARS);
        assert_eq!(s, console_sandbox::wasapi::default_id(flow, slot).unwrap());
        roles.push((flow, slot));
    }
    assert!(roles.contains(&(0, 0)) && roles.contains(&(0, 1)) && roles.contains(&(0, 2)), "render console/multimedia/communications defaults");

    let (hr, e) = create(0xd, 0, dev, 0x26, &[]);
    assert_eq!(hr, 0);
    let ps = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::PROPERTY_GET_COUNT, ps);
    assert_eq!(send(&mut e), 0);
    let count = u32_at(&e, 0x10);
    assert!(count > 0);
    let mut e = envelope(crossvm::PROPERTY_GET_AT, ps);
    assert_eq!(send(&mut e), 0);
    let k0 = e[0x14..0x24].to_vec();
    assert_ne!(k0, vec![0u8; 16], "GetAt fmtid at R+4");
    let get = |fmtid: [u8; 16], pid: u32| {
        let mut e = envelope(crossvm::PROPERTY_GET_VALUE, ps);
        e[0x10..0x20].copy_from_slice(&fmtid);
        e[0x20..0x24].copy_from_slice(&pid.to_le_bytes());
        let hr = send(&mut e);
        (hr, e)
    };
    let (hr, e) = get(PKEY_DEVICE_FRIENDLY_NAME.0, PKEY_DEVICE_FRIENDLY_NAME.1);
    assert_eq!(hr, 0);
    assert_eq!(u32_at(&e, 0x24), 1, "VT_LPWSTR travels as tag 1");
    let name = wide_at(&e, 0x28, 0x82);
    let size = u64::from_le_bytes(e[0x10 + 0x11c..0x18 + 0x11c].try_into().unwrap()) as usize;
    println!("PKEY_Device_FriendlyName {name} ({size} bytes)");
    assert!(!name.is_empty() && size == (name.encode_utf16().count() + 1) * 2);
    let (hr, e) = get(PKEY_AUDIO_ENGINE_DEVICE_FORMAT.0, PKEY_AUDIO_ENGINE_DEVICE_FORMAT.1);
    assert_eq!(hr, 0);
    assert_eq!(u32_at(&e, 0x24), 3, "VT_BLOB travels as a serialised PROPVARIANT, tag 3");
    assert_eq!(u16::from_le_bytes([e[0x28], e[0x29]]), 0x41);
    let size = u64::from_le_bytes(e[0x10 + 0x11c..0x18 + 0x11c].try_into().unwrap()) as usize;
    assert!(size > 8 + 0x12, "blob bytes follow at data+8, cb = size - 8");

    let (hr, e) = create(0x12, 1, 0, 0x4e, &[]);
    assert_eq!(hr, 0);
    let vol = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::ENDPOINT_VOLUME_RANGE, vol);
    assert_eq!(send(&mut e), 0);
    let (lo, hi) = (f32_at(&e, 0x10), f32_at(&e, 0x14));
    assert!(lo < hi, "GetVolumeRange min/max/inc from the PC endpoint");
    let mut e = envelope(crossvm::SET_ENDPOINT_VOLUME_SCALAR, vol);
    e[0x10..0x14].copy_from_slice(&0.5f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::GET_ENDPOINT_VOLUME_SCALAR, vol);
    assert_eq!(send(&mut e), 0);
    assert_eq!(f32_at(&e, 0x10), 0.5, "endpoint volume f32 at A+0");
    let mut e = envelope(crossvm::GET_ENDPOINT_VOLUME_LEVEL, vol);
    assert_eq!(send(&mut e), 0);
    println!("endpoint scalar 0.5 = {} dB (range {lo}..{hi})", f32_at(&e, 0x10));
    let mut e = envelope(crossvm::SET_ENDPOINT_MUTE, vol);
    e[0x10] = 1;
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::SET_ENDPOINT_MUTE, vol);
    e[0x10] = 1;
    assert_eq!(send(&mut e), 1, "SetMute to the current value is S_FALSE");
    let mut e = envelope(crossvm::GET_ENDPOINT_MUTE, vol);
    assert_eq!(send(&mut e), 0);
    assert_eq!(e[0x10], 1);
    let mut e = envelope(crossvm::SET_ENDPOINT_MUTE, vol);
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::SET_ENDPOINT_VOLUME_SCALAR, vol);
    e[0x10..0x14].copy_from_slice(&1.0f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::ENDPOINT_REGISTER, vol);
    assert_eq!(send(&mut e), 0);
    assert!(crossvm::notifications_registered(vol));
    let mut e = envelope(crossvm::ENDPOINT_UNREGISTER, vol);
    assert_eq!(send(&mut e), 0);
    assert!(!crossvm::notifications_registered(vol));

    let (hr, e) = create(0x14, 1, 0, 0x67, &[]);
    assert_eq!(hr, 0);
    let sess = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::SESSION_GET_STATE, sess);
    assert_eq!(send(&mut e), 0);
    assert!(u32_at(&e, 0x10) <= 2);
    let mut e = envelope(crossvm::SESSION_GET_IDENTIFIER, sess);
    assert_eq!(send(&mut e), 0);
    let sid = wide_at(&e, 0x10, crossvm::SESSION_STRING_CHARS);
    println!("session identifier {sid}");
    assert!(!sid.is_empty());
    let mut e = envelope(crossvm::SESSION_IS_SYSTEM_SOUNDS, sess);
    assert_eq!(send(&mut e), 0);
    assert_eq!(u32_at(&e, 0x10), 1, "the title session is not the system sounds session (S_FALSE)");
    let mut e = envelope(crossvm::SESSION_SET_DISPLAY_NAME, sess);
    let label: Vec<u8> = "TomBox guest".encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect();
    e[0x10..0x10 + label.len()].copy_from_slice(&label);
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::SESSION_GET_DISPLAY_NAME, sess);
    assert_eq!(send(&mut e), 0);
    assert_eq!(wide_at(&e, 0x10, crossvm::DISPLAY_NAME_CHARS), "TomBox guest", "display name round-trips through the PC session");
    let mut e = envelope(crossvm::SESSION_SET_CHANNEL_VOLUME, sess);
    e[0x14..0x18].copy_from_slice(&0.75f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::SESSION_GET_CHANNEL_VOLUME, sess);
    assert_eq!(send(&mut e), 0);
    assert_eq!(f32_at(&e, 0x10), 0.75);
    let mut e = envelope(crossvm::SESSION_SET_CHANNEL_VOLUME, sess);
    e[0x14..0x18].copy_from_slice(&1.0f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);

    let mut e = envelope(crossvm::IS_FORMAT_SUPPORTED, dev);
    e[0x10] = 1;
    let mut odd = mix.to_vec();
    odd[4..8].copy_from_slice(&11025u32.to_le_bytes());
    let ba = u16::from_le_bytes([odd[0xc], odd[0xd]]) as u32;
    odd[8..12].copy_from_slice(&(11025 * ba).to_le_bytes());
    e[0x16..0x16 + odd.len()].copy_from_slice(&odd);
    let hr = send(&mut e);
    println!("IsFormatSupported 11025 Hz -> {hr:#010x}, closest flag {}", e[0x11]);
    if hr == 1 {
        assert_eq!(e[0x11], 1, "A+1 = 1 when the host gives a closest match");
        assert_eq!(&e[0x10 + 0x4e..0x10 + 0x4e + 0x12][4..8], &mix[4..8], "closest match at A+0x4e");
    }
    let mut e = envelope(crossvm::IS_FORMAT_SUPPORTED, dev);
    e[0x16..0x16 + mix.len()].copy_from_slice(mix);
    assert_eq!(send(&mut e), 0, "the mix format is supported in shared mode");

    let mut e = envelope(0x64, dev);
    assert_eq!(send(&mut e), 0x8007_0057u32 as i32, "0x64 on a device object, not a 0x61 MediaManager, is refused");
    let mut e = envelope(0xfff, dev);
    assert_eq!(send(&mut e), 0x8000_4001u32 as i32, "an id with no decoded layout reports E_NOTIMPL");
    for o in [ps, vol, sess, ep, dev, enumerator] {
        let mut e = envelope(crossvm::RELEASE_OBJECT, o);
        assert_eq!(send(&mut e), 0);
    }
}

fn init_stream(mix: &[u8], flags: u32) -> (u32, i32, Vec<u8>) {
    let (hr, e) = create(2, 1, 0, 0x2f, &[]);
    assert_eq!(hr, 0);
    let object = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::INITIALIZE, object);
    let a = 0x10;
    e[a + crossvm::INIT_FLAGS..a + crossvm::INIT_FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
    e[a + 8..a + 16].copy_from_slice(&1_000_000i64.to_le_bytes());
    e[a + 0x18..a + 0x18 + mix.len()].copy_from_slice(mix);
    let hr = send(&mut e);
    (object, hr, e[a..].to_vec())
}

fn guest_position(cb: *const u8, align: u64) -> Option<(u64, i64)> {
    let r32 = |o: usize| unsafe { (cb.add(o) as *const u32).read_volatile() };
    let r64 = |o: usize| unsafe { (cb.add(o) as *const u64).read_volatile() };
    let flags = r32(crossvm::CB_FLAGS);
    if flags & 4 == 0 {
        return Some((r64(0x98) / align, -1));
    }
    if flags & 2 == 0 {
        return None;
    }
    let mut got = None;
    for _ in 0..3 {
        let idx = r32(0xa0) as usize;
        let rec = 0x20 + idx * 0x28;
        let v = (r64(rec), r64(rec + 0x18) as i64, unsafe { (cb.add(rec + 0x20) as *const f32).read_volatile() }, r32(rec + 0x24));
        if r32(0xa0) as usize == idx {
            got = Some(v);
            break;
        }
    }
    let (pos, qpc, rate, valid) = got.expect("the guest retries the seqlock 3 times (12cdb -> 3b6b0)");
    assert_eq!(valid, 1);
    let (base, pad, anchor) = (r64(0x70), r64(0x80), r64(0x98) / align);
    let now = qpc_100ns();
    assert!(now >= qpc, "record QPC {qpc} is not in the future of {now}");
    if pos < base + pad {
        return Some((anchor, now - qpc));
    }
    let elapsed = ((now - qpc) as f64 * rate as f64 / 1e7 + 0.5) as u64;
    let read = r64(crossvm::CB_READ) / align;
    Some(((pos - base - pad + elapsed + anchor).min(read), now - qpc))
}

fn position(mix: &[u8]) {
    let align = u16::from_le_bytes([mix[0xc], mix[0xd]]) as u64;
    let rate = u32_at(mix, 4);
    let (object, hr, _) = init_stream(mix, 0);
    assert_eq!(hr, 0);
    let (base, _, _, _, _) = crossvm::stream_info(object).unwrap();
    let cb = base as *mut u8;
    let r32 = |o: usize| unsafe { (cb.add(o) as *const u32).read_volatile() };
    assert_eq!(r32(crossvm::CB_FLAGS), crossvm::FLAG_POSITION_IS_READ, "before the first clock record the guest reads READ");
    let c = crossvm::CB_CONFIG;
    let (start, end) = (r32(c + crossvm::CFG_DATA_START) as usize, r32(c + crossvm::CFG_DATA_END) as usize);
    let bytes = ((end - start) / 2 / align as usize) * align as usize;
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    let mut written = 0usize;
    let mut samples = Vec::new();
    while t.elapsed().as_millis() < 1500 {
        let read = unsafe { (cb.add(crossvm::CB_READ) as *const i64).read_volatile() } as usize;
        if written - read < bytes / 2 {
            let n = bytes / 2;
            for i in 0..n {
                unsafe { *cb.add(start + (written + i) % (end - start)) = 0 };
            }
            written += n;
            unsafe { (*(cb.add(crossvm::CB_WRITE) as *const std::sync::atomic::AtomicI64)).fetch_add(n as i64, std::sync::atomic::Ordering::SeqCst) };
        }
        if let Some(p) = guest_position(cb, align) {
            samples.push(p);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let (records, rec_rate, _) = crossvm::stream_records(object).unwrap();
    println!("{records} WASAPI clock records, {} guest GetPosition samples, last {:?}, rate {rec_rate}", samples.len(), samples.last());
    assert!(records > 10, "the host publishes IAudioClock records while running");
    assert_eq!(rec_rate, rate as f32, "the record rate equals the stream rate, so the guest takes the non-adjusted path");
    assert!(samples.len() > 50, "flags bits 1 and 2 are set once the first record lands");
    let slack = rate as u64 / 1000;
    let back = samples.windows(2).map(|w| w[0].0.saturating_sub(w[1].0)).max().unwrap_or(0);
    println!("largest step back between guest GetPosition samples: {back} frames (1 ms = {slack})");
    assert!(back <= slack, "guest position is monotonic within 1 ms of clock jitter");
    assert!(samples.iter().all(|s| s.1 < 2_000_000), "every record QPC is within 200 ms of the guest QPC");
    let played = samples.last().unwrap().0;
    let wall = (t.elapsed().as_secs_f64() * rate as f64) as u64;
    println!("guest position {played} frames after {wall} wall-clock frames");
    assert!(played > wall / 4 && played <= wall + rate as u64 / 10, "the interpolated position follows the PC device clock");
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    while r32(crossvm::CB_FLAGS) & 4 != 0 && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let read_frames = unsafe { (cb.add(crossvm::CB_READ) as *const u64).read_volatile() } / align;
    let s1 = guest_position(cb, align).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let s2 = guest_position(cb, align).unwrap();
    println!("stopped: guest GetPosition {} then {} frames (READ {read_frames})", s1.0, s2.0);
    assert_eq!(s1.1, -1, "after Stop flags bit 2 is clear and the guest returns cb+0x98");
    assert_eq!(s1.0, s2.0, "the stopped position is frozen");
    assert!(s1.0 <= read_frames && s1.0 + slack >= played, "the frozen value is the PC clock at Stop, not READ");
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let mut resumed = Vec::new();
    let t = std::time::Instant::now();
    while t.elapsed().as_millis() < 400 {
        if let Some(p) = guest_position(cb, align) {
            resumed.push(p.0);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    let back = resumed.iter().fold((s1.0, 0u64), |(prev, worst), &p| (p, worst.max(prev.saturating_sub(p)))).1;
    println!("after restart: {} samples from {:?} to {:?}, largest step back {back}", resumed.len(), resumed.first(), resumed.last());
    assert!(back <= slack, "restart continues from the frozen position (cb+0x98 anchor) without going back");
    let mut e = envelope(crossvm::RESET, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    while crossvm::stream_records(object).unwrap().2 == 0 && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(guest_position(cb, align).unwrap(), (0, -1), "after Reset the guest reads position 0 from cb+0x98");
    unsafe { (*(cb.add(crossvm::CB_READ) as *const std::sync::atomic::AtomicI64)).store(0, std::sync::atomic::Ordering::SeqCst) };
    unsafe { (*(cb.add(crossvm::CB_WRITE) as *const std::sync::atomic::AtomicI64)).store(0, std::sync::atomic::Ordering::SeqCst) };
    let t = std::time::Instant::now();
    while crossvm::stream_records(object).unwrap().2 < 2 && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(crossvm::stream_records(object).unwrap().2, 2, "a 19041 guest Reset (local 20b70 zeroes READ) also resets the PC client");
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    let mut fed = 0i64;
    while t.elapsed().as_millis() < 300 {
        let read = unsafe { (cb.add(crossvm::CB_READ) as *const i64).read_volatile() };
        if fed - read < (bytes / 2) as i64 {
            unsafe { (*(cb.add(crossvm::CB_WRITE) as *const std::sync::atomic::AtomicI64)).fetch_add((bytes / 2) as i64, std::sync::atomic::Ordering::SeqCst) };
            fed += (bytes / 2) as i64;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let before = crossvm::stream_records(object).unwrap().2;
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    unsafe { (*(cb.add(crossvm::CB_READ) as *const std::sync::atomic::AtomicI64)).store(0, std::sync::atomic::Ordering::SeqCst) };
    unsafe { (*(cb.add(crossvm::CB_WRITE) as *const std::sync::atomic::AtomicI64)).store((bytes / 2) as i64, std::sync::atomic::Ordering::SeqCst) };
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    while crossvm::stream_records(object).unwrap().2 == before && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(crossvm::stream_records(object).unwrap().2, before + 1, "Stop, guest Reset, Start back to back still resets the PC client");
    std::thread::sleep(std::time::Duration::from_millis(30));
    let quick = guest_position(cb, align).map(|p| p.0).unwrap_or(0);
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    println!("back-to-back Stop/Reset/Start: guest GetPosition {quick} frames 30 ms after Start (before reset {})", s2.0);
    assert!(quick < rate as u64 / 10, "after a back-to-back guest Reset the position restarts near 0");
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);
}

fn shared(mix: &[u8]) {
    let rate = u32_at(mix, 4);
    let mut e = envelope(crossvm::GET_SHARED_MODE_ENGINE_PERIOD, 0);
    e[0x10..0x10 + mix.len()].copy_from_slice(mix);
    assert_eq!(send(&mut e), 0);
    let (def, fund, min, max) = (u32_at(&e, 0x58), u32_at(&e, 0x5c), u32_at(&e, 0x60), u32_at(&e, 0x64));
    println!("engine periods default {def} fundamental {fund} min {min} max {max} frames");
    assert!(fund > 0 && min <= def && def <= max);
    let (hr, e) = create(2, 1, 0, 0x2f, &[]);
    assert_eq!(hr, 0);
    let object = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::CLOCK_SET_SAMPLE_RATE, object);
    e[0x10..0x14].copy_from_slice(&(rate as f32).to_le_bytes());
    assert_eq!(send(&mut e), 0x8889_0001u32 as i32, "SetSampleRate before the stream exists is AUDCLNT_E_NOT_INITIALIZED");
    let mut e = envelope(crossvm::INITIALIZE_SHARED_AUDIO_STREAM, object);
    let a = 0x10;
    e[a..a + mix.len()].copy_from_slice(mix);
    e[a + crossvm::SHARED_INIT_PERIOD..a + crossvm::SHARED_INIT_PERIOD + 4].copy_from_slice(&def.to_le_bytes());
    let hr = send(&mut e);
    println!("InitializeSharedAudioStream({def} frames) -> {hr:#010x}");
    assert_eq!(hr, 0);
    let r = e[a..].to_vec();
    let l = &crossvm::SHARED_INIT_LAYOUT;
    assert_eq!(&r[..mix.len()], mix, "the reply keeps the format at A+0 (parser checks cbSize at env+0x20)");
    assert_eq!(u32_at(&r, crossvm::SHARED_INIT_PERIOD), def);
    assert_eq!(&r[l.clsid..l.clsid + 4], &0xCD773740u32.to_le_bytes(), "render endpoint CLSID at +0x60");
    assert_eq!(u32_at(&r, l.event1), crossvm::KIND_NONE);
    assert_eq!(u32_at(&r, l.event2), crossvm::KIND_NONE);
    assert_eq!(u32_at(&r, l.event3), crossvm::KIND_CROSS_VM_EVENT);
    assert_eq!(u32_at(&r, l.event3 + 0x24), crossvm::EVENT_KIND_GAMECORE);
    assert_eq!(u32_at(&r, 0x3f0), 0, "+0x3f0 (the Initialize +0x400 flag) is 0");
    assert_eq!(u32_at(&r, l.memory), crossvm::KIND_CROSS_VM_MEMORY);
    for o in [0x70usize, 0x74, 0x78, 0x7c, 0x80, 0x518, 0x51c, 0x520] {
        assert_eq!(u32_at(&r, o), 0, "reply field {o:#x} is 0");
    }
    let (_, pool, frames, _, _) = crossvm::stream_info(object).unwrap();
    assert_eq!(u32_at(&r, l.memory + 0x24), pool as u32);
    assert!(frames >= def * 2, "the ring holds two engine periods ({frames} frames)");
    let mut e = envelope(crossvm::INITIALIZE_SHARED_AUDIO_STREAM, object);
    e[a..a + mix.len()].copy_from_slice(mix);
    e[a + crossvm::SHARED_INIT_PERIOD..a + crossvm::SHARED_INIT_PERIOD + 4].copy_from_slice(&def.to_le_bytes());
    assert_eq!(send(&mut e), 0x8889_0002u32 as i32, "a second initialisation is AUDCLNT_E_ALREADY_INITIALIZED");
    let s0 = crossvm::stream_signals(object).unwrap();
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    std::thread::sleep(std::time::Duration::from_millis(150));
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    assert!(crossvm::stream_signals(object).unwrap() - s0 >= 2, "the shared-stream path signals the period event");
    let mut e = envelope(crossvm::CLOCK_SET_SAMPLE_RATE, object);
    e[0x10..0x14].copy_from_slice(&(rate as f32 + 10.0).to_le_bytes());
    let hr = send(&mut e);
    println!("SetSampleRate on a stream without RATEADJUST -> {hr:#010x}");
    assert!(hr < 0, "WASAPI refuses IAudioClockAdjustment without AUDCLNT_STREAMFLAGS_RATEADJUST");
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);

    let (adj, hr, _) = init_stream(mix, console_sandbox::wasapi::STREAMFLAGS_RATEADJUST);
    println!("Initialize with RATEADJUST -> {hr:#010x}");
    assert_eq!(hr, 0);
    let target = rate as f32 + 10.0;
    let mut e = envelope(crossvm::CLOCK_SET_SAMPLE_RATE, adj);
    e[0x10..0x14].copy_from_slice(&target.to_le_bytes());
    let hr = send(&mut e);
    println!("IAudioClockAdjustment::SetSampleRate({target}) -> {hr:#010x}");
    assert_eq!(hr, 0);
    assert_eq!(crossvm::stream_records(adj).unwrap().1, target, "the PC IAudioClockAdjustment took the rate");
    let mut e = envelope(crossvm::RELEASE_OBJECT, adj);
    assert_eq!(send(&mut e), 0);
}

fn endpoint(_mix: &[u8]) {
    use console_sandbox::wasapi;
    let (hr, e) = create(1, 1, 0, 0, &[]);
    assert_eq!(hr, 0);
    let enumerator = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::ENUMERATOR_SUBSCRIBE, enumerator);
    assert_eq!(send(&mut e), 0);
    assert!(crossvm::notifications_registered(enumerator), "0x23 (sent from the enumerator init and re-sent on reconnect) subscribes");
    let mut e = envelope(crossvm::ENUMERATOR_UNSUBSCRIBE, enumerator);
    assert_eq!(send(&mut e), 0);
    assert!(!crossvm::notifications_registered(enumerator), "0x24 (sent from the enumerator destructor) unsubscribes");

    let (hr, e) = create(5, 0, enumerator, crossvm::METHOD_DEFAULT_ENDPOINT, &[0u8; 8]);
    assert_eq!(hr, 0);
    let dev = u32_at(&e, 0x1c);
    let id = crossvm::object_device(dev).unwrap();
    let mut e = envelope(crossvm::ENDPOINT_HOST_STREAM_MODES, dev);
    e[0x10..0x18].fill(0xcc);
    assert_eq!(send(&mut e), 0);
    assert_eq!((u32_at(&e, 0x10), u32_at(&e, 0x14)), (0, 0), "0x29 R+0/R+4: the host forces no console stream-flag bits 25/26");
    let mut e = envelope(crossvm::ENDPOINT_OFFLOAD_SERVICE, dev);
    e[0x10..0x14].fill(0xcc);
    assert_eq!(send(&mut e), 0);
    let off = wasapi::endpoint_offload(Some(&id)).unwrap();
    println!("0x2a offload {} (PC IAudioClient2::IsOffloadCapable over all categories {off})", u32_at(&e, 0x10));
    assert_eq!(u32_at(&e, 0x10), off);

    let pc = match wasapi::spatial(Some(&id)) {
        Ok(p) => p,
        Err(hr) => {
            let (chr, _) = create(crossvm::FORMAT_ENUM_TYPE, 0, dev, 0, &[]);
            assert_eq!(chr, hr, "no PC ISpatialAudioClient: type 4 creation returns the PC error");
            let mut e = envelope(crossvm::SPATIAL_STATIC_POSITION, dev);
            assert_eq!(send(&mut e), hr, "no PC ISpatialAudioClient: the PC error is returned");
            println!("PC ISpatialAudioClient unavailable {hr:#010x}");
            let mut e = envelope(crossvm::RELEASE_OBJECT, dev);
            assert_eq!(send(&mut e), 0);
            return;
        }
    };
    let (hr, e) = create(crossvm::FORMAT_ENUM_TYPE, 0, dev, 0, &[]);
    let direct = pc.format_enumerator();
    let sc = match &direct {
        Ok(_) => {
            assert_eq!(hr, 0, "125980 -> 124574: type 4 wraps the PC IAudioFormatEnumerator");
            u32_at(&e, 0x1c)
        }
        Err(pe) => {
            assert_eq!(hr, *pe, "type 4 returns the PC GetSupportedAudioObjectFormatEnumerator error");
            0
        }
    };
    let mut e = envelope(crossvm::SPATIAL_STATIC_POSITION, dev);
    e[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
    let hr = send(&mut e);
    match pc.static_position(2) {
        Ok(p) => {
            assert_eq!(hr, 0);
            let got = [f32_at(&e, 0x14), f32_at(&e, 0x18), f32_at(&e, 0x1c)];
            println!("GetStaticObjectPosition(FrontLeft) {got:?}");
            assert_eq!(got, p, "x/y/z at R+4/R+8/R+0xc");
        }
        Err(pe) => assert_eq!(hr, pe),
    }
    let mut fmt = None;
    if let Ok(en) = &direct {
        let mut e = envelope(crossvm::SPATIAL_STATIC_POSITION, sc);
        assert_eq!(send(&mut e), 0x8000_4001u32 as i32, "map 1ac4d0 has only 0x3a / 0x3b: 125e00 answers E_NOTIMPL for anything else");
        let mut e = envelope(crossvm::FORMAT_ENUM_COUNT, dev);
        assert_eq!(send(&mut e), 0x8000_4001u32 as i32, "0x3a is served only by the type 4 object");
        let mut e = envelope(crossvm::FORMAT_ENUM_COUNT, sc);
        let hr = send(&mut e);
        let n = en.count().unwrap();
        assert_eq!(hr, 0);
        assert_eq!(u32_at(&e, 0x10), n, "125a10 -> IAudioFormatEnumerator::GetCount into R+0");
        println!("object format count {n} (console 2a3f0 answers 1)");
        let mut e = envelope(crossvm::FORMAT_ENUM_GET, sc);
        let hr = send(&mut e);
        let f = en.get(0).unwrap();
        assert_eq!(hr, 0);
        let plain = crossvm::plain_format(&f);
        assert_eq!(&e[0x14..0x26], &plain[..], "WAVEFORMATEX (0x12 bytes) at R+4");
        assert_eq!(u16::from_le_bytes([e[0x24], e[0x25]]), 0, "cbSize 0: the guest copies only 0x12 bytes");
        println!("object format tag {:#x} {} Hz {} ch (console 299c0: float 48000 Hz mono)", u16::from_le_bytes([plain[0], plain[1]]), u32::from_le_bytes(plain[4..8].try_into().unwrap()), plain[2]);
        let mut e = envelope(crossvm::FORMAT_ENUM_GET, sc);
        e[0x10..0x14].copy_from_slice(&n.to_le_bytes());
        assert_eq!(send(&mut e), 0x8000_000bu32 as i32, "299c0: an index past the count is E_BOUNDS");
        let mut e = envelope(crossvm::FORMAT_ENUM_GET, sc);
        e[0x24..0x26].copy_from_slice(&0x37u16.to_le_bytes());
        assert_eq!(send(&mut e), -2147024809, "1238bc: cbSize + 0x12 above 0x48 is E_INVALIDARG");
        fmt = Some(f);
    }
    if let Some(f) = fmt {
        let mut e = envelope(crossvm::SPATIAL_MAX_FRAMES_FOR_CATEGORY, dev);
        e[0x10..0x10 + f.len()].copy_from_slice(&f);
        e[0x10 + crossvm::SPATIAL_CATEGORY_AT..0x14 + crossvm::SPATIAL_CATEGORY_AT].copy_from_slice(&6u32.to_le_bytes());
        let hr = send(&mut e);
        match pc.max_frames_for_category(6, false, &f) {
            Ok(n) => {
                assert_eq!(hr, 0);
                assert_eq!(u32_at(&e, 0x10 + crossvm::SPATIAL_FRAMES_AT), n, "frames at R+0x4c");
                println!("GetMaxFrameCountForCategory(GameEffects) {n}");
            }
            Err(pe) => assert_eq!(hr, pe),
        }
    }
    for o in [sc, dev, enumerator] {
        if o == 0 {
            continue;
        }
        let mut e = envelope(crossvm::RELEASE_OBJECT, o);
        assert_eq!(send(&mut e), 0);
    }
}

fn items(mix: &[u8]) {
    use console_sandbox::wasapi;
    let (object, hr, _) = init_stream(mix, 0);
    assert_eq!(hr, 0);
    let (hr, e) = create(crossvm::ITEM_SERVICE_TYPE, 0, object, 0, &[]);
    assert_eq!(hr, 0, "AudioSes 8516c creates the d19f8e95 service object as type 0xb under the stream");
    let svc = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::ITEM_COUNT, svc);
    assert_eq!(send(&mut e), 0);
    let count = u32_at(&e, 0x10);
    let mut e = envelope(crossvm::ITEM_FLAG, svc);
    assert_eq!(send(&mut e), 0);
    let flag = e[0x10];
    println!("PC IAudioClient d19f8e95 service: count {count}, flag {flag}");
    assert!(flag <= 1);
    for i in 0..count {
        let mut e = envelope(crossvm::ITEM_GET, svc);
        e[0x10..0x14].copy_from_slice(&i.to_le_bytes());
        assert_eq!(send(&mut e), 0, "0x3c index {i} exists on the PC");
        let mut e = envelope(crossvm::ITEM_VALUE, svc);
        e[0x10..0x14].copy_from_slice(&i.to_le_bytes());
        e[0x14..0x18].copy_from_slice(&0xdeadu32.to_le_bytes());
        assert_eq!(send(&mut e), 0);
        assert_eq!(u32_at(&e, 0x10), i, "0x3f keeps the index at A+0");
        let v = u32_at(&e, 0x14);
        println!("item {i}: value {v} (0x3f R+4)");
        assert_ne!(v, 0xdead, "0x3f writes the child value over the zero half at R+4");
    }
    let mut e = envelope(crossvm::ITEM_GET, svc);
    e[0x10..0x14].copy_from_slice(&count.to_le_bytes());
    assert_eq!(send(&mut e) as u32, 0x80070057, "an index past the PC count fails the way the PC Get does");
    let mut e = envelope(crossvm::ITEM_COUNT, 0x7fff_0000);
    assert_ne!(send(&mut e), 0, "an object outside a stream has no client to ask");

    let (hr, e) = create(1, 1, 0, 0, &[]);
    assert_eq!(hr, 0);
    let enumerator = u32_at(&e, 0x1c);
    let (hr, e) = create(5, 0, enumerator, crossvm::METHOD_DEFAULT_ENDPOINT, &[0u8; 8]);
    assert_eq!(hr, 0);
    let dev = u32_at(&e, 0x1c);
    let id = crossvm::object_device(dev).unwrap();
    let (hr, e) = create(0xd, 0, dev, 0x26, &[]);
    assert_eq!(hr, 0);
    let ps = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::PROPERTY_GET_COUNT, ps);
    assert_eq!(send(&mut e), 0);
    let n = u32_at(&e, 0x10);
    let (mut vectors, mut served, mut tags) = (0, 0, [0u32; 5]);
    for i in 0..n {
        let mut e = envelope(crossvm::PROPERTY_GET_AT, ps);
        e[0x10..0x14].copy_from_slice(&i.to_le_bytes());
        assert_eq!(send(&mut e), 0);
        let mut key = wasapi::PropertyKey::default();
        key.fmtid.copy_from_slice(&e[0x14..0x24]);
        key.pid = u32_at(&e, 0x24);
        let pc = wasapi::property_value(Some(&id), &key);
        let mut e = envelope(crossvm::PROPERTY_GET_VALUE, ps);
        e[0x10..0x20].copy_from_slice(&key.fmtid);
        e[0x20..0x24].copy_from_slice(&key.pid.to_le_bytes());
        let hr = send(&mut e);
        let tag = u32_at(&e, 0x24);
        let len = u64::from_le_bytes(e[0x10 + 0x11c..0x18 + 0x11c].try_into().unwrap()) as usize;
        let data = &e[0x28..0x28 + len.min(crossvm::PROP_DATA)];
        if hr == 0 {
            served += 1;
            tags[tag.min(4) as usize] += 1;
        }
        match pc {
            Ok(Some(wasapi::Prop::Vector(vt, cnt, bytes))) => {
                vectors += 1;
                println!("property {i} pid {} vt {vt:#x}: {cnt} elements, {} bytes", key.pid, bytes.len());
                if bytes.len() + 8 <= crossvm::PROP_DATA {
                    assert_eq!(hr, 0);
                    assert_eq!(tag, 3);
                    assert_eq!(u16::from_le_bytes([data[0], data[1]]), vt);
                    assert_eq!(u32_at(data, 4), cnt, "cElems at data+4 (AudioSes 2bf0a)");
                    assert_eq!(&data[8..], &bytes[..], "elements are a raw copy from data+8");
                }
            }
            Ok(Some(wasapi::Prop::Serialized(vt, bytes))) if bytes.len() + 8 <= crossvm::PROP_DATA => {
                assert_eq!((hr, tag), (0, 3));
                assert_eq!(u16::from_le_bytes([data[0], data[1]]), vt);
                assert_eq!(&data[8..], &bytes[..]);
            }
            _ => {}
        }
    }
    println!("{n} PC properties: {served} served (tags empty/text/-/serialised/dword {tags:?}), {vectors} vectors");
    assert!(served > 0);
    let v = crossvm::serialized_prop(0x101f, Some(2), &[b'a', 0, 0, 0, b'b', 0, 0, 0]);
    assert_eq!(&v[..8], &[0x1f, 0x10, 0, 0, 2, 0, 0, 0], "REG_MULTI_SZ serialises as VT_VECTOR|VT_LPWSTR with cElems");
}

fn capture() {
    use console_sandbox::wasapi;
    let Ok(pc_id) = wasapi::default_id(1, 0) else {
        println!("NO PC CAPTURE ENDPOINT");
        return;
    };
    let (hr, e) = create(1, 1, 0, 0, &[]);
    assert_eq!(hr, 0);
    let enumerator = u32_at(&e, 0x1c);
    let mut args = [0u8; 8];
    args[0..4].copy_from_slice(&1u32.to_le_bytes());
    let (hr, e) = create(5, 0, enumerator, crossvm::METHOD_DEFAULT_ENDPOINT, &args);
    assert_eq!(hr, 0);
    let dev = u32_at(&e, 0x1c);
    assert_eq!(crossvm::object_device(dev).unwrap(), pc_id);
    assert_eq!(wasapi::device_flow(Some(&pc_id)), Ok(1));
    let mut e = envelope(crossvm::GET_MIX_FORMAT, dev);
    assert_eq!(send(&mut e), 0);
    let mix_len = 0x12 + u16::from_le_bytes([e[0x20], e[0x21]]) as usize;
    let mix = e[0x10..0x10 + mix_len].to_vec();
    let align = u16::from_le_bytes([mix[0xc], mix[0xd]]) as u64;
    let rate = u32_at(&mix, 4);
    let (hr, e) = create(2, 0, dev, 0x2f, &[]);
    assert_eq!(hr, 0);
    let object = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::INITIALIZE, object);
    let a = 0x10;
    e[a + 8..a + 16].copy_from_slice(&1_000_000i64.to_le_bytes());
    e[a + 0x18..a + 0x18 + mix.len()].copy_from_slice(&mix);
    assert_eq!(send(&mut e), 0, "capture Initialize on the PC capture endpoint");
    let clsid = &e[a + crossvm::INIT_ENDPOINT_CLSID..a + crossvm::INIT_ENDPOINT_CLSID + 16];
    assert_eq!(clsid, &[0xa6, 0x6d, 0x4a, 0xf6, 0xaf, 0xe8, 0x7b, 0x4b, 0xbc, 0xa8, 0x84, 0x7a, 0xe7, 0x65, 0xd5, 0x38], "reply +0x70 is f64a6da6, which AudioSes 15fb0 maps to CCrossProcessClientInputEndpoint");
    let (base, _, _, _, _) = crossvm::stream_info(object).unwrap();
    let cb = base as *mut u8;
    let r32 = |o: usize| unsafe { (cb.add(o) as *const u32).read_volatile() };
    let c = crossvm::CB_CONFIG;
    let (queue, entries) = (r32(c + crossvm::CFG_QUEUE) as usize, r32(c + crossvm::CFG_ENTRIES));
    let (start, end) = (r32(c + crossvm::CFG_DATA_START), r32(c + crossvm::CFG_DATA_END));
    assert!(entries > 1 && queue > 0);
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    let (mut packets, mut frames, mut silent, mut last_end, mut back, mut disc) = (0u64, 0u64, 0u64, None::<u64>, 0u64, 0u64);
    while t.elapsed().as_millis() < 1500 {
        loop {
            let r = r32(wasapi::CAPTURE_QUEUE_READ);
            if r == r32(wasapi::CAPTURE_QUEUE_WRITE) {
                break;
            }
            let ent = queue + r as usize * wasapi::CAPTURE_ENTRY;
            let off = r32(ent + 4);
            let kind = r32(ent + 8);
            let n = r32(ent + 0x38) as u64;
            let flags = r32(ent + 0x34);
            let pos = unsafe { (cb.add(ent + 0x18) as *const u64).read_volatile() };
            let qpc = unsafe { (cb.add(ent + 0x28) as *const u64).read_volatile() } as i64;
            let erate = unsafe { (cb.add(ent + 0x30) as *const f32).read_volatile() };
            assert!(n > 0 && (kind == 1 || kind == 2) && flags & !5 == 0);
            assert_eq!(erate, rate as f32);
            assert!(qpc <= qpc_100ns(), "the packet QPC is a past PC capture time");
            if kind == 1 {
                assert!(off >= start && off as u64 + n * align <= end as u64, "a packet never straddles the data end");
            } else {
                silent += 1;
            }
            if flags & 1 != 0 {
                disc += 1;
            } else if let Some(l) = last_end {
                back = back.max(l.saturating_sub(pos));
            }
            last_end = Some(pos + n);
            packets += 1;
            frames += n;
            unsafe { (*(cb.add(crossvm::CB_READ) as *const std::sync::atomic::AtomicI64)).fetch_add((n * align) as i64, std::sync::atomic::Ordering::SeqCst) };
            let next = if r + 1 >= entries { 0 } else { r + 1 };
            unsafe { (*(cb as *const std::sync::atomic::AtomicU32)).swap(next, std::sync::atomic::Ordering::SeqCst) };
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let wall = (t.elapsed().as_secs_f64() * rate as f64) as u64;
    println!("capture: {packets} packets, {frames} frames in {wall} wall frames, {silent} silent, {disc} discontinuities, device position step back {back}");
    assert!(packets > 10, "the PC capture client delivers packets while started");
    assert!(frames > wall / 4 && frames <= wall + rate as u64 / 5, "captured frames follow the PC capture clock");
    assert_eq!(back, 0, "device positions advance by the packet frames");
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut e = envelope(crossvm::RESET, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    while crossvm::stream_records(object).unwrap().2 == 0 && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(r32(wasapi::CAPTURE_QUEUE_READ), r32(wasapi::CAPTURE_QUEUE_WRITE), "Reset drops unread packets");
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);
    capture_stall(dev, &mix);
    capture_period(dev, &mix);
}

fn capture_queue(object: u32) -> (*mut u8, usize, u32, u32, u32) {
    let (base, _, _, _, _) = crossvm::stream_info(object).unwrap();
    let cb = base as *mut u8;
    let r32 = |o: usize| unsafe { (cb.add(o) as *const u32).read_volatile() };
    let c = crossvm::CB_CONFIG;
    (cb, r32(c + crossvm::CFG_QUEUE) as usize, r32(c + crossvm::CFG_ENTRIES), r32(c + crossvm::CFG_DATA_START), r32(c + crossvm::CFG_DATA_END))
}

fn capture_drain(cb: *mut u8, queue: usize, entries: u32, align: u64) -> Vec<(u32, u32, u32, u64)> {
    let r32 = |o: usize| unsafe { (cb.add(o) as *const u32).read_volatile() };
    let mut out = Vec::new();
    loop {
        let r = r32(console_sandbox::wasapi::CAPTURE_QUEUE_READ);
        if r == r32(console_sandbox::wasapi::CAPTURE_QUEUE_WRITE) {
            return out;
        }
        let ent = queue + r as usize * console_sandbox::wasapi::CAPTURE_ENTRY;
        let n = r32(ent + 0x38) as u64;
        out.push((r32(ent + 4), r32(ent + 8), r32(ent + 0x34), n));
        unsafe { (*(cb.add(crossvm::CB_READ) as *const std::sync::atomic::AtomicI64)).fetch_add((n * align) as i64, std::sync::atomic::Ordering::SeqCst) };
        let next = if r + 1 >= entries { 0 } else { r + 1 };
        unsafe { (*(cb as *const std::sync::atomic::AtomicU32)).swap(next, std::sync::atomic::Ordering::SeqCst) };
    }
}

fn capture_stall(dev: u32, mix: &[u8]) {
    let align = u16::from_le_bytes([mix[0xc], mix[0xd]]) as u64;
    let (hr, e) = create(2, 0, dev, 0x2f, &[]);
    assert_eq!(hr, 0);
    let object = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::INITIALIZE, object);
    e[0x18..0x20].copy_from_slice(&1_000_000i64.to_le_bytes());
    e[0x28..0x28 + mix.len()].copy_from_slice(mix);
    assert_eq!(send(&mut e), 0);
    let (cb, queue, entries, start, end) = capture_queue(object);
    let r32 = |o: usize| unsafe { (cb.add(o) as *const u32).read_volatile() };
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let (r, w) = (r32(console_sandbox::wasapi::CAPTURE_QUEUE_READ), r32(console_sandbox::wasapi::CAPTURE_QUEUE_WRITE));
    let queued = (w + entries - r) % entries;
    let mut ranges = Vec::new();
    let mut i = r;
    while i != w {
        let ent = queue + i as usize * console_sandbox::wasapi::CAPTURE_ENTRY;
        if r32(ent + 8) == 1 {
            let off = r32(ent + 4) as u64;
            ranges.push((off, off + r32(ent + 0x38) as u64 * align));
        }
        i = if i + 1 >= entries { 0 } else { i + 1 };
    }
    ranges.sort();
    println!("stalled 400 ms: {queued} packets queued of {} slots, {} data ranges in [{start:#x}, {end:#x})", entries - 1, ranges.len());
    assert!(queued <= entries - 1, "the queue never fills past capacity - 1");
    assert!(queued > 0);
    for (lo, hi) in &ranges {
        assert!(*lo >= start as u64 && *hi <= end as u64, "every queued packet lies inside the data region");
    }
    for p in ranges.windows(2) {
        assert!(p[0].1 <= p[1].0, "no two unread packets overlap");
    }
    let first = capture_drain(cb, queue, entries, align);
    assert_eq!(first.len() as u32, queued);
    let t = std::time::Instant::now();
    let mut after = Vec::new();
    while after.is_empty() && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        after = capture_drain(cb, queue, entries, align);
    }
    println!("first packet after the stall: flags {:#x}", after[0].2);
    assert_eq!(after[0].2 & 1, 1, "the first packet after host drops carries DATA_DISCONTINUITY");
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);
}

fn capture_period(dev: u32, mix: &[u8]) {
    let align = u16::from_le_bytes([mix[0xc], mix[0xd]]) as u64;
    let mut e = envelope(crossvm::GET_SHARED_MODE_ENGINE_PERIOD, dev);
    e[0x10..0x10 + mix.len()].copy_from_slice(mix);
    assert_eq!(send(&mut e), 0);
    let (def, min) = (u32_at(&e, 0x58), u32_at(&e, 0x60));
    let (hr, e) = create(2, 0, dev, 0x2f, &[]);
    assert_eq!(hr, 0);
    let object = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::INITIALIZE_SHARED_AUDIO_STREAM, object);
    e[0x10..0x10 + mix.len()].copy_from_slice(mix);
    e[0x10 + crossvm::SHARED_INIT_PERIOD..0x14 + crossvm::SHARED_INIT_PERIOD].copy_from_slice(&min.to_le_bytes());
    assert_eq!(send(&mut e), 0, "0x0f on the capture endpoint goes through PC IAudioClient3::InitializeSharedAudioStream");
    let (cb, queue, entries, _, _) = capture_queue(object);
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    let mut got = Vec::new();
    while t.elapsed().as_millis() < 500 {
        got.extend(capture_drain(cb, queue, entries, align));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    let mut sizes: Vec<u64> = got.iter().map(|p| p.3).collect();
    sizes.sort();
    let median = sizes.get(sizes.len() / 2).copied().unwrap_or(0);
    println!("capture engine periods default {def} min {min}; 0x0f({min}) packets {} median {median} frames", got.len());
    assert!(!got.is_empty());
    assert_eq!(median, min as u64, "PC capture packets follow the period requested through 0x0f");
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);
}

fn wait_frame(listener: u32, after: usize) -> Option<(i32, Vec<u8>)> {
    let t = std::time::Instant::now();
    while t.elapsed().as_secs() < 3 {
        let l = crossvm::sent_notifications();
        if let Some(f) = l.iter().skip(after).find(|(_, f)| u32_at(f, 4) == listener) {
            return Some(f.clone());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

fn notify() {
    let rec = crossvm::device_record(4, "{0.0.0.00000000}.{x}", 0, 1, None).unwrap();
    assert_eq!(u32_at(&rec, 0) as usize, rec.len());
    assert_eq!(rec.len(), 0x34 + 2 * 21);
    assert_eq!((u32_at(&rec, 0x18), u32_at(&rec, 0x1c), u32_at(&rec, 0x20)), (4, 0, 1), "kind 4 OnDefaultDeviceChanged: flow +0x1c, role +0x20 (MMDev 4bfd4)");
    assert_eq!(wide_at(&rec, 0x34, 32), "{0.0.0.00000000}.{x}");
    let k = console_sandbox::wasapi::PropertyKey { fmtid: [7; 16], pid: 9 };
    let rec = crossvm::device_record(5, "a", 0, 0, Some(&k)).unwrap();
    assert_eq!((&rec[0x1c..0x2c], u32_at(&rec, 0x2c)), (&[7u8; 16][..], 9));
    assert!(crossvm::device_record(2, &"x".repeat(crossvm::DEVICE_RECORD_MAX_CHARS), 0, 0, None).is_none(), "an id longer than the 0x810 frame allows is not sent");

    let (hr, e) = create(1, 1, 0, 0, &[]);
    assert_eq!(hr, 0);
    let enumerator = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::ENUMERATOR_SUBSCRIBE, enumerator);
    assert_eq!(send(&mut e), 0, "0x23 registers a PC IMMNotificationClient");

    let (hr, e) = create(0x12, 1, 0, 0x4e, &[]);
    assert_eq!(hr, 0);
    let vol = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::ENDPOINT_REGISTER, vol);
    assert_eq!(send(&mut e), 0, "0x4f registers the listener on this endpoint");
    let ctx = [0x5au8; 16];
    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::GET_ENDPOINT_VOLUME_SCALAR, vol);
    assert_eq!(send(&mut e), 0);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(crossvm::sent_notifications().iter().skip(before).all(|(_, f)| u32_at(f, 4) != vol), "a read does not notify");

    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_ENDPOINT_VOLUME_SCALAR, vol);
    e[0x10..0x14].copy_from_slice(&0.37f32.to_le_bytes());
    e[0x14..0x24].copy_from_slice(&ctx);
    assert_eq!(send(&mut e), 0);
    let (hr, f) = wait_frame(vol, before).expect("SetMasterVolumeLevelScalar notifies every 0x4f listener of the endpoint");
    let r = &f[crossvm::NOTIFY_RECORD..];
    let n = u32_at(r, 0x30);
    println!("endpoint scalar 0.37: frame to listener {vol:#x}, +8 {}, ctx {:02x?}, master {}, muted {}, {n} channels, push {hr:#010x}", u32_at(&f, 8), &r[0x18..0x20], f32_at(r, 0x2c), u32_at(r, 0x28));
    assert_eq!(f.len(), 0x810);
    assert_eq!(u32_at(&f, 8), 0, "+8 0 = notification, 1 would disconnect every listener");
    assert_eq!(&r[0x18..0x28], &ctx[..], "guidEventContext is the GUID the guest passed at args+4 (AudioSrv 71570)");
    assert!((f32_at(r, 0x2c) - 0.37).abs() < 1e-6, "fMasterVolume at record +0x2c");
    assert_eq!(u32_at(r, 0x28), 0);
    assert!(n > 0 && u32_at(r, 0x34) == 0, "channel array at +0x38 with offset 0");

    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_ENDPOINT_CHANNEL_SCALAR, vol);
    e[0x10..0x14].copy_from_slice(&0u32.to_le_bytes());
    e[0x14..0x18].copy_from_slice(&0.5f32.to_le_bytes());
    e[0x18..0x28].copy_from_slice(&[0x33; 16]);
    assert_eq!(send(&mut e), 0);
    let (_, f) = wait_frame(vol, before).expect("SetChannelVolumeLevelScalar notifies");
    let r = &f[crossvm::NOTIFY_RECORD..];
    assert_eq!(&r[0x18..0x28], &[0x33u8; 16][..], "channel set context GUID at args+8 (AudioSrv 71430)");
    assert!((f32_at(r, 0x38) - 0.5).abs() < 1e-6, "afChannelVolumes[0]");

    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_ENDPOINT_MUTE, vol);
    e[0x10] = 1;
    e[0x11..0x21].copy_from_slice(&[0x44; 16]);
    assert_eq!(send(&mut e), 0);
    let (_, f) = wait_frame(vol, before).expect("SetMute notifies");
    let r = &f[crossvm::NOTIFY_RECORD..];
    assert_eq!((u32_at(r, 0x28), &r[0x18..0x28]), (1, &[0x44u8; 16][..]), "bMuted and the context GUID at args+1 (AudioSrv 71610)");
    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_ENDPOINT_MUTE, vol);
    e[0x10] = 1;
    assert_eq!(send(&mut e), 1, "S_FALSE: already muted");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(crossvm::sent_notifications().iter().skip(before).all(|(_, f)| u32_at(f, 4) != vol), "an unchanged mute does not notify");

    let mut e = envelope(crossvm::ENDPOINT_UNREGISTER, vol);
    assert_eq!(send(&mut e), 0);
    let quiet = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_ENDPOINT_VOLUME_SCALAR, vol);
    e[0x10..0x14].copy_from_slice(&0.6f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(crossvm::sent_notifications().iter().skip(quiet).all(|(_, f)| u32_at(f, 4) != vol), "after 0x50 the listener gets no frames");

    let (hr, e) = create(0x14, 1, 0, 0x67, &[]);
    assert_eq!(hr, 0);
    let sess = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::SESSION_REGISTER_NOTIFICATION, sess);
    assert_eq!(send(&mut e), 0, "0x1e registers a PC IAudioSessionEvents on the title session");
    let mut e = envelope(crossvm::SESSION_GET_INSTANCE_IDENTIFIER, sess);
    assert_eq!(send(&mut e), 0);
    let sid = wide_at(&e, 0x10, crossvm::SESSION_STRING_CHARS);

    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_SIMPLE_VOLUME, sess);
    e[0x10..0x14].copy_from_slice(&0.4f32.to_le_bytes());
    e[0x14..0x24].copy_from_slice(&[0x61; 16]);
    assert_eq!(send(&mut e), 0);
    let (_, f) = wait_frame(sess, before).expect("ISimpleAudioVolume::SetMasterVolume notifies the 0x1e listener");
    let r = session_valid(&f, &sid);
    assert_eq!(u32_at(r, 4), 1, "kind 1 OnSimpleVolumeChanged");
    assert_eq!(r.len(), 0x48 + u32_at(r, 0x18) as usize, "kind 1 size is exactly 0x48 + session id bytes (AudioSes 7cb9)");
    assert_eq!(&r[0x1c..0x2c], &[0x61u8; 16][..], "context GUID from 0x16 args+4");
    assert_eq!((f32_at(r, 0x30), r[0x34]), (0.4, 0));
    println!("session simple volume: frame to {sess:#x}, record {} bytes, sid {sid}", r.len());

    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SESSION_SET_CHANNEL_VOLUME, sess);
    e[0x14..0x18].copy_from_slice(&0.5f32.to_le_bytes());
    e[0x18..0x28].copy_from_slice(&[0x62; 16]);
    assert_eq!(send(&mut e), 0);
    let (_, f) = wait_frame(sess, before).expect("IChannelAudioVolume::SetChannelVolume notifies");
    let r = session_valid(&f, &sid);
    let off = u32_at(r, 0x30) as usize;
    let n = u32_at(r, 0x38) as usize;
    assert_eq!(u32_at(r, 4), 2);
    assert_eq!((off & 3, off >= 0x48 + u32_at(r, 0x18) as usize, r.len()), (0, true, off + 4 * n), "channel array offset aligned, size = offset + 4n");
    assert_eq!((u32_at(r, 0x34), f32_at(r, off)), (0, 0.5));
    assert_eq!(&r[0x1c..0x2c], &[0x62u8; 16][..]);
    let mut e = envelope(crossvm::SESSION_SET_CHANNEL_VOLUME, sess);
    e[0x14..0x18].copy_from_slice(&1.0f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);

    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SESSION_SET_DISPLAY_NAME, sess);
    let label: Vec<u8> = "TomBox notify".encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect();
    e[0x10..0x10 + label.len()].copy_from_slice(&label);
    e[0x10 + crossvm::DISPLAY_NAME_CHARS * 2..0x20 + crossvm::DISPLAY_NAME_CHARS * 2].copy_from_slice(&[0x63; 16]);
    assert_eq!(send(&mut e), 0);
    let (_, f) = wait_frame(sess, before).expect("the PC OnDisplayNameChanged becomes a kind 0x10 frame");
    let r = session_valid(&f, &sid);
    let (len, at) = (u32_at(r, 0x30) as usize, u32_at(r, 0x34) as usize);
    assert_eq!(u32_at(r, 4), 0x10);
    assert_eq!(at, 0x48 + u32_at(r, 0x18) as usize, "name offset = 0x48 + session id bytes");
    assert_eq!((len, r.len()), (label.len(), at + len));
    assert_eq!(&r[at..at + len], &label[..]);
    assert_eq!(&r[0x1c..0x2c], &[0x63u8; 16][..], "the PC event carries the guest's context GUID through");
    println!("session display name: PC event -> frame to {sess:#x}, name at +{at:#x}, {len} bytes");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(crossvm::sent_notifications().iter().skip(before).filter(|(_, f)| u32_at(f, 4) == sess && u32_at(f, 0x10) == 0x10).count(), 1, "the PC fires OnDisplayNameChanged more than once per change; the guest gets it once");

    let sess_live = sess;
    crossvm::set_hello(2);
    let (hr, e) = create(0x18, 1, 0, crossvm::METHOD_SESSION_ENUMERATOR, &[]);
    assert_eq!(hr, 0, "0x77 enumerates the PC IAudioSessionManager2 sessions");
    let en = u32_at(&e, 0x1c);
    let count = u32_at(&e, 0x10 + crossvm::METHOD_REPLY_COUNT);
    assert_eq!(u32_at(&e, 0x20), crossvm::METHOD_SESSION_ENUMERATOR, "the reply validator a6b78 needs the method echoed at R+0x10");
    assert!(count >= 1);
    let mut own = None;
    let mut ids = Vec::new();
    for i in 0..count {
        let (hr, e) = create(0xa, 1, en, crossvm::METHOD_ENUMERATED_SESSION, &i.to_le_bytes());
        assert_eq!(hr, 0);
        let o = u32_at(&e, 0x1c);
        let mut e = envelope(crossvm::SESSION_GET_INSTANCE_IDENTIFIER, o);
        assert_eq!(send(&mut e), 0);
        let id = wide_at(&e, 0x10, crossvm::SESSION_STRING_CHARS);
        let mut e = envelope(crossvm::SESSION_GET_STATE, o);
        assert_eq!(send(&mut e), 0);
        assert!(u32_at(&e, 0x10) <= 2);
        if id == sid {
            own = Some(o);
        }
        ids.push(id);
    }
    let (hr, _) = create(0xa, 1, en, crossvm::METHOD_ENUMERATED_SESSION, &count.to_le_bytes());
    assert_eq!(hr, -2147024809, "index >= count is E_INVALIDARG (AudioSes a7070)");
    let mut uniq = ids.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), ids.len());
    println!("0x77: {count} PC sessions; 0x78 index {count} -> E_INVALIDARG");
    let own = own.expect("the title's own PC session is among the enumerated sessions");
    let mut e = envelope(crossvm::SESSION_REGISTER_NOTIFICATION, own);
    assert_eq!(send(&mut e), 0);
    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SET_SIMPLE_VOLUME, own);
    e[0x10..0x14].copy_from_slice(&0.3f32.to_le_bytes());
    e[0x14..0x24].copy_from_slice(&[0x64; 16]);
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::GET_SIMPLE_VOLUME, sess_live);
    assert_eq!(send(&mut e), 0);
    assert_eq!(f32_at(&e, 0x10), 0.3, "the enumerated title session and the title session object share one volume");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let got: Vec<u32> = crossvm::sent_notifications().iter().skip(before).map(|(_, f)| u32_at(f, 4)).collect();
    assert_eq!((got.iter().filter(|&&l| l == own).count(), got.iter().filter(|&&l| l == sess_live).count()), (1, 1), "one kind 1 frame per listener");
    let before = crossvm::sent_notifications().len();
    let mut e = envelope(crossvm::SESSION_SET_DISPLAY_NAME, own);
    let label: Vec<u8> = "TomBox enum".encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect();
    e[0x10..0x10 + label.len()].copy_from_slice(&label);
    assert_eq!(send(&mut e), 0);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let all: Vec<(u32, u32, String)> = crossvm::sent_notifications().iter().skip(before).map(|(_, f)| (u32_at(f, 4), u32_at(f, 0x10), wide_at(f, 0xc + u32_at(f, 0x40) as usize, 32))).collect();
    println!("frames after the enumerated SetDisplayName: {all:?}");
    let got: Vec<u32> = all.iter().filter(|(_, k, _)| *k == 0x10).map(|(l, _, _)| *l).collect();
    assert_eq!((got.iter().filter(|&&l| l == own).count(), got.iter().filter(|&&l| l == sess_live).count()), (1, 1), "one PC IAudioSessionEvents per session instance id: each listener gets the display name change once");
    let key = |s: &str| -> Vec<u8> {
        let mut b = vec![0u8; crossvm::EXISTING_SESSION_CHARS * 2];
        for (i, c) in s.encode_utf16().enumerate() {
            b[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        b
    };
    let (hr, e) = create(crossvm::EXISTING_SESSION_TYPE, 1, 0, crossvm::METHOD_EXISTING_SESSION, &key(&sid));
    assert_eq!(hr, 0, "0x7b looks the title's session up by its instance identifier (AudioSrv GetExistingSession)");
    let ex = u32_at(&e, 0x1c);
    let mut e = envelope(crossvm::SESSION_GET_INSTANCE_IDENTIFIER, ex);
    assert_eq!(send(&mut e), 0);
    assert!(wide_at(&e, 0x10, crossvm::SESSION_STRING_CHARS).eq_ignore_ascii_case(&sid), "the 0x7b session control is the keyed session");
    let mut e = envelope(crossvm::SET_SIMPLE_VOLUME, ex);
    e[0x10..0x14].copy_from_slice(&0.6f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let mut e = envelope(crossvm::GET_SIMPLE_VOLUME, sess_live);
    assert_eq!(send(&mut e), 0);
    assert_eq!(f32_at(&e, 0x10), 0.6, "the 0x7b control and the title session object share one volume");
    let mut e = envelope(crossvm::RELEASE_OBJECT, ex);
    assert_eq!(send(&mut e), 0);
    let (hr, _) = create(crossvm::EXISTING_SESSION_TYPE, 1, 0, crossvm::METHOD_EXISTING_SESSION, &key("{0.0.0.00000000}.{00000000-0000-0000-0000-000000000000}|tombox-none%b{00000000-0000-0000-0000-000000000000}|1%b0"));
    assert_eq!(hr, 0x80070002u32 as i32, "an unknown instance id is ERROR_FILE_NOT_FOUND (audiosrv 1fd50)");
    let (hr, _) = create(crossvm::EXISTING_SESSION_TYPE, 1, 0, crossvm::METHOD_EXISTING_SESSION, &key(""));
    assert_eq!(hr, 0x80004005u32 as i32, "an empty key fails CAudioSessionInstanceId::Initialize (E_FAIL)");
    let mut bad = key(&sid);
    let n = bad.len();
    bad[n - 2] = b'x';
    let (hr, _) = create(crossvm::EXISTING_SESSION_TYPE, 1, 0, crossvm::METHOD_EXISTING_SESSION, &bad);
    assert_eq!(hr, -2147024809, "validator a6bc0 needs a NUL WCHAR at args+0x3fe");
    let (hr, _) = create(crossvm::EXISTING_SESSION_TYPE, 1, 0, crossvm::METHOD_SESSION_ENUMERATOR, &key(&sid));
    assert_eq!(hr, -2147024809, "type 0x19 only takes method 0x7b");
    crossvm::set_hello(1);
    let (hr, _) = create(crossvm::EXISTING_SESSION_TYPE, 1, 0, crossvm::METHOD_EXISTING_SESSION, &key(&sid));
    assert_eq!(hr, -2147024809, "19041 hosts have no type 0x19");
    crossvm::set_hello(2);
    println!("0x7b: existing session {ex:#x} by instance id, unknown -> 0x80070002, empty -> 0x80004005, no NUL -> E_INVALIDARG");
    let mut e = envelope(crossvm::SET_SIMPLE_VOLUME, sess_live);
    e[0x10..0x14].copy_from_slice(&1.0f32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    println!("enumerated own session {own:#x}: one volume with title object {sess_live:#x}, one frame per listener for host and PC events");
    crossvm::set_hello(1);
    let mut e = envelope(crossvm::RELEASE_OBJECT, sess);
    assert_eq!(send(&mut e), 0);
}

fn spatial_args(mask: u32, min: u32, max: u32, category: u32, tag: u16, channels: u16, bits: u16, extra: u16) -> Vec<u8> {
    let mut a = vec![0u8; 0x10 + 0x12 + extra as usize];
    a[0..4].copy_from_slice(&mask.to_le_bytes());
    a[4..8].copy_from_slice(&min.to_le_bytes());
    a[8..12].copy_from_slice(&max.to_le_bytes());
    a[12..16].copy_from_slice(&category.to_le_bytes());
    let f = &mut a[0x10..];
    let align = channels * bits / 8;
    f[0..2].copy_from_slice(&tag.to_le_bytes());
    f[2..4].copy_from_slice(&channels.to_le_bytes());
    f[4..8].copy_from_slice(&48000u32.to_le_bytes());
    f[8..12].copy_from_slice(&(48000 * align as u32).to_le_bytes());
    f[12..14].copy_from_slice(&align.to_le_bytes());
    f[14..16].copy_from_slice(&bits.to_le_bytes());
    f[16..18].copy_from_slice(&extra.to_le_bytes());
    a
}

fn spatial_stream() {
    use std::sync::atomic::Ordering;
    crossvm::set_hello(2);
    let good = spatial_args(0x6, 0, 0, 6, 3, 1, 32, 0);
    let before = crossvm::SPATIAL_ACTIVATIONS.load(Ordering::SeqCst);
    let (hr, e) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM, &good);
    assert_eq!(hr, 0, "type 0xf / 0x36 activates the PC ISpatialAudioObjectRenderStream and creates the guest object");
    assert_eq!(crossvm::SPATIAL_ACTIVATIONS.load(Ordering::SeqCst), before + 1, "the PC accepted the decoded static-mask activation params");
    let o = u32_at(&e, 0x1c);
    assert_eq!(&e[0x10 + crossvm::SPATIAL_REPLY.clsid..0x10 + crossvm::SPATIAL_REPLY.clsid + 16], &[0xff, 0xe7, 0x71, 0x93, 0x2d, 0xdf, 0x62, 0x49, 0x95, 0x85, 0x40, 0x42, 0x4d, 0x05, 0x45, 0x50], "123d94: the 0x36 reply carries the endpoint CLSID at A+0x70");
    assert_eq!(send(&mut envelope(crossvm::RELEASE_OBJECT, o)), 0);
    let (hr, _) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM, &spatial_args(0x6, 0, 0, 6, 3, 1, 32, 0x37));
    assert_eq!(hr, -2147024809, "124e00: cbSize + 0x12 above 0x48 is E_INVALIDARG");
    let (hr, _) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_HRTF_STREAM, &good);
    assert_eq!(hr, -2147024809, "type 0xf takes 0x36 or 0x82 only");
    let mut packed = good.clone();
    packed.resize(crossvm::SPATIAL_ARG_OPTIONS + 4 - 0x14, 0);
    let before = crossvm::SPATIAL_ACTIVATIONS.load(Ordering::SeqCst);
    let (hr, e) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM2, &packed);
    assert_eq!(hr, 0, "0x82 (what XD 13fc58 sends; validator 123708) activates the PC stream with options from A+0x70");
    assert_eq!(crossvm::SPATIAL_ACTIVATIONS.load(Ordering::SeqCst), before + 1);
    assert_eq!(send(&mut envelope(crossvm::RELEASE_OBJECT, u32_at(&e, 0x1c))), 0);
    let (hr, e) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM, &spatial_args(0, 0, 1, 6, 3, 1, 32, 0));
    if hr == 0 {
        assert_eq!(send(&mut envelope(crossvm::RELEASE_OBJECT, u32_at(&e, 0x1c))), 0);
    }
    if console_sandbox::wasapi::spatial(None).and_then(|s| s.max_dynamic()) == Ok(0) {
        assert!(hr < 0 && hr != 0x8000_4001u32 as i32, "with no dynamic objects on this PC, max dynamic 1 gets the PC activation error ({hr:#x})");
    }
    let (hr, _) = create(crossvm::SPATIAL_HRTF_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM, &good);
    assert_eq!(hr, -2147024809, "125160: type 0x10 takes 0x37 or 0x83");
    let (hr, _) = create(crossvm::SPATIAL_METADATA_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM, &good);
    assert_eq!(hr, -2147024809, "1255d0: type 0x16 takes 0x38 or 0x84");
    let pcm = spatial_args(0x6, 0, 0, 6, 1, 2, 16, 0);
    let before = crossvm::SPATIAL_ACTIVATIONS.load(Ordering::SeqCst);
    let (hr, _) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM, &pcm);
    assert!(hr < 0 && hr != 0x8000_4001u32 as i32, "a format the PC spatial client rejects returns the PC error ({hr:#x})");
    assert_eq!(crossvm::SPATIAL_ACTIVATIONS.load(Ordering::SeqCst), before);
    println!("spatial stream: static mask 0x6 activated on the PC, 16-bit stereo PCM -> {:#010x}", hr as u32);
    crossvm::set_hello(1);
}

fn spatial_pump() {
    use console_sandbox::crossvm::{dcas, pingpong};
    use std::sync::atomic::Ordering;
    crossvm::set_hello(2);
    let r = &crossvm::SPATIAL_REPLY2;
    let mut packed = spatial_args(0x6, 0, 0, 6, 3, 1, 32, 0);
    packed.resize(crossvm::SPATIAL_ARG_OPTIONS + 4 - 0x14, 0);
    let (hr, e) = create(crossvm::SPATIAL_STREAM_TYPE, 1, 0, crossvm::METHOD_SPATIAL_STREAM2, &packed);
    assert_eq!(hr, 0, "0xf / 0x82 creates the guest spatial stream");
    let o = u32_at(&e, 0x1c);
    let a = &e[0x10..];
    assert_eq!(&a[r.clsid..r.clsid + 16], &[0xff, 0xe7, 0x71, 0x93, 0x2d, 0xdf, 0x62, 0x49, 0x95, 0x85, 0x40, 0x42, 0x4d, 0x05, 0x45, 0x50], "123c08: CLSID 9371e7ff at p+0x60, which 1455b0 maps to the client output endpoint f8c94");
    assert_eq!(u32_at(a, r.extra), 0, "p+0x58 = host stream+0x6d8; the PC oracle gives 0 for a static stream");
    assert_eq!((u32_at(a, r.event), u32_at(a, r.event + 0x24)), (2, crossvm::EVENT_KIND_GAMECORE), "p+0x84: kind 2 period-event descriptor");
    assert_eq!(u32_at(a, r.memory), 3, "p+0x1ac: kind 3 memory descriptor naming the DCAS");
    let (base, page, shape, regions) = crossvm::spatial_control(o).expect("spatial host state");
    assert_eq!(u32_at(a, r.memory + 0x24), page as u32, "the memory descriptor names the DCAS pool page");
    let total = u32::from_le_bytes(unsafe { *((base + dcas::TOTAL) as *const [u8; 4]) }) as usize;
    let dcas_bytes = unsafe { core::slice::from_raw_parts(base as *const u8, total) };
    assert_eq!(dcas::validate(dcas_bytes), 0, "fe6c0 accepts the DCAS the host wrote");
    let (ps, pr) = dcas::parse(dcas_bytes).unwrap();
    assert_eq!((ps, pr), (shape, regions), "fd780 recomputes the shape");
    let frames = u32_at(dcas_bytes, dcas::FRAMES);
    let pc_max = console_sandbox::wasapi::spatial(None).unwrap().max_frames(&packed[0x10..0x22]).unwrap();
    let mix = console_sandbox::wasapi::sample_rate(&console_sandbox::wasapi::device_info_for(None).unwrap().mix);
    let pc_frames = crossvm::engine_frames(pc_max, 48000, mix);
    assert_eq!((frames, u32_at(dcas_bytes, dcas::STATIC_OBJECTS), u32_at(dcas_bytes, dcas::DYNAMIC_OBJECTS), shape.properties), (pc_frames, 2, 0, 0x20), "the DCAS shape follows the PC engine: trunc(1.1f * (max frame count + SRC priming)), popcount(mask) static objects, 0x20-byte records");
    assert_eq!(u32_at(dcas_bytes, pingpong::FRAMES_PER_PACKET), pc_max.min(u32_at(dcas_bytes, pingpong::FRAMES_PER_PACKET)), "+0x34 stays the per-pass packet, at most the PC max frame count");
    let ctl = unsafe { pingpong::Control::at(base as *mut u8) };
    assert_eq!((ctl.state(), ctl.flags()), (6, pingpong::FLAG_VALID), "state 6, flags valid");
    let mut ev = envelope(crossvm::SPATIAL_RESET, o);
    assert_eq!(send(&mut ev), 0, "0x46 (13b780) on an initialized, stopped stream");
    let mut ev = envelope(crossvm::SPATIAL_PHASE_THREE, o);
    assert_eq!(send(&mut ev), 0, "0x39 PhaseThree activates the static PC objects (137cf0)");
    assert_eq!(crossvm::spatial_static_objects(o).unwrap(), vec![2, 4], "FrontLeft and FrontRight are active on the PC");
    let mut ev = envelope(crossvm::SPATIAL_PRE_STOP, o);
    assert_eq!(send(&mut ev), 1, "0x45 PreStopStream on a stream that never started is S_FALSE");
    let mut ev = envelope(crossvm::SPATIAL_PRE_START, o);
    assert_eq!(send(&mut ev), 0);
    assert_eq!(ev[0x10], 1, "13b84c: the stream needs a real start");
    let mut ev = envelope(crossvm::SPATIAL_PRE_START, o);
    assert_eq!(send(&mut ev), 0x8889_010au32 as i32, "13b84c: second PreStart is SPTLAUDCLNT_E_STREAM_NOT_STOPPED");
    let mut ev = envelope(crossvm::SPATIAL_RESET, o);
    assert_eq!(send(&mut ev), 0x8889_010au32 as i32, "13b780: Reset while started");
    assert_eq!(ctl.transport(true), 0, "ff210: guest Start needs flags valid");
    let mut ev = envelope(crossvm::START, o);
    assert_eq!(send(&mut ev), 0, "id 7 starts the PC stream");
    let c = crossvm::spatial_counters(o).unwrap();
    let props = shape.properties as usize;
    let fb = shape.frame_bytes as usize;
    let data = base + dcas::DATA;
    let write_pass = |eos: bool| -> bool {
        let Ok(i) = ctl.write_acquire() else { return false };
        let packet = ctl.frames_per_packet(frames);
        for k in 0..2usize {
            let rec = (data + regions.properties[i as usize] + k * props) as *mut u8;
            unsafe {
                core::ptr::write_bytes(rec, 0, props);
                (rec as *mut u32).write(k as u32 + 1);
                (rec.add(4) as *mut u32).write(packet);
                (rec.add(8) as *mut u32).write(2 << k);
                (rec.add(0xc) as *mut u32).write(if eos { 0xa | 1 } else { 3 });
                (rec.add(0x1c) as *mut f32).write(if k == 0 { 1.0 } else { 0.5 });
                core::ptr::write_bytes((data + regions.samples[i as usize] + k * fb) as *mut u8, 0, fb);
            }
        }
        assert_eq!(ctl.write_commit(i, 2, packet), 0, "4c1de CommitWrite");
        true
    };
    let t0 = std::time::Instant::now();
    let mut written = 0u64;
    while (c.buffers.load(Ordering::SeqCst) < 6 || written < 3) && t0.elapsed().as_secs() < 5 {
        if write_pass(false) {
            written += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(c.buffers.load(Ordering::SeqCst) >= 6, "the pump copied guest buffers into the PC objects: passes {} reads {} starved {} buffers {} errors {} last {:#x}", c.passes.load(Ordering::SeqCst), c.reads.load(Ordering::SeqCst), c.starved.load(Ordering::SeqCst), c.buffers.load(Ordering::SeqCst), c.errors.load(Ordering::SeqCst), c.last_error.load(Ordering::SeqCst));
    assert_eq!(c.silent.load(Ordering::SeqCst), c.buffers.load(Ordering::SeqCst), "TOMBOX_AUDIO_MUTE: every PC buffer is zero-filled");
    assert!(c.volumes.load(Ordering::SeqCst) >= 1, "the 0.5 record volume reached ISpatialAudioObject::SetVolume");
    assert!(ctl.frames_committed() > 0, "CommitWrite adds frames per packet to DCAS+0x28");
    let t0 = std::time::Instant::now();
    while !write_pass(true) && t0.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let t0 = std::time::Instant::now();
    while c.end_of_stream.load(Ordering::SeqCst) < 2 && t0.elapsed().as_secs() < 3 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(c.end_of_stream.load(Ordering::SeqCst), 2, "flags 0xa reach SetEndOfStream on both PC objects");
    assert!(crossvm::spatial_static_objects(o).unwrap().is_empty(), "objects past end of stream are released");
    assert_eq!(c.errors.load(Ordering::SeqCst), 0, "no PC spatial call failed (last {:#x})", c.last_error.load(Ordering::SeqCst));
    let mut ev = envelope(crossvm::SPATIAL_PRE_STOP, o);
    assert_eq!(send(&mut ev), 0, "0x45 PreStopStream");
    let mut ev = envelope(crossvm::STOP, o);
    assert_eq!(send(&mut ev), 0, "id 8 stops the PC stream");
    assert_eq!(ctl.transport(false), 0);
    assert_eq!(ctl.flags() & pingpong::FLAG_STARTED, 0, "ff330 clears started");
    println!("spatial pump: {} passes, {} reads, {} starved, {} buffers (all zero-filled), {} guest writes, {} frames committed, PC max frames {frames}", c.passes.load(Ordering::SeqCst), c.reads.load(Ordering::SeqCst), c.starved.load(Ordering::SeqCst), c.buffers.load(Ordering::SeqCst), written, ctl.frames_committed());
    assert_eq!(send(&mut envelope(crossvm::RELEASE_OBJECT, o)), 0);
    assert!(crossvm::spatial_control(o).is_none());
    crossvm::set_hello(1);
}

#[test]
fn crossvm_audio_spatial_pump_reads_guest_records_into_the_pc_stream() {
    run_step("spatial_pump");
}

#[test]
fn crossvm_audio_ping_pong_state_machine_follows_the_console_tables() {
    use console_sandbox::crossvm::pingpong::*;
    let mut store = vec![0u64; 0x20];
    let base = store.as_mut_ptr() as *mut u8;
    let c = unsafe { Control::at(base) };
    let raw = |o: usize| unsafe { (base.add(o) as *const u32).read() };
    let set = |s: u32| unsafe { (base.add(STATE) as *mut u32).write(s) };
    let clear = || unsafe { core::ptr::write_bytes(base, 0, 0x100) };
    for s in 0..STATES as u32 {
        clear();
        unsafe { (base.add(FRAMES_PER_PACKET) as *mut u32).write(480) };
        set(s);
        let n = WRITE_ACQUIRE[s as usize];
        let r = c.write_acquire();
        match WRITER_INDEX[n as usize] {
            3 => assert_eq!((r, c.state()), (Err(E_INVALID_CONTROL), FLUSH[n as usize]), "fd5c4 fault from {s}"),
            2 => assert_eq!((r, c.state(), raw(WRITE_STARVED)), (Err(E_WRITE_STARVED), n, 1), "fd5c4 starve from {s}"),
            i => assert_eq!((r, c.state(), raw(BUFFER_FRAMES + i as usize * 4)), (Ok(i), n, 480), "fd5c4 acquire from {s}"),
        }
        clear();
        set(s);
        let n = WRITE_COMMIT[s as usize];
        let hr = c.write_commit(1, 2, 480);
        assert_eq!((c.state(), raw(BUFFER_OBJECTS + 4)), (n, 2), "4c1de from {s}");
        if WRITER_INDEX[n as usize] == 3 {
            assert_eq!((hr, c.frames_committed()), (E_INVALID_CONTROL, 0));
        } else {
            assert_eq!((hr, c.frames_committed()), (0, 480));
        }
        clear();
        set(s);
        c.write_cancel();
        assert_eq!(c.state(), WRITE_CANCEL[s as usize], "4bde0 cancel from {s}");
        clear();
        set(s);
        let n = READ_ACQUIRE[s as usize];
        let r = c.read_acquire();
        match READER_INDEX[n as usize] {
            3 => assert_eq!((r, c.state()), (Err(E_INVALID_CONTROL), FLUSH[n as usize]), "fd498 fault from {s}"),
            2 => assert_eq!((r, c.state(), raw(READ_STARVED)), (Err(E_READ_STARVED), n, 1), "fd498 starve from {s}"),
            i => assert_eq!((r, c.state()), (Ok(i), n), "fd498 acquire from {s}"),
        }
        clear();
        set(s);
        let n = READ_COMMIT[s as usize];
        let hr = c.read_commit(0);
        assert_eq!((hr, c.state()), (if READER_INDEX[n as usize] == 3 { E_INVALID_CONTROL } else { 0 }, n), "fcc84 from {s}");
        clear();
        set(s);
        unsafe { (base.add(FRAMES_COMMITTED) as *mut u64).write(999) };
        c.flush();
        assert_eq!((c.state(), raw(FLAGS) & FLAG_DISCONTINUITY, c.frames_committed()), (FLUSH[s as usize], FLAG_DISCONTINUITY, 0), "ff194 from {s}");
        if WRITE_ACQUIRE[s as usize] != FAULTED {
            assert_eq!(WRITE_CANCEL[WRITE_ACQUIRE[s as usize] as usize], s, "cancel undoes exactly one writer hold ({s})");
        }
    }
    clear();
    set(0x20);
    assert_eq!(c.state(), FAULTED, "f9cd0: a state above 0x10 is forced to 0x10");
    assert_eq!(raw(STATE), FAULTED);
    clear();
    set(6);
    unsafe { (base.add(FRAMES_PER_PACKET) as *mut u32).write(480) };
    let mut path = vec![c.state()];
    let mut idx = Vec::new();
    for _ in 0..2 {
        idx.push(c.write_acquire().unwrap());
        path.push(c.state());
        assert_eq!(c.write_commit(*idx.last().unwrap(), 2, 480), 0);
        path.push(c.state());
        let i = c.read_acquire().unwrap();
        assert_eq!(i, *idx.last().unwrap(), "the reader gets the buffer the writer committed");
        path.push(c.state());
        assert_eq!(c.read_commit(i), 0);
        path.push(c.state());
    }
    assert_eq!(path, vec![6, 7, 8, 11, 14, 15, 0, 3, 6], "PC sample 6 -> 7 -> 8 -> 0 lies on the ping-pong cycle");
    assert_eq!(idx, vec![0, 1]);
    assert_eq!(c.frames_committed(), 960);
    assert_eq!(c.read_acquire(), Err(E_READ_STARVED), "nothing committed: the reader starves");
    assert_eq!((c.state(), c.counters()), (6, (1, 0)));
    let a = c.write_acquire().unwrap();
    assert_eq!(c.write_commit(a, 1, 480), 0);
    let b = c.write_acquire().unwrap();
    assert_eq!(c.write_commit(b, 1, 480), 0);
    assert_ne!(a, b, "two passes fill both buffers");
    assert_eq!(c.write_acquire(), Err(E_WRITE_STARVED), "both buffers full: the writer starves");
    assert_eq!((c.read_acquire(), c.read_commit(a)), (Ok(a), 0), "oldest first");
    assert_eq!((c.read_acquire(), c.read_commit(b)), (Ok(b), 0));
    assert_eq!(c.take_counters(), (1, 1));
    assert_eq!(c.counters(), (0, 0));
    assert_eq!(c.write_commit(2, 1, 480), E_BAD_WRITE_INDEX, "4c1de: index above 1");
    assert_eq!(c.read_commit(2), 0x8007_0057u32 as i32, "fcc84: index above 1");
    unsafe { (base.add(FRAMES_PER_PACKET) as *mut u32).write(0) };
    assert_eq!(c.frames_per_packet(535), 535, "4c300: 0 frames per packet means the max");
    unsafe { (base.add(FRAMES_PER_PACKET) as *mut u32).write(600) };
    assert_eq!(c.frames_per_packet(535), 535);
    unsafe { (base.add(FRAMES_PER_PACKET) as *mut u32).write(480) };
    assert_eq!(c.frames_per_packet(535), 480);
    clear();
    assert_eq!(c.transport(true), E_INVALID_CONTROL, "ff210: (flags & 0x81) must be 1");
    c.set_flags(FLAG_VALID | FLAG_POSITION_LATCHED, 0);
    assert_eq!((c.transport(true), c.flags()), (0, FLAG_VALID | FLAG_STARTED | FLAG_POSITION_LATCHED));
    assert_eq!((c.transport(false), c.flags()), (0, FLAG_VALID), "ff330 clears 0xa");
    c.set_flags(FLAG_CLOSED, 0);
    assert_eq!(c.reset(), E_INVALID_CONTROL);
    c.set_flags(0, FLAG_CLOSED);
    set(8);
    assert_eq!((c.reset(), c.state()), (0, 14), "ff090 -> ff194: reset drops the full buffer");
    drop(store);
}

fn hrtf_args(packed: bool) -> Vec<u8> {
    let mut a = vec![0u8; if packed { 0xb0 } else { 0xa8 }];
    a[0..4].copy_from_slice(&6u32.to_le_bytes());
    a[12..16].copy_from_slice(&6u32.to_le_bytes());
    a[0x10..0x14].copy_from_slice(&[1, 1, 1, 1]);
    a[0x14..0x18].copy_from_slice(&0u32.to_le_bytes());
    a[0x18..0x1c].copy_from_slice(&1.0f32.to_le_bytes());
    a[0x1c..0x20].copy_from_slice(&1.0e-5f32.to_le_bytes());
    a[0x20..0x24].copy_from_slice(&1.0f32.to_le_bytes());
    a[0x24..0x28].copy_from_slice(&100.0f32.to_le_bytes());
    a[0x38..0x3c].copy_from_slice(&1u32.to_le_bytes());
    for (i, v) in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0].iter().enumerate() {
        a[0x3c + i * 4..0x40 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    let f = spatial_args(0, 0, 0, 0, 3, 1, 32, 0);
    a[0x60..0x72].copy_from_slice(&f[0x10..0x22]);
    a
}

fn metadata_args(packed: bool) -> Vec<u8> {
    let mut a = vec![0u8; if packed { 0x74 } else { 0x58 }];
    a[0..4].copy_from_slice(&6u32.to_le_bytes());
    a[12..16].copy_from_slice(&6u32.to_le_bytes());
    a[0x10..0x20].copy_from_slice(&[0x11; 16]);
    let f = spatial_args(0, 0, 0, 0, 3, 1, 32, 0);
    if packed {
        a[0x20..0x24].copy_from_slice(&8u32.to_le_bytes());
        a[0x24..0x36].copy_from_slice(&f[0x10..0x22]);
    } else {
        a[0x20..0x22].copy_from_slice(&8u16.to_le_bytes());
        a[0x22..0x34].copy_from_slice(&f[0x10..0x22]);
    }
    a
}

fn hrtf_pump(o: u32, a: &[u8], lay: &crossvm::SpatialLayout) {
    use console_sandbox::crossvm::{dcas, pingpong};
    use std::sync::atomic::Ordering;
    let r = &lay.reply;
    assert_eq!(&a[r.clsid..r.clsid + 4], &[0xff, 0xe7, 0x71, 0x93], "123f20 / 1240b8: endpoint CLSID");
    assert_eq!((u32_at(a, r.event), u32_at(a, r.memory)), (2, 3));
    let (base, page, shape, regions) = crossvm::spatial_control(o).unwrap();
    assert_eq!(u32_at(a, r.memory + 0x24), page as u32);
    assert_eq!((shape.objects, shape.properties, shape.metadata), (2, crossvm::SPATIAL_HRTF_PROPERTY_BYTES, 0), "PC oracle: HRTF DCAS records are 0xb4 bytes");
    let ctl = unsafe { pingpong::Control::at(base as *mut u8) };
    assert_eq!(send(&mut envelope(crossvm::SPATIAL_PHASE_THREE, o)), 0);
    assert_eq!(crossvm::spatial_static_objects(o).unwrap(), vec![2, 4]);
    assert_eq!(send(&mut envelope(crossvm::SPATIAL_PRE_START, o)), 0);
    assert_eq!(send(&mut envelope(crossvm::START, o)), 0);
    let c = crossvm::spatial_counters(o).unwrap();
    let data = base + dcas::DATA;
    let props = shape.properties as usize;
    let fb = shape.frame_bytes as usize;
    let frames = u32::from_le_bytes(unsafe { *((base + dcas::FRAMES) as *const [u8; 4]) });
    let t0 = std::time::Instant::now();
    let mut writes = 0;
    while (c.hrtf_properties.load(Ordering::SeqCst) < 10 || c.buffers.load(Ordering::SeqCst) < 2) && t0.elapsed().as_secs() < 5 {
        if let Ok(i) = ctl.write_acquire() {
            let packet = ctl.frames_per_packet(frames);
            for k in 0..2usize {
                let rec = (data + regions.properties[i as usize] + k * props) as *mut u8;
                unsafe {
                    core::ptr::write_bytes(rec, 0, props);
                    (rec.add(4) as *mut u32).write(packet);
                    (rec.add(8) as *mut u32).write(2 << k);
                    (rec.add(0xc) as *mut u32).write(0x13);
                    (rec.add(0x1c) as *mut f32).write(1.0);
                    (rec.add(crossvm::HRTF_DIRTY) as *mut u32).write(0x1f);
                    (rec.add(crossvm::HRTF_GAIN) as *mut f32).write(0.5);
                    for (j, v) in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0].iter().enumerate() {
                        (rec.add(crossvm::HRTF_ORIENTATION + j * 4) as *mut f32).write(*v);
                    }
                    (rec.add(crossvm::HRTF_ENVIRONMENT) as *mut u32).write(1);
                    let d: [u32; 5] = [0, 1.0f32.to_bits(), 1.0e-5f32.to_bits(), 1.0f32.to_bits(), 100.0f32.to_bits()];
                    core::ptr::copy_nonoverlapping(d.as_ptr() as *const u8, rec.add(crossvm::HRTF_DECAY), 0x14);
                    let dir: [u32; 4] = [0, 0, 0, 0];
                    core::ptr::copy_nonoverlapping(dir.as_ptr() as *const u8, rec.add(crossvm::HRTF_DIRECTIVITY), 0x10);
                    core::ptr::write_bytes((data + regions.samples[i as usize] + k * fb) as *mut u8, 0, fb);
                }
            }
            assert_eq!(ctl.write_commit(i, 2, packet), 0);
            writes += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    println!("hrtf pump: {} passes, {} buffers (zero-filled {}), {} HRTF property calls, {} guest writes, errors {} last {:#x}", c.passes.load(Ordering::SeqCst), c.buffers.load(Ordering::SeqCst), c.silent.load(Ordering::SeqCst), c.hrtf_properties.load(Ordering::SeqCst), writes, c.errors.load(Ordering::SeqCst), c.last_error.load(Ordering::SeqCst));
    assert!(c.buffers.load(Ordering::SeqCst) >= 2 && c.hrtf_properties.load(Ordering::SeqCst) >= 10, "the HRTF records reached the PC ISpatialAudioObjectForHrtf setters");
    assert_eq!(c.silent.load(Ordering::SeqCst), c.buffers.load(Ordering::SeqCst));
    assert_eq!(c.errors.load(Ordering::SeqCst), 0, "last {:#x}", c.last_error.load(Ordering::SeqCst));
    assert_eq!(send(&mut envelope(crossvm::SPATIAL_PRE_STOP, o)), 0);
    assert_eq!(send(&mut envelope(crossvm::STOP, o)), 0);
    assert_eq!(send(&mut envelope(crossvm::RELEASE_OBJECT, o)), 0);
}

fn spatial_variants() {
    use console_sandbox::wasapi;
    crossvm::set_hello(2);
    let sp = wasapi::spatial(None).unwrap();
    for (kind, method, args) in [
        (crossvm::SPATIAL_HRTF_STREAM_TYPE, crossvm::METHOD_SPATIAL_HRTF_STREAM, hrtf_args(false)),
        (crossvm::SPATIAL_HRTF_STREAM_TYPE, crossvm::METHOD_SPATIAL_HRTF_STREAM2, hrtf_args(true)),
        (crossvm::SPATIAL_METADATA_STREAM_TYPE, crossvm::METHOD_SPATIAL_METADATA_STREAM, metadata_args(false)),
        (crossvm::SPATIAL_METADATA_STREAM_TYPE, crossvm::METHOD_SPATIAL_METADATA_STREAM2, metadata_args(true)),
    ] {
        let lay = crossvm::spatial_layout(kind, method).unwrap();
        let mut full = vec![0u8; 0x14];
        full.extend_from_slice(&args);
        let fmt = full[lay.format..lay.format + 0x12].to_vec();
        assert_eq!(u16::from_le_bytes([fmt[0], fmt[1]]), 3, "format at A+{:#x} for type {kind:#x} method {method:#x}", lay.format);
        let extra = if kind == crossvm::SPATIAL_HRTF_STREAM_TYPE {
            wasapi::SpatialExtra::Hrtf { decay: Some(full[0x28..0x3c].try_into().unwrap()), directivity: Some(full[0x3c..0x4c].try_into().unwrap()), environment: Some(1), orientation: Some(full[0x50..0x74].try_into().unwrap()) }
        } else {
            let n = if method == crossvm::METHOD_SPATIAL_METADATA_STREAM2 { u32_at(&full, 0x34) } else { u16::from_le_bytes([full[0x34], full[0x35]]) as u32 };
            assert_eq!(n, 8, "max metadata items at A+0x34");
            wasapi::SpatialExtra::Metadata { format_id: [0x11; 16], max_items: 8 }
        };
        let direct = sp.activate_stream(&wasapi::SpatialStreamParams { format: fmt, mask: 6, min: 0, max: 0, category: 6, options: 0, extra }).map(|_| ()).err();
        let (hr, e) = create(kind, 1, 0, method, &args);
        match direct {
            Some(pc) => assert_eq!(hr, pc, "type {kind:#x} method {method:#x}: the host returns the PC activation error"),
            None if kind == crossvm::SPATIAL_HRTF_STREAM_TYPE => {
                assert_eq!(hr, 0, "type {kind:#x} method {method:#x}: the PC HRTF stream backs a guest object");
                hrtf_pump(u32_at(&e, 0x1c), &e[0x10..], &lay);
            }
            None => assert_eq!(hr, 0x8000_4001u32 as i32, "type {kind:#x} method {method:#x}: PC metadata stream activated; the DCAS metadata bytes are not decoded"),
        }
        println!("spatial variant type {kind:#x} method {method:#x}: PC ActivateSpatialAudioStream -> {:#010x}", hr as u32);
        let mut short = args.clone();
        let fo = lay.format - 0x14 + 0x10;
        short[fo..fo + 2].copy_from_slice(&0x37u16.to_le_bytes());
        let (hr, _) = create(kind, 1, 0, method, &short);
        assert_eq!(hr, -2147024809, "type {kind:#x} method {method:#x}: cbSize + 0x12 above 0x48");
    }
    crossvm::set_hello(1);
}

#[test]
fn crossvm_audio_hrtf_and_metadata_streams_forward_the_console_params_to_the_pc() {
    run_step("spatial_variants");
}

#[test]
fn crossvm_audio_dcas_frames_match_the_live_pc_dcas_on_every_render_endpoint() {
    use console_sandbox::wasapi;
    unsafe { wasapi::com_init() };
    let mut fmt = vec![0u8; 0x12];
    fmt[0..2].copy_from_slice(&3u16.to_le_bytes());
    fmt[2..4].copy_from_slice(&1u16.to_le_bytes());
    fmt[4..8].copy_from_slice(&48000u32.to_le_bytes());
    fmt[8..12].copy_from_slice(&192000u32.to_le_bytes());
    fmt[12..14].copy_from_slice(&4u16.to_le_bytes());
    fmt[14..16].copy_from_slice(&32u16.to_le_bytes());
    let _pc = PC_AUDIO.lock().unwrap_or_else(|e| e.into_inner());
    let mut checked = Vec::new();
    for (id, _) in wasapi::endpoints(0, 1).unwrap() {
        let Ok(sp) = wasapi::spatial(Some(&id)) else { continue };
        let Ok(max) = sp.max_frames(&fmt) else { continue };
        let mix = wasapi::sample_rate(&wasapi::device_info_for(Some(&id)).unwrap().mix);
        let before = pc_dcas_blocks();
        let params = wasapi::SpatialStreamParams { format: fmt.clone(), mask: 2, min: 0, max: 0, category: 6, options: 0, extra: wasapi::SpatialExtra::Plain };
        let Ok(stream) = sp.activate_stream(&params) else { continue };
        let fresh: Vec<usize> = pc_dcas_blocks().into_iter().filter(|b| !before.contains(b)).collect();
        assert_eq!(fresh.len(), 1, "one new PC DCAS per activation on {id}");
        let dcas = unsafe { core::slice::from_raw_parts(fresh[0] as *const u8, 0x340) };
        let pc = u32_at(dcas, crossvm::dcas::FRAMES);
        let host = crossvm::engine_frames(max, 48000, mix);
        println!("endpoint mix {mix} Hz: PC max frames {max}, PC DCAS +0x204 {pc}, host {host}, PC +0x34 {}", u32_at(dcas, crossvm::pingpong::FRAMES_PER_PACKET));
        assert_eq!(host, pc, "the host DCAS frame count equals the PC engine's for a {mix} Hz endpoint");
        let available = u32_at(dcas, crossvm::pingpong::FRAMES_PER_PACKET);
        assert!(available >= max && available <= pc, "PC +0x34 at activation {available} lies between the max frame count {max} and the DCAS frame count {pc}");
        drop(stream);
        checked.push(mix);
    }
    assert!(!checked.is_empty(), "at least one render endpoint activates a PC spatial stream");
}

#[test]
fn crossvm_audio_stream_status_errors_are_the_42a00_class() {
    for hr in [crossvm::SPTLAUDCLNT_E_DESTROYED, crossvm::AUDCLNT_E_DEVICE_INVALIDATED, crossvm::AUDCLNT_E_RESOURCES_INVALIDATED] {
        assert!(crossvm::stream_status_error(hr));
    }
    for hr in [0, 1, 0x8889_010au32 as i32, 0x887c_0061u32 as i32, 0x8007_0057u32 as i32] {
        assert!(!crossvm::stream_status_error(hr));
    }
    assert_eq!((crossvm::engine_frames(480, 48000, 48000), crossvm::engine_frames(480, 48000, 96000)), (528, 535), "PC oracle: 48 kHz endpoints 528, 96 kHz endpoints 535");
}

#[test]
fn crossvm_audio_spatial_activation_blobs_follow_the_console_layouts() {
    use console_sandbox::wasapi::{spatial_params_blob, SpatialExtra, SpatialStreamParams};
    let at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let fmt = vec![3u8; 0x12];
    let plain = SpatialStreamParams { format: fmt.clone(), mask: 6, min: 1, max: 2, category: 6, options: 1, extra: SpatialExtra::Plain };
    let (b, _) = spatial_params_blob(&plain, 0x1234);
    assert_eq!(b.len(), 0x2c, "124e00: VT_BLOB 0x2c");
    assert_eq!((at(&b, 0), u32_at(&b, 8), u32_at(&b, 0xc), u32_at(&b, 0x10), u32_at(&b, 0x14), at(&b, 0x18), at(&b, 0x20), u32_at(&b, 0x28)), (plain.format.as_ptr() as u64, 6, 1, 2, 6, 0x1234, 0, 1));
    let h = SpatialStreamParams { format: fmt.clone(), mask: 6, min: 0, max: 0, category: 6, options: 1, extra: SpatialExtra::Hrtf { decay: Some([1; 0x14]), directivity: None, environment: Some(3), orientation: Some([2; 0x24]) } };
    let (b, _) = spatial_params_blob(&h, 0x99);
    assert_eq!(b.len(), 0x4c, "125160: VT_BLOB 0x4c");
    assert_eq!((at(&b, 0x18), at(&b, 0x20), at(&b, 0x30), u32_at(&b, 0x48)), (0x99, 0, 0, 1), "event 0x18, notify 0x20, absent directivity 0x30, options 0x48");
    assert_ne!(at(&b, 0x28), 0);
    assert_ne!(at(&b, 0x38), 0);
    assert_ne!(at(&b, 0x40), 0);
    if let SpatialExtra::Hrtf { decay: Some(d), environment: Some(e), .. } = &h.extra {
        assert_eq!(at(&b, 0x28), d.as_ptr() as u64);
        assert_eq!(at(&b, 0x38), e as *const u32 as u64);
    }
    let m = SpatialStreamParams { format: fmt.clone(), mask: 6, min: 0, max: 0, category: 6, options: 1, extra: SpatialExtra::Metadata { format_id: [0x22; 16], max_items: 9 } };
    let (b, _) = spatial_params_blob(&m, 0x77);
    assert_eq!(b.len(), 0x48, "1255d0: VT_BLOB 0x48");
    assert_eq!((&b[0x20..0x30], u32_at(&b, 0x30), at(&b, 0x34), at(&b, 0x3c), u32_at(&b, 0x44)), (&[0x22u8; 16][..], 9, 0, 0, 1), "GUID 0x20, max items 0x30, activation params 0x34, notify 0x3c, options 0x44");
    for (kind, method, fmt_at, clsid, memory) in [(0xf, 0x36, 0x24, 0x70, 0x1bc), (0xf, 0x82, 0x24, 0x74, 0x1c0), (0x10, 0x37, 0x74, 0xc0, 0x20c), (0x10, 0x83, 0x74, 0xc4, 0x210), (0x16, 0x38, 0x36, 0x82, 0x1ce), (0x16, 0x84, 0x38, 0x88, 0x1d4)] {
        let l = crossvm::spatial_layout(kind, method).unwrap();
        assert_eq!((l.format, l.reply.clsid, l.reply.memory), (fmt_at, clsid, memory), "type {kind:#x} method {method:#x}");
        assert_eq!(l.reply.event, l.reply.clsid + 0x24, "event descriptor follows CLSID, u32, u64, u64");
        assert!(crossvm::spatial_layout_end(&l) <= 0x800);
    }
    assert!(crossvm::spatial_layout(0x10, 0x36).is_none() && crossvm::spatial_layout(0x16, 0x82).is_none());
}

fn media() {
    use console_sandbox::wasapi::{HANDLER_INVOKES, HANDLER_LIVE};
    use std::sync::atomic::Ordering;
    crossvm::set_hello(2);
    let (hr, _) = create(crossvm::MEDIA_MANAGER_TYPE, 1, 0, crossvm::METHOD_DEFAULT_ENDPOINT, &[0; 8]);
    assert_eq!(hr, -2147024809, "the type 0x13 creator 8ecf0 rejects a method other than 0x61 (8bfd8)");
    let (hr, _) = create(0x2, 1, 0, crossvm::METHOD_MEDIA_MANAGER, &[]);
    assert_eq!(hr, -2147024809, "method 0x61 belongs to type 0x13");
    let live = HANDLER_LIVE.load(Ordering::SeqCst);
    let (hr, e) = create(crossvm::MEDIA_MANAGER_TYPE, 1, 0, crossvm::METHOD_MEDIA_MANAGER, &[]);
    assert_eq!(hr, 0, "0x61 opens the PC Windows.Media.Internal.MediaManager statics and adds the no-op EventHandler<Object>");
    let m = u32_at(&e, 0x1c);
    assert!(m != 0);
    assert_eq!(HANDLER_LIVE.load(Ordering::SeqCst), live + 1, "the PC MediaManager keeps the host handler while the object lives");
    for id in [crossvm::MEDIA_PLAY_TO_STREAM_STATE, crossvm::MEDIA_CASTING_APP_STATE, crossvm::MEDIA_VOIP_CALL_STATE] {
        let mut e = envelope(id, m);
        e[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(send(&mut e), -2147024809, "id {id:#x}: the PC MediaManager local path rejects a state above 1 (f6820 / 64670 / 5e740, as XD 119de0)");
        let mut e = envelope(id, m);
        assert_eq!(send(&mut e), 0, "id {id:#x}: state 0 reaches PC AudioSrv PlaybackManager");
    }
    let mut e = envelope(crossvm::MEDIA_PLAY_TO_STREAM_STATE, 0x7fff_fff0);
    assert_eq!(send(&mut e), -2147024809, "no MediaManager object behind the id");
    let mut e = envelope(crossvm::RELEASE_OBJECT, m);
    assert_eq!(send(&mut e), 0);
    let mut released = false;
    for _ in 0..50 {
        if HANDLER_LIVE.load(Ordering::SeqCst) == live {
            released = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(released, "ReleaseObject removes the handler from the PC MediaManager, which drops its reference");
    println!("media: 0x61 object {m:#x}, 0x64-0x66 state 2 -> E_INVALIDARG and state 0 -> S_OK on the PC, handler removed on release, {} handler invokes", HANDLER_INVOKES.load(Ordering::SeqCst));
    crossvm::set_hello(1);
}

fn session_valid<'a>(f: &'a [u8], sid: &str) -> &'a [u8] {
    assert_eq!(f.len(), 0x810);
    assert_eq!(u32_at(f, 8), 0);
    let r = &f[crossvm::NOTIFY_RECORD..];
    let size = u32_at(r, 0) as usize;
    let kind = u32_at(r, 4);
    let sl = u32_at(r, 0x18) as usize;
    assert!(size >= 0x50 && kind & 0xffe17000 == 0 && kind.count_ones() == 1, "AudioSes 7be0 header checks");
    assert!(sl != 0 && sl <= size && 0x48 + sl <= size && u16::from_le_bytes([r[0x46 + sl], r[0x47 + sl]]) == 0);
    assert_eq!(wide_at(r, 0x48, sl / 2), sid, "+0x48 is the session instance identifier the guest compares (wcsicmp)");
    assert_eq!(2 * (sid.encode_utf16().count() + 1), sl);
    &r[..size]
}
