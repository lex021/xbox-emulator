use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::console::reset_security_watermark_in;
use super::store::{self, Store};
use super::tlg::{self, EventData};
use super::{hresult_from_win32, HResult};

pub const STATUS_ACCESS_DENIED: i32 = 0xC000_0022u32 as i32;
pub const STATUS_NOT_FOUND: i32 = 0xC000_0225u32 as i32;
pub const STATUS_ERA_NOT_IMPLEMENTED: i32 = 0x8043_0006u32 as i32;
pub const HR_FILE_NOT_FOUND: HResult = 0x8007_0002u32 as i32;
pub const HR_INSUFFICIENT_BUFFER: HResult = 0x8007_007Au32 as i32;

pub const ERA_FILTER_BOOT_OPTION_STATUS: i32 = STATUS_ERA_NOT_IMPLEMENTED;
pub const ERA_SECURE_BOOT_STATUS: i32 = STATUS_ERA_NOT_IMPLEMENTED;
pub const ERA_HSTI_STATUS: i32 = STATUS_NOT_FOUND;
pub const ERA_PRODUCT_TYPE: u8 = 1;
pub const ERA_KD_DEBUGGER_ENABLED: bool = false;

const _: () = assert!(ERA_SECURE_BOOT_STATUS < 0 && ERA_SECURE_BOOT_STATUS != STATUS_ACCESS_DENIED && ERA_PRODUCT_TYPE == 1);
const _: () = assert!(ERA_HSTI_STATUS == STATUS_NOT_FOUND && !ERA_KD_DEBUGGER_ENABLED);

pub const HASH_SIZE: usize = 0x20;
pub const DMCFG_HASH_RESULT: HResult = 1;
pub const BCD_OS_LOADER: u32 = 0x1020_0003;
pub const BCD_BOOT_MANAGER: u32 = 0x1020_0002;
pub const OS_LOADER_ELEMENTS: [u32; 4] = [0x2600_00a0, 0x1600_0010, 0x1600_0049, 0x2600_00f2];
pub const BOOT_MANAGER_ELEMENTS: [u32; 2] = [0x1600_0010, 0x1600_0049];

pub const LOG_IMAGE_VERSION: [u16; 4] = [10, 0, 19041, 3948];
pub const LOG_HEADER_RVA: u32 = 0x1f170;
pub const LOG_HEADER_LEN: usize = 0x252;
pub const FMT_BCD_OS_LOADER: u32 = 0x1eea0;
pub const FMT_BCD_BOOT_MANAGER: u32 = 0x1eee0;
pub const FMT_MANUFACTURING: u32 = 0x1f0f0;
pub const FMT_HSTI: u32 = 0x1f120;
pub const LOG_LINE_MAX: usize = 0x3ff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Era {
    pub filter_boot_option: i32,
    pub manufacturing: Result<bool, u32>,
}

