//! Parsers for the standard (PC-format) metadata chunks that survive in XSX shader
//! containers: ISG1/OSG1 signatures, PSV0 resource bindings and the RTS0 root signature.
//! These give the HLSL emitter the D3D-side view (semantics, registers, root parameters).

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("meta: u32 out of bounds at {off:#x} (len {})", b.len()))
}

fn cstr_at(b: &[u8], off: usize) -> String {
    let tail = b.get(off..).unwrap_or(&[]);
    let end = tail.iter().position(|&c| c == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).into_owned()
}

/// One element of an ISG1/OSG1 signature.
#[derive(Debug, Clone)]
pub struct SigElement {
    pub name: String,
    pub index: u32,
    /// D3D_NAME system value (0 = none, 1 = SV_Position, 64 = SV_Target, ...).
    pub sysval: u32,
    /// D3D_REGISTER_COMPONENT_TYPE: 1 = uint, 2 = int, 3 = float.
    pub comp_type: u32,
    pub register: u32,
    pub mask: u8,
    pub rw_mask: u8,
}

pub fn parse_signature(tag: &[u8; 4], body: &[u8]) -> Result<Vec<SigElement>, String> {
    let count = u32_at(body, 0)? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > 64 {
        return Err(format!("implausible signature element count {count}"));
    }
    let (stride, prefix) = match tag {
        b"ISGN" | b"OSGN" | b"PCSG" | b"PSGN" => (24, 0),
        b"OSG5" => (28, 4),
        _ => (32, 4),
    };
    if 8 + count * stride > body.len() {
        return Err(format!("signature body truncated for {count} elements"));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let record = 8 + i * stride;
        if tag == b"OSG5" && u32_at(body, record)? != 0 {
            return Err("nonzero geometry output streams in OSG5 are unsupported".into());
        }
        let e = record + prefix;
        let name_off = u32_at(body, e)? as usize;
        out.push(SigElement {
            name: cstr_at(body, name_off),
            index: u32_at(body, e + 4)?,
            sysval: u32_at(body, e + 8)?,
            comp_type: u32_at(body, e + 12)?,
            register: u32_at(body, e + 16)?,
            mask: body[e + 20],
            rw_mask: body[e + 21],
        });
    }
    Ok(out)
}

/// A PSV0 resource binding.
#[derive(Debug, Clone)]
pub struct PsvResource {
    /// 1 Sampler, 2 CBV, 3 SRVTyped, 4 SRVRaw, 5 SRVStructured, 6 UAVTyped, 7 UAVRaw, 8 UAVStructured...
    pub res_type: u32,
    pub space: u32,
    pub lower: u32,
    pub upper: u32,
    pub kind: u32,
}

