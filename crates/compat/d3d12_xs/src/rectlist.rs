use compat_common::record;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub const RECTLIST: u32 = 17;
pub const TRIANGLELIST: u32 = 4;
const TOPOLOGY_TYPE_TRIANGLE: u32 = 3;
const D_VS: usize = 0x08;
const D_PS: usize = 0x18;
const D_DS: usize = 0x28;
const D_HS: usize = 0x38;
const D_GS: usize = 0x48;
const D_SO_ENTRIES: usize = 0x60;
const D_TOPOLOGY_TYPE: usize = 0x23c;
const KIND_ARBITRARY: u8 = 0;
const KIND_POSITION: u8 = 3;
const KIND_RT_ARRAY_INDEX: u8 = 4;
const KIND_VIEWPORT_INDEX: u8 = 5;
const KIND_CLIP: u8 = 6;
const KIND_CULL: u8 = 7;
const KIND_PRIMITIVE_ID: u8 = 10;
const COMP_UINT: u8 = 1;
const COMP_SINT: u8 = 2;
const COMP_FLOAT: u8 = 3;
const INTERP_CONSTANT: u8 = 1;

pub fn forwarded(t: u32) -> u32 {
    if t == RECTLIST { TRIANGLELIST } else { t }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct El {
    pub name: String,
    pub index: u32,
    pub rows: u8,
    pub start_row: u8,
    pub cols: u8,
    pub start_col: u8,
    pub kind: u8,
    pub comp: u8,
    pub interp: u8,
}

impl El {
    fn same_place(&self, o: &El) -> bool {
        let named = self.kind != KIND_ARBITRARY || self.name.eq_ignore_ascii_case(&o.name);
        named && (self.index, self.rows, self.start_row, self.cols, self.start_col, self.kind, self.comp) == (o.index, o.rows, o.start_row, o.cols, o.start_col, o.kind, o.comp)
    }

    fn label(&self) -> String {
        format!("{}{} r{}c{}x{} kind={} comp={}", self.name, self.index, self.start_row, self.start_col, self.cols, self.kind, self.comp)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Position,
    Perspective,
    Affine,
    Flat,
}

fn part<'a>(c: &'a [u8], fourcc: &[u8; 4]) -> Result<&'a [u8], String> {
    let u = |o: usize| c.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or_else(|| format!("container truncated at {o:#x}"));
    if c.get(0..4) != Some(b"DXBC") {
        return Err("not a DXBC/DXIL container".into());
    }
    for i in 0..u(28)? {
        let at = u(32 + i * 4)?;
        if c.get(at..at + 4) == Some(&fourcc[..]) {
            let n = u(at + 4)?;
            return c.get(at + 8..at + 8 + n).ok_or_else(|| format!("{} part truncated", String::from_utf8_lossy(fourcc)));
        }
    }
    Err(format!("no {} part", String::from_utf8_lossy(fourcc)))
}

struct Rd<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.o..self.o + n).ok_or_else(|| format!("PSV0 truncated at {:#x}+{n}", self.o))?;
        self.o += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
}

pub fn signature(c: &[u8]) -> Result<(Vec<El>, Vec<El>), String> {
    let mut r = Rd { b: part(c, b"PSV0")?, o: 0 };
    let isz = r.u32()? as usize;
    let info = r.take(isz)?;
    if isz < 36 {
        return Err(format!("PSV0 runtime info is {isz} bytes; it carries no signature elements"));
    }
    let (nin, nout, npc) = (info[28] as usize, info[29] as usize, info[30] as usize);
    let nres = r.u32()? as usize;
    if nres > 0 {
        let rsz = r.u32()? as usize;
        r.take(rsz * nres)?;
    }
    let ssz = r.u32()? as usize;
    let strings = r.take(ssz)?;
    let nidx = r.u32()? as usize;
    let mut idx = Vec::with_capacity(nidx);
    for _ in 0..nidx {
        idx.push(r.u32()?);
    }
    if nin + nout + npc == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let esz = r.u32()? as usize;
    if esz < 16 {
        return Err(format!("PSV0 signature elements are {esz} bytes"));
    }
    let mut read = |n: usize| -> Result<Vec<El>, String> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let e = r.take(esz)?;
            let w = |o: usize| u32::from_le_bytes(e[o..o + 4].try_into().unwrap()) as usize;
            let name = strings.get(w(0)..).map(|s| s.split(|&b| b == 0).next().unwrap_or(&[])).map(|s| String::from_utf8_lossy(s).into_owned()).unwrap_or_default();
            let index = idx.get(w(4)).copied().unwrap_or(0);
            v.push(El { name, index, rows: e[8], start_row: e[9], cols: e[10] & 0xf, start_col: (e[10] >> 4) & 3, kind: e[11], comp: e[12], interp: e[13] });
        }
        Ok(v)
    };
    let ins = read(nin)?;
    let outs = read(nout)?;
    Ok((ins, outs))
}