impl Era {
    pub fn retail(s: &Store) -> Era {
        Era { filter_boot_option: ERA_FILTER_BOOT_OPTION_STATUS, manufacturing: Ok(s.manufacturing_mode()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecureBoot {
    pub hr: HResult,
    pub state: u32,
    pub failed: u32,
}

pub fn secure_boot_query(status: i32) -> SecureBoot {
    SecureBoot { hr: (status as u32 | 0x1000_0000) as i32, state: 0, failed: 1 }
}

pub fn secure_boot_cached() -> SecureBoot {
    static C: Mutex<Option<SecureBoot>> = Mutex::new(None);
    let mut g = C.lock().unwrap_or_else(|e| e.into_inner());
    *g.get_or_insert_with(|| secure_boot_query(ERA_SECURE_BOOT_STATUS))
}

pub fn hsti_check() -> (HResult, bool) {
    (0, false)
}

pub fn filter_hit(status: i32) -> bool {
    let st = status as u32 | 0x1000_0000;
    st == 0x1000_0000 || st == 0x9043_0006
}

pub fn console_log_image_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(s) = compat_common::title_os::title_system32() {
        v.push(s.join("wldp.dll"));
    }
    if let Ok(titles) = std::fs::read_dir(compat_common::title_os::DUMPS) {
        for t in titles.flatten().map(|e| e.path().join("extracted")) {
            v.push(t.join(compat_common::title_os::SYSTEM32).join("wldp.dll"));
            if let Ok(inner) = std::fs::read_dir(&t) {
                v.extend(inner.flatten().map(|e| e.path().join(compat_common::title_os::SYSTEM32).join("wldp.dll")));
            }
        }
    }
    v.into_iter().filter(|p| p.is_file()).collect()
}

pub struct Image {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

impl Image {
    pub fn read(path: &Path) -> Option<Image> {
        Some(Image { path: path.to_path_buf(), bytes: std::fs::read(path).ok()? })
    }

    pub fn rva_offset(&self, rva: u32) -> Option<usize> {
        let b = &self.bytes;
        let rd32 = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()));
        let rd16 = |o: usize| b.get(o..o + 2).map(|x| u16::from_le_bytes(x.try_into().unwrap()));
        let pe = rd32(0x3c)? as usize;
        let count = rd16(pe + 6)? as usize;
        let opt = rd16(pe + 20)? as usize;
        let first = pe + 24 + opt;
        (0..count).find_map(|i| {
            let h = first + i * 40;
            let vsize = rd32(h + 8)?;
            let va = rd32(h + 12)?;
            let raw_size = rd32(h + 16)?;
            let raw = rd32(h + 20)?;
            let span = vsize.max(raw_size);
            (rva >= va && rva < va + span && rva - va < raw_size).then(|| (raw + rva - va) as usize)
        })
    }

    pub fn slice(&self, rva: u32, len: usize) -> Option<&[u8]> {
        let o = self.rva_offset(rva)?;
        self.bytes.get(o..o + len)
    }

    pub fn cstr(&self, rva: u32) -> Option<&[u8]> {
        let o = self.rva_offset(rva)?;
        let rest = self.bytes.get(o..)?;
        rest.iter().position(|&c| c == 0).map(|n| &rest[..n])
    }
}

pub fn log_image() -> Option<&'static Image> {
    static I: OnceLock<Option<Image>> = OnceLock::new();
    I.get_or_init(|| {
        console_log_image_candidates()
            .into_iter()
            .find(|p| compat_common::title_os::file_version(p) == Some(LOG_IMAGE_VERSION))
            .and_then(|p| Image::read(&p))
    })
    .as_ref()
}

pub fn vsnprintf(fmt: &[u8], args: &[u32], max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut args = args.iter();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        let zero = fmt.get(i) == Some(&b'0');
        if zero {
            i += 1;
        }
        let mut width = 0usize;
        while let Some(d) = fmt.get(i).filter(|d| d.is_ascii_digit()) {
            width = width * 10 + (d - b'0') as usize;
            i += 1;
        }
        while fmt.get(i) == Some(&b'l') {
            i += 1;
        }
        let conv = *fmt.get(i)?;
        i += 1;
        let v = *args.next()?;
        let digits = match conv {
            b'x' => format!("{v:x}"),
            b'X' => format!("{v:X}"),
            b'u' => format!("{v}"),
            b'd' | b'i' => format!("{}", v as i32),
            _ => return None,
        };
        let pad = width.saturating_sub(digits.len());
        out.extend(std::iter::repeat(if zero { b'0' } else { b' ' }).take(pad));
        out.extend_from_slice(digits.as_bytes());
    }
    (out.len() <= max).then_some(out)
}

pub struct Log<'a> {
    store: &'a Store,
    file: Option<std::fs::File>,
    pub lines: Vec<Vec<u8>>,
}

impl<'a> Log<'a> {
    pub fn new(store: &'a Store) -> Log<'a> {
        Log { store, file: None, lines: Vec::new() }
    }

    pub fn path(&self) -> PathBuf {
        self.store.title_relative_path(store::NON_PRODUCTION_ERRORS)
    }

    fn open(&mut self) -> HResult {
        if self.file.is_some() {
            return 0;
        }
        let Some(header) = log_image().and_then(|i| i.slice(LOG_HEADER_RVA, LOG_HEADER_LEN)) else {
            store::log("WldpCheckSecurityWatermarkState", "unsupported: console wldp 10.0.19041.3948 image not found; Non-Production Errors.txt not written");
            return HR_FILE_NOT_FOUND;
        };
        let mut f = match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(self.path()) {
            Ok(f) => f,
            Err(e) => return store::io_hresult(&e),
        };
        let hr = match f.write_all(header) {
            Ok(()) => 0,
            Err(e) => store::io_hresult(&e),
        };
        self.file = Some(f);
        hr
    }

    pub fn line(&mut self, fmt_rva: u32, args: &[u32]) {
        if self.file.is_none() && self.open() < 0 {
            return;
        }
        let Some(fmt) = log_image().and_then(|i| i.cstr(fmt_rva)) else {
            store::log("WldpCheckSecurityWatermarkState", &format!("console wldp 10.0.19041.3948 image not found; log line {fmt_rva:#x} unavailable"));
            return;
        };
        let Some(text) = vsnprintf(fmt, args, LOG_LINE_MAX) else { return };
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(&text);
        }
        self.lines.push(text);
    }

    pub fn close(&mut self) {
        self.file = None;
    }
}

