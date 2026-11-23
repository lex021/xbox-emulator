use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_CONTAINER: usize = 0x40_0000;
const MAX_IN_MEMORY: u64 = 512 << 20;
const CHUNK: usize = 64 << 20;
const MAX_DEPTH: u8 = 3;

pub fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

pub fn dxbc_len(d: &[u8], p: usize) -> Option<usize> {
    if d.get(p..p + 4)? != b"DXBC" {
        return None;
    }
    let total = u32_at(d, p + 24)? as usize;
    let n = u32_at(d, p + 28)? as usize;
    if !(64..=MAX_CONTAINER).contains(&total) || p + total > d.len() || n == 0 || n > 32 || 32 + 4 * n > total {
        return None;
    }
    for i in 0..n {
        let off = u32_at(d, p + 32 + 4 * i)? as usize;
        if off < 32 + 4 * n || off + 8 > total {
            return None;
        }
        let cc = d.get(p + off..p + off + 4)?;
        if !cc.iter().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
    }
    Some(total)
}

pub fn lzo1x(src: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(out_len);
    let mut ip = 0usize;
    let byte = |i: usize| src.get(i).copied();
    let run = |ip: &mut usize, base: usize| -> Option<usize> {
        let mut n = 0usize;
        while byte(*ip)? == 0 {
            n += 255;
            *ip += 1;
        }
        let v = n + base + byte(*ip)? as usize;
        *ip += 1;
        Some(v)
    };
    fn lit(out: &mut Vec<u8>, src: &[u8], ip: &mut usize, n: usize) -> Option<()> {
        out.extend_from_slice(src.get(*ip..*ip + n)?);
        *ip += n;
        Some(())
    }
    fn mat(out: &mut Vec<u8>, dist: usize, n: usize) -> Option<()> {
        let s = out.len().checked_sub(dist)?;
        for k in 0..n {
            let b = out[s + k];
            out.push(b);
        }
        Some(())
    }
    let mut state = 0usize;
    if byte(0)? > 17 {
        let n = byte(0)? as usize - 17;
        ip = 1;
        lit(&mut out, src, &mut ip, n)?;
        state = if n >= 4 { 4 } else { n };
    }
    loop {
        let t = byte(ip)? as usize;
        ip += 1;
        let tail;
        if t < 16 {
            if state == 0 {
                let n = if t == 0 { run(&mut ip, 15)? } else { t };
                lit(&mut out, src, &mut ip, n + 3)?;
                state = 4;
                continue;
            }
            let b = byte(ip)? as usize;
            ip += 1;
            if state == 4 {
                mat(&mut out, 1 + 0x800 + (t >> 2) + (b << 2), 3)?;
            } else {
                mat(&mut out, 1 + (t >> 2) + (b << 2), 2)?;
            }
            tail = t;
        } else if t >= 64 {
            let b = byte(ip)? as usize;
            ip += 1;
            mat(&mut out, 1 + ((t >> 2) & 7) + (b << 3), (t >> 5) + 1)?;
            tail = t;
        } else if t >= 32 {
            let n = if t & 31 == 0 { run(&mut ip, 31)? } else { t & 31 };
            let d = u16_at(src, ip)? as usize;
            ip += 2;
            mat(&mut out, 1 + (d >> 2), n + 2)?;
            tail = d;
        } else {
            let n = if t & 7 == 0 { run(&mut ip, 7)? } else { t & 7 };
            let d = u16_at(src, ip)? as usize;
            ip += 2;
            let dist = ((t & 8) << 11) + (d >> 2);
            if dist == 0 {
                break;
            }
            mat(&mut out, dist + 0x4000, n + 2)?;
            tail = d;
        }
        if out.len() > out_len {
            return None;
        }
        let s = tail & 3;
        lit(&mut out, src, &mut ip, s)?;
        state = s;
    }
    (out.len() == out_len).then_some(out)
}

pub fn gbx_body(d: &[u8]) -> Option<Vec<u8>> {
    if d.get(0..3)? != b"GBX" {
        return None;
    }
    let ver = u16_at(d, 3)?;
    let fmt = d.get(5..9)?;
    let mut p = 13usize;
    if ver >= 6 {
        p += 4 + u32_at(d, p)? as usize;
    }
    let ext = u32_at(d, p + 4)?;
    p += 8;
    if ext != 0 {
        return None;
    }
    if fmt[2] != b'C' {
        return Some(d.get(p..)?.to_vec());
    }
    let ul = u32_at(d, p)? as usize;
    let cl = u32_at(d, p + 4)? as usize;
    if ul > 256 << 20 {
        return None;
    }
    lzo1x(d.get(p + 8..p + 8 + cl)?, ul)
}

