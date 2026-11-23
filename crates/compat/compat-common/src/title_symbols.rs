use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};

pub const VAR: &str = "TOMBOX_TITLE_SYMBOLS";
const MAX_RAW: usize = 1 << 30;
const CHUNK: usize = 1 << 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sym {
    pub rva: usize,
    pub size: usize,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Table {
    pub source: String,
    pub image: String,
    pub total: usize,
    found: HashMap<String, Vec<Sym>>,
}

impl Table {
    pub fn get(&self, name: &str) -> Option<Sym> {
        match self.all(name) {
            [one] => Some(*one),
            _ => None,
        }
    }

    pub fn all(&self, name: &str) -> &[Sym] {
        self.found.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn unique_found(&self) -> usize {
        self.found.values().filter(|v| v.len() == 1).count()
    }

    pub fn require(&self, names: &[&str]) -> Result<Vec<Sym>, String> {
        let missing: Vec<&str> = names.iter().copied().filter(|n| self.get(n).is_none()).collect();
        if missing.is_empty() {
            Ok(names.iter().map(|n| self.get(n).unwrap()).collect())
        } else {
            Err(format!("{} lacks a unique symbol for {}", self.source, missing.join(", ")))
        }
    }
}

struct Z<R: Read> {
    src: R,
    inbuf: Vec<u8>,
    in_pos: usize,
    in_len: usize,
    src_done: bool,
    state: Box<InflateState>,
    out: Vec<u8>,
    out_pos: usize,
    out_len: usize,
    ended: bool,
    total: usize,
}

impl<R: Read> Z<R> {
    fn new(src: R) -> Self {
        Z { src, inbuf: vec![0; CHUNK], in_pos: 0, in_len: 0, src_done: false, state: InflateState::new_boxed(DataFormat::Zlib), out: vec![0; CHUNK], out_pos: 0, out_len: 0, ended: false, total: 0 }
    }

    fn fill(&mut self) -> Result<bool, String> {
        while self.out_pos == self.out_len {
            if self.ended {
                return Ok(false);
            }
            if self.in_pos == self.in_len && !self.src_done {
                self.in_len = self.src.read(&mut self.inbuf).map_err(|e| e.to_string())?;
                self.in_pos = 0;
                self.src_done = self.in_len == 0;
            }
            let r = inflate(&mut self.state, &self.inbuf[self.in_pos..self.in_len], &mut self.out, MZFlush::None);
            self.in_pos += r.bytes_consumed;
            self.out_pos = 0;
            self.out_len = r.bytes_written;
            self.total += r.bytes_written;
            match r.status {
                Ok(MZStatus::StreamEnd) => self.ended = true,
                Ok(_) => {}
                Err(e) => return Err(format!("inflate failed: {e:?}")),
            }
            if r.bytes_consumed == 0 && r.bytes_written == 0 && self.src_done && !self.ended {
                return Err("compressed data ends early".into());
            }
        }
        Ok(true)
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), String> {
        let mut at = 0;
        while at < buf.len() {
            if !self.fill()? {
                return Err(format!("unpacked data ends early at {:#x}", self.total));
            }
            let n = (self.out_len - self.out_pos).min(buf.len() - at);
            buf[at..at + n].copy_from_slice(&self.out[self.out_pos..self.out_pos + n]);
            self.out_pos += n;
            at += n;
        }
        Ok(())
    }

    fn skip(&mut self, mut n: usize) -> Result<(), String> {
        while n > 0 {
            if !self.fill()? {
                return Err(format!("unpacked data ends early at {:#x}", self.total));
            }
            let k = (self.out_len - self.out_pos).min(n);
            self.out_pos += k;
            n -= k;
        }
        Ok(())
    }

    fn u32(&mut self) -> Result<u32, String> {
        let mut b = [0u8; 4];
        self.read(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn at_end(&mut self) -> Result<bool, String> {
        Ok(!self.fill()?)
    }
}

fn header(src: &mut impl Read, len: u64) -> Result<usize, String> {
    let mut h = [0u8; 8];
    src.read_exact(&mut h).map_err(|e| format!("header: {e}"))?;
    let raw = u32::from_le_bytes(h[0..4].try_into().unwrap()) as usize;
    let packed = u32::from_le_bytes(h[4..8].try_into().unwrap()) as u64;
    if packed + 8 != len {
        return Err(format!("header says {packed} packed bytes, file has {}", len.saturating_sub(8)));
    }
    if raw > MAX_RAW {
        return Err(format!("unpacked size {raw} is too large"));
    }
    Ok(raw)
}

fn prologue<R: Read>(z: &mut Z<R>) -> Result<(String, usize), String> {
    let version = z.u32()?;
    if version != 1 {
        return Err(format!("unknown version {version}"));
    }
    let name_len = z.u32()? as usize;
    if name_len > 4096 {
        return Err(format!("image name length {name_len}"));
    }
    let mut name = vec![0u8; name_len];
    z.read(&mut name)?;
    z.skip(8)?;
    let image = String::from_utf8_lossy(name.split(|&c| c == 0).next().unwrap_or(&[])).to_string();
    Ok((image, z.u32()? as usize))
}

pub fn parse_with<R: Read>(open: impl Fn() -> Result<(R, u64), String>, wanted: &[&str], source: &str) -> Result<Table, String> {
    let err = |e: String| format!("{source}: {e}");
    let (mut f, len) = open().map_err(err)?;
    let raw = header(&mut f, len).map_err(err)?;
    let mut z = Z::new(f);
    let (image, count) = prologue(&mut z).map_err(err)?;
    z.skip(count.checked_mul(12).ok_or_else(|| err("record count overflows".into()))?).map_err(err)?;
    if z.u32().map_err(err)? as usize != count {
        return Err(err("name count does not match record count".into()));
    }
    let want: HashMap<&[u8], &str> = wanted.iter().map(|w| (w.as_bytes(), *w)).collect();
    let longest = wanted.iter().map(|w| w.len() + 1).max().unwrap_or(0);
    let mut hits: Vec<(usize, &str)> = Vec::new();
    let mut buf = vec![0u8; longest];
    for i in 0..count {
        let n = z.u32().map_err(err)? as usize;
        if n == 0 || n > 1 << 20 {
            return Err(err(format!("name {i} has length {n}")));
        }
        if n > longest {
            z.skip(n).map_err(err)?;
            continue;
        }
        z.read(&mut buf[..n]).map_err(err)?;
        let s = &buf[..n];
        let s = s.strip_suffix(&[0]).unwrap_or(s);
        if let Some(w) = want.get(s) {
            hits.push((i, w));
        }
    }
    if !z.at_end().map_err(err)? {
        return Err(err("trailing bytes after the name table".into()));
    }
    if z.total != raw {
        return Err(err(format!("unpacked {} bytes, header says {raw}", z.total)));
    }
    let mut found: HashMap<String, Vec<Sym>> = HashMap::new();
    if !hits.is_empty() {
        hits.sort();
        let (mut f, len) = open().map_err(err)?;
        header(&mut f, len).map_err(err)?;
        let mut z = Z::new(f);
        prologue(&mut z).map_err(err)?;
        let mut next = 0usize;
        for &(i, w) in &hits {
            z.skip((i - next) * 12).map_err(err)?;
            let rva = z.u32().map_err(err)? as usize;
            let size = z.u32().map_err(err)? as usize;
            z.skip(4).map_err(err)?;
            next = i + 1;
            found.entry(w.to_string()).or_default().push(Sym { rva, size });
        }
    }
    Ok(Table { source: source.to_string(), image, total: count, found })
}

pub fn parse_bytes(file: &[u8], wanted: &[&str], source: &str) -> Result<Table, String> {
    parse_with(|| Ok((std::io::Cursor::new(file), file.len() as u64)), wanted, source)
}

pub fn candidates(exe: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var(VAR) {
        let p = p.trim();
        if p == "0" {
            return out;
        }
        if !p.is_empty() {
            out.push(PathBuf::from(p));
            return out;
        }
    }
    let Some(stem) = exe.file_stem().map(|s| s.to_string_lossy().to_string()) else { return out };
    let file = format!("{stem}-Symbols.bin");
    let mut dir = exe.parent();
    for depth in 0..6 {
        let Some(d) = dir else { break };
        if depth == 0 {
            out.push(d.join(&file));
        }
        out.push(d.join("Symbols").join(&file));
        dir = d.parent();
    }
    out
}

pub fn locate(exe: &Path) -> Option<PathBuf> {
    candidates(exe).into_iter().find(|p| p.is_file())
}

pub fn load(exe: &Path, wanted: &[&str]) -> Result<Table, String> {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let path = locate(exe).ok_or_else(|| format!("no {stem}-Symbols.bin next to the title"))?;
    let source = path.display().to_string();
    let open = || {
        let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let len = f.metadata().map_err(|e| e.to_string())?.len();
        Ok((std::io::BufReader::with_capacity(CHUNK, f), len))
    };
    let t = parse_with(open, wanted, &source)?;
    if !t.image.eq_ignore_ascii_case(&stem) {
        return Err(format!("{source} describes {}, not {stem}", t.image));
    }
    Ok(t)
}

pub fn all_wanted() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    v.extend_from_slice(&crate::gdk_user::XASYNC_NAMES);
    v.extend_from_slice(&crate::gdk_user::XUSER_NAMES);
    v.extend_from_slice(&crate::gdk_store::XSTORE_NAMES);
    v.extend_from_slice(&crate::os_host::fortnite_match::SYMBOLS);
    v
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cost {
    pub ms: u128,
    pub peak_private_delta: usize,
}

#[repr(C)]
#[derive(Default)]
struct Counters {
    cb: u32,
    page_faults: u32,
    peak_ws: usize,
    ws: usize,
    quota_peak_paged: usize,
    quota_paged: usize,
    quota_peak_nonpaged: usize,
    quota_nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn K32GetProcessMemoryInfo(p: *mut core::ffi::c_void, c: *mut Counters, cb: u32) -> i32;
    fn GetCurrentProcess() -> *mut core::ffi::c_void;
}

fn counters() -> Counters {
    let mut c = Counters { cb: core::mem::size_of::<Counters>() as u32, ..Default::default() };
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    c
}

pub fn measured<T>(f: impl FnOnce() -> T) -> (T, Cost) {
    let before = counters();
    let st = std::time::Instant::now();
    let r = f();
    let ms = st.elapsed().as_millis();
    let after = counters();
    (r, Cost { ms, peak_private_delta: after.peak_pagefile.saturating_sub(before.pagefile.max(before.peak_pagefile)) })
}

pub fn for_this_title() -> Result<&'static Table, String> {
    static T: OnceLock<Result<Table, String>> = OnceLock::new();
    T.get_or_init(|| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if locate(&exe).is_none() {
            return Err(format!("no {}-Symbols.bin next to the title", exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
        }
        let wanted = all_wanted();
        let (t, cost) = measured(|| load(&exe, &wanted));
        match &t {
            Ok(t) => crate::record_raw("title_symbols", "load", &format!("{} symbols in {}; {} of {} wanted found once; {} ms, peak private +{} KB", t.total, t.source, t.unique_found(), wanted.len(), cost.ms, cost.peak_private_delta / 1024)),
            Err(e) => crate::record_raw("title_symbols", "load", &format!("{e} ({} ms)", cost.ms)),
        }
        t
    })
    .as_ref()
    .map_err(|e| e.clone())
}

#[cfg(test)]
pub fn build_for_test(image: &str, syms: &[(&str, u32, u32)]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&1u32.to_le_bytes());
    let mut name = image.as_bytes().to_vec();
    name.push(0);
    while name.len() % 4 != 0 {
        name.push(0);
    }
    p.extend_from_slice(&(name.len() as u32).to_le_bytes());
    p.extend_from_slice(&name);
    p.extend_from_slice(&[0u8; 8]);
    p.extend_from_slice(&(syms.len() as u32).to_le_bytes());
    for (_, rva, size) in syms {
        p.extend_from_slice(&rva.to_le_bytes());
        p.extend_from_slice(&size.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
    }
    p.extend_from_slice(&(syms.len() as u32).to_le_bytes());
    for (n, _, _) in syms {
        p.extend_from_slice(&(n.len() as u32 + 1).to_le_bytes());
        p.extend_from_slice(n.as_bytes());
        p.push(0);
    }
    pack_for_test(&p)
}

#[cfg(test)]
pub fn pack_for_test(p: &[u8]) -> Vec<u8> {
    let z = miniz_oxide::deflate::compress_to_vec_zlib(p, 6);
    let mut f = Vec::new();
    f.extend_from_slice(&(p.len() as u32).to_le_bytes());
    f.extend_from_slice(&(z.len() as u32).to_le_bytes());
    f.extend_from_slice(&z);
    f
}

#[cfg(test)]
pub fn table_for_test(image: &str, syms: &[(&str, u32, u32)], wanted: &[&str]) -> Table {
    parse_bytes(&build_for_test(image, syms), wanted, "t").unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_the_wanted_names() {
        let t = table_for_test("Game", &[("A", 0x100, 0x20), ("B::c", 0x200, 0x40), ("Other", 0x300, 8)], &["A", "B::c", "Missing"]);
        assert_eq!((t.image.as_str(), t.total), ("Game", 3));
        assert_eq!(t.get("B::c"), Some(Sym { rva: 0x200, size: 0x40 }));
        assert_eq!(t.get("Other"), None);
        assert!(t.require(&["A", "Missing"]).unwrap_err().contains("Missing"));
    }

    #[test]
    fn large_tables_stream_across_many_chunks() {
        let names: Vec<String> = (0..40_000).map(|i| format!("Namespace::Function_{i}_with_a_long_decorated_name")).collect();
        let syms: Vec<(&str, u32, u32)> = names.iter().enumerate().map(|(i, n)| (n.as_str(), 0x1000 + i as u32 * 16, 16)).collect();
        let t = table_for_test("Big", &syms, &[names[3].as_str(), names[39_999].as_str()]);
        assert_eq!(t.get(&names[3]), Some(Sym { rva: 0x1030, size: 16 }));
        assert_eq!(t.get(&names[39_999]), Some(Sym { rva: 0x1000 + 39_999 * 16, size: 16 }));
    }

    #[test]
    fn duplicated_names_are_not_trusted() {
        let t = table_for_test("Game", &[("A", 0x100, 0x20), ("A", 0x500, 0x20)], &["A"]);
        assert_eq!(t.get("A"), None);
        assert_eq!(t.all("A"), &[Sym { rva: 0x100, size: 0x20 }, Sym { rva: 0x500, size: 0x20 }]);
    }

    #[test]
    fn damaged_files_are_rejected() {
        let f = build_for_test("Game", &[("A", 0x100, 0x20)]);
        assert!(parse_bytes(&f[..f.len() - 1], &["A"], "t").unwrap_err().contains("packed"));
        let mut bad = f.clone();
        bad[0] ^= 1;
        assert!(parse_bytes(&bad, &["A"], "t").unwrap_err().contains("header says"));
        let mut corrupt = f.clone();
        let n = corrupt.len();
        corrupt[n - 3] ^= 0xff;
        assert!(parse_bytes(&corrupt, &["A"], "t").is_err());
        let base = {
            let mut p = Vec::new();
            p.extend_from_slice(&1u32.to_le_bytes());
            p.extend_from_slice(&4u32.to_le_bytes());
            p.extend_from_slice(b"Gam\0");
            p.extend_from_slice(&[0u8; 8]);
            p.extend_from_slice(&1u32.to_le_bytes());
            p.extend_from_slice(&[0x10, 0, 0, 0, 0x20, 0, 0, 0, 0, 0, 0, 0]);
            p.extend_from_slice(&1u32.to_le_bytes());
            p.extend_from_slice(&2u32.to_le_bytes());
            p.extend_from_slice(b"A\0");
            p
        };
        assert_eq!(parse_bytes(&pack_for_test(&base), &["A"], "t").unwrap().get("A"), Some(Sym { rva: 0x10, size: 0x20 }));
        let mut trailing = base.clone();
        trailing.push(0);
        assert!(parse_bytes(&pack_for_test(&trailing), &["A"], "t").unwrap_err().contains("trailing"));
        let mut v2 = base.clone();
        v2[0] = 2;
        assert!(parse_bytes(&pack_for_test(&v2), &["A"], "t").unwrap_err().contains("version"));
        assert!(parse_bytes(&pack_for_test(&base[..base.len() - 1]), &["A"], "t").unwrap_err().contains("ends early"));
    }

    #[test]
    fn the_file_is_found_beside_the_exe_or_in_an_ancestor_symbols_folder() {
        let root = std::env::temp_dir().join(format!("tombox_syms_{}", std::process::id()));
        let bin = root.join("Game").join("Binaries").join("XSX");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(root.join("Symbols")).unwrap();
        let exe = bin.join("Title-XSX.exe");
        assert!(locate(&exe).is_none());
        assert!(load(&exe, &["A"]).unwrap_err().contains("no Title-XSX-Symbols.bin"));
        std::fs::write(root.join("Symbols").join("Title-XSX-Symbols.bin"), build_for_test("Title-XSX", &[("A", 0x10, 0x20)])).unwrap();
        assert_eq!(load(&exe, &["A"]).unwrap().get("A"), Some(Sym { rva: 0x10, size: 0x20 }));
        std::fs::write(bin.join("Title-XSX-Symbols.bin"), build_for_test("SomethingElse", &[("A", 0x10, 0x20)])).unwrap();
        assert!(load(&exe, &["A"]).unwrap_err().contains("not Title-XSX"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore]
    fn cost_of_loading_the_dump_symbols() {
        let Ok(path) = std::env::var("TOMBOX_FN_DUMP_EXE") else { return };
        let wanted = all_wanted();
        let (t, cost) = measured(|| load(Path::new(&path), &wanted));
        let t = t.unwrap();
        let overloaded: Vec<&str> = wanted.iter().copied().filter(|w| t.all(w).len() > 1).collect();
        eprintln!("symbols_cost total={} unique={}/{} overloaded={overloaded:?} ms={} peak_private_delta_kb={}", t.total, t.unique_found(), wanted.len(), cost.ms, cost.peak_private_delta / 1024);
        assert!(wanted.iter().all(|w| !t.all(w).is_empty()));
    }
}