/// Parse the PSV0 chunk far enough to recover the resource-binding list.
/// Geometry-shader facts from the PSV0 runtime info.
#[derive(Debug, Clone, Copy)]
pub struct GsInfo {
    /// D3D input primitive: 1 point, 2 line, 3 triangle, 6 line-adj, 7 triangle-adj.
    pub input_primitive: u32,
    /// D3D output topology: 1 point list, 3 line strip, 5 triangle strip.
    pub output_topology: u32,
    pub max_vertex_count: u16,
    pub mesh: Option<MeshInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshInfo {
    pub stage: u8,
    pub groupshared_bytes: u32,
    pub payload_bytes: u32,
    pub max_vertices: u16,
    pub max_primitives: u16,
    pub prim_vectors: u8,
    pub topology: u8,
    pub num_threads: [u32; 3],
}

pub(crate) fn mesh_thread_count(dims: [u32; 3]) -> Result<u32, String> {
    mesh_thread_count_for(14, dims)
}

pub(crate) fn mesh_thread_count_for(stage: u8, dims: [u32; 3]) -> Result<u32, String> {
    let count = dims.into_iter().try_fold(1u32, u32::checked_mul);
    match count {
        Some(n @ 1..=128) => Ok(n),
        Some(n @ 129..=256) if stage == 13 && n % 2 == 0 => Ok(n),
        _ => Err(format!("mesh/amplification thread-group size {dims:?} outside 1..128")),
    }
}

pub fn parse_psv0_mesh(info: &[u8]) -> Option<MeshInfo> {
    let stage = *info.get(24)?;
    let num_threads = if info.len() >= 48 { [u32_at(info, 36).ok()?, u32_at(info, 40).ok()?, u32_at(info, 44).ok()?] } else { [0; 3] };
    match stage {
        13 => Some(MeshInfo {
            stage,
            groupshared_bytes: u32_at(info, 0).ok()?,
            payload_bytes: u32_at(info, 8).ok()?,
            max_vertices: u16::from_le_bytes([*info.get(12)?, *info.get(13)?]),
            max_primitives: u16::from_le_bytes([*info.get(14)?, *info.get(15)?]),
            prim_vectors: *info.get(26)?,
            topology: *info.get(27)?,
            num_threads,
        }),
        14 => Some(MeshInfo {
            stage,
            groupshared_bytes: 0,
            payload_bytes: u32_at(info, 0).ok()?,
            max_vertices: 0,
            max_primitives: 0,
            prim_vectors: 0,
            topology: 0,
            num_threads,
        }),
        _ => None,
    }
}

pub fn parse_psv0_cs_threads(body: &[u8]) -> Option<[u32; 3]> {
    let info_size = u32_at(body, 0).ok()? as usize;
    if info_size < 48 {
        return None;
    }
    let info = body.get(4..4 + info_size)?;
    let t = [u32_at(info, 36).ok()?, u32_at(info, 40).ok()?, u32_at(info, 44).ok()?];
    let prod = (t[0] as u64).checked_mul(t[1] as u64)?.checked_mul(t[2] as u64)?;
    if t.iter().any(|d| !(1..=1024).contains(d)) || prod > 1024 {
        return None;
    }
    Some(t)
}

/// Parse the GS part of PSV0's PSVRuntimeInfo: the stage union starts at +4
/// ({InputPrimitive u32, OutputTopology u32, OutputStreamMask u32, ...}); PSVRuntimeInfo1
/// adds ShaderStage u8 @+24, UsesViewID u8 @+25 and MaxVertexCount u16 @+26.
pub fn parse_psv0_gs(body: &[u8]) -> Option<GsInfo> {
    let info_size = u32_at(body, 0).ok()? as usize;
    if info_size < 28 {
        return None;
    }
    let info = body.get(4..4 + info_size)?;
    Some(GsInfo {
        input_primitive: u32_at(info, 0).ok()?,
        output_topology: u32_at(info, 4).ok()?,
        max_vertex_count: u16::from_le_bytes([*info.get(26)?, *info.get(27)?]),
        mesh: parse_psv0_mesh(info),
    })
}

pub fn parse_psv0(body: &[u8]) -> Result<Vec<PsvResource>, String> {
    let info_size = u32_at(body, 0)? as usize;
    let mut p = 4 + info_size;
    let count = u32_at(body, p)? as usize;
    p += 4;
    if count == 0 {
        return Ok(vec![]);
    }
    if count > 256 {
        return Err(format!("implausible PSV0 resource count {count}"));
    }
    let bind_size = u32_at(body, p)? as usize;
    p += 4;
    if bind_size < 16 {
        return Err(format!("PSV0 bind size {bind_size} too small"));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let e = p + i * bind_size;
        out.push(PsvResource {
            res_type: u32_at(body, e)?,
            space: u32_at(body, e + 4)?,
            lower: u32_at(body, e + 8)?,
            upper: u32_at(body, e + 12)?,
            kind: if bind_size >= 24 { u32_at(body, e + 16)? } else { 0 },
        });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct RootRange {
    /// 0 SRV, 1 UAV, 2 CBV, 3 Sampler.
    pub range_type: u32,
    pub num: u32,
    pub base_register: u32,
    pub space: u32,
    pub offset_in_table: u32,
}

#[derive(Debug, Clone)]
pub enum RootParamKind {
    Table(Vec<RootRange>),
    Constants { register: u32, space: u32, num32: u32 },
    /// 2 = CBV, 3 = SRV, 4 = UAV root descriptor.
    Descriptor { kind: u32, register: u32, space: u32 },
}

#[derive(Debug, Clone)]
pub struct RootParam {
    pub visibility: u32,
    pub kind: RootParamKind,
}

#[derive(Debug, Clone)]
pub struct RootSig {
    pub version: u32,
    pub flags: u32,
    pub params: Vec<RootParam>,
    pub num_static_samplers: u32,
    pub static_samplers: Vec<StaticSampler>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticSampler {
    pub filter: u32,
    pub address_v: u32,
    pub address_w: u32,
    pub max_anisotropy: u32,
    pub comparison_func: u32,
    pub border_color: u32,
    pub min_lod_bits: u32,
    pub max_lod_bits: u32,
    pub address_u: u32,
    pub register: u32,
    pub space: u32,
    pub visibility: u32,
}

/// Maximum number of static samplers in a D3D12 root signature.
/// Defined by the D3D12 specification as D3D12_REQ_ROOT_COMMAND_LIST_DESCRIPTOR_STATIC_SAMPLERS = 2032.
pub const D3D12_REQ_ROOT_COMMAND_LIST_DESCRIPTOR_STATIC_SAMPLERS: u32 = 2032;

/// The Scarlett packed graphics table uses shader-local descriptor types and
/// register numbers. The serialized signature only describes its first pair.
pub const SCARLETT_PACKED_TABLE_SLOTS: u32 = 7;
pub const SCARLETT_PACKED_SRV_REGISTER: u32 = 32;
pub const SCARLETT_PACKED_CBV_REGISTER: u32 = 16;
pub const SCARLETT_MEDIA_UAV_SLOT: u32 = 4;

/// Flags the compiler emits on Scarlett root signatures. They combine freely
/// across the signatures a single title creates, so every check below tests
/// bits rather than comparing the whole word: a run over 103 distinct
/// signatures produced `0x0`, `0x1`, `0x400`, `0x401`, `0x800`, `0x801`,
/// `0xc00`, `0xc01` and `0x3000000` for otherwise identical tables.
const FLAG_ALLOW_INPUT_ASSEMBLER: u32 = 0x1;
#[allow(dead_code)]
const FLAG_DENY_VERTEX_SHADER_ROOT_ACCESS: u32 = 0x8;
#[allow(dead_code)]
const FLAG_DENY_HULL_SHADER_ROOT_ACCESS: u32 = 0x10;
#[allow(dead_code)]
const FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS: u32 = 0x20;
#[allow(dead_code)]
const FLAG_DENY_PIXEL_SHADER_ROOT_ACCESS: u32 = 0x40;
#[allow(dead_code)]
const FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS: u32 = 0x80;
/// Set when the signature declares overrides the console-only compiler needs.
const FLAG_CONSOLE_OVERRIDE: u32 = 0x3000000;

fn ranges_in(ranges: &[RootRange], space: u32) -> impl Iterator<Item = &RootRange> {
    ranges.iter().filter(move |r| r.space == space)
}

fn has_range(ranges: &[RootRange], kind: u32, register: u32, space: u32) -> bool {
    ranges.iter().any(|r| r.range_type == kind && r.base_register == register && r.space == space)
}

/// The packed graphics table keeps a handful of shader-local descriptors in a
/// single unbounded-looking table. RTS0 only describes its first pair, so the
/// shape test is the table + trailing constants layout, not the exact contents.
pub fn scarlett_packed_graphics_table(root: &RootSig) -> bool {
    let [table, constants] = root.params.as_slice() else { return false };
    let RootParamKind::Table(ranges) = &table.kind else { return false };
    root.version == 2
        && root.flags & FLAG_ALLOW_INPUT_ASSEMBLER != 0
        && root.flags & FLAG_CONSOLE_OVERRIDE == 0
        && root.num_static_samplers == 0
        && table.visibility == 0 && constants.visibility == 0
        && matches!(constants.kind, RootParamKind::Constants { register: 500, space: 0, num32: 2 })
        && has_range(ranges, 0, 2, 1) && has_range(ranges, 2, 0, 1)
        && ranges.iter().all(|r| r.space == 1 || r.space == 0)
}

/// The media pipeline uses the same packed table, with only b27 described
/// in RTS0. The pixel code embeds several complete sampler descriptors.
pub fn scarlett_media_table(root: &RootSig) -> bool {
    let [table] = root.params.as_slice() else { return false };
    let RootParamKind::Table(ranges) = &table.kind else { return false };
    root.version == 2
        && root.flags & FLAG_ALLOW_INPUT_ASSEMBLER != 0
        && root.flags & FLAG_CONSOLE_OVERRIDE == 0
        && root.num_static_samplers == 0
        && table.visibility == 0
        && ranges.len() == 1
        && has_range(ranges, 2, 27, 1)
        && matches!(ranges[0].offset_in_table, 0 | u32::MAX)
}

/// The footage pipelines put their shader-local descriptors in the second
/// table. The CBV is b5 in PSV0 but can occupy slot 6 or 7 in the physical table.
/// Entry point is the six-range space-1 table, which is unique to this family;
/// the shared table and the static sampler count vary with the stage.
pub fn scarlett_footage_table(root: &RootSig) -> bool {
    let [shared, footage] = root.params.as_slice() else { return false };
    let (RootParamKind::Table(shared_ranges), RootParamKind::Table(ranges)) =
        (&shared.kind, &footage.kind) else { return false };
    root.version == 2
        && root.flags & FLAG_ALLOW_INPUT_ASSEMBLER != 0
        && root.flags & FLAG_CONSOLE_OVERRIDE == 0
        && shared.visibility == 0 && footage.visibility == 0
        && ranges.len() == 6
        && ranges_in(ranges, 1).count() == 6
        && has_range(ranges, 2, 5, 1)
        && has_range(ranges, 0, 83, 1) && has_range(ranges, 0, 82, 1)
        && has_range(ranges, 0, 71, 1) && has_range(ranges, 0, 80, 1)
        && has_range(ranges, 0, 78, 1)
        && shared_ranges.iter().all(|r| r.space == 0)
}

pub fn scarlett_graphics_table(root: &RootSig) -> Option<(usize, u32)> {
    if scarlett_media_table(root) {
        Some((0, 8))
    } else if scarlett_footage_table(root) {
        Some((1, 8))
    } else if scarlett_packed_graphics_table(root) {
        Some((0, SCARLETT_PACKED_TABLE_SLOTS))
    } else {
        None
    }
}

pub fn scarlett_graphics_table_slots(root: &RootSig) -> Option<u32> {
    scarlett_graphics_table(root).map(|(_, slots)| slots)
}

pub const SCARLETT_MEDIA_SAMPLERS: [(u32, [u32; 4]); 5] = [
    (27, [0x000, 0x00c00000, 0x14500000, 0]),
    (28, [0x092, 0x00c00000, 0x18500000, 0]),
    (29, [0x092, 0x00c00000, 0x14000000, 0]),
    (30, [0x400, 0x00c00000, 0x18f00000, 0]),
    (31, [0x092, 0x00c00000, 0x14500000, 0]),
];

/// Read the type of one static descriptor in the compiler's FSSM binding list.
/// The offsets in source 3 are dword offsets within a user-data descriptor table.
pub(crate) fn scarlett_fssm_table_kind(
    fssm: &[u8], table_sgpr: u8, byte_offset: u32,
) -> Option<u32> {
    if fssm.get(..4) != Some(b"FSSM") || byte_offset % 32 != 0 {
        return None;
    }
    let start = (u32_at(fssm, 0xf0).ok()? as usize).checked_add(16)?;
    let count = u32_at(fssm, 0xfc).ok()? as usize;
    let end = start.checked_add(count.checked_mul(20)?)?;
    let mut kind = None;
    for record in fssm.get(start..end)?.chunks_exact(20) {
        let source = u32_at(record, 0).ok()?;
        let location = u32_at(record, 16).ok()?;
        if source >> 8 != 3 || (location >> 8) & 0xff != u32::from(table_sgpr)
            || u32_at(record, 4).ok()? != byte_offset / 4
        {
            continue;
        }
        let next = match source & 0xff {
            0 => 0,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        if kind.is_some_and(|previous| previous != next) {
            return None;
        }
        kind = Some(next);
    }
    kind
}

/// Parse an RTS0 (serialized root signature) chunk body.
pub fn parse_rts0(body: &[u8]) -> Result<RootSig, String> {
    let version = u32_at(body, 0)?;
    let nparams = u32_at(body, 4)? as usize;
    let params_off = u32_at(body, 8)? as usize;
    let num_static_samplers = u32_at(body, 12)?;
    let flags = u32_at(body, 20)?;
    if nparams > 64 {
        return Err(format!("implausible root parameter count {nparams}"));
    }
    let available_params = body.get(params_off..).map_or(0, |remaining| remaining.len() / 12);
    if nparams > available_params {
        return Err(format!("root signature has {nparams} root parameters but only {available_params} fit in RTS0"));
    }
    let v11 = version >= 2;
    let mut params = Vec::with_capacity(nparams);
    for i in 0..nparams {
        let e = params_off + i * 12;
        let ptype = u32_at(body, e)?;
        let visibility = u32_at(body, e + 4)?;
        let off = u32_at(body, e + 8)? as usize;
        let kind = match ptype {
            0 => {
                let nranges = u32_at(body, off)? as usize;
                let roff = u32_at(body, off + 4)? as usize;
                let stride = if v11 { 24 } else { 20 };
                let available = body.get(roff..).map_or(0, |remaining| remaining.len() / stride);
                if nranges > available {
                    return Err(format!("root table has {nranges} ranges but only {available} fit in RTS0"));
                }
                let mut ranges = Vec::with_capacity(nranges);
                for r in 0..nranges {
                    let q = roff + r * stride;
                    ranges.push(RootRange {
                        range_type: u32_at(body, q)?,
                        num: u32_at(body, q + 4)?,
                        base_register: u32_at(body, q + 8)?,
                        space: u32_at(body, q + 12)?,
                        offset_in_table: u32_at(body, q + if v11 { 20 } else { 16 })?,
                    });
                }
                RootParamKind::Table(ranges)
            }
            1 => RootParamKind::Constants {
                register: u32_at(body, off)?,
                space: u32_at(body, off + 4)?,
                num32: u32_at(body, off + 8)?,
            },
            2..=4 => RootParamKind::Descriptor {
                kind: ptype,
                register: u32_at(body, off)?,
                space: u32_at(body, off + 4)?,
            },
            other => return Err(format!("unknown root parameter type {other}")),
        };
        params.push(RootParam { visibility, kind });
    }
    if num_static_samplers > D3D12_REQ_ROOT_COMMAND_LIST_DESCRIPTOR_STATIC_SAMPLERS {
        return Err(format!(
            "root signature static sampler count {num_static_samplers} exceeds D3D12 limit of {D3D12_REQ_ROOT_COMMAND_LIST_DESCRIPTOR_STATIC_SAMPLERS}"
        ));
    }
    let mut static_samplers = Vec::with_capacity(num_static_samplers as usize);
    if num_static_samplers > 0 {
        let so = u32_at(body, 16)? as usize;
        let stride = if version >= 3 { 56 } else { 52 };
        let available = body.get(so..).map_or(0, |remaining| remaining.len() / stride);
        if (num_static_samplers as usize) > available {
            return Err(format!(
                "root signature has {num_static_samplers} static samplers but only {available} fit in RTS0"
            ));
        }
        for i in 0..num_static_samplers as usize {
            let q = so + i * stride;
            static_samplers.push(StaticSampler {
                filter: u32_at(body, q)?,
                address_u: u32_at(body, q + 4)?,
                address_v: u32_at(body, q + 8)?,
                address_w: u32_at(body, q + 12)?,
                max_anisotropy: u32_at(body, q + 20)?,
                comparison_func: u32_at(body, q + 24)?,
                border_color: u32_at(body, q + 28)?,
                min_lod_bits: u32_at(body, q + 32)?,
                max_lod_bits: u32_at(body, q + 36)?,
                register: u32_at(body, q + 40)?,
                space: u32_at(body, q + 44)?,
                visibility: u32_at(body, q + 48)?,
            });
        }
    }
    Ok(RootSig { version, flags, params, num_static_samplers, static_samplers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_24byte_isgn_and_32byte_isg1_signatures() {
        let mut isgn = Vec::new();
        isgn.extend_from_slice(&1u32.to_le_bytes());
        isgn.extend_from_slice(&8u32.to_le_bytes());
        let name_offset_24 = 8 + 24;
        isgn.extend_from_slice(&(name_offset_24 as u32).to_le_bytes());
        isgn.extend_from_slice(&0u32.to_le_bytes());
        isgn.extend_from_slice(&0u32.to_le_bytes());
        isgn.extend_from_slice(&3u32.to_le_bytes());
        isgn.extend_from_slice(&0u32.to_le_bytes());
        isgn.push(0x0f);
        isgn.push(0x0f);
        isgn.extend_from_slice(&[0, 0]);
        isgn.extend_from_slice(b"POSITION\0");

        let sig_24 = parse_signature(b"ISGN", &isgn).expect("parse ISGN");
        assert_eq!(sig_24.len(), 1);
        assert_eq!(sig_24[0].name, "POSITION");
        assert_eq!(sig_24[0].comp_type, 3);
        assert_eq!(sig_24[0].register, 0);
        assert_eq!(sig_24[0].mask, 0x0f);
        assert_eq!(sig_24[0].rw_mask, 0x0f);

        let mut isg1 = Vec::new();
        isg1.extend_from_slice(&1u32.to_le_bytes());
        isg1.extend_from_slice(&8u32.to_le_bytes());
        let name_offset_32 = 8 + 32;
        isg1.extend_from_slice(&0u32.to_le_bytes());
        isg1.extend_from_slice(&(name_offset_32 as u32).to_le_bytes());
        isg1.extend_from_slice(&0u32.to_le_bytes());
        isg1.extend_from_slice(&0u32.to_le_bytes());
        isg1.extend_from_slice(&3u32.to_le_bytes());
        isg1.extend_from_slice(&0u32.to_le_bytes());
        isg1.push(0x0f);
        isg1.push(0x0f);
        isg1.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        isg1.extend_from_slice(b"POSITION\0");

        let sig_32 = parse_signature(b"ISG1", &isg1).expect("parse ISG1");
        assert_eq!(sig_32.len(), 1);
        assert_eq!(sig_32[0].name, "POSITION");
        assert_eq!(sig_32[0].comp_type, 3);
        assert_eq!(sig_32[0].register, 0);
        assert_eq!(sig_32[0].mask, 0x0f);
        assert_eq!(sig_32[0].rw_mask, 0x0f);
    }

    #[test]
    fn stream_signature_keeps_the_second_outputs_semantic_and_masks() {
        let mut body = Vec::new();
        for word in [2u32, 8, 0, 64, 0, 1, 3, 0, 0x000f, 0, 76, 3, 0, 3, 1, 0x0c03] {
            body.extend(word.to_le_bytes());
        }
        body.extend(b"SV_Position\0TEXCOORD\0");
        let container = crate::Dxbc {
            digest: [0; 16],
            total_size: body.len() as u32,
            chunks: vec![crate::Chunk { fourcc: *b"OSG5", offset: 0, size: body.len() }],
        };
        let sig = container.output_signature(&body).unwrap().unwrap();
        assert_eq!(sig.len(), 2);
        assert_eq!((sig[0].name.as_str(), sig[0].sysval, sig[0].mask), ("SV_Position", 1, 15));
        let uv = &sig[1];
        assert_eq!((uv.name.as_str(), uv.index, uv.register, uv.comp_type, uv.mask, uv.rw_mask), ("TEXCOORD", 3, 1, 3, 3, 12));
        body[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert!(container.output_signature(&body).unwrap_err().contains("nonzero geometry output streams"));
        assert!(parse_signature(b"OSG5", &body[..60]).is_err());
    }

    fn cs_blob(info_size: u32, dims: [u32; 3]) -> Vec<u8> {
        let mut b = vec![0u8; 4 + info_size as usize];
        b[..4].copy_from_slice(&info_size.to_le_bytes());
        if (info_size as usize) >= 48 {
            for (i, d) in dims.iter().enumerate() {
                let o = 4 + 36 + i * 4;
                b[o..o + 4].copy_from_slice(&d.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn cs_thread_counts_reject_a_bad_group_size() {
        assert_eq!(parse_psv0_cs_threads(&cs_blob(48, [8, 8, 1])), Some([8, 8, 1]));
        assert_eq!(parse_psv0_cs_threads(&cs_blob(48, [32, 1, 1])), Some([32, 1, 1]));
        assert_eq!(parse_psv0_cs_threads(&cs_blob(48, [32, 32, 1])), Some([32, 32, 1]));
        assert_eq!(parse_psv0_cs_threads(&cs_blob(36, [8, 8, 1])), None);
        assert_eq!(parse_psv0_cs_threads(&cs_blob(12, [8, 8, 1])), None);
        assert_eq!(parse_psv0_cs_threads(&cs_blob(48, [0, 8, 1])), None);
        assert_eq!(parse_psv0_cs_threads(&cs_blob(48, [1025, 1, 1])), None);
        assert_eq!(parse_psv0_cs_threads(&cs_blob(48, [32, 32, 2])), None);
        assert_eq!(parse_psv0_cs_threads(&48u32.to_le_bytes()), None);
        let mut union_only = cs_blob(48, [4, 2, 1]);
        union_only[4..16].copy_from_slice(&[8, 0, 0, 0, 8, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(parse_psv0_cs_threads(&union_only), Some([4, 2, 1]));
    }
}

#[cfg(test)]
mod wide_table_tests {
    use super::*;

    #[test]
    fn reads_large_root_tables_and_rejects_truncated_ranges() {
        for (version, stride) in [(1, 20), (2, 24)] {
            let count = 261usize;
            let mut body = vec![0; 44 + count * stride];
            for (off, value) in [(0, version), (4, 1), (8, 24), (32, 36), (36, count as u32), (40, 44)] {
                body[off..off + 4].copy_from_slice(&value.to_le_bytes());
            }
            for register in 0..count {
                let off = 44 + register * stride + 8;
                body[off..off + 4].copy_from_slice(&(register as u32).to_le_bytes());
            }
            let root = parse_rts0(&body).unwrap();
            let RootParamKind::Table(ranges) = &root.params[0].kind else { panic!("expected table") };
            assert_eq!(ranges.len(), count);
            assert_eq!(ranges[count - 1].base_register, 260);
            body.pop();
            assert!(parse_rts0(&body).is_err());
        }
    }
}

#[cfg(test)]
mod static_sampler_tests {
    use super::*;

    fn build_rts0_with_samplers(
        version: u32,
        num_samplers: u32,
        samplers_offset: u32,
        extra_bytes: usize,
    ) -> Vec<u8> {
        let stride = if version >= 3 { 56 } else { 52 };
        let total_size = (samplers_offset as usize)
            .saturating_add((num_samplers as usize).saturating_mul(stride))
            .saturating_add(extra_bytes);
        let mut body = vec![0u8; total_size.max(24)];
        body[0..4].copy_from_slice(&version.to_le_bytes());
        body[4..8].copy_from_slice(&0u32.to_le_bytes()); // nparams = 0
        body[8..12].copy_from_slice(&24u32.to_le_bytes()); // params_off = 24
        body[12..16].copy_from_slice(&num_samplers.to_le_bytes());
        body[16..20].copy_from_slice(&samplers_offset.to_le_bytes());
        body[20..24].copy_from_slice(&0u32.to_le_bytes()); // flags = 0

        for i in 0..num_samplers as usize {
            let q = (samplers_offset as usize) + i * stride;
            if q + stride <= body.len() {
                // filter: 1
                body[q..q + 4].copy_from_slice(&1u32.to_le_bytes());
                // address_u: 2
                body[q + 4..q + 8].copy_from_slice(&2u32.to_le_bytes());
                // address_v: 3
                body[q + 8..q + 12].copy_from_slice(&3u32.to_le_bytes());
                // address_w: 4
                body[q + 12..q + 16].copy_from_slice(&4u32.to_le_bytes());
                // max_anisotropy: 16
                body[q + 20..q + 24].copy_from_slice(&16u32.to_le_bytes());
                // comparison_func: 5
                body[q + 24..q + 28].copy_from_slice(&5u32.to_le_bytes());
                // border_color: 1
                body[q + 28..q + 32].copy_from_slice(&1u32.to_le_bytes());
                // min_lod_bits: 0
                body[q + 32..q + 36].copy_from_slice(&0u32.to_le_bytes());
                // max_lod_bits: 1000
                body[q + 36..q + 40].copy_from_slice(&1000u32.to_le_bytes());
                // register: (i as u32)
                body[q + 40..q + 44].copy_from_slice(&(i as u32).to_le_bytes());
                // space: 0
                body[q + 44..q + 48].copy_from_slice(&0u32.to_le_bytes());
                // visibility: 0
                body[q + 48..q + 52].copy_from_slice(&0u32.to_le_bytes());
            }
        }
        body
    }

    #[test]
    fn parses_boundary_static_sampler_counts() {
        for version in [1, 2, 3] {
            // Boundary cases: 0, 1, 64, 65, 2032
            for &count in &[0, 1, 64, 65, 2032] {
                let body = build_rts0_with_samplers(version, count, 24, 0);
                let root = parse_rts0(&body).unwrap_or_else(|e| {
                    panic!("v{version} with count {count} should parse, but failed: {e}")
                });
                assert_eq!(root.num_static_samplers, count);
                assert_eq!(root.static_samplers.len(), count as usize);
                if count > 0 {
                    assert_eq!(root.static_samplers[0].filter, 1);
                    assert_eq!(root.static_samplers[0].address_u, 2);
                    assert_eq!(root.static_samplers[0].address_v, 3);
                    assert_eq!(root.static_samplers[0].address_w, 4);
                    assert_eq!(root.static_samplers[0].register, 0);
                    let last = &root.static_samplers[(count - 1) as usize];
                    assert_eq!(last.register, count - 1);
                }
            }

            // Exceeding limit: 2033 must be rejected safely
            let body_2033 = build_rts0_with_samplers(version, 2033, 24, 0);
            let err = parse_rts0(&body_2033).unwrap_err();
            assert!(
                err.contains("exceeds D3D12 limit of 2032"),
                "Expected D3D12 limit error, but got: {err}"
            );

            // Large/overflowing count
            let mut body_overflow = build_rts0_with_samplers(version, 0, 24, 0);
            body_overflow[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(parse_rts0(&body_overflow).is_err());
        }
    }

    #[test]
    fn rejects_truncated_static_samplers_and_malformed_offsets() {
        for version in [1, 2, 3] {
            // Truncation: 1 byte missing for count = 65
            let mut body_65 = build_rts0_with_samplers(version, 65, 24, 0);
            body_65.pop();
            let err = parse_rts0(&body_65).unwrap_err();
            assert!(err.contains("fit in RTS0"), "Error was: {err}");

            // Truncation: 1 byte missing for count = 2032
            let mut body_2032 = build_rts0_with_samplers(version, 2032, 24, 0);
            body_2032.pop();
            let err = parse_rts0(&body_2032).unwrap_err();
            assert!(err.contains("fit in RTS0"), "Error was: {err}");

            // Malformed offset pointing beyond buffer
            let mut body_bad_offset = build_rts0_with_samplers(version, 1, 24, 0);
            body_bad_offset[16..20].copy_from_slice(&0x10000u32.to_le_bytes());
            let err = parse_rts0(&body_bad_offset).unwrap_err();
            assert!(err.contains("fit in RTS0"), "Error was: {err}");

            // Malformed offset at u32::MAX
            let mut body_max_offset = build_rts0_with_samplers(version, 1, 24, 0);
            body_max_offset[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
            let err = parse_rts0(&body_max_offset).unwrap_err();
            assert!(err.contains("fit in RTS0"), "Error was: {err}");
        }
    }

    #[test]
    fn parses_combined_params_and_static_samplers() {
        // 1 root constants param (12 bytes header at offset 24 + 12 bytes descriptor at offset 36)
        // followed by 65 static samplers at offset 48
        let version = 2u32;
        let num_samplers = 65u32;
        let samplers_offset = 48u32;
        let stride = 52usize;
        let total = (samplers_offset as usize) + (num_samplers as usize) * stride;
        let mut body = vec![0u8; total];

        body[0..4].copy_from_slice(&version.to_le_bytes());
        body[4..8].copy_from_slice(&1u32.to_le_bytes()); // nparams = 1
        body[8..12].copy_from_slice(&24u32.to_le_bytes()); // params_off = 24
        body[12..16].copy_from_slice(&num_samplers.to_le_bytes());
        body[16..20].copy_from_slice(&samplers_offset.to_le_bytes());
        body[20..24].copy_from_slice(&0u32.to_le_bytes()); // flags = 0

        // Param header at 24: ptype = 1 (Constants), visibility = 0, off = 36
        body[24..28].copy_from_slice(&1u32.to_le_bytes());
        body[28..32].copy_from_slice(&0u32.to_le_bytes());
        body[32..36].copy_from_slice(&36u32.to_le_bytes());

        // Constant descriptor at 36: register = 5, space = 0, num32 = 4
        body[36..40].copy_from_slice(&5u32.to_le_bytes());
        body[40..44].copy_from_slice(&0u32.to_le_bytes());
        body[44..48].copy_from_slice(&4u32.to_le_bytes());

        // Fill samplers
        for i in 0..num_samplers as usize {
            let q = (samplers_offset as usize) + i * stride;
            body[q..q + 4].copy_from_slice(&1u32.to_le_bytes()); // filter
            body[q + 40..q + 44].copy_from_slice(&(i as u32).to_le_bytes()); // register
        }

        let root = parse_rts0(&body).expect("combined signature should parse");
        assert_eq!(root.params.len(), 1);
        match &root.params[0].kind {
            RootParamKind::Constants { register, space, num32 } => {
                assert_eq!(*register, 5);
                assert_eq!(*space, 0);
                assert_eq!(*num32, 4);
            }
            _ => panic!("expected Constants param"),
        }
        assert_eq!(root.num_static_samplers, 65);
        assert_eq!(root.static_samplers.len(), 65);
        assert_eq!(root.static_samplers[64].register, 64);
    }
}