pub fn zip_entries(d: &[u8]) -> Option<Vec<Vec<u8>>> {
    if d.len() < 22 {
        return None;
    }
    let lo = d.len().saturating_sub(22 + 0xffff);
    let eocd = (lo..=d.len() - 22).rev().find(|&i| d[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let n = u16_at(d, eocd + 10)? as usize;
    let mut cd = u32_at(d, eocd + 16)? as usize;
    let mut out = Vec::new();
    for _ in 0..n {
        if u32_at(d, cd)? != 0x0201_4b50 {
            break;
        }
        let method = u16_at(d, cd + 10)?;
        let csize = u32_at(d, cd + 20)? as usize;
        let usize_ = u32_at(d, cd + 24)? as usize;
        let (fl, el, cl) = (u16_at(d, cd + 28)? as usize, u16_at(d, cd + 30)? as usize, u16_at(d, cd + 32)? as usize);
        let lh = u32_at(d, cd + 42)? as usize;
        cd += 46 + fl + el + cl;
        if u32_at(d, lh)? != 0x0403_4b50 || usize_ > 256 << 20 {
            continue;
        }
        let start = lh + 30 + u16_at(d, lh + 26)? as usize + u16_at(d, lh + 28)? as usize;
        let Some(raw) = d.get(start..start + csize) else { continue };
        let body = match method {
            0 => Some(raw.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, usize_.max(1)).ok(),
            _ => None,
        };
        if let Some(b) = body {
            out.push(b);
        }
    }
    Some(out)
}

#[derive(Default, Debug)]
pub struct Scan {
    pub blobs: Vec<(u64, Vec<u8>)>,
    pub files: usize,
    pub bytes: u64,
    pub zip_entries: usize,
    pub gbx: usize,
    pub ue_entries: usize,
    pub errors: usize,
    seen: HashSet<u64>,
}

impl Scan {
    fn take(&mut self, blob: &[u8]) {
        let h = fnv1a(blob);
        if self.seen.insert(h) {
            self.blobs.push((h, blob.to_vec()));
        }
    }

    fn raw(&mut self, d: &[u8]) -> usize {
        let mut p = 0usize;
        let mut last = 0usize;
        while let Some(i) = memfind(&d[p..], b"DXBC") {
            let at = p + i;
            match dxbc_len(d, at) {
                Some(n) => {
                    self.take(&d[at..at + n]);
                    p = at + n;
                    last = p;
                }
                None => p = at + 4,
            }
        }
        last
    }

    pub fn bytes_in(&mut self, d: &[u8], depth: u8) {
        if depth < MAX_DEPTH {
            if d.starts_with(b"PK\x03\x04") {
                if let Some(es) = zip_entries(d) {
                    self.zip_entries += es.len();
                    for e in es {
                        self.bytes_in(&e, depth + 1);
                    }
                    return;
                }
            }
            if d.starts_with(b"GBX") {
                match gbx_body(d) {
                    Some(b) => {
                        self.gbx += 1;
                        self.raw(&b);
                    }
                    None => self.errors += 1,
                }
                return;
            }
            let arcs = crate::uearchive::find_archives(d);
            if !arcs.is_empty() {
                for a in &arcs {
                    for &(off, size, usize_) in &a.entries {
                        let Some(src) = d.get(off..off + size) else { continue };
                        let dec = if size == usize_ { Some(src.to_vec()) } else { crate::uearchive::lz4_block(src, usize_) };
                        match dec {
                            Some(b) => {
                                self.ue_entries += 1;
                                self.raw(&b);
                            }
                            None => self.errors += 1,
                        }
                    }
                }
                return;
            }
        }
        self.raw(d);
    }

    pub fn file(&mut self, p: &Path, len: u64) {
        self.files += 1;
        self.bytes += len;
        if len <= MAX_IN_MEMORY {
            match std::fs::read(p) {
                Ok(d) => self.bytes_in(&d, 0),
                Err(_) => self.errors += 1,
            }
            return;
        }
        let Ok(mut f) = std::fs::File::open(p) else {
            self.errors += 1;
            return;
        };
        let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + MAX_CONTAINER);
        let mut tmp = vec![0u8; CHUNK];
        loop {
            let n = match f.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => {
                    self.errors += 1;
                    break;
                }
            };
            buf.extend_from_slice(&tmp[..n]);
            if buf.len() < CHUNK {
                continue;
            }
            let done = self.raw(&buf);
            let keep_from = done.max(buf.len().saturating_sub(MAX_CONTAINER));
            buf.drain(..keep_from);
        }
        self.raw(&buf);
    }
}

