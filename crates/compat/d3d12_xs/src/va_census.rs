use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const P_CREATE: u8 = 1;
pub const P_SRV: u8 = 2;
pub const P_UAV: u8 = 4;
pub const P_RTV: u8 = 8;
pub const P_DSV: u8 = 16;
const TOP: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub va: u64,
    pub len: u64,
    pub fmt: u32,
    pub w: u64,
    pub h: u32,
    pub mips: u16,
    pub dim: u32,
    pub title_va: bool,
    pub seq: u64,
}

#[derive(Default)]
struct S {
    reg: HashMap<usize, Info>,
    touched: HashMap<usize, u8>,
    how: HashMap<usize, String>,
    seq: u64,
}

static ST: Mutex<Option<S>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut S) -> R) -> R {
    let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(S::default))
}

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_VA_ALIAS_CENSUS").map(|v| v.trim() == "1").unwrap_or(false))
}

pub fn note(res: usize, desc: &[u8], va: u64, len: u64, title_va: bool) {
    if !on() || res == 0 || desc.len() < 0x24 {
        return;
    }
    let u32at = |o: usize| u32::from_le_bytes(desc[o..o + 4].try_into().unwrap());
    let info = |seq| Info {
        va,
        len: len.max(1),
        fmt: u32at(0x20),
        w: u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap()),
        h: u32at(0x18),
        mips: u16::from_le_bytes([desc[0x1e], desc[0x1f]]),
        dim: u32at(0),
        title_va,
        seq,
    };
    with(|s| {
        s.seq += 1;
        let i = info(s.seq);
        s.reg.insert(res, i);
        s.touched.remove(&res);
        s.how.remove(&res);
    });
}

pub fn set_len(res: usize, len: u64) {
    if !on() || res == 0 || len == 0 {
        return;
    }
    with(|s| {
        if let Some(i) = s.reg.get_mut(&res) {
            i.len = len;
        }
    });
}

pub fn same_start(c: &Cluster) -> usize {
    let mut starts: Vec<u64> = c.members.iter().map(|m| m.1.va).collect();
    starts.sort_unstable();
    let n = starts.len();
    starts.dedup();
    n - starts.len()
}

pub fn note_how(res: usize, how: String) {
    if !on() || res == 0 {
        return;
    }
    with(|s| {
        s.how.insert(res, how);
    });
}

pub fn touch(res: usize, path: u8) {
    if !on() || res == 0 || res == usize::MAX {
        return;
    }
    with(|s| *s.touched.entry(res).or_default() |= path);
}

fn paths(p: u8) -> String {
    let names = [(P_CREATE, "create"), (P_SRV, "SRV"), (P_UAV, "UAV"), (P_RTV, "RTV"), (P_DSV, "DSV")];
    let v: Vec<&str> = names.iter().filter(|(b, _)| p & b != 0).map(|(_, n)| *n).collect();
    v.join(",")
}

pub struct Cluster {
    pub va: u64,
    pub bytes: u64,
    pub members: Vec<(usize, Info, u8, String)>,
}

pub fn clusters(items: &[(usize, Info, u8, String)]) -> Vec<Cluster> {
    let mut v: Vec<(usize, Info, u8, String)> = items.iter().filter(|x| x.1.title_va).cloned().collect();
    v.sort_by_key(|x| (x.1.va, x.0));
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let start = v[i].1.va;
        let mut end = start.saturating_add(v[i].1.len);
        let mut j = i + 1;
        while j < v.len() && v[j].1.va < end {
            end = end.max(v[j].1.va.saturating_add(v[j].1.len));
            j += 1;
        }
        let mut members = v[i..j].to_vec();
        members.dedup_by_key(|m| m.0);
        if members.len() > 1 {
            out.push(Cluster { va: start, bytes: end - start, members });
        }
        i = j;
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.va.cmp(&b.va)));
    out
}

