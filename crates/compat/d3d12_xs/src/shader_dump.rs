use compat_common::record;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

static SEEN: Mutex<Option<HashSet<(u64, [u8; 8], u64)>>> = Mutex::new(None);
static WRITTEN: AtomicUsize = AtomicUsize::new(0);

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn title_name() -> String {
    std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "title".into())
}

fn dir() -> Option<&'static PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| {
        let v = compat_common::live::value("TOMBOX_SHADER_DUMP", "shader_dump").unwrap_or_else(|| "1".to_string());
        let v = v.trim();
        if v.is_empty() || v == "0" || v.eq_ignore_ascii_case("off") {
            return None;
        }
        let base = if v == "1" || v.eq_ignore_ascii_case("on") { std::env::temp_dir().join("tombox_shader_dump") } else { PathBuf::from(v) };
        let d = base.join(title_name());
        if let Err(e) = std::fs::create_dir_all(&d) {
            record("d3d12_xs", "shader_dump", &format!("TOMBOX_SHADER_DUMP: cannot create {}: {e}; dump off", d.display()));
            return None;
        }
        record("d3d12_xs", "shader_dump", &format!("TOMBOX_SHADER_DUMP: every shader given to the translator is written once to {} as <hash>_<rtfmt>.xbox (+ .<roothash>.root for mesh pipelines); check offline with xshader_corpus", d.display()));
        Some(d)
    })
    .as_ref()
}

/// Each returned root belongs to this exact captured shader and render-target variant.
pub(crate) fn captured_roots(blob: &[u8], hash: u64) -> Vec<([u8; 8], Vec<u8>)> {
    type RootDump = ([u8; 8], u64, PathBuf, PathBuf);
    static INDEX: OnceLock<HashMap<u64, Vec<RootDump>>> = OnceLock::new();
    let Some(dir) = dir() else { return Vec::new() };
    let index = INDEX.get_or_init(|| {
        let mut index = HashMap::<u64, Vec<RootDump>>::new();
        let Ok(files) = std::fs::read_dir(dir) else { return index };
        for entry in files.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some((stem, hash_suffix)) = name.strip_suffix(".root").and_then(|name| name.rsplit_once('.')) else { continue };
            let xbox = format!("{stem}.xbox");
            let Some((shader_hash, rtfmt)) = crate::shader_xlat::parse_src_name(&xbox) else { continue };
            let Ok(root_hash) = u64::from_str_radix(hash_suffix, 16) else { continue };
            index.entry(shader_hash).or_default().push((rtfmt, root_hash, dir.join(xbox), entry.path()));
        }
        index
    });
    let mut candidates = Vec::new();
    for (rtfmt, root_hash, xbox, root) in index.get(&hash).into_iter().flatten() {
        let Ok(captured_shader) = std::fs::read(xbox) else { continue };
        if captured_shader != blob {
            continue;
        }
        let Ok(root) = std::fs::read(root) else { continue };
        if fnv1a(&root) == *root_hash {
            candidates.push((*rtfmt, root));
        }
    }
    candidates
}

fn write_new(p: &std::path::Path, data: &[u8]) -> bool {
    if p.exists() {
        return false;
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, p).is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(&tmp);
    false
}

pub fn note(blob: &[u8], hash: u64, rtfmt: &[u8; 8], root: Option<&[u8]>) {
    let Some(d) = dir() else { return };
    note_in(d, blob, hash, rtfmt, root);
}

fn note_in(d: &std::path::Path, blob: &[u8], hash: u64, rtfmt: &[u8; 8], root: Option<&[u8]>) {
    let rh = root.map(fnv1a).unwrap_or(0);
    {
        let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        if !g.get_or_insert_with(HashSet::new).insert((hash, *rtfmt, rh)) {
            return;
        }
    }
    let stem = format!("{hash:016x}_{}", crate::shader_xlat::rtfmt_hex(rtfmt));
    let mut wrote = write_new(&d.join(format!("{stem}.xbox")), blob);
    if let Some(r) = root {
        wrote |= write_new(&d.join(format!("{stem}.{rh:016x}.root")), r);
    }
    if wrote {
        let n = WRITTEN.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n % 250 == 0 {
            record("d3d12_xs", "shader_dump", &format!("{n} new shaders dumped to {}", d.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_each_shader_once_with_its_root() {
        let d = std::env::temp_dir().join(format!("tombox_shader_dump_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let blob = b"DXBC test blob for the dump".to_vec();
        let hash = fnv1a(&blob);
        let rt = [1u8, 0, 0, 0, 0, 0, 0, 2];
        note_in(&d, &blob, hash, &rt, None);
        note_in(&d, &blob, hash, &rt, None);
        note_in(&d, &blob, hash, &[0; 8], Some(b"root bytes"));
        let mut names: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let stem = format!("{hash:016x}");
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&format!("{stem}_0100000000000002.xbox")));
        assert!(names.contains(&format!("{stem}_0000000000000000.xbox")));
        assert!(names.contains(&format!("{stem}_0000000000000000.{:016x}.root", fnv1a(b"root bytes"))));
        assert_eq!(std::fs::read(d.join(format!("{stem}_0100000000000002.xbox"))).unwrap(), blob);
        let _ = std::fs::remove_dir_all(&d);
    }
}