fn memfind(h: &[u8], n: &[u8]) -> Option<usize> {
    let first = n[0];
    let mut i = 0usize;
    while i + n.len() <= h.len() {
        match h[i..h.len() - n.len() + 1].iter().position(|&b| b == first) {
            Some(j) => {
                i += j;
                if &h[i..i + n.len()] == n {
                    return Some(i);
                }
                i += 1;
            }
            None => return None,
        }
    }
    None
}

pub fn list_files(root: &Path) -> Vec<(PathBuf, u64, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0u8)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut ents: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        ents.sort();
        for p in ents {
            let Ok(m) = std::fs::metadata(&p) else { continue };
            if m.is_dir() {
                if depth < 12 {
                    stack.push((p, depth + 1));
                }
            } else if m.is_file() {
                let mt = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                out.push((p, m.len(), mt));
            }
        }
    }
    out.sort();
    out
}

pub fn signature(root: &Path, files: &[(PathBuf, u64, u64)]) -> u64 {
    let mut s = String::from("game_scan v1\n");
    for (p, len, mt) in files {
        let rel = p.strip_prefix(root).unwrap_or(p);
        s.push_str(&format!("{}\t{len}\t{mt}\n", rel.to_string_lossy()));
    }
    fnv1a(s.as_bytes())
}

pub fn scan_root(root: &Path) -> (Scan, u64) {
    let files = list_files(root);
    let sig = signature(root, &files);
    let mut s = Scan::default();
    for (p, len, _) in &files {
        s.file(p, *len);
    }
    (s, sig)
}

pub const STAMP: &str = "scan.txt";

pub fn stamp_line(sig: u64, blobs: usize) -> String {
    format!("v1 {sig:016x} {blobs}\n")
}

pub fn stamp_sig(line: &str) -> Option<u64> {
    let mut it = line.split_whitespace();
    if it.next()? != "v1" {
        return None;
    }
    u64::from_str_radix(it.next()?, 16).ok()
}

pub fn blob_name(h: u64) -> String {
    format!("{h:016x}_0000000000000000.xbox")
}

#[derive(Debug, Default)]
pub struct Sync {
    pub skipped: bool,
    pub found: usize,
    pub written: usize,
    pub scan: Option<Scan>,
}

