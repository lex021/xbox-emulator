//! Offline warm-up for the unencrypted, raw-Deflate shader records in shaders.rpf.
use compat_common::record;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};
use tombox_xshader::{dxc, game_scan, pack_key, shader_cache};

const MAX_SHADER: usize = game_scan::MAX_CONTAINER;
const MAX_ARCHIVE: u64 = 1 << 30;
const INDEX_HEADER: &[u8; 8] = b"TBSRPF01";
const MODULE: &str = "d3d12_xs";
const LOG: &str = "shader_archive";

fn inflate(bytes: &[u8], size: usize) -> Option<Vec<u8>> {
    let blob = miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, size).ok()?;
    (blob.len() == size && game_scan::dxbc_len(&blob, 0) == Some(size)).then_some(blob)
}

#[derive(Clone, Copy)]
struct Entry {
    offset: u64,
    stored: u32,
    size: u32,
}

fn entries(data: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut name = 0usize;
    let mut resume = 4usize;
    for (offset, byte) in data.iter().copied().enumerate().skip(4) {
        if offset < resume {
            continue;
        }
        if (0x21..=0x7e).contains(&byte) {
            name += 1;
            continue;
        }
        let name_len = std::mem::take(&mut name);
        if byte != 0 || name_len < 3 {
            continue;
        }
        let Some(header) = data.get(offset + 2..offset + 18) else { break };
        let stored = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let size = u32::from_le_bytes(header[12..16].try_into().unwrap());
        if header[0..4] != header[8..12] || !(17..=MAX_SHADER).contains(&(stored as usize)) || !(64..=MAX_SHADER).contains(&(size as usize)) {
            continue;
        }
        let start = offset + 18;
        let end = start + stored as usize;
        let Some(payload) = data.get(start..end) else { continue };
        if inflate(payload, size as usize).is_some() {
            entries.push(Entry { offset: start as u64, stored, size });
            resume = end;
        }
    }
    entries
}

pub(crate) fn index(archive: &Path, destination: &Path) -> Result<usize, String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    file.take(MAX_ARCHIVE + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_ARCHIVE || !data.starts_with(b"9FPR") {
        return Err("expected a shaders.rpf archive (RPF9, at most 1 GiB)".into());
    }
    let found = entries(&data);
    let mut index = File::create(destination).map_err(|e| e.to_string())?;
    index.write_all(INDEX_HEADER).map_err(|e| e.to_string())?;
    for entry in &found {
        let mut row = [0; 16];
        row[..8].copy_from_slice(&entry.offset.to_le_bytes());
        row[8..12].copy_from_slice(&entry.stored.to_le_bytes());
        row[12..].copy_from_slice(&entry.size.to_le_bytes());
        index.write_all(&row).map_err(|e| e.to_string())?;
    }
    record(MODULE, LOG, &format!("indexed {} DXBC containers in {}", found.len(), archive.display()));
    Ok(found.len())
}

fn read_index(path: &Path) -> Result<Vec<Entry>, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    if !raw.starts_with(INDEX_HEADER) || (raw.len() - INDEX_HEADER.len()) % 16 != 0 {
        return Err("invalid shader archive index".into());
    }
    Ok(raw[8..].chunks_exact(16).map(|row| Entry {
        offset: u64::from_le_bytes(row[..8].try_into().unwrap()),
        stored: u32::from_le_bytes(row[8..12].try_into().unwrap()),
        size: u32::from_le_bytes(row[12..].try_into().unwrap()),
    }).collect())
}

fn read_shader(file: &File, entry: Entry) -> Result<Vec<u8>, String> {
    if !(17..=MAX_SHADER).contains(&(entry.stored as usize)) || !(64..=MAX_SHADER).contains(&(entry.size as usize)) {
        return Err("shader record exceeds archive bounds".into());
    }
    let mut payload = vec![0; entry.stored as usize];
    let mut read = 0;
    while read < payload.len() {
        let offset = entry.offset.checked_add(read as u64).ok_or("archive offset overflow")?;
        let n = file.seek_read(&mut payload[read..], offset).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("shader record is truncated".into());
        }
        read += n;
    }
    inflate(&payload, entry.size as usize).ok_or_else(|| "shader record failed DXBC validation".into())
}

enum Outcome {
    Hit,
    Compiled,
    Context(String),
    Failed(String),
}

pub(crate) fn literal_sampler_needs_root(blob: &[u8], error: &str) -> bool {
    use tombox_xshader::fssm_bind::{self, Source};

    if !error.starts_with("translate: mimg at ") || !error.contains("not a sampler descriptor (Some(Literal(") {
        return false;
    }
    let Ok(shader) = tombox_xshader::extract(blob) else { return false };
    if shader.stage == tombox_xshader::Stage::Compute || shader.root_sig.is_some() || shader.has_root_sig {
        return false;
    }
    let Ok(Some(table)) = fssm_bind::parse(&shader.fssm) else { return false };
    let mut registers = table.uses.iter().filter(|u| u.source == Source::StaticSampler).map(|u| u.register);
    registers.next().is_some_and(|first| registers.any(|reg| reg != first))
}

