use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn FlushInstructionCache(p: *mut c_void, base: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

pub const TITLE_EXE: &str = "fortniteclient-xsx-shipping.exe";

pub const VALUE_KEYS: [&str; 9] = ["fn.start_game_instance_site", "fn.browse", "fn.browse_vtable_slot0", "fn.browse_vtable_slot1", "fn.browse_vtable_slot2", "fn.furl_ctor", "fn.furl_ctor_call_site", "fn.furl_dtor", "fn.browse_vcall_site"];
pub const BYTE_KEYS: [&str; 5] = ["fn.start_game_instance_original", "fn.start_game_instance_patched", "fn.browse_prologue", "fn.furl_dtor_prologue", "fn.browse_vcall"];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub site: usize,
    pub browse: usize,
    pub slots: Vec<usize>,
    pub ctor: usize,
    pub ctor_call_site: usize,
    pub dtor: usize,
    pub vcall_site: usize,
    pub original: [u8; 7],
    pub patched: [u8; 7],
    pub browse_prologue: Vec<u8>,
    pub dtor_prologue: Vec<u8>,
    pub vcall: Vec<u8>,
}

pub fn layout(o: &crate::title_offsets::Offsets) -> Result<Layout, String> {
    let v = o.require(&VALUE_KEYS)?;
    let b = o.require_bytes(&BYTE_KEYS)?;
    let seven = |i: usize| -> Result<[u8; 7], String> { b[i].as_slice().try_into().map_err(|_| format!("{}: {} must be 7 bytes", o.source, BYTE_KEYS[i])) };
    Ok(Layout {
        site: v[0],
        browse: v[1],
        slots: vec![v[2], v[3], v[4]],
        ctor: v[5],
        ctor_call_site: v[6],
        dtor: v[7],
        vcall_site: v[8],
        original: seven(0)?,
        patched: seven(1)?,
        browse_prologue: b[2].clone(),
        dtor_prologue: b[3].clone(),
        vcall: b[4].clone(),
    })
}

pub const FURL_SIZE: usize = 0x68;
pub const TRAVEL_PARTIAL: u32 = 1;

pub const DEFAULT_MAP: &str = "/Game/Athena/Apollo/Maps/Apollo_Terrain";
pub const DEFAULT_GAMEMODE: &str = "/Game/Athena/Athena_GameMode.Athena_GameMode_C";
pub const FALLBACK_GAMEMODE: &str = "/Script/FortniteGame.FortGameModeAthena";
pub const DEFAULT_PLAYLIST: &str = "Playlist_DefaultSquad";
pub const DEFAULT_SENTINEL: &str = "127.0.0.1:7777";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Replay,
    Map,
    Play,
}

pub const DEFAULT_MODE: Mode = Mode::Play;

pub fn parse_mode(v: Option<&str>) -> Mode {
    match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("replay") => Mode::Replay,
        Some("map") => Mode::Map,
        Some("off") | Some("0") | Some("none") | Some("false") | Some("disabled") => Mode::Off,
        _ => DEFAULT_MODE,
    }
}

pub fn mode() -> Mode {
    parse_mode(std::env::var("TOMBOX_FN_MATCH").ok().as_deref())
}