pub fn filter_boot_options(log: &mut Log, status: i32) -> bool {
    let mut banned = false;
    for (elements, fmt) in [(&OS_LOADER_ELEMENTS[..], FMT_BCD_OS_LOADER), (&BOOT_MANAGER_ELEMENTS[..], FMT_BCD_BOOT_MANAGER)] {
        for &e in elements {
            if filter_hit(status) {
                banned = true;
                log.line(fmt, &[e]);
            }
        }
    }
    banned
}

pub fn retail_unlocked_event(hr: HResult) -> Option<u32> {
    let p = tlg::watermark();
    if !p.enabled(tlg::RETAIL_UNLOCKED_EVENT_DESC[1], tlg::desc_keyword(tlg::RETAIL_UNLOCKED_EVENT_DESC)) {
        return None;
    }
    Some(p.write(tlg::RETAIL_UNLOCKED_EVENT_DESC, tlg::RETAIL_UNLOCKED_EVENT_META, &[EventData::of(&hr)]))
}

pub fn hsti_failure_event(hr: HResult) -> Option<u32> {
    let p = tlg::watermark();
    if !p.enabled(tlg::HSTI_FAILURE_EVENT_DESC[1], tlg::desc_keyword(tlg::HSTI_FAILURE_EVENT_DESC)) {
        return None;
    }
    let result = 0u32;
    Some(p.write(tlg::HSTI_FAILURE_EVENT_DESC, tlg::HSTI_FAILURE_EVENT_META, &[EventData::of(&result), EventData::of(&hr)]))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub hr: HResult,
    pub out: u32,
    pub bcd_banned: bool,
    pub manufacturing: bool,
    pub secure_boot: bool,
    pub hsti_verified: bool,
    pub lines: Vec<Vec<u8>>,
}

pub fn check_security_watermark_state_in(s: &Store, era: &Era) -> Outcome {
    let mut log = Log::new(s);
    let mut secure_boot = true;
    let mut hsti_verified = true;
    let mut manufacturing = false;
    let bcd_banned = filter_boot_options(&mut log, era.filter_boot_option);
    if bcd_banned {
        let hr = reset_security_watermark_in(s);
        retail_unlocked_event(hr);
    }
    let hr: HResult = 'exit: {
        let mode = match era.manufacturing {
            Ok(m) => m,
            Err(e) => break 'exit hresult_from_win32(e as i32),
        };
        manufacturing = mode;
        if mode {
            reset_security_watermark_in(s);
            log.line(FMT_MANUFACTURING, &[]);
        }
        let hsti_hash = [0u8; HASH_SIZE];
        let sb_hash = [0u8; HASH_SIZE];
        if DMCFG_HASH_RESULT < 0 {
            break 'exit DMCFG_HASH_RESULT;
        }
        let mut buf = [0u8; HASH_SIZE];
        let hr = s.read_file(store::DPP_HSTI, &mut buf);
        if hr < 0 && hr != HR_FILE_NOT_FOUND {
            break 'exit hr;
        }
        if hr == HR_FILE_NOT_FOUND || buf != hsti_hash {
            let (hr, verified) = hsti_check();
            hsti_verified = verified;
            if hr < 0 {
                break 'exit hr;
            }
            if verified {
                s.write_file(store::DPP_HSTI, &hsti_hash);
            } else {
                log.line(FMT_HSTI, &[]);
                hsti_failure_event(hr);
                let hr = s.delete_file(store::DPP_HSTI);
                if hr < 0 {
                    break 'exit hr;
                }
            }
        }
        let mut buf = [0u8; HASH_SIZE];
        let hr = s.read_file(store::DPP_SB, &mut buf);
        if hr < 0 && hr != HR_FILE_NOT_FOUND {
            secure_boot = false;
            break 'exit hr;
        }
        if hr >= 0 && buf == sb_hash {
            break 'exit 0;
        }
        let sb = secure_boot_cached();
        secure_boot = false;
        sb.hr
    };
    let out = !(!manufacturing && !bcd_banned && secure_boot && hsti_verified) as u32;
    log.close();
    if out == 0 {
        let _ = std::fs::remove_file(log.path());
    }
    store::log("WldpCheckSecurityWatermarkState", &format!("bcd_banned={bcd_banned} manufacturing={manufacturing} hsti_verified={hsti_verified} secure_boot={secure_boot} out={out} hr={hr:#x}"));
    Outcome { hr, out, bcd_banned, manufacturing, secure_boot, hsti_verified, lines: log.lines }
}

#[no_mangle]
pub unsafe extern "system" fn WldpCheckSecurityWatermarkState(out: *mut u32) -> HResult {
    *out = 0;
    let s = Store::open();
    let o = check_security_watermark_state_in(&s, &Era::retail(&s));
    *out = o.out;
    o.hr
}
