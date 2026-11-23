use core::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn ReadProcessMemory(h: *mut c_void, addr: *const c_void, buf: *mut c_void, n: usize, read: *mut usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetThreadId(h: *mut c_void) -> u32;
    fn GetExitCodeThread(h: *mut c_void, code: *mut u32) -> i32;
    fn QueryThreadCycleTime(h: *mut c_void, cycles: *mut u64) -> i32;
    fn GetThreadDescription(h: *mut c_void, desc: *mut *mut u16) -> i32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn GetLastError() -> u32;
}

pub const VAR: &str = "TOMBOX_WEBM_PROBE";
pub const KEYS: [&str; 8] = ["webm.init_count", "webm.thread_handle", "webm.quit_flag", "webm.affinity_mask", "webm.decoders", "webm.decoder_count", "webm.round_robin", "webm.wake_time"];
pub const DECODER_FIELDS: &str = "webm.decoder";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Globals {
    pub init_count: usize,
    pub thread_handle: usize,
    pub quit_flag: usize,
    pub affinity_mask: usize,
    pub decoders: usize,
    pub decoder_count: usize,
    pub round_robin: usize,
    pub wake_time: usize,
}

pub fn layout(o: &crate::title_offsets::Offsets) -> Result<(Globals, Vec<(String, usize, usize)>), String> {
    let v = o.require(&KEYS)?;
    let g = Globals { init_count: v[0], thread_handle: v[1], quit_flag: v[2], affinity_mask: v[3], decoders: v[4], decoder_count: v[5], round_robin: v[6], wake_time: v[7] };
    Ok((g, o.fields(DECODER_FIELDS).to_vec()))
}

pub const MAX_DECODERS: usize = 8;

fn read(addr: usize, len: usize) -> Option<u64> {
    let mut v = 0u64;
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), addr as *const c_void, &mut v as *mut u64 as *mut c_void, len.min(8), &mut got) };
    (ok != 0 && got == len.min(8)).then_some(v)
}

fn thread_line(h: usize) -> (String, u64) {
    if h == 0 {
        return ("thread handle 0 (not created)".into(), 0);
    }
    let hp = h as *mut c_void;
    let tid = unsafe { GetThreadId(hp) };
    let mut code = 0u32;
    let alive = unsafe { GetExitCodeThread(hp, &mut code) } != 0;
    let mut cyc = 0u64;
    let cyc_ok = unsafe { QueryThreadCycleTime(hp, &mut cyc) } != 0;
    let mut p: *mut u16 = core::ptr::null_mut();
    let name = if unsafe { GetThreadDescription(hp, &mut p) } >= 0 && !p.is_null() {
        let mut n = 0usize;
        while n < 128 && unsafe { *p.add(n) } != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p, n) });
        unsafe { LocalFree(p as *mut c_void) };
        s
    } else {
        String::new()
    };
    let exit = if !alive { format!("exitcode? err={}", unsafe { GetLastError() }) } else if code == 259 { "running".into() } else { format!("exited {code:#x}") };
    (format!("thread h={h:#x} tid={tid} name='{name}' {exit} cycle_query={}", if cyc_ok { "ok" } else { "failed" }), cyc)
}

fn snapshot(base: usize, l: &Globals, fields: &[(String, usize, usize)]) -> (String, Vec<String>, u64) {
    let g = |rva: usize, len: usize| read(base + rva, len).unwrap_or(u64::MAX);
    let h = g(l.thread_handle, 8) as usize;
    let mask = g(l.affinity_mask, 4);
    let topo = super::affinity::topology();
    let host = topo.to_host(mask, super::affinity::process_console_mask());
    let (tl, cycles) = thread_line(if h == usize::MAX { 0 } else { h });
    let head = format!(
        "init={} quit={} mask={mask:#x} host={} count={} rr={} wake={} {}",
        g(l.init_count, 4),
        g(l.quit_flag, 4),
        host.map(|v| format!("{v:#x}")).unwrap_or_else(|| "REJECTED".into()),
        g(l.decoder_count, 4),
        g(l.round_robin, 4),
        g(l.wake_time, 4),
        tl
    );
    let arr = g(l.decoders, 8) as usize;
    let n = (g(l.decoder_count, 4) as usize).min(MAX_DECODERS);
    let mut decs = Vec::new();
    for i in 0..n {
        let d = read(arr + i * 8, 8).unwrap_or(0) as usize;
        if d == 0 {
            decs.push(format!("dec[{i}]=0"));
            continue;
        }
        let parts: Vec<String> = fields.iter().map(|(k, off, len)| match read(d + off, *len) {
            Some(v) => format!("{k}={v:#x}"),
            None => format!("{k}=?"),
        }).collect();
        decs.push(format!("dec[{i}]@{d:#x} {}", parts.join(" ")));
    }
    (head, decs, cycles)
}