fn ty(e: &El) -> Result<String, String> {
    let base = match e.comp {
        COMP_UINT => "uint",
        COMP_SINT => "int",
        COMP_FLOAT => "float",
        c => return Err(format!("output {} has component type {c} (16/64-bit or unknown)", e.label())),
    };
    if e.rows != 1 || e.cols == 0 || e.cols > 4 {
        return Err(format!("output {} spans {} rows", e.label(), e.rows));
    }
    Ok(if e.cols == 1 { base.to_string() } else { format!("{base}{}", e.cols) })
}

fn semantic(e: &El) -> Result<String, String> {
    Ok(match e.kind {
        KIND_ARBITRARY => format!("{}{}", e.name, e.index),
        KIND_POSITION => "SV_Position".into(),
        KIND_RT_ARRAY_INDEX => "SV_RenderTargetArrayIndex".into(),
        KIND_VIEWPORT_INDEX => "SV_ViewportArrayIndex".into(),
        KIND_CLIP => format!("SV_ClipDistance{}", e.index),
        KIND_CULL => format!("SV_CullDistance{}", e.index),
        k => return Err(format!("output {} is system value kind {k}", e.label())),
    })
}

fn modifier(interp: u8) -> &'static str {
    match interp {
        1 => "nointerpolation ",
        3 => "centroid ",
        4 => "noperspective ",
        5 => "noperspective centroid ",
        6 => "sample ",
        7 => "noperspective sample ",
        _ => "",
    }
}

pub fn mode(e: &El, ps_in: &[El]) -> Mode {
    if e.kind == KIND_POSITION {
        return Mode::Position;
    }
    if e.comp != COMP_FLOAT || matches!(e.kind, KIND_RT_ARRAY_INDEX | KIND_VIEWPORT_INDEX) {
        return Mode::Flat;
    }
    let interp = ps_in.iter().find(|p| p.kind == e.kind && p.index == e.index && (e.kind != KIND_ARBITRARY || p.name.eq_ignore_ascii_case(&e.name))).map_or(e.interp, |p| p.interp);
    match interp {
        INTERP_CONSTANT => Mode::Flat,
        4 | 5 | 7 => Mode::Affine,
        _ => Mode::Perspective,
    }
}