pub fn format(title: &str, items: &[(usize, Info, u8, String)], untracked: usize) -> String {
    let cl = clusters(items);
    let bytes: u64 = cl.iter().map(|c| c.bytes).sum();
    let known = items.iter().filter(|x| x.1.title_va).count();
    let dup: usize = cl.iter().map(same_start).sum();
    let mut out = format!("{title}: {} resources ({known} at a title VA, {} at a synthetic VA, {untracked} not in the creation registry); {} title VA ranges map to more than one PC resource, {bytes:#x} bytes ({:.1} MiB); {dup} resources share an exact start VA with another (lengths use the title layout size)\n",
        items.len() + untracked, items.len() - known, cl.len(), bytes as f64 / 1048576.0);
    for (n, c) in cl.iter().take(TOP).enumerate() {
        out.push_str(&format!("  #{n} va={:#x} size={:#x} resources={} same-start={}\n", c.va, c.bytes, c.members.len(), same_start(c)));
        for (res, i, p, how) in &c.members {
            out.push_str(&format!("    res {res:#x} fmt={} {}x{} mips={} dim={} [{how}] va={:#x} len={:#x} paths={} created#{}\n",
                i.fmt, i.w, i.h, i.mips, i.dim, i.va, i.len, paths(*p), i.seq));
        }
    }
    out
}

pub fn census(views: &[(usize, u8)]) -> String {
    let (reg, touched, hows) = with(|s| (s.reg.clone(), core::mem::take(&mut s.touched), s.how.clone()));
    let how = |r: usize| -> String {
        let base = hows.get(&r).cloned().unwrap_or_else(|| "creation path not logged".to_string());
        if crate::placed::lookup(r as *mut core::ffi::c_void).is_some() { format!("{base}; in placement registry") } else { base }
    };
    let mut used: HashMap<usize, u8> = touched;
    for (r, p) in views {
        *used.entry(*r).or_default() |= *p;
    }
    let mut items = Vec::new();
    let mut untracked = 0;
    for (r, p) in &used {
        match reg.get(r) {
            Some(i) => items.push((*r, *i, *p | P_CREATE, how(*r))),
            None => untracked += 1,
        }
    }
    let mut out = format("alias census, resources in use (RTV/DSV of this frame's captured passes plus every SRV/UAV descriptor currently recorded in the title's heaps)", &items, untracked);
    let all: Vec<(usize, Info, u8, String)> = reg.iter().map(|(r, i)| (*r, *i, P_CREATE | used.get(r).copied().unwrap_or(0), String::new())).collect();
    let cl = clusters(&all);
    let bytes: u64 = cl.iter().map(|c| c.bytes).sum();
    out.push_str(&format!("\nalias census, every resource ever created (a released one stays listed until its address is reused): {} resources, {} title VA ranges with more than one, {bytes:#x} bytes\n", all.len(), cl.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(va: u64, len: u64, fmt: u32, seq: u64) -> Info {
        Info { va, len, fmt, w: 2048, h: 512, mips: 12, dim: 3, title_va: true, seq }
    }

    #[test]
    fn overlapping_title_ranges_with_two_resources_form_a_cluster() {
        let mut syn = i(0x1000, 0x100, 1, 9);
        syn.title_va = false;
        let items = vec![
            (0xa, i(0x23ccb0000, 0x560000, 87, 1), P_SRV, "CreatePlacedResourceX separate committed".to_string()),
            (0xb, i(0x23ccb0000, 0x560000, 90, 2), P_RTV, "CreatePlacedResourceX aliased Title heap 0x10+0x0".to_string()),
            (0xc, i(0x23d4a0000, 0x40000, 61, 3), P_SRV, String::new()),
            (0xd, i(0x100000, 0x1000, 61, 4), P_SRV, String::new()),
            (0xe, i(0x100800, 0x2000, 61, 5), P_UAV, String::new()),
            (0xf, syn, P_SRV, String::new()),
            (0x10, syn, P_SRV, String::new()),
        ];
        let c = clusters(&items);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].va, c[0].bytes, c[0].members.len()), (0x23ccb0000, 0x560000, 2));
        assert_eq!((c[1].va, c[1].bytes), (0x100000, 0x2800));
        let s = format("t", &items, 3);
        assert!(s.starts_with("t: 10 resources (5 at a title VA, 2 at a synthetic VA, 3 not in the creation registry); 2 title VA ranges map to more than one PC resource, 0x562800 bytes (5.4 MiB); 1 resources share an exact start VA with another"), "{s}");
        assert!(s.contains("    res 0xb fmt=90 2048x512 mips=12 dim=3 [CreatePlacedResourceX aliased Title heap 0x10+0x0] va=0x23ccb0000 len=0x560000 paths=RTV created#2\n"), "{s}");
        assert_eq!(paths(P_CREATE | P_SRV | P_RTV), "create,SRV,RTV");
    }
}