pub fn enabled() -> bool {
    std::env::var(VAR).ok().map(|v| v.trim() != "0" && !v.trim().is_empty()).unwrap_or(false)
}

pub fn start() {
    if !enabled() {
        return;
    }
    let base = unsafe { GetModuleHandleW(core::ptr::null()) } as usize;
    if base == 0 {
        return;
    }
    let Some(o) = crate::title_offsets::for_this_title() else {
        crate::record_raw("webm_probe", "inert", &crate::title_offsets::status_for_this_title());
        return;
    };
    let (l, fields) = match layout(o) {
        Ok(v) => v,
        Err(e) => {
            crate::record_raw("webm_probe", "inert", &e);
            return;
        }
    };
    let stamp = unsafe { crate::title_offsets::image_timestamp(base as *const u8) };
    if !o.matches_image(stamp) {
        crate::record_raw("webm_probe", "inert", &format!("{} was made for another build (image timestamp {stamp:#x})", o.source));
        return;
    }
    let period = std::env::var(VAR).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|v| *v >= 100).unwrap_or(1000);
    let _ = std::thread::Builder::new().name("tombox-webm-probe".into()).spawn(move || {
        crate::record_raw("webm_probe", "start", &format!("base={base:#x} period={period}ms offsets={}", o.source));
        let mut last_head = String::new();
        let mut last_decs: Vec<String> = Vec::new();
        let mut last_cycles = 0u64;
        let mut last_running: Option<bool> = None;
        let mut lines = 0u32;
        for tick in 0..(240_000 / period) {
            std::thread::sleep(std::time::Duration::from_millis(period));
            let (head, decs, cycles) = snapshot(base, &l, &fields);
            let ran = if last_cycles == 0 { 0 } else { cycles.saturating_sub(last_cycles) };
            let running = ran > 0;
            if head != last_head || (last_cycles != 0 && last_running != Some(running)) {
                if last_cycles != 0 {
                    last_running = Some(running);
                }
                crate::record_raw("webm_probe", "state", &format!("t={}ms {head} cycles_delta={ran}", (tick + 1) * period));
                last_head = head;
                lines += 1;
            }
            for (i, d) in decs.iter().enumerate() {
                if last_decs.get(i) != Some(d) {
                    crate::record_raw("webm_probe", "decoder", &format!("t={}ms cycles_delta={ran} {d}", (tick + 1) * period));
                    lines += 1;
                }
            }
            last_decs = decs;
            last_cycles = cycles;
            if lines > 600 {
                crate::record_raw("webm_probe", "stop", "line cap reached");
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_comes_from_the_offsets_file() {
        let o = crate::title_offsets::parse(r#"{"exe":"T.exe","values":{"webm.init_count":1,"webm.thread_handle":2,"webm.quit_flag":3,"webm.affinity_mask":4,"webm.decoders":5,"webm.decoder_count":6,"webm.round_robin":7,"webm.wake_time":8},"fields":{"webm.decoder":[["frames","0x10",4]]}}"#, "t").unwrap();
        let (g, f) = layout(&o).unwrap();
        assert_eq!((g.init_count, g.wake_time, g.decoders), (1, 8, 5));
        assert_eq!(f, vec![("frames".to_string(), 0x10, 4)]);
        let partial = crate::title_offsets::parse(r#"{"exe":"T.exe","values":{"webm.init_count":1}}"#, "t").unwrap();
        assert!(layout(&partial).unwrap_err().contains("webm.wake_time"));
    }

    #[test]
    fn a_snapshot_reads_through_the_layout() {
        let mut img = vec![0u8; 0x100];
        img[0x10..0x14].copy_from_slice(&7u32.to_le_bytes());
        img[0x30..0x34].copy_from_slice(&0u32.to_le_bytes());
        let base = img.as_ptr() as usize;
        let l = Globals { init_count: 0x10, thread_handle: 0x18, quit_flag: 0x20, affinity_mask: 0x24, decoders: 0x28, decoder_count: 0x30, round_robin: 0x34, wake_time: 0x38 };
        let (head, decs, _) = snapshot(base, &l, &[]);
        assert!(head.starts_with("init=7 quit=0"), "{head}");
        assert!(decs.is_empty());
    }
}