fn needs_pipeline_context(error: &str) -> bool {
    ["missing root signature", "needs a runtime table", "no visible static sampler"].iter().any(|hint| error.contains(hint))
}

type PipelineVariant = (bool, [u8; 8], Arc<[u8]>, Arc<tombox_xshader::hlsl::InputInterp>);

fn captured_pipeline_variants(blob: &[u8]) -> Vec<PipelineVariant> {
    use crate::pso_xrec::{self, Journal};

    static JOURNAL: OnceLock<Option<Journal>> = OnceLock::new();
    let journal = JOURNAL.get_or_init(|| {
        let dir = shader_cache::title_dir(&crate::shader_xlat::title_name_pub());
        Journal::decode(&std::fs::read(dir.join("pso_xrec.bin")).ok()?)
    });
    let Some(journal) = journal else { return Vec::new() };
    let id = pso_xrec::blob_id(blob);
    if !journal.blobs.get(&id).is_some_and(|data| data.as_ref() == blob) {
        return Vec::new();
    }
    let mut variants = Vec::new();
    for rec in &journal.recs {
        let compute = rec.kind == pso_xrec::CS;
        if !compute && (rec.kind != pso_xrec::GFX || rec.desc.len() < 0x264) { continue; }
        let Some((_, root_id)) = rec.root else { continue };
        let Some(root) = journal.blobs.get(&root_id) else { continue };
        let ps = rec.shaders.iter().find(|&&(slot, _)| slot == 0x18);
        let feeder = [0x48, 0x28, 0x08].into_iter().find(|&slot| rec.shaders.iter().any(|&(offset, _)| offset == slot));
        for &(slot, _) in rec.shaders.iter().filter(|&&(_, shader)| shader == id) {
            let word = |offset| u32::from_le_bytes(rec.desc[offset..offset + 4].try_into().unwrap());
            let rtfmt = if !compute && slot == 0x18 {
                let formats = std::array::from_fn(|i| word(0x244 + 4 * i));
                crate::shader_xlat::export_formats(word(0x240), &formats)
            } else {
                [0; 8]
            };
            let link = if !compute && feeder == Some(slot) {
                ps.and_then(|&(_, id)| journal.blobs.get(&id))
                    .map(|ps| crate::shader_xlat::ps_link(ps, false))
                    .unwrap_or_else(crate::shader_xlat::no_link)
            } else {
                crate::shader_xlat::no_link()
            };
            let variant = (compute, rtfmt, root.clone(), link);
            if !variants.contains(&variant) {
                variants.push(variant);
            }
        }
    }
    variants
}

fn compile(file: &File, entry: Entry, mode: &pack_key::Mode) -> Outcome {
    let blob = match read_shader(file, entry) {
        Ok(blob) => blob,
        Err(error) => return Outcome::Failed(error),
    };
    let hash = crate::shader_xlat::fnv1a(&blob);
    let base = crate::shader_xlat::runtime_source(&blob, hash, &[0; 8], &crate::shader_xlat::no_link(), None, mode);
    match base {
        Ok(source) => compile_source(source, mode),
        Err(error) if error.starts_with("translate: mimg at ")
            && (error.contains("not a sampler descriptor (Some(Literal(") || error.contains("FSSM retry: translate: mimg at ")) => {
            let mut roots = captured_pipeline_variants(&blob);
            roots.extend(crate::shader_dump::captured_roots(&blob, hash).into_iter()
                .map(|(format, root)| (false, format, Arc::from(root), crate::shader_xlat::no_link())));
            if roots.is_empty() {
                if literal_sampler_needs_root(&blob, &error) {
                    return Outcome::Context(format!("literal sampler needs the captured pipeline root signature: {error}"));
                }
                return compile_fssm_retry(&blob, mode, error);
            }
            let mut hit = false;
            let mut compiled = false;
            let mut errors = Vec::new();
            for (compute, rtfmt, root, link) in roots {
                let source = if compute {
                    crate::shader_xlat::prewarm_compute_source(&blob, hash, &root, mode)
                } else {
                    crate::shader_xlat::runtime_source(&blob, hash, &rtfmt, &link, Some(&root), mode)
                };
                match source {
                    Ok(source) => match compile_source(source, mode) {
                        Outcome::Compiled => compiled = true,
                        Outcome::Hit => hit = true,
                        Outcome::Failed(error) | Outcome::Context(error) => errors.push(error),
                    },
                    Err(error) => errors.push(error),
                }
            }
            match (compiled, hit) {
                (true, _) => Outcome::Compiled,
                (false, true) => Outcome::Hit,
                _ => Outcome::Failed(format!("captured pipeline root retry: {}", errors.join(" | "))),
            }
        }
        Err(error) => compile_fssm_retry(&blob, mode, error),
    }
}