pub fn sync_dir(game_root: &Path, out: &Path) -> std::io::Result<Sync> {
    let files = list_files(game_root);
    let sig = signature(game_root, &files);
    let stamp = out.join(STAMP);
    if std::fs::read_to_string(&stamp).ok().as_deref().and_then(stamp_sig) == Some(sig) {
        return Ok(Sync { skipped: true, ..Default::default() });
    }
    let mut s = Scan::default();
    for (p, len, _) in &files {
        s.file(p, *len);
    }
    std::fs::create_dir_all(out)?;
    let mut written = 0usize;
    for (h, b) in &s.blobs {
        let p = out.join(blob_name(*h));
        if p.exists() {
            continue;
        }
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, b)?;
        std::fs::rename(&tmp, &p)?;
        written += 1;
    }
    std::fs::write(&stamp, stamp_line(sig, s.blobs.len()))?;
    Ok(Sync { skipped: false, found: s.blobs.len(), written, scan: Some(s) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxbc(tag: u8) -> Vec<u8> {
        let mut b = vec![0u8; 96];
        b[0..4].copy_from_slice(b"DXBC");
        b[16] = tag;
        b[20..24].copy_from_slice(&1u32.to_le_bytes());
        b[24..28].copy_from_slice(&96u32.to_le_bytes());
        b[28..32].copy_from_slice(&1u32.to_le_bytes());
        b[32..36].copy_from_slice(&36u32.to_le_bytes());
        b[36..40].copy_from_slice(b"SHEX");
        b[40..44].copy_from_slice(&48u32.to_le_bytes());
        b
    }

    #[test]
    fn dxbc_headers_are_validated() {
        let b = dxbc(1);
        assert_eq!(dxbc_len(&b, 0), Some(96));
        let mut bad = b.clone();
        bad[36] = 0;
        assert_eq!(dxbc_len(&bad, 0), None);
        let mut short = b.clone();
        short[24..28].copy_from_slice(&4096u32.to_le_bytes());
        assert_eq!(dxbc_len(&short, 0), None);
        assert_eq!(dxbc_len(b"DXBCxx", 0), None);
    }

    #[test]
    fn raw_scan_dedupes_and_skips_false_magic() {
        let mut d = b"junkDXBCjunk".to_vec();
        d.extend(dxbc(1));
        d.extend(b"DXBC");
        d.extend(dxbc(2));
        d.extend(dxbc(1));
        let mut s = Scan::default();
        s.bytes_in(&d, 0);
        assert_eq!(s.blobs.len(), 2);
        assert_eq!(s.blobs[0].0, fnv1a(&dxbc(1)));
    }

    #[test]
    fn lzo_literals_short_and_long_matches() {
        let mut src = vec![17 + 4];
        src.extend(b"abcd");
        src.push(0x00);
        src.push(0x00);
        src.push(0x00);
        assert_eq!(lzo1x(&[17 + 4, b'a', b'b', b'c', b'd', 0x11, 0, 0], 4).as_deref(), Some(&b"abcd"[..]));
        let m2 = [17 + 4, b'a', b'b', b'c', b'd', (2 << 5) | (3 << 2), 0, 0x11, 0, 0];
        assert_eq!(lzo1x(&m2, 7).as_deref(), Some(&b"abcdabc"[..]));
        let m3 = [17 + 2, b'x', b'y', 32 | 6, (1u8) << 2, 0, 0x11, 0, 0];
        assert_eq!(lzo1x(&m3, 10).as_deref(), Some(&b"xyxyxyxyxy"[..]));
        let lit = [0u8, 2, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', 0x11, 0, 0];
        assert_eq!(lzo1x(&lit, 20), None);
        assert_eq!(lzo1x(&[17 + 4, b'a', b'b', b'c', b'd', 0x11, 0, 0], 5), None);
        assert_eq!(lzo1x(&[17 + 4, b'a'], 4), None);
    }

    #[test]
    fn gbx_uncompressed_and_compressed_bodies() {
        let inner = dxbc(7);
        let mut g = b"GBX".to_vec();
        g.extend(6u16.to_le_bytes());
        g.extend(b"BUUR");
        g.extend(0x0905_3000u32.to_le_bytes());
        g.extend(0u32.to_le_bytes());
        g.extend(1u32.to_le_bytes());
        g.extend(0u32.to_le_bytes());
        let mut u = g.clone();
        u.extend(&inner);
        assert_eq!(gbx_body(&u).as_deref(), Some(&inner[..]));
        let mut c = g.clone();
        c[7] = b'C';
        let mut stream = vec![0u8, (inner.len() - 18) as u8];
        stream.extend(&inner);
        stream.extend([0x11, 0, 0]);
        c.extend((inner.len() as u32).to_le_bytes());
        c.extend((stream.len() as u32).to_le_bytes());
        c.extend(&stream);
        assert_eq!(gbx_body(&c).as_deref(), Some(&inner[..]));
        let mut s = Scan::default();
        s.bytes_in(&c, 0);
        assert_eq!((s.gbx, s.blobs.len()), (1, 1));
    }

    fn zip_of(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut z = Vec::new();
        let mut cd = Vec::new();
        for (name, data, deflate) in entries {
            let body = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let off = z.len() as u32;
            z.extend(0x0403_4b50u32.to_le_bytes());
            z.extend([20, 0, 0, 0]);
            z.extend((if *deflate { 8u16 } else { 0 }).to_le_bytes());
            z.extend([0u8; 8]);
            z.extend((body.len() as u32).to_le_bytes());
            z.extend((data.len() as u32).to_le_bytes());
            z.extend((name.len() as u16).to_le_bytes());
            z.extend(0u16.to_le_bytes());
            z.extend(name.as_bytes());
            z.extend(&body);
            cd.extend(0x0201_4b50u32.to_le_bytes());
            cd.extend([20, 0, 20, 0, 0, 0]);
            cd.extend((if *deflate { 8u16 } else { 0 }).to_le_bytes());
            cd.extend([0u8; 8]);
            cd.extend((body.len() as u32).to_le_bytes());
            cd.extend((data.len() as u32).to_le_bytes());
            cd.extend((name.len() as u16).to_le_bytes());
            cd.extend([0u8; 12]);
            cd.extend(off.to_le_bytes());
            cd.extend(name.as_bytes());
        }
        let cdo = z.len() as u32;
        z.extend(&cd);
        z.extend(0x0605_4b50u32.to_le_bytes());
        z.extend([0u8; 4]);
        z.extend((entries.len() as u16).to_le_bytes());
        z.extend((entries.len() as u16).to_le_bytes());
        z.extend((cd.len() as u32).to_le_bytes());
        z.extend(cdo.to_le_bytes());
        z.extend(0u16.to_le_bytes());
        z
    }

    #[test]
    fn zip_entries_stored_and_deflated_are_scanned() {
        let a = dxbc(3);
        let b = dxbc(4);
        let z = zip_of(&[("a.bin", &a, false), ("b.bin", &b, true)]);
        let es = zip_entries(&z).unwrap();
        assert_eq!(es, vec![a.clone(), b.clone()]);
        let mut s = Scan::default();
        s.bytes_in(&z, 0);
        assert_eq!((s.zip_entries, s.blobs.len()), (2, 2));
    }

    #[test]
    fn sync_writes_blobs_once_and_skips_an_unchanged_tree() {
        let base = std::env::temp_dir().join(format!("tb_game_scan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("game");
        let out = base.join("out");
        std::fs::create_dir_all(game.join("sub")).unwrap();
        let mut f = b"head".to_vec();
        f.extend(dxbc(5));
        std::fs::write(game.join("sub").join("x.bin"), &f).unwrap();
        std::fs::write(game.join("z.zip"), zip_of(&[("y", &dxbc(6), true)])).unwrap();
        let r = sync_dir(&game, &out).unwrap();
        assert_eq!((r.skipped, r.found, r.written), (false, 2, 2));
        assert!(out.join(blob_name(fnv1a(&dxbc(5)))).exists());
        let r2 = sync_dir(&game, &out).unwrap();
        assert!(r2.skipped);
        std::fs::write(game.join("new.bin"), dxbc(8)).unwrap();
        let r3 = sync_dir(&game, &out).unwrap();
        assert_eq!((r3.skipped, r3.found, r3.written), (false, 3, 1));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn streamed_large_file_scan_finds_containers_across_chunk_edges() {
        let mut s = Scan::default();
        let mut d = vec![0u8; CHUNK - 40];
        d.extend(dxbc(9));
        d.extend(vec![0u8; 100]);
        let n = s.raw(&d);
        assert_eq!(n, CHUNK - 40 + 96);
        assert_eq!(s.blobs.len(), 1);
    }

    #[test]
    #[ignore = "reads the Trackmania dump read-only: cargo test -p tombox-xshader --lib game_scan_trackmania -- --ignored --nocapture"]
    fn game_scan_trackmania_pack_holds_every_runtime_shader() {
        let zip = Path::new("D:/xbox-pc-port/games/Trackmania/extracted/Ubisoft.TrackmaniaNext_1.6.2.0_neutral__ngz4m417e0mpw/Mount/Packs/GpuCache_XboxOne.zip");
        let src = Path::new("D:/xbox-pc-port/emulator/input/shader_cache/titles/GbxGame/src");
        if !zip.exists() || !src.exists() {
            return;
        }
        let t0 = std::time::Instant::now();
        let mut s = Scan::default();
        s.file(zip, std::fs::metadata(zip).unwrap().len());
        let found: HashSet<u64> = s.blobs.iter().map(|b| b.0).collect();
        let known: HashSet<u64> = std::fs::read_dir(src).unwrap().flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_suffix(".xbox").and_then(|s| u64::from_str_radix(&s[..16], 16).ok())
        }).collect();
        let hit = known.iter().filter(|h| found.contains(h)).count();
        println!("scan {:.2} s: {} zip entries, {} gbx, {} errors, {} unique DXBC; runtime-seen {hit}/{}", t0.elapsed().as_secs_f64(), s.zip_entries, s.gbx, s.errors, found.len(), known.len());
        assert_eq!(s.errors, 0);
        assert_eq!(found.len(), 2741);
        assert_eq!(hit, known.len());
    }
}
