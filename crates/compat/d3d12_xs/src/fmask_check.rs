use crate::record;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub const FMASK_MAP: u32 = 0x1920;
pub const FMASK_SEL: u32 = 4;

#[derive(Default)]
struct S {
    shaders: HashMap<u64, Vec<(u32, u32, u32)>>,
    views: HashMap<usize, HashSet<(u32, u32)>>,
    checks: HashMap<u64, u32>,
    xlat: HashMap<u64, (bool, bool, usize)>,
    reported: HashSet<(u64, bool)>,
    pso_hash: HashMap<usize, u64>,
}

static ARMED: AtomicBool = AtomicBool::new(false);
static NO_TRACE: AtomicBool = AtomicBool::new(false);
static ST: Mutex<Option<S>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut S) -> R) -> R {
    f(ST.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(S::default))
}

pub fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

pub fn note_translation(hash: u64, hlsl: &str, pairs: &[(u32, u32, u32)]) {
    let ms = hlsl.contains("Texture2DMS<");
    let ident = hlsl.contains("0x88888810u");
    with(|s| s.xlat.insert(hash, (ms, ident, pairs.len())));
    note_shader(hash, pairs);
}

pub fn translation_text(hash: u64) -> String {
    let gate = tombox_xshader::hlsl::msaa_dim_on();
    match with(|s| s.xlat.get(&hash).copied()) {
        Some((ms, ident, n)) => format!("translated live in this process (TOMBOX_XS_MSAA_DIM={}, so xlat_index is bypassed): Texture2DMS declared={ms}, identity FMASK emitted={ident}, FMASK pairs={n}", gate as u8),
        None => format!("NOT translated live in this process: served from the shader pack/xlat_index or translated before tracking (TOMBOX_XS_MSAA_DIM={})", gate as u8),
    }
}

pub fn note_shader(hash: u64, pairs: &[(u32, u32, u32)]) {
    if pairs.is_empty() {
        return;
    }
    with(|s| s.shaders.insert(hash, pairs.to_vec()));
    ARMED.store(true, Ordering::Relaxed);
    if !crate::rtcap::enabled() && !NO_TRACE.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "fmask_check", &format!("shader {hash:016x} reads FMASK through the layout rule (table param, FMASK index, MS index) {pairs:?}; the bind-time descriptor cross-check needs TOMBOX_RTCAP=1 descriptor tracking and is NOT verified in this run"));
    }
}

pub fn note_view(res: usize, map: u32, sel: u32) {
    if !tombox_xshader::hlsl::msaa_dim_on() {
        return;
    }
    with(|s| {
        if s.views.len() < 4096 {
            s.views.entry(res).or_default().insert((map, sel));
        }
    });
}

fn ps_hash(pso: usize) -> u64 {
    let real = crate::pso_async::real_of(pso);
    crate::device_wrap::pso_summary(pso)
        .or_else(|| if real != 0 { crate::device_wrap::pso_summary(real) } else { None })
        .and_then(|(_, p, _)| if p == 0 { None } else { crate::shader_xlat::origin_hash(p) })
        .unwrap_or(0)
}

pub fn verdict(fm: Option<(usize, u32, u32)>, ms: Option<(usize, u32, u32)>, sels: &HashSet<(u32, u32)>) -> Result<String, String> {
    let show = |e: Option<(usize, u32, u32)>| e.map(|(r, d, m)| format!("res {r:#x} dim={d} map={m:#x}")).unwrap_or_else(|| "no SRV recorded".into());
    let (Some((fr, fd, fmap)), Some((mr, md, _))) = (fm, ms) else {
        return Err(format!("FMASK slot {} / MS slot {}", show(fm), show(ms)));
    };
    let sel_ok = sels.contains(&(fmap, FMASK_SEL));
    if fr != 0 && fr == mr && md == 6 && fd == 6 && fmap == FMASK_MAP && sel_ok {
        return Ok(format!("FMASK slot is a map {FMASK_MAP:#x} desc+0x34={FMASK_SEL} view of the bound MS resource {fr:#x}"));
    }
    Err(format!("FMASK slot {} (desc+0x34 values seen for that resource and map: {:?}) / MS slot {}; expected the same MS resource, dim 6, map {FMASK_MAP:#x}, desc+0x34={FMASK_SEL}",
        show(fm), sels.iter().filter(|v| v.0 == fmap).map(|v| v.1).collect::<Vec<_>>(), show(ms)))
}

pub fn on_draw(list: usize, pso: usize) {
    if pso == 0 {
        return;
    }
    let known = with(|s| s.pso_hash.get(&pso).copied());
    let h = match known {
        Some(h) => h,
        None => {
            let h = ps_hash(pso);
            with(|s| s.pso_hash.insert(pso, h));
            h
        }
    };
    let Some(pairs) = with(|s| {
        let p = s.shaders.get(&h)?.clone();
        let c = s.checks.entry(h).or_default();
        if *c >= 4 {
            return None;
        }
        *c += 1;
        Some(p)
    }) else { return };
    for (param, fi, mi) in pairs {
        let fm = crate::bindtrace::table_srv(list, false, param, fi);
        let ms = crate::bindtrace::table_srv(list, false, param, mi);
        let sels = with(|s| fm.and_then(|f| s.views.get(&f.0).cloned()).unwrap_or_default());
        let v = verdict(fm, ms, &sels);
        let ok = v.is_ok();
        if with(|s| s.reported.insert((h, ok))) {
            match v {
                Ok(m) => record("d3d12_xs", "fmask_check", &format!("shader {h:016x} param {param} [{fi}] vs [{mi}]: OK, {m}")),
                Err(m) => record("d3d12_xs", "fmask_check", &format!("MISMATCH shader {h:016x} param {param} [{fi}] vs [{mi}]: the layout rule reads [{fi}] as FMASK (identity) but {m}; see docs/BLOCKERS.md FMASK entry")),
            }
        }
    }
}