pub fn gs_hlsl(vs_out: &[El], ps_in: &[El]) -> Result<String, String> {
    let pos = vs_out.iter().position(|e| e.kind == KIND_POSITION).ok_or("vertex shader writes no SV_Position")?;
    if vs_out[pos].comp != COMP_FLOAT || vs_out[pos].cols != 4 {
        return Err(format!("SV_Position is {}", vs_out[pos].label()));
    }
    let pid = ps_in.iter().any(|e| e.kind == KIND_PRIMITIVE_ID);
    let mut fields = String::new();
    for (i, e) in vs_out.iter().enumerate() {
        fields.push_str(&format!("    {}{} f{i} : {};\n", modifier(e.interp), ty(e)?, semantic(e)?));
    }
    let mut h = format!("struct GV {{\n{fields}}};\nstruct GO {{\n{fields}");
    if pid {
        h.push_str("    uint pid : SV_PrimitiveID;\n");
    }
    h.push_str("};\n[maxvertexcount(6)]\nvoid main(triangle GV v[3], uint pid : SV_PrimitiveID, inout TriangleStream<GO> s) {\n");
    let p = format!("f{pos}");
    h.push_str(&format!("    float q0 = 1.0 / v[0].{p}.w;\n    float q1 = 1.0 / v[1].{p}.w;\n    float q2 = 1.0 / v[2].{p}.w;\n"));
    h.push_str(&format!("    bool same = v[0].{p}.w == v[1].{p}.w && v[0].{p}.w == v[2].{p}.w;\n"));
    h.push_str("    float q3 = q1 + q2 - q0;\n    float w3 = 1.0 / q3;\n    GO c[4];\n");
    for k in 0..3 {
        for i in 0..vs_out.len() {
            h.push_str(&format!("    c[{k}].f{i} = v[{k}].f{i};\n"));
        }
    }
    for (i, e) in vs_out.iter().enumerate() {
        let f = format!("f{i}");
        let line = match mode(e, ps_in) {
            Mode::Position => format!("    c[3].{f} = same ? v[1].{f} + v[2].{f} - v[0].{f} : float4((v[1].{f}.xyz * q1 + v[2].{f}.xyz * q2 - v[0].{f}.xyz * q0) * w3, w3);\n"),
            Mode::Perspective => format!("    c[3].{f} = same ? v[1].{f} + v[2].{f} - v[0].{f} : (v[1].{f} * q1 + v[2].{f} * q2 - v[0].{f} * q0) * w3;\n"),
            Mode::Affine => format!("    c[3].{f} = v[1].{f} + v[2].{f} - v[0].{f};\n"),
            Mode::Flat => format!("    c[3].{f} = v[0].{f};\n"),
        };
        h.push_str(&line);
    }
    if pid {
        h.push_str("    c[0].pid = pid;\n    c[1].pid = pid;\n    c[2].pid = pid;\n    c[3].pid = pid;\n");
    }
    h.push_str("    s.Append(c[0]);\n    s.Append(c[1]);\n    s.Append(c[3]);\n    s.RestartStrip();\n    s.Append(c[0]);\n    s.Append(c[3]);\n    s.Append(c[2]);\n    s.RestartStrip();\n}\n");
    Ok(h)
}

pub fn check_gs(gs: &[u8], vs_out: &[El], ps_in: &[El]) -> Result<(), String> {
    let (gin, gout) = signature(gs).map_err(|e| format!("compiled GS: {e}"))?;
    let pid = ps_in.iter().any(|e| e.kind == KIND_PRIMITIVE_ID);
    let sig_in: Vec<&El> = gin.iter().filter(|e| e.kind != KIND_PRIMITIVE_ID).collect();
    if sig_in.len() != vs_out.len() || sig_in.iter().zip(vs_out).any(|(a, b)| !a.same_place(b)) {
        return Err(format!("GS input signature [{}] does not match the VS output signature [{}]", sig_in.iter().map(|e| e.label()).collect::<Vec<_>>().join(", "), vs_out.iter().map(|e| e.label()).collect::<Vec<_>>().join(", ")));
    }
    let sig_out: Vec<&El> = gout.iter().filter(|e| e.kind != KIND_PRIMITIVE_ID).collect();
    if sig_out.len() != vs_out.len() || sig_out.iter().zip(vs_out).any(|(a, b)| !a.same_place(b)) {
        return Err(format!("GS output signature [{}] does not match the VS output signature [{}]", sig_out.iter().map(|e| e.label()).collect::<Vec<_>>().join(", "), vs_out.iter().map(|e| e.label()).collect::<Vec<_>>().join(", ")));
    }
    if pid != gout.iter().any(|e| e.kind == KIND_PRIMITIVE_ID) {
        return Err("GS SV_PrimitiveID output does not follow the pixel shader".into());
    }
    Ok(())
}

pub fn build(vs: &[u8], ps: &[u8], compile: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<Vec<u8>, String> {
    let (_, vs_out) = signature(vs).map_err(|e| format!("VS: {e}"))?;
    let ps_in = if ps.is_empty() { Vec::new() } else { signature(ps).map_err(|e| format!("PS: {e}"))?.0 };
    let h = gs_hlsl(&vs_out, &ps_in)?;
    let gs = compile(&h).map_err(|e| format!("GS compile: {}", e.lines().next().unwrap_or("")))?;
    check_gs(&gs, &vs_out, &ps_in)?;
    Ok(gs)
}

type Built = Result<Arc<Vec<u8>>, String>;
static GS: Mutex<Option<HashMap<(u64, u64), Built>>> = Mutex::new(None);
static REFUSED: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

fn compile_cached(h: &str) -> Result<Vec<u8>, String> {
    let mode = crate::shader_xlat::runtime_mode();
    let more = ["rect-gs"];
    tombox_xshader::shader_cache::compile_cached_with(h, "gs_6_0", mode.fma, &tombox_xshader::pack_key::extra(mode, &more), || tombox_xshader::dxc::compile(h, "gs_6_0")).map(|c| c.code)
}

fn gs_for(vs: &[u8], ps: &[u8]) -> Built {
    let key = (crate::shader_xlat::fnv1a(vs), crate::shader_xlat::fnv1a(ps));
    if let Some(v) = GS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key).cloned()) {
        return v;
    }
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(vs, ps, &compile_cached))).unwrap_or_else(|_| Err("GS build panicked".into())).map(Arc::new);
    GS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, r.clone());
    r
}