fn compile_fssm_retry(blob: &[u8], mode: &pack_key::Mode, error: String) -> Outcome {
    let Ok(shader) = tombox_xshader::extract(blob) else { return Outcome::Failed(error) };
    match pack_key::prelaunch_fssm(&shader, &[0; 8], mode, &error) {
        Some(Ok(variant)) => compile_source(crate::shader_xlat::RtSrc { hlsl: variant.src, profile: variant.profile, use_dxc: true }, mode),
        Some(Err(retry)) if needs_pipeline_context(&retry) => Outcome::Context(retry),
        Some(Err(retry)) => Outcome::Failed(format!("FSSM retry: {retry}")),
        None if needs_pipeline_context(&error) => Outcome::Context(error),
        None => Outcome::Failed(error),
    }
}

fn compile_source(source: crate::shader_xlat::RtSrc, mode: &pack_key::Mode) -> Outcome {
    if !source.use_dxc {
        return Outcome::Context("variant uses a different compiler path".into());
    }
    let flags: &[&str] = if std::env::var("TOMBOX_STARTUP_FAST_DXC").as_deref() == Ok("1") { &["-O0"] } else { &[] };
    let extra = pack_key::extra(mode, flags);
    match shader_cache::compile_cached_with(&source.hlsl, &source.profile, mode.fma, &extra, || dxc::compile_fma(&source.hlsl, &source.profile, mode.fma, flags)) {
        Ok(compiled) if compiled.source == shader_cache::Source::Pack => Outcome::Hit,
        Ok(_) if shader_cache::contains(pack_key::key(&source.hlsl, &source.profile, mode, flags)) => Outcome::Compiled,
        Ok(_) => Outcome::Failed("DXC succeeded but the cache did not store its output".into()),
        Err(error) => Outcome::Failed(error),
    }
}

pub(crate) fn warm(archive: &Path, index: &Path, start: usize, count: usize, jobs: usize) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let entries = read_index(index)?;
    let end = start.saturating_add(count).min(entries.len());
    let selected = entries.get(start..end).ok_or("archive batch begins past end")?;
    if selected.is_empty() {
        return Ok(());
    }
    if !dxc::available() {
        return Err("DXC is unavailable".into());
    }
    let mode = crate::shader_xlat::runtime_mode();
    let jobs = jobs.clamp(1, 16).min(selected.len());
    let next = AtomicUsize::new(0);
    let processed = AtomicUsize::new(0);
    let hits = AtomicUsize::new(0);
    let new = AtomicUsize::new(0);
    let contextual = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let report = |n: usize| {
        record(MODULE, LOG, &format!("batch {start} {n}/{} processed | hits {} | new {} | deferred {} | failed {}", selected.len(), hits.load(Relaxed), new.load(Relaxed), contextual.load(Relaxed), failed.load(Relaxed)));
    };
    let worker = || {
        loop {
            let i = next.fetch_add(1, Relaxed);
            let Some(&entry) = selected.get(i) else { break };
            let item = start + i + 1;
            let outcome = std::panic::catch_unwind(|| compile(&file, entry, mode)).unwrap_or_else(|_| Outcome::Failed("shader worker panicked".into()));
            match outcome {
                Outcome::Hit => { hits.fetch_add(1, Relaxed); }
                Outcome::Compiled => { new.fetch_add(1, Relaxed); }
                Outcome::Context(reason) => {
                    if contextual.fetch_add(1, Relaxed) < 4 {
                        record(MODULE, LOG, &format!("variant {item} deferred to game context: {}", reason.lines().next().unwrap_or_default()));
                    }
                }
                Outcome::Failed(reason) => {
                    failed.fetch_add(1, Relaxed);
                    let reason = reason.split_whitespace().take(96).collect::<Vec<_>>().join(" ");
                    record(MODULE, LOG, &format!("variant {item} failed: {reason}"));
                }
            }
            let n = processed.fetch_add(1, Relaxed) + 1;
            if n == 1 || n % 16 == 0 && n < selected.len() {
                report(n);
            }
        }
    };
    std::thread::scope(|scope| -> Result<(), String> {
        let mut threads = Vec::with_capacity(jobs);
        for _ in 0..jobs {
            threads.push(std::thread::Builder::new().name("tombox-archive".into()).stack_size(64 << 20).spawn_scoped(scope, &worker).map_err(|e| e.to_string())?);
        }
        for thread in threads {
            thread.join().map_err(|_| "shader archive worker panicked")?;
        }
        Ok(())
    })?;
    report(processed.load(Relaxed));
    Ok(())
}
