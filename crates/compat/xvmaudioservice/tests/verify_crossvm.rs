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

fn run_step(step: &str) {
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
    assert_eq!(hr, 0, "type 0x19 is accepted after the 0x00020008 hello");
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

    let (hr, e) = create(4, 0, dev, 0, &[]);
    assert_eq!(hr, 0);
    let sc = u32_at(&e, 0x1c);
    let pc = match wasapi::spatial(Some(&id)) {
        Ok(p) => p,
        Err(hr) => {
            let mut e = envelope(crossvm::SPATIAL_STATIC_POSITION, sc);
            assert_eq!(send(&mut e), hr, "no PC ISpatialAudioClient: the PC error is returned");
            println!("PC ISpatialAudioClient unavailable {hr:#010x}");
            return;
        }
    };
    let mut e = envelope(crossvm::SPATIAL_STATIC_POSITION, sc);
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
    let mut e = envelope(crossvm::FORMAT_ENUM_COUNT, sc);
    let hr = send(&mut e);
    match pc.format_count() {
        Ok(n) => {
            assert_eq!(hr, 0);
            assert_eq!(u32_at(&e, 0x10), n);
            println!("object format count {n}");
        }
        Err(pe) => assert_eq!(hr, pe),
    }
    let mut e = envelope(crossvm::FORMAT_ENUM_GET, sc);
    let hr = send(&mut e);
    let fmt = match pc.format_at(0) {
        Ok(f) => {
            assert_eq!(hr, 0);
            let plain = crossvm::plain_format(&f);
            assert_eq!(&e[0x14..0x26], &plain[..], "WAVEFORMATEX (0x12 bytes) at R+4");
            assert_eq!(u16::from_le_bytes([e[0x24], e[0x25]]), 0, "cbSize 0: the guest copies only 0x12 bytes");
            println!("object format tag {:#x} {} Hz {} ch", u16::from_le_bytes([plain[0], plain[1]]), u32::from_le_bytes(plain[4..8].try_into().unwrap()), plain[2]);
            Some(f)
        }
        Err(pe) => {
            assert_eq!(hr, pe);
            None
        }
    };
    if let Some(f) = fmt {
        let mut e = envelope(crossvm::SPATIAL_MAX_FRAMES_FOR_CATEGORY, sc);
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
        let mut e = envelope(crossvm::RELEASE_OBJECT, o);
        assert_eq!(send(&mut e), 0);
    }
}