pub fn parse_endpoint(v: &str) -> Option<(String, u16)> {
    let (h, p) = v.trim().rsplit_once(':')?;
    let port = p.trim().parse::<u16>().ok()?;
    let host = h.trim().trim_start_matches('[').trim_end_matches(']').to_string();
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

pub fn sentinel() -> (String, u16) {
    std::env::var("TOMBOX_FN_GAMESERVER")
        .ok()
        .and_then(|v| parse_endpoint(&v))
        .unwrap_or_else(|| parse_endpoint(DEFAULT_SENTINEL).unwrap())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
}

pub fn playlist() -> String {
    env_or("TOMBOX_FN_PLAYLIST", DEFAULT_PLAYLIST)
}

pub fn gamemode() -> String {
    env_or("TOMBOX_FN_GAMEMODE", DEFAULT_GAMEMODE)
}

pub fn map() -> String {
    env_or("TOMBOX_FN_MAP", DEFAULT_MAP)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiteState {
    Original,
    AlreadyPatched,
    Mismatch,
}

pub fn classify_site(call_byte: u8, bytes: &[u8; 7], original: &[u8; 7], patched: &[u8; 7]) -> SiteState {
    if call_byte != 0xE8 {
        return SiteState::Mismatch;
    }
    if bytes == original {
        SiteState::Original
    } else if bytes == patched {
        SiteState::AlreadyPatched
    } else {
        SiteState::Mismatch
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

unsafe fn protected_write(dst: *mut u8, src: &[u8]) -> bool {
    let mut old = 0u32;
    if VirtualProtect(dst as *mut c_void, src.len(), 0x40, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
    let mut tmp = 0u32;
    VirtualProtect(dst as *mut c_void, src.len(), old, &mut tmp);
    FlushInstructionCache(GetCurrentProcess(), dst as *const c_void, src.len());
    true
}

pub unsafe fn patch_cmdline_at(site: *mut u8, original: &[u8; 7], patched: &[u8; 7]) -> Result<SiteState, String> {
    let call_byte = *site.sub(5);
    let mut cur = [0u8; 7];
    core::ptr::copy_nonoverlapping(site, cur.as_mut_ptr(), 7);
    match classify_site(call_byte, &cur, original, patched) {
        SiteState::Original => {
            if protected_write(site, patched) {
                Ok(SiteState::Original)
            } else {
                Err(format!("VirtualProtect failed at {:#x}", site as usize))
            }
        }
        SiteState::AlreadyPatched => Ok(SiteState::AlreadyPatched),
        SiteState::Mismatch => Err(format!("{:02X} | {}", call_byte, hex(&cur))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlView {
    pub protocol: String,
    pub host: String,
    pub port: i32,
    pub map: String,
    pub ops: Vec<String>,
}

impl UrlView {
    pub fn describe(&self) -> String {
        let mut s = String::new();
        if !self.host.is_empty() {
            s.push_str(&format!("{}:{}", self.host, self.port));
        }
        s.push_str(&self.map);
        for o in &self.ops {
            s.push('?');
            s.push_str(o);
        }
        s
    }
}

pub fn is_sentinel(url: &UrlView, sentinel: &(String, u16)) -> bool {
    !url.host.is_empty() && url.host.eq_ignore_ascii_case(&sentinel.0) && url.port == sentinel.1 as i32
}

pub fn rewrite_text(url: &UrlView, sentinel: &(String, u16), map: &str, gamemode: &str, playlist: &str) -> Option<String> {
    if !is_sentinel(url, sentinel) {
        return None;
    }
    let mut t = format!("{map}?game={gamemode}");
    if !playlist.is_empty() {
        t.push_str(&format!("?Playlist={playlist}"));
    }
    Some(t)
}

unsafe fn read_fstring(p: *const u8) -> String {
    let data = core::ptr::read_unaligned(p as *const *const u16);
    let num = core::ptr::read_unaligned(p.add(8) as *const i32);
    if data.is_null() || num <= 0 || num > 0x10000 {
        return String::new();
    }
    let mut n = num as usize;
    let s = core::slice::from_raw_parts(data, n);
    while n > 0 && s[n - 1] == 0 {
        n -= 1;
    }
    String::from_utf16_lossy(&s[..n])
}

unsafe fn read_url(p: *const u8) -> UrlView {
    let ops_data = core::ptr::read_unaligned(p.add(0x48) as *const *const u8);
    let ops_num = core::ptr::read_unaligned(p.add(0x50) as *const i32);
    let mut ops = Vec::new();
    if !ops_data.is_null() && ops_num > 0 && ops_num < 256 {
        for i in 0..ops_num as usize {
            ops.push(read_fstring(ops_data.add(i * 16)));
        }
    }
    UrlView {
        protocol: read_fstring(p),
        host: read_fstring(p.add(0x10)),
        port: core::ptr::read_unaligned(p.add(0x20) as *const i32),
        map: read_fstring(p.add(0x28)),
        ops,
    }
}

type BrowseFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u8, *mut c_void) -> usize;
type UrlCtorFn = unsafe extern "system" fn(*mut u8, *const u8, *const u16, u32) -> *mut u8;
type UrlDtorFn = unsafe extern "system" fn(*mut u8);

static BASE: AtomicUsize = AtomicUsize::new(0);
static CTOR: AtomicUsize = AtomicUsize::new(0);
static DTOR: AtomicUsize = AtomicUsize::new(0);
static ORIG_BROWSE: AtomicUsize = AtomicUsize::new(0);
static BROWSE_LOGS: AtomicUsize = AtomicUsize::new(0);

#[repr(C, align(16))]
struct UrlBuf([u8; 0x100]);

unsafe extern "system" fn browse_hook(this: *mut c_void, ctx: *mut c_void, url: *mut u8, err: *mut c_void) -> usize {
    let orig: BrowseFn = core::mem::transmute(ORIG_BROWSE.load(Ordering::Acquire));
    let base = BASE.load(Ordering::Acquire);
    let (ctor_at, dtor_at) = (CTOR.load(Ordering::Acquire), DTOR.load(Ordering::Acquire));
    if url.is_null() || base == 0 || ctor_at == 0 || dtor_at == 0 {
        return orig(this, ctx, url, err);
    }
    let view = read_url(url);
    let sent = sentinel();
    match rewrite_text(&view, &sent, &map(), &gamemode(), &playlist()) {
        Some(text) => {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let ctor: UrlCtorFn = core::mem::transmute(ctor_at);
            let dtor: UrlDtorFn = core::mem::transmute(dtor_at);
            let mut tmp = UrlBuf([0u8; 0x100]);
            ctor(tmp.0.as_mut_ptr(), url, wide.as_ptr(), TRAVEL_PARTIAL);
            let new_view = read_url(tmp.0.as_ptr());
            dtor(url);
            core::ptr::copy_nonoverlapping(tmp.0.as_ptr(), url, FURL_SIZE);
            crate::record_raw("fn-match", "browse", &format!("rewrite {} -> {}", view.describe(), new_view.describe()));
            orig(this, ctx, url, err)
        }
        None => {
            if BROWSE_LOGS.fetch_add(1, Ordering::Relaxed) < 64 {
                crate::record_raw("fn-match", "browse", &format!("pass {} (sentinel {}:{})", view.describe(), sent.0, sent.1));
            }
            orig(this, ctx, url, err)
        }
    }
}

unsafe fn bytes_at(p: *const u8, want: &[u8]) -> bool {
    core::slice::from_raw_parts(p, want.len()) == want
}

pub fn call_target(site: usize, call: &[u8]) -> Option<usize> {
    if call.len() < 5 || call[0] != 0xE8 {
        return None;
    }
    let rel = i32::from_le_bytes(call[1..5].try_into().ok()?) as isize;
    Some((site as isize + 5 + rel) as usize)
}

unsafe fn ctor_call_target_ok(base: usize, l: &Layout) -> bool {
    let site = base + l.ctor_call_site;
    call_target(site, core::slice::from_raw_parts(site as *const u8, 5)) == Some(base + l.ctor)
}

unsafe fn install_browse(base: usize, l: &Layout) -> Result<usize, String> {
    let f = (base + l.browse) as *const u8;
    if !bytes_at(f, &l.browse_prologue) {
        return Err(format!("Browse prologue mismatch {}", hex(core::slice::from_raw_parts(f, l.browse_prologue.len()))));
    }
    if !bytes_at((base + l.dtor) as *const u8, &l.dtor_prologue) {
        return Err("FURL destructor bytes mismatch".into());
    }
    if !ctor_call_target_ok(base, l) {
        return Err("FURL constructor call site mismatch".into());
    }
    if !bytes_at((base + l.vcall_site) as *const u8, &l.vcall) {
        return Err("Browse virtual call site mismatch".into());
    }
    CTOR.store(base + l.ctor, Ordering::Release);
    DTOR.store(base + l.dtor, Ordering::Release);
    BASE.store(base, Ordering::Release);
    ORIG_BROWSE.store(base + l.browse, Ordering::Release);
    let hook = browse_hook as *const () as usize;
    let mut n = 0;
    for &rva in &l.slots {
        let slot = (base + rva) as *mut usize;
        let cur = core::ptr::read_volatile(slot);
        if cur == hook {
            n += 1;
            continue;
        }
        if cur != base + l.browse {
            return Err(format!("vtable slot {rva:#x} holds {cur:#x}, expected {:#x}", base + l.browse));
        }
        if !protected_write(slot as *mut u8, &hook.to_le_bytes()) {
            return Err(format!("VirtualProtect failed on vtable slot {rva:#x}"));
        }
        n += 1;
    }
    Ok(n)
}

unsafe fn title_base() -> Option<usize> {
    for m in super::patch::loaded_modules() {
        let path = super::patch::module_path(m);
        let name = path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase();
        if name == TITLE_EXE {
            return Some(m as usize);
        }
    }
    None
}

pub fn apply() {
    let m = mode();
    if m == Mode::Off {
        return;
    }
    let base = match unsafe { title_base() } {
        Some(b) => b,
        None => return,
    };
    let from_symbols = crate::title_symbols::for_this_title().and_then(|t| {
        let (size, data) = unsafe { image_sections(base as *const u8) };
        let img = unsafe { core::slice::from_raw_parts(base as *const u8, size) };
        layout_from_image(img, base, &data, t).map(|l| (l, t.source.clone()))
    });
    let chosen = match from_symbols {
        Ok(v) => Ok(v),
        Err(se) => crate::title_offsets::load_for(TITLE_EXE)
            .and_then(|o| {
                let stamp = unsafe { crate::title_offsets::image_timestamp(base as *const u8) };
                if o.pe_timestamp.is_none() || !o.matches_image(stamp) {
                    return Err(format!("{} does not match image timestamp {stamp:#x}", o.source));
                }
                layout(&o).map(|l| (l, o.source.clone()))
            })
            .map_err(|je| format!("symbols: {se}; offsets file: {je}")),
    };
    let l = match chosen {
        Ok((l, source)) => {
            crate::record_raw("fn-match", "layout", &format!("from {source}: {} Browse vtable slots", l.slots.len()));
            l
        }
        Err(e) => {
            crate::record_raw("fn-match", "inert", &format!("{e} - StartGameInstance patch and Browse hook NOT installed"));
            return;
        }
    };
    let site = (base + l.site) as *mut u8;
    match unsafe { patch_cmdline_at(site, &l.original, &l.patched) } {
        Ok(SiteState::AlreadyPatched) => crate::record_raw("fn-match", "patched", &format!("StartGameInstance cmdline at {:#x} (already in place)", site as usize)),
        Ok(_) => crate::record_raw("fn-match", "patched", &format!("StartGameInstance cmdline at {:#x}", site as usize)),
        Err(e) => crate::record_raw("fn-match", "StartGameInstance", &format!("bytes mismatch {e}")),
    }
    let (sh, sp) = sentinel();
    let raw = std::env::var("TOMBOX_FN_MATCH").unwrap_or_else(|_| "<unset>".into());
    crate::record_raw("fn-match", "mode", &format!("{m:?} (TOMBOX_FN_MATCH={raw}) map={} game={} playlist={} sentinel={sh}:{sp}", map(), gamemode(), playlist()));
    if m == Mode::Play {
        match unsafe { install_browse(base, &l) } {
            Ok(n) => crate::record_raw("fn-match", "browse", &format!("hooked UEngine::Browse at {:#x} in {n} vtables", base + l.browse)),
            Err(e) => crate::record_raw("fn-match", "browse", &format!("hook NOT installed: {e}")),
        }
    }
}

pub const SYMBOLS: [&str; 4] = ["UGameInstance::StartGameInstance", "UEngine::Browse", "FURL::FURL", "FURL::~FURL"];
pub const UNIQUE_SYMBOLS: [&str; 3] = ["UGameInstance::StartGameInstance", "UEngine::Browse", "FURL::~FURL"];

pub fn mov_from_rax(lea: &[u8]) -> Option<[u8; 7]> {
    if lea.len() < 7 || lea[0] & 0xFB != 0x48 || lea[1] != 0x8D || lea[2] & 0xC7 != 0x05 {
        return None;
    }
    let dst = ((lea[0] >> 2) & 1) << 3 | (lea[2] >> 3) & 7;
    Some([0x48 | (dst >> 3), 0x89, 0xC0 | (dst & 7), 0x90, 0x90, 0x90, 0x90])
}

fn unique(hits: Vec<usize>, what: &str) -> Result<usize, String> {
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!("no {what} found")),
        many => Err(format!("{} candidates for {what}", many.len())),
    }
}

pub fn layout_from_image(img: &[u8], base: usize, data: &[(usize, usize)], t: &crate::title_symbols::Table) -> Result<Layout, String> {
    let s = t.require(&UNIQUE_SYMBOLS)?;
    let (start, browse, dtor) = (s[0], s[1], s[2]);
    let ctors: Vec<usize> = t.all("FURL::FURL").iter().map(|c| c.rva).collect();
    if ctors.is_empty() {
        return Err(format!("{} has no FURL::FURL", t.source));
    }
    let body = img.get(start.rva..start.rva + start.size).ok_or("StartGameInstance lies outside the image")?;
    let at = |i: usize| start.rva + i;
    let site = unique(
        (5..body.len().saturating_sub(7)).filter(|&i| body[i - 5] == 0xE8 && mov_from_rax(&body[i..i + 7]).is_some()).map(at).collect(),
        "StartGameInstance command-line load after a call",
    )?;
    let sets_travel_type = |i: usize| body[i.saturating_sub(24)..i].windows(2).any(|w| w == [0x41, 0xB9]);
    let ctor_call_site = unique(
        (0..body.len().saturating_sub(5))
            .filter(|&i| call_target(at(i), &body[i..i + 5]).map(|c| ctors.contains(&c)).unwrap_or(false) && sets_travel_type(i))
            .map(at)
            .collect(),
        "FURL(Base, URL, TravelType) constructor call in StartGameInstance",
    )?;
    let ctor = call_target(ctor_call_site, &img[ctor_call_site..ctor_call_site + 5]).unwrap();
    let vcall_site = unique((0..body.len().saturating_sub(7)).filter(|&i| body[i..i + 3] == [0x41, 0xFF, 0x92]).map(at).collect(), "Browse virtual call in StartGameInstance")?;
    let original: [u8; 7] = img[site..site + 7].try_into().unwrap();
    let patched = mov_from_rax(&original).unwrap();
    let want = ((base + browse.rva) as u64).to_le_bytes();
    let mut slots = Vec::new();
    for &(rva, len) in data {
        let Some(sec) = img.get(rva..rva + len) else { continue };
        for (k, q) in sec.chunks_exact(8).enumerate() {
            if q == want {
                slots.push(rva + k * 8);
            }
        }
    }
    if slots.is_empty() || slots.len() > 16 {
        return Err(format!("{} vtable slots hold UEngine::Browse", slots.len()));
    }
    Ok(Layout {
        site,
        browse: browse.rva,
        slots,
        ctor,
        ctor_call_site,
        dtor: dtor.rva,
        vcall_site,
        original,
        patched,
        browse_prologue: Vec::new(),
        dtor_prologue: Vec::new(),
        vcall: img[vcall_site..vcall_site + 7].to_vec(),
    })
}

pub const SCN_INITIALIZED_DATA: u32 = 1 << 6;
pub const SCN_DISCARDABLE: u32 = 1 << 25;
pub const SCN_EXECUTE: u32 = 1 << 29;

pub fn is_data_section(ch: u32) -> bool {
    ch & SCN_EXECUTE == 0 && ch & SCN_DISCARDABLE == 0 && ch & SCN_INITIALIZED_DATA != 0
}

pub unsafe fn image_sections(base: *const u8) -> (usize, Vec<(usize, usize)>) {
    let pe = core::ptr::read_unaligned(base.add(0x3c) as *const u32) as usize;
    let n = core::ptr::read_unaligned(base.add(pe + 6) as *const u16) as usize;
    let opt = core::ptr::read_unaligned(base.add(pe + 20) as *const u16) as usize;
    let size = core::ptr::read_unaligned(base.add(pe + 24 + 56) as *const u32) as usize;
    let mut data = Vec::new();
    for i in 0..n {
        let s = base.add(pe + 24 + opt + i * 40);
        let vsz = core::ptr::read_unaligned(s.add(8) as *const u32) as usize;
        let va = core::ptr::read_unaligned(s.add(12) as *const u32) as usize;
        let ch = core::ptr::read_unaligned(s.add(36) as *const u32);
        if is_data_section(ch) {
            data.push((va, vsz & !7));
        }
    }
    (size, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: [u8; 7] = [0x4C, 0x8D, 0x3D, 0x11, 0x22, 0x33, 0x04];
    const PATCHED: [u8; 7] = [0x49, 0x89, 0xC7, 0x90, 0x90, 0x90, 0x90];

    fn site_buf(call: u8, bytes: &[u8; 7]) -> Vec<u8> {
        let mut v = vec![0u8; 32];
        v[8] = call;
        v[13..20].copy_from_slice(bytes);
        v
    }

    #[test]
    fn patch_only_on_exact_bytes() {
        let mut good = site_buf(0xE8, &ORIGINAL);
        let r = unsafe { patch_cmdline_at(good.as_mut_ptr().add(13), &ORIGINAL, &PATCHED) };
        assert_eq!(r, Ok(SiteState::Original));
        assert_eq!(&good[13..20], &PATCHED);
        let again = unsafe { patch_cmdline_at(good.as_mut_ptr().add(13), &ORIGINAL, &PATCHED) };
        assert_eq!(again, Ok(SiteState::AlreadyPatched));
        assert_eq!(&good[13..20], &PATCHED);

        let wrong = [0x4C, 0x8D, 0x3D, 0x00, 0x00, 0x00, 0x00];
        let mut bad = site_buf(0xE8, &wrong);
        let r = unsafe { patch_cmdline_at(bad.as_mut_ptr().add(13), &ORIGINAL, &PATCHED) };
        assert!(r.is_err());
        assert_eq!(&bad[13..20], &wrong);

        let mut nocall = site_buf(0x90, &ORIGINAL);
        let r = unsafe { patch_cmdline_at(nocall.as_mut_ptr().add(13), &ORIGINAL, &PATCHED) };
        assert!(r.is_err());
        assert_eq!(&nocall[13..20], &ORIGINAL);
    }

    fn fixture() -> String {
        let vals: Vec<String> = VALUE_KEYS.iter().enumerate().map(|(i, k)| format!("\"{k}\":{}", 0x1000 + i * 0x10)).collect();
        let bytes = ["4C8D3D11223304", "4989C790909090", "4055", "4053", "41FF92"];
        let b: Vec<String> = BYTE_KEYS.iter().zip(bytes).map(|(k, v)| format!("\"{k}\":\"{v}\"")).collect();
        format!("{{\"exe\":\"{TITLE_EXE}\",\"values\":{{{}}},\"bytes\":{{{}}}}}", vals.join(","), b.join(","))
    }

    #[test]
    fn the_layout_comes_from_the_offsets_file() {
        let o = crate::title_offsets::parse(&fixture(), "t").unwrap();
        let l = layout(&o).unwrap();
        assert_eq!((l.site, l.slots.clone(), l.vcall_site), (0x1000, vec![0x1020, 0x1030, 0x1040], 0x1080));
        assert_eq!((l.original, l.patched), (ORIGINAL, PATCHED));
        assert_eq!(l.vcall, vec![0x41, 0xFF, 0x92]);
        let short = fixture().replace("4989C790909090", "4989C7");
        assert!(layout(&crate::title_offsets::parse(&short, "t").unwrap()).unwrap_err().contains("7 bytes"));
        let missing = fixture().replace("fn.furl_dtor\"", "fn.other\"");
        assert!(layout(&crate::title_offsets::parse(&missing, "t").unwrap()).unwrap_err().contains("fn.furl_dtor"));
    }

    #[test]
    fn call_targets_decode_rel32() {
        assert_eq!(call_target(0x1000, &[0xE8, 0x10, 0x00, 0x00, 0x00]), Some(0x1015));
        assert_eq!(call_target(0x1000, &[0xE8, 0xFB, 0xFF, 0xFF, 0xFF]), Some(0x1000));
        assert_eq!(call_target(0x1000, &[0xE9, 0, 0, 0, 0]), None);
    }

    fn url(host: &str, port: i32, map: &str) -> UrlView {
        UrlView { protocol: "unreal".into(), host: host.into(), port, map: map.into(), ops: vec!["Name=XboxEmulator".into()] }
    }

    #[test]
    fn furl_rewrite_only_for_sentinel() {
        let s = parse_endpoint("127.0.0.1:7777").unwrap();
        let hit = rewrite_text(&url("127.0.0.1", 7777, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "Playlist_DefaultSquad");
        assert_eq!(
            hit.as_deref(),
            Some("/Game/Athena/Apollo/Maps/Apollo_Terrain?game=/Game/Athena/Athena_GameMode.Athena_GameMode_C?Playlist=Playlist_DefaultSquad")
        );
        assert!(rewrite_text(&url("127.0.0.1", 7778, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "p").is_none());
        assert!(rewrite_text(&url("10.0.0.5", 7777, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "p").is_none());
        assert!(rewrite_text(&url("", 7777, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "p").is_none());
        let no_pl = rewrite_text(&url("127.0.0.1", 7777, ""), &s, DEFAULT_MAP, FALLBACK_GAMEMODE, "");
        assert_eq!(no_pl.as_deref(), Some("/Game/Athena/Apollo/Maps/Apollo_Terrain?game=/Script/FortniteGame.FortGameModeAthena"));
    }

    #[test]
    fn endpoint_and_mode_parsing() {
        assert_eq!(parse_endpoint("127.0.0.1:7777"), Some(("127.0.0.1".into(), 7777)));
        assert_eq!(parse_endpoint("[::1]:15000"), Some(("::1".into(), 15000)));
        assert_eq!(parse_endpoint("nohost"), None);
        assert_eq!(parse_endpoint(":7777"), None);
        assert_eq!(parse_mode(Some("Play")), Mode::Play);
        assert_eq!(parse_mode(Some("replay")), Mode::Replay);
        assert_eq!(parse_mode(Some("MAP")), Mode::Map);
        assert_eq!(parse_mode(Some("off")), Mode::Off);
        assert_eq!(parse_mode(Some(" OFF ")), Mode::Off);
        assert_eq!(parse_mode(Some("0")), Mode::Off);
        assert_eq!(parse_mode(None), Mode::Play);
        assert_eq!(parse_mode(Some("")), Mode::Play);
        assert_eq!(DEFAULT_MODE, Mode::Play);
    }

    struct Pe {
        f: std::fs::File,
        base: u64,
        stamp: u32,
        secs: Vec<(u32, u32, u32)>,
        data: Vec<(usize, usize)>,
    }

    impl Pe {
        fn open(p: &str) -> Option<Pe> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(p).ok()?;
            let mut h = vec![0u8; 0x1000];
            f.read_exact(&mut h).ok()?;
            let pe = u32::from_le_bytes(h[0x3c..0x40].try_into().ok()?) as usize;
            let stamp = u32::from_le_bytes(h[pe + 8..pe + 12].try_into().ok()?);
            let nsec = u16::from_le_bytes(h[pe + 6..pe + 8].try_into().ok()?) as usize;
            let opt = u16::from_le_bytes(h[pe + 20..pe + 22].try_into().ok()?) as usize;
            let base = u64::from_le_bytes(h[pe + 24 + 24..pe + 24 + 32].try_into().ok()?);
            let st = pe + 24 + opt;
            let mut secs = Vec::new();
            let mut data = Vec::new();
            for i in 0..nsec {
                let s = &h[st + i * 40..st + i * 40 + 40];
                let vsz = u32::from_le_bytes(s[8..12].try_into().ok()?);
                let va = u32::from_le_bytes(s[12..16].try_into().ok()?);
                let raw = u32::from_le_bytes(s[20..24].try_into().ok()?);
                let ch = u32::from_le_bytes(s[36..40].try_into().ok()?);
                if is_data_section(ch) {
                    data.push((va as usize, vsz as usize & !7));
                }
                secs.push((va, vsz, raw));
            }
            f.seek(SeekFrom::Start(0)).ok()?;
            Some(Pe { f, base, stamp, secs, data })
        }
        fn read(&mut self, rva: usize, n: usize) -> Vec<u8> {
            use std::io::{Read, Seek, SeekFrom};
            let (va, _, raw) = *self.secs.iter().find(|(va, vsz, _)| rva >= *va as usize && rva < (*va + *vsz) as usize).unwrap();
            let mut b = vec![0u8; n];
            self.f.seek(SeekFrom::Start((rva - va as usize + raw as usize) as u64)).unwrap();
            self.f.read_exact(&mut b).unwrap();
            b
        }
    }

    #[test]
    fn local_offsets_match_the_dump() {
        let Ok(o) = crate::title_offsets::load_for(TITLE_EXE) else {
            eprintln!("no local offsets for {TITLE_EXE}; dump check not run");
            return;
        };
        let Ok(path) = std::env::var("TOMBOX_FN_DUMP_EXE") else {
            eprintln!("TOMBOX_FN_DUMP_EXE unset; dump check not run");
            return;
        };
        let Some(mut pe) = Pe::open(&path) else {
            eprintln!("dump exe not present at {path}; dump check not run");
            return;
        };
        let l = layout(&o).unwrap();
        assert!(o.matches_image(pe.stamp), "offsets file is for another build");
        let call = pe.read(l.site - 5, 12);
        assert_eq!(classify_site(call[0], &call[5..12].try_into().unwrap(), &l.original, &l.patched), SiteState::Original);
        let getter = call_target(l.site - 5, &call[..5]).unwrap();
        let g = pe.read(getter, 0x1d);
        assert_eq!(&g[0x11..0x14], &[0x48, 0x8D, 0x05]);
        assert_eq!(g[0x1c], 0xC3);
        assert_eq!(pe.read(l.browse, l.browse_prologue.len()), l.browse_prologue);
        assert_eq!(pe.read(l.dtor, l.dtor_prologue.len()), l.dtor_prologue);
        let c = pe.read(l.ctor_call_site, 5);
        assert_eq!(call_target(l.ctor_call_site, &c), Some(l.ctor));
        assert_eq!(pe.read(l.vcall_site, l.vcall.len()), l.vcall);
        for &slot in &l.slots {
            let v = u64::from_le_bytes(pe.read(slot, 8).try_into().unwrap());
            assert_eq!(v, pe.base + l.browse as u64, "slot {slot:#x}");
        }
    }

    #[test]
    fn url_describe_matches_ue_shape() {
        let u = url("127.0.0.1", 7777, "/Game/Maps/Frontend");
        assert_eq!(u.describe(), "127.0.0.1:7777/Game/Maps/Frontend?Name=XboxEmulator");
    }

    #[test]
    fn mov_from_rax_follows_the_lea_destination() {
        assert_eq!(mov_from_rax(&[0x4C, 0x8D, 0x3D, 1, 2, 3, 4]), Some([0x49, 0x89, 0xC7, 0x90, 0x90, 0x90, 0x90]));
        assert_eq!(mov_from_rax(&[0x48, 0x8D, 0x0D, 1, 2, 3, 4]), Some([0x48, 0x89, 0xC1, 0x90, 0x90, 0x90, 0x90]));
        assert_eq!(mov_from_rax(&[0x4C, 0x8D, 0x7D, 1, 2, 3, 4]), None);
        assert_eq!(mov_from_rax(&[0x4C, 0x8B, 0x3D, 1, 2, 3, 4]), None);
    }

    fn synthetic_image(base: usize) -> (Vec<u8>, crate::title_symbols::Table) {
        let mut img = vec![0u8; 0x4000];
        img[0x110] = 0xE8;
        img[0x115..0x11c].copy_from_slice(&[0x4C, 0x8D, 0x3D, 0x10, 0x20, 0x30, 0x00]);
        img[0x124..0x12a].copy_from_slice(&[0x41, 0xB9, 0x01, 0x00, 0x00, 0x00]);
        img[0x130] = 0xE8;
        img[0x131..0x135].copy_from_slice(&((0x800i32 - 0x135) as i32).to_le_bytes());
        img[0x170] = 0xE8;
        img[0x171..0x175].copy_from_slice(&((0xc00i32 - 0x175) as i32).to_le_bytes());
        img[0x150..0x157].copy_from_slice(&[0x41, 0xFF, 0x92, 0x40, 0x04, 0x00, 0x00]);
        img[0x2040..0x2048].copy_from_slice(&((base + 0x900) as u64).to_le_bytes());
        img[0x2088..0x2090].copy_from_slice(&((base + 0x900) as u64).to_le_bytes());
        let f = crate::title_symbols::build_for_test(TITLE_EXE, &[("UGameInstance::StartGameInstance", 0x100, 0x80), ("UEngine::Browse", 0x900, 0x40), ("FURL::FURL", 0x800, 0x40), ("FURL::FURL", 0xc00, 0x40), ("FURL::~FURL", 0xa00, 0x20)]);
        let t = crate::title_symbols::parse_bytes(&f, &SYMBOLS, "t").unwrap();
        (img, t)
    }

    #[test]
    fn symbols_and_the_image_give_the_whole_layout() {
        let base = 5usize << 30;
        let (img, t) = synthetic_image(base);
        let l = layout_from_image(&img, base, &[(0x2000, 0x100)], &t).unwrap();
        assert_eq!((l.site, l.ctor_call_site, l.vcall_site, l.browse, l.ctor, l.dtor), (0x115, 0x130, 0x150, 0x900, 0x800, 0xa00));
        assert_eq!(l.slots, vec![0x2040, 0x2088]);
        assert_eq!(l.original, [0x4C, 0x8D, 0x3D, 0x10, 0x20, 0x30, 0x00]);
        assert_eq!(l.patched, [0x49, 0x89, 0xC7, 0x90, 0x90, 0x90, 0x90]);
        assert!(layout_from_image(&img, base, &[(0x3000, 0x100)], &t).unwrap_err().contains("0 vtable slots"));
        let mut two = img.clone();
        two[0x160] = 0xE8;
        two[0x165..0x16c].copy_from_slice(&[0x48, 0x8D, 0x05, 0, 0, 0, 0]);
        assert!(layout_from_image(&two, base, &[(0x2000, 0x100)], &t).unwrap_err().contains("2 candidates"));
        let missing = crate::title_symbols::parse_bytes(&crate::title_symbols::build_for_test(TITLE_EXE, &[("UEngine::Browse", 0x900, 0x40)]), &SYMBOLS, "t").unwrap();
        assert!(layout_from_image(&img, base, &[(0x2000, 0x100)], &missing).unwrap_err().contains("StartGameInstance"));
    }

    #[test]
    fn dump_symbols_reproduce_the_offsets_file() {
        let Ok(path) = std::env::var("TOMBOX_FN_DUMP_EXE") else {
            eprintln!("TOMBOX_FN_DUMP_EXE unset; symbol check not run");
            return;
        };
        let Some(mut pe) = Pe::open(&path) else {
            eprintln!("dump exe not present at {path}; symbol check not run");
            return;
        };
        let t = match crate::title_symbols::load(std::path::Path::new(&path), &SYMBOLS) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e}; symbol check not run");
                return;
            }
        };
        let end = pe.secs.iter().map(|(va, vsz, _)| (*va + *vsz) as usize).max().unwrap();
        let mut img = vec![0u8; end];
        let secs = pe.secs.clone();
        for (va, vsz, _) in &secs {
            let b = pe.read(*va as usize, *vsz as usize);
            img[*va as usize..*va as usize + b.len()].copy_from_slice(&b);
        }
        let data: Vec<(usize, usize)> = pe.data.clone();
        let l = layout_from_image(&img, pe.base as usize, &data, &t).unwrap();
        eprintln!("symbol layout: site {:#x} slots {}", l.site, l.slots.len());
        if let Ok(o) = crate::title_offsets::load_for(TITLE_EXE) {
            let j = layout(&o).unwrap();
            assert_eq!((l.site, l.browse, l.ctor, l.ctor_call_site, l.dtor, l.vcall_site), (j.site, j.browse, j.ctor, j.ctor_call_site, j.dtor, j.vcall_site));
            assert_eq!((l.original, l.patched, &l.vcall), (j.original, j.patched, &j.vcall));
            let mut a = l.slots.clone();
            let mut b = j.slots.clone();
            a.sort();
            b.sort();
            assert_eq!(a, b);
        }
    }
}