unsafe fn blob<'a>(d: &[u8], o: usize) -> &'a [u8] {
    let rd = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let (p, n) = (rd(o) as *const u8, rd(o + 8) as usize);
    if p.is_null() || n == 0 || n > 0x100_0000 { &[] } else { unsafe { core::slice::from_raw_parts(p, n) } }
}

pub fn refuse(vs_ptr: usize, vs_hash: u64, why: &str) {
    if !REFUSED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(vs_hash) {
        return;
    }
    let console = crate::shader_xlat::origin_hash(vs_ptr).map_or("unknown".to_string(), |h| format!("{h:016x}"));
    record("d3d12_xs", "rectlist", &format!("REFUSED: RECTLIST (topology 17) with VS {vs_hash:016x} (console shader {console}) cannot be expanded: {why}. Its rect draws run as TRIANGLELIST, so each rect draws only its first triangle (v0, v1, v2)"));
}

pub fn refuse_desc(d: &[u8], why: &str) {
    let vs = if d.len() >= D_VS + 16 { unsafe { blob(d, D_VS) } } else { &[] };
    refuse(vs.as_ptr() as usize, crate::shader_xlat::fnv1a(vs), why);
}

pub fn expand(d: &mut [u8]) -> bool {
    if d.len() < D_TOPOLOGY_TYPE + 4 {
        return false;
    }
    let vs = unsafe { blob(d, D_VS) };
    let ps = unsafe { blob(d, D_PS) };
    let vs_ptr = vs.as_ptr() as usize;
    let vs_hash = crate::shader_xlat::fnv1a(vs);
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let why = if vs.is_empty() {
        Some("the pipeline has no vertex shader".to_string())
    } else if !unsafe { blob(d, D_GS) }.is_empty() || !unsafe { blob(d, D_HS) }.is_empty() || !unsafe { blob(d, D_DS) }.is_empty() {
        Some("the pipeline already has a geometry or tessellation stage".to_string())
    } else if u(D_SO_ENTRIES) != 0 {
        Some("the pipeline streams out vertices".to_string())
    } else if !matches!(u(D_TOPOLOGY_TYPE), 0 | TOPOLOGY_TYPE_TRIANGLE) {
        Some(format!("the pipeline topology type is {}, not triangle", u(D_TOPOLOGY_TYPE)))
    } else {
        None
    };
    if let Some(why) = why {
        refuse(vs_ptr, vs_hash, &why);
        return false;
    }
    match gs_for(vs, ps) {
        Ok(gs) => {
            d[D_GS..D_GS + 8].copy_from_slice(&(gs.as_ptr() as u64).to_le_bytes());
            d[D_GS + 8..D_GS + 16].copy_from_slice(&(gs.len() as u64).to_le_bytes());
            d[D_TOPOLOGY_TYPE..D_TOPOLOGY_TYPE + 4].copy_from_slice(&TOPOLOGY_TYPE_TRIANGLE.to_le_bytes());
            true
        }
        Err(e) => {
            refuse(vs_ptr, vs_hash, &e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fourth(p: [[f64; 4]; 3], a: [f64; 3], m: Mode) -> ([f64; 4], f64) {
        let q = [1.0 / p[0][3], 1.0 / p[1][3], 1.0 / p[2][3]];
        let q3 = q[1] + q[2] - q[0];
        let w3 = 1.0 / q3;
        let mut pos = [0.0; 4];
        for c in 0..3 {
            pos[c] = (p[1][c] * q[1] + p[2][c] * q[2] - p[0][c] * q[0]) * w3;
        }
        pos[3] = w3;
        let attr = match m {
            Mode::Perspective | Mode::Position => (a[1] * q[1] + a[2] * q[2] - a[0] * q[0]) * w3,
            Mode::Affine => a[1] + a[2] - a[0],
            Mode::Flat => a[0],
        };
        (pos, attr)
    }

    fn screen(p: [f64; 4]) -> [f64; 3] {
        [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
    }

    fn plane_at(s: [[f64; 2]; 3], f: [f64; 3], at: [f64; 2]) -> f64 {
        let det = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        let l1 = ((at[0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (at[1] - s[0][1])) / det;
        let l2 = ((s[1][0] - s[0][0]) * (at[1] - s[0][1]) - (at[0] - s[0][0]) * (s[1][1] - s[0][1])) / det;
        f[0] + l1 * (f[1] - f[0]) + l2 * (f[2] - f[0])
    }

    #[test]
    fn fourth_corner_completes_the_parallelogram_in_screen_space_and_stays_on_the_attribute_plane() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9 * (1.0 + a.abs().max(b.abs()));
        let rects: [[[f64; 4]; 3]; 3] = [
            [[-1.0, 1.0, 0.5, 1.0], [-1.0, -1.0, 0.5, 1.0], [1.0, 1.0, 0.5, 1.0]],
            [[-0.5 * 2.0, 0.5 * 2.0, 0.2 * 2.0, 2.0], [-0.5 * 4.0, -0.25 * 4.0, 0.3 * 4.0, 4.0], [0.75 * 3.0, 0.5 * 3.0, 0.1 * 3.0, 3.0]],
            [[0.1, 0.2, 0.0, 0.5], [0.3 * 1.5, 0.2 * 1.5, 0.4 * 1.5, 1.5], [0.1 * 0.8, 0.9 * 0.8, 0.9 * 0.8, 0.8]],
        ];
        let attr = [0.25, 3.0, -7.5];
        for p in rects {
            let (v3, a3) = fourth(p, attr, Mode::Perspective);
            let s: Vec<[f64; 3]> = p.iter().map(|&v| screen(v)).collect();
            let s3 = screen(v3);
            for c in 0..3 {
                assert!(close(s3[c], s[1][c] + s[2][c] - s[0][c]), "screen corner {c}: {} vs {}", s3[c], s[1][c] + s[2][c] - s[0][c]);
            }
            let xy = [[s[0][0], s[0][1]], [s[1][0], s[1][1]], [s[2][0], s[2][1]]];
            let at = [s3[0], s3[1]];
            let q = [1.0 / p[0][3], 1.0 / p[1][3], 1.0 / p[2][3]];
            let q_at = plane_at(xy, q, at);
            let aq_at = plane_at(xy, [attr[0] * q[0], attr[1] * q[1], attr[2] * q[2]], at);
            assert!(close(1.0 / v3[3], q_at), "1/w is affine in screen space");
            assert!(close(a3, aq_at / q_at), "perspective attribute: {a3} vs {}", aq_at / q_at);
            let (_, n3) = fourth(p, attr, Mode::Affine);
            assert!(close(n3, plane_at(xy, attr, at)), "noperspective attribute: {n3}");
            assert_eq!(fourth(p, attr, Mode::Flat).1, attr[0]);
        }
        let blit = rects[0];
        let (v3, a3) = fourth(blit, attr, Mode::Perspective);
        assert_eq!(v3, [1.0, -1.0, 0.5, 1.0]);
        assert_eq!(a3, attr[1] + attr[2] - attr[0]);
    }

    fn el(name: &str, index: u32, row: u8, col: u8, cols: u8, kind: u8, comp: u8, interp: u8) -> El {
        El { name: name.into(), index, rows: 1, start_row: row, cols, start_col: col, kind, comp, interp }
    }

    #[test]
    fn gs_source_keeps_the_vs_outputs_and_emits_two_triangles_led_by_v0() {
        let vs = [el("", 0, 0, 0, 4, KIND_POSITION, COMP_FLOAT, 4), el("TEXCOORD", 0, 1, 0, 2, 0, COMP_FLOAT, 2), el("TEXCOORD", 1, 1, 2, 1, 0, COMP_UINT, 1), el("COLOR", 0, 2, 0, 4, 0, COMP_FLOAT, 2)];
        let ps = [el("COLOR", 0, 2, 0, 4, 0, COMP_FLOAT, 4), el("", 0, 0xff, 0, 1, KIND_PRIMITIVE_ID, COMP_UINT, 1)];
        let h = gs_hlsl(&vs, &ps).unwrap();
        assert!(h.contains("    noperspective float4 f0 : SV_Position;\n    float2 f1 : TEXCOORD0;\n    nointerpolation uint f2 : TEXCOORD1;\n    float4 f3 : COLOR0;\n"), "{h}");
        assert!(h.contains("c[3].f0 = same ? v[1].f0 + v[2].f0 - v[0].f0 : float4((v[1].f0.xyz * q1"), "{h}");
        assert!(h.contains("c[3].f1 = same ? v[1].f1 + v[2].f1 - v[0].f1 : (v[1].f1 * q1"), "{h}");
        assert!(h.contains("c[3].f2 = v[0].f2;"), "{h}");
        assert!(h.contains("c[3].f3 = v[1].f3 + v[2].f3 - v[0].f3;"), "{h}");
        assert!(h.contains("uint pid : SV_PrimitiveID;\n};"), "{h}");
        assert!(h.contains("s.Append(c[0]);\n    s.Append(c[1]);\n    s.Append(c[3]);\n    s.RestartStrip();\n    s.Append(c[0]);\n    s.Append(c[3]);\n    s.Append(c[2]);"), "{h}");
        assert!(gs_hlsl(&vs[1..], &[]).is_err());
        assert!(gs_hlsl(&[el("", 0, 0, 0, 4, KIND_POSITION, COMP_FLOAT, 0), el("X", 0, 1, 0, 4, 0, 6, 0)], &[]).is_err());
        assert_eq!(forwarded(RECTLIST), TRIANGLELIST);
        for t in [0, 1, 2, 3, 4, 5, 19, 33] {
            assert_eq!(forwarded(t), t);
        }
    }

    const VS: &str = "struct O { float4 p : SV_Position; float2 uv : TEXCOORD0; nointerpolation uint id : TEXCOORD1; noperspective float3 n : NORMAL0; float c : SV_ClipDistance0; };\nO main(uint v : SV_VertexID) { O o; o.p = float4(v, 0, 0, 1); o.uv = float2(v, 1); o.id = v; o.n = v; o.c = 1; return o; }\n";
    const PS: &str = "float4 main(float4 p : SV_Position, float2 uv : TEXCOORD0, nointerpolation uint id : TEXCOORD1, noperspective float3 n : NORMAL0, uint pid : SV_PrimitiveID) : SV_Target { return float4(uv, n.x + id + pid, 1); }\n";

    #[test]
    fn compiled_gs_signatures_match_the_vertex_shader_exactly() {
        if !tombox_xshader::dxc::available() {
            println!("DXC not available: compile check not run");
            return;
        }
        let vs = tombox_xshader::dxc::compile(VS, "vs_6_0").expect("vs");
        let ps = tombox_xshader::dxc::compile(PS, "ps_6_0").expect("ps");
        let (_, out) = signature(&vs).unwrap();
        assert_eq!(out.len(), 5, "{out:?}");
        assert_eq!(out[0].kind, KIND_POSITION);
        assert_eq!((out[2].comp, out[2].interp), (COMP_UINT, INTERP_CONSTANT));
        assert_eq!(out[3].interp, 4);
        let gs = build(&vs, &ps, &|h| tombox_xshader::dxc::compile(h, "gs_6_0")).unwrap();
        let (_, gout) = signature(&gs).unwrap();
        assert!(gout.iter().any(|e| e.kind == KIND_PRIMITIVE_ID));
        let gs2 = build(&vs, &[], &|h| tombox_xshader::dxc::compile(h, "gs_6_0")).unwrap();
        assert!(!signature(&gs2).unwrap().1.iter().any(|e| e.kind == KIND_PRIMITIVE_ID));
        let mut wrong = out.clone();
        wrong[1].start_row += 1;
        assert!(check_gs(&gs2, &wrong, &[]).is_err());
    }
}

#[cfg(test)]
#[path = "rectlist_hw.rs"]
mod hw;
