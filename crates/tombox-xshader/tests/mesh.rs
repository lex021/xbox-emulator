#![cfg(windows)]
#![allow(dead_code)]

#[path = "../../tools/xreplay/src/d3d.rs"]
mod d3d;

#[path = "support/mesh_isa.rs"]
mod mesh_isa;
#[path = "support/mesh_gpu.rs"]
mod mesh_gpu;

use mesh_gpu::{mesh_gpu, mesh_stream, render_pixel, warp_device, IID_DEVICE2};
use mesh_isa::*;
use core::ffi::c_void;
use d3d::Guid;
use tombox_xshader::hlsl::translate;
use tombox_xshader::meta::{parse_psv0_mesh, GsInfo, MeshInfo, SigElement};
use tombox_xshader::{dxc, extract, rdna2, ShaderExtract, Stage};

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

fn sig(name: &str, sysval: u32, comp_type: u32, register: u32, mask: u8) -> SigElement {
    SigElement { name: name.into(), index: 0, sysval, comp_type, register, mask, rw_mask: 0 }
}

fn mesh_info(threads: u32, mv: u16, mp: u16, topology: u8) -> MeshInfo {
    MeshInfo { stage: 13, groupshared_bytes: 0, payload_bytes: 0, max_vertices: mv, max_primitives: mp, prim_vectors: 0, topology, num_threads: [threads, 1, 1] }
}

fn ms(words: &[u32], mi: MeshInfo, outs: Vec<SigElement>) -> ShaderExtract {
    let isa: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    ShaderExtract {
        stage: Stage::Mesh,
        shader_model: (6, 5),
        bitcode_stripped: true,
        fssm: Vec::new(),
        entry: Some("main".into()),
        isa_data: isa.clone(),
        isa,
        isa_offset_in_fssm: 0,
        isa_preamble: [0; 4],
        has_input_sig: false,
        has_output_sig: true,
        has_psv0: true,
        has_root_sig: false,
        input_sig: vec![],
        output_sig: outs,
        resources: Vec::new(),
        gs_info: Some(GsInfo { input_primitive: 0, output_topology: 0, max_vertex_count: 0, mesh: Some(mi) }),
        root_sig: None,
    }
}

fn pos_param_outs() -> Vec<SigElement> {
    vec![sig("SV_Position", 1, 3, 0, 0xf), sig("TEXCOORD", 0, 1, 1, 0x3)]
}

#[test]
fn psv0_mesh_info_layout() {
    let mut info = vec![0u8; 52];
    info[0..4].copy_from_slice(&256u32.to_le_bytes());
    info[8..12].copy_from_slice(&0u32.to_le_bytes());
    info[12..14].copy_from_slice(&64u16.to_le_bytes());
    info[14..16].copy_from_slice(&126u16.to_le_bytes());
    info[24] = 13;
    info[26] = 0;
    info[27] = 2;
    info[36..40].copy_from_slice(&128u32.to_le_bytes());
    info[40..44].copy_from_slice(&1u32.to_le_bytes());
    info[44..48].copy_from_slice(&1u32.to_le_bytes());
    let m = parse_psv0_mesh(&info).expect("mesh info");
    assert_eq!(m, MeshInfo { stage: 13, groupshared_bytes: 256, payload_bytes: 0, max_vertices: 64, max_primitives: 126, prim_vectors: 0, topology: 2, num_threads: [128, 1, 1] });
    info[24] = 14;
    info[0..4].copy_from_slice(&48u32.to_le_bytes());
    let a = parse_psv0_mesh(&info).expect("amplification info");
    assert_eq!((a.stage, a.payload_bytes, a.num_threads), (14, 48, [128, 1, 1]));
    info[24] = 5;
    assert!(parse_psv0_mesh(&info).is_none());
}

#[test]
fn encoded_sample_decodes_as_console_mesh_abi() {
    let insns = rdna2::disassemble(&console_ms(67, 1, TRI).iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>());
    let m: Vec<&str> = insns.iter().map(|i| i.mnemonic.as_str()).collect();
    for want in ["s_mov_b64", "v_mbcnt_lo_u32_b32", "v_mbcnt_hi_u32_b32", "s_bfe_u32", "v_lshl_add_u32", "v_readfirstlane_b32", "v_cmpx_eq_u32", "s_cbranch_execz", "s_sendmsg", "s_barrier", "v_cmp_eq_u32", "v_cndmask_b32", "exp", "s_endpgm"] {
        assert!(m.contains(&want), "missing {want} in {m:?}");
    }
}

#[test]
fn mesh_hlsl_entry_and_tail() {
    let t = translate(&ms(&console_ms(67, 1, TRI), mesh_info(128, 128, 128, 2), pos_param_outs())).expect("translate");
    assert_eq!(t.profile, "ms_6_5");
    let h = &t.hlsl;
    for want in [
        "[numthreads(128, 1, 1)]",
        "[outputtopology(\"triangle\")]",
        "out vertices SOut verts[128], out indices uint3 tris[128], out primitives SPrim prims[128]",
        "v5 = gid.x * 128u + gi; s3 = (gi / 64u) << 24;",
        "s2 = (gid.x * 128u + (gi & 0xffffffc0u));",
        "(min(gi & 63u, 32u) + 0x0u)",
        "(max(gi & 63u, 32u) - 32u + v0)",
        "if (exec_lo & 1u) tb_mscnt = m0;",
        "SetMeshOutputCounts(tb_nv, tb_np);",
        "if (gi < tb_nv) verts[gi] = o;",
        "tris[gi] = uint3(tb_prim & 0x1ffu, (tb_prim >> 10) & 0x1ffu, (tb_prim >> 20) & 0x1ffu); prims[gi].cull = (tb_prim >> 31) != 0u;",
        "precise float4 o0 : SV_Position0;",
        "uint2 o1 : TEXCOORD0;",
        "if (exec_lo & 1u) tb_prim = v9;",
    ] {
        assert!(h.contains(want), "missing `{want}` in\n{h}");
    }
    let bar = h.matches("GroupMemoryBarrierWithGroupSync();").count();
    assert_eq!(bar, 3, "{h}");
}

#[test]
fn mesh_line_topology_and_wave32() {
    let mut w = vec![];
    w.extend(vop3(0x365, 0, [ic(-1), ic(0), 0]));
    w.extend([sop1(3, M0, LIT), (1 << 12) | 2]);
    w.push(sopp(0x10, 9));
    w.extend([vop1(1, 9, LIT), 1 << 10]);
    w.extend(exp(12, 0xf, false, [0, 0, 0, 0]));
    w.extend(exp(20, 0x1, true, [9, 0, 0, 0]));
    w.push(S_ENDPGM);
    let t = translate(&ms(&w, mesh_info(32, 2, 1, 1), vec![sig("SV_Position", 1, 3, 0, 0xf)])).expect("translate");
    assert!(t.hlsl.contains("[outputtopology(\"line\")]"), "{}", t.hlsl);
    assert!(t.hlsl.contains("out indices uint2 tris[1]"), "{}", t.hlsl);
    assert!(t.hlsl.contains("((gi & 31u) + 0x0u)"), "{}", t.hlsl);
    assert!(t.hlsl.contains("tris[gi] = uint2(tb_prim & 0x1ffu, (tb_prim >> 10) & 0x1ffu);"), "{}", t.hlsl);
}

#[test]
fn mesh_clip_distance_export_maps_to_pos1() {
    let mut w = vec![];
    w.extend([sop1(3, M0, LIT), (1 << 12) | 3]);
    w.push(sopp(0x10, 9));
    w.extend(exp(12, 0xf, false, [0, 0, 0, 0]));
    w.extend(exp(13, 0x3, false, [1, 2, 0, 0]));
    w.extend(exp(20, 0x1, true, [9, 0, 0, 0]));
    w.push(S_ENDPGM);
    let outs = vec![sig("SV_Position", 1, 3, 0, 0xf), sig("SV_ClipDistance", 2, 3, 1, 0x3)];
    let t = translate(&ms(&w, mesh_info(64, 64, 64, 2), outs)).expect("translate");
    assert!(t.hlsl.contains("if (exec_lo & 1u) o.o1.x = asfloat(v1);"), "{}", t.hlsl);
    assert!(t.hlsl.contains("if (exec_lo & 1u) o.o1.y = asfloat(v2);"), "{}", t.hlsl);
    let mut w2 = w.clone();
    let i = w2.iter().position(|x| *x == exp(13, 0x3, false, [1, 2, 0, 0])[0]).unwrap();
    w2[i] = exp(14, 0x3, false, [1, 2, 0, 0])[0];
    let outs = vec![sig("SV_Position", 1, 3, 0, 0xf), sig("SV_ClipDistance", 2, 3, 1, 0x3)];
    let e = translate(&ms(&w2, mesh_info(64, 64, 64, 2), outs)).err().expect("refusal");
    assert!(e.contains("no clip/cull distance element"), "{e}");
}

#[test]
fn mesh_scalar_buffer_load_uses_root_descriptor_for_aliased_register() {
    use tombox_xshader::meta::{RootParam, RootParamKind, RootSig};

    // Copy a user SGPR through two aliases, then read a dword from that buffer.
    let mut words = vec![sop1(3, 0, 16), sop1(3, 28, 0), 0xf420_000e, 0xfa00_0000];
    words.extend(console_ms(3, 1, 0));
    let mut shader = ms(&words, mesh_info(128, 128, 128, 2), pos_param_outs());
    let Err(error) = translate(&shader) else {
        panic!("a scalar buffer load requires a known root descriptor");
    };
    assert!(error.contains("missing root signature for base s28 (root s16)"), "{error}");

    shader.has_root_sig = true;
    shader.root_sig = Some(RootSig {
        version: 2,
        flags: 0,
        params: vec![RootParam {
            visibility: 0,
            kind: RootParamKind::Descriptor { kind: 2, register: 7, space: 2 },
        }],
        num_static_samplers: 0,
        static_samplers: Vec::new(),
    });
    let translated = translate(&shader).expect("explicit CBV root descriptor");
    assert!(translated.hlsl.contains("register(b7, space2)"), "{}", translated.hlsl);
}

#[test]
fn mesh_typed_buffer_load_uses_root_table_range() {
    use tombox_xshader::meta::{RootParam, RootParamKind, RootRange, RootSig};

    // Read slot 1 via the table in s16, then use that descriptor for a typed load.
    let mut words = vec![0xf408_0008, 0xfa00_0020, 0xe000_2000, 0x8000_0101];
    words.extend(console_ms(3, 1, 0));
    let mut shader = ms(&words, mesh_info(128, 128, 128, 2), pos_param_outs());
    let Err(error) = translate(&shader) else {
        panic!("a typed buffer load requires a known descriptor table");
    };
    assert!(error.contains("missing root signature for descriptor table in s16"), "{error}");

    shader.has_root_sig = true;
    shader.root_sig = Some(RootSig {
        version: 2,
        flags: 0,
        params: vec![RootParam {
            visibility: 0,
            kind: RootParamKind::Table(vec![RootRange {
                range_type: 0,
                num: 1,
                base_register: 7,
                space: 2,
                offset_in_table: 1,
            }]),
        }],
        num_static_samplers: 0,
        static_samplers: Vec::new(),
    });
    let translated = translate(&shader).expect("explicit SRV descriptor range");
    assert!(translated.hlsl.contains("Buffer<"), "{}", translated.hlsl);
    assert!(translated.hlsl.contains("register(t7, space2)"), "{}", translated.hlsl);
}

#[test]
fn mesh_and_amplification_validate_thread_dimensions() {
    for dims in [[1, 0, 1], [0, 1, 1], [129, 1, 1], [65536, 65536, 1], [u32::MAX; 3]] {
        let mut info = mesh_info(1, 3, 1, 2);
        info.num_threads = dims;
        let mesh = ms(&console_ms(3, 1, 0), info, pos_param_outs());
        assert!(translate(&mesh).err().unwrap().contains("thread-group size"));
        info.stage = 14;
        let mut amp = ms(&[S_ENDPGM], info, vec![]);
        amp.stage = Stage::Amplification;
        assert!(translate(&amp).err().unwrap().contains("thread-group size"));
    }

    for dims in [[1, 1, 1], [1, 3, 1], [8, 4, 4]] {
        let mut info = mesh_info(1, 0, 0, 0);
        info.num_threads = dims;
        info.stage = 14;
        let mut amp = ms(&[S_ENDPGM], info, vec![]);
        amp.stage = Stage::Amplification;
        let t = translate(&amp).unwrap();
        let threads: u32 = dims.into_iter().product();
        assert!(t.hlsl.contains(&format!("tb_k < 4u; tb_k += {threads}u) tb_args[tb_k] = 0u;")));
        dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).unwrap();
    }
}

#[test]
fn mesh_refusals_are_explicit() {
    let base = console_ms(3, 1, 0);
    let mut mi = mesh_info(128, 128, 128, 2);
    mi.prim_vectors = 1;
    assert!(translate(&ms(&base, mi, pos_param_outs())).err().expect("refusal").contains("per-primitive outputs"));
    let mut mi = mesh_info(128, 128, 128, 2);
    mi.payload_bytes = 16;
    let t = translate(&ms(&base, mi, pos_param_outs())).expect("mesh shader with an amplification payload");
    assert!(t.hlsl.contains("struct TbPayload { uint w[4]; };") && t.hlsl.contains("in payload TbPayload tb_pl"), "{}", t.hlsl);
    let mut mi = mesh_info(128, 128, 128, 2);
    mi.payload_bytes = 16388;
    assert!(translate(&ms(&base, mi, pos_param_outs())).err().expect("refusal").contains("payload of 16388 bytes exceeds the D3D12 limit 16384"));
    assert!(translate(&ms(&base, mesh_info(64, 128, 64, 2), pos_param_outs())).err().expect("refusal").contains("multi-row export"));
    assert!(translate(&ms(&base, mesh_info(128, 128, 128, 0), pos_param_outs())).err().expect("refusal").contains("topology 0"));
    let mut bad = base.clone();
    let i = bad.iter().position(|x| *x == sopp(0x10, 9)).unwrap();
    bad[i] = sopp(0x10, 2);
    assert!(translate(&ms(&bad, mesh_info(128, 128, 128, 2), pos_param_outs())).err().expect("refusal").contains("message 2 in a mesh shader"));
    let mut mask = base.clone();
    let i = mask.iter().position(|x| *x == vop3(0x365, 0, [ic(-1), ic(0), 0])[1]).unwrap();
    mask[i] = vop3(0x365, 0, [ic(3), ic(0), 0])[1];
    assert!(translate(&ms(&mask, mesh_info(128, 128, 128, 2), pos_param_outs())).err().expect("refusal").contains("mask other than all lanes"));
    let mut a = ms(&base, mesh_info(128, 128, 128, 2), pos_param_outs());
    a.stage = Stage::Amplification;
    let e = translate(&a).err().expect("refusal");
    assert!(e.contains("s_sendmsg outside a geometry shader"), "{e}");
}

#[test]
fn mesh_divergent_barrier_with_lds_is_refused() {
    let mut w = vec![];
    w.push(vopc(0xd2, ic(0), 0));
    w.push(sopp(8, 1));
    w.push(sopp(0x0a, 0));
    w.extend([0xd834_0000, 0x0000_0100]);
    w.extend([sop1(3, M0, LIT), (1 << 12) | 3]);
    w.push(sopp(0x10, 9));
    w.extend(exp(12, 0xf, true, [0, 0, 0, 0]));
    w.push(S_ENDPGM);
    let e = translate(&ms(&w, mesh_info(64, 64, 64, 2), vec![sig("SV_Position", 1, 3, 0, 0xf)])).err().expect("refusal");
    assert!(e.contains("divergent path"), "{e}");
}

fn gta_dir() -> Option<std::path::PathBuf> {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../analysis/shaders/mesh");
    p.is_dir().then_some(p)
}

#[test]
#[cfg_attr(not(tombox_fixtures_mesh), ignore = "needs analysis/shaders/mesh fixtures extracted from local title dumps")]
fn gta_mesh_programs_extract_with_mesh_runtime_info() {
    let dir = gta_dir().expect("analysis/shaders/mesh fixtures");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x != "dxbc").unwrap_or(true) {
            continue;
        }
        let ex = extract(&std::fs::read(&p).unwrap()).expect("extract");
        assert_eq!(ex.stage, Stage::Mesh, "{}", p.display());
        let m = ex.gs_info.and_then(|g| g.mesh).expect("mesh info");
        assert_eq!((m.stage, m.topology, m.prim_vectors, m.payload_bytes), (13, 2, 0, 0), "{}", p.display());
        assert!(m.max_vertices as u32 <= m.num_threads[0] && m.max_primitives as u32 <= m.num_threads[0], "{}", p.display());
        let insns = rdna2::disassemble(&ex.isa);
        assert!(insns.iter().any(|i| i.mnemonic == "s_sendmsg" && i.aux.imm & 0xf == 9), "{}", p.display());
        assert!(insns.iter().any(|i| i.mnemonic == "exp" && i.aux.imm == 20), "{}", p.display());
        assert!(!ex.has_root_sig, "{}", p.display());
        let r = translate(&ex);
        if let Err(e) = &r {
            assert!(!e.contains("mesh") && !e.contains("sendmsg") && !e.contains("export"), "{}: {e}", p.display());
        }
        n += 1;
    }
    assert!(n >= 3);
}

const PS: &str = "uint4 main(float4 p : SV_Position, nointerpolation uint2 t : TEXCOORD0) : SV_Target { return uint4(t.x, t.y, 0x77u, 0x88u); }";

unsafe fn render(words: &[u32], groups: u32) -> Result<[u32; 4], String> {
    let t = translate(&ms(words, mesh_info(128, 128, 128, 2), pos_param_outs()))?;
    let msb = dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).map_err(|e| format!("{e}\n{}", t.hlsl))?;
    let psb = dxc::compile(PS, "ps_6_5")?;
    let (mut gpu, kind) = mesh_gpu();
    eprintln!("mesh render on {kind}");
    let d3d12 = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 =
        core::mem::transmute(GetProcAddress(d3d12, b"D3D12SerializeRootSignature\0".as_ptr()));
    let desc = [0u8; 40];
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    assert!(ser(desc.as_ptr(), 1, &mut blob, &mut err) >= 0);
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(gpu.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, vcall!(blob, 3, fn() -> *const u8), vcall!(blob, 4, fn() -> usize), &d3d::IID_ROOTSIG, &mut rs);
    assert!(hr >= 0);
    let s = mesh_stream(rs, &msb, &psb);
    let sd: [u64; 2] = [s.len() as u64, s.as_ptr() as u64];
    let mut dev2: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(gpu.device, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE2, &mut dev2) >= 0, "ID3D12Device2");
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(dev2, 47, fn(*const u64, *const Guid, *mut *mut c_void) -> i32, sd.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        return Err(format!("CreatePipelineState(mesh) hr={:#010x}{}\n{}", hr as u32, gpu.messages(), t.hlsl));
    }
    let pixel = render_pixel(&mut gpu, pso, rs, None, groups);
    for obj in [pso, dev2, rs, blob, err, gpu.list, gpu.alloc, gpu.queue, gpu.fence, gpu.device] {
        d3d::release(obj);
    }
    pixel
}

#[test]
fn gpu_mesh_renders_console_abi_triangle() {
    let got = unsafe { render(&console_ms(67, 1, TRI), 3) }.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got, [2 * 128 + 64, 1, 0x77, 0x88]);
}

#[test]
fn gpu_mesh_divergent_barrier_without_memory_writes() {
    let w = console_ms_b(67, 1, TRI, true);
    let t = translate(&ms(&w, mesh_info(128, 128, 128, 2), pos_param_outs())).expect("translate");
    assert_eq!(t.hlsl.matches("GroupMemoryBarrierWithGroupSync();").count(), 3, "{}", t.hlsl);
    let got = unsafe { render(&w, 3) }.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got, [2 * 128 + 64, 1, 0x77, 0x88]);
}

#[test]
fn mesh_alloc_and_lds_refusals() {
    let mut w = console_ms(67, 1, TRI);
    let i = w.iter().position(|x| *x == sopp(0x10, 9)).unwrap();
    w[i] = sopp(0, 0);
    let e = translate(&ms(&w, mesh_info(128, 128, 128, 2), pos_param_outs())).err().expect("refusal");
    assert!(e.contains("without s_sendmsg GS_ALLOC_REQ"), "{e}");
    let mut mi = mesh_info(128, 128, 128, 2);
    mi.groupshared_bytes = 32768;
    let e = translate(&ms(&console_ms(67, 1, TRI), mi, pos_param_outs())).err().expect("refusal");
    assert!(e.contains("groupshared memory"), "{e}");
}

#[test]
fn mesh_divergent_barrier_with_memory_store_is_refused() {
    let mut w = console_ms_b(67, 1, TRI, true);
    let end = w.len() - 1;
    w.splice(end..end, [0xdc70_8000, 0x007d_0100]);
    let insns = rdna2::disassemble(&w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    assert!(insns.iter().any(|i| i.enc == rdna2::Enc::Flat && i.op == 0x1c), "{:?}", insns.iter().map(|i| i.mnemonic.clone()).collect::<Vec<_>>());
    let e = translate(&ms(&w, mesh_info(128, 128, 128, 2), pos_param_outs())).err().expect("refusal");
    assert!(e.contains("writes LDS or memory"), "{e}");
}

#[test]
fn gpu_mesh_culled_and_empty_outputs_draw_nothing() {
    let clear = [5, 6, 7, 8];
    let culled = unsafe { render(&console_ms(67, 1, TRI | 0x8000_0000), 1) }.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(culled, clear);
    let empty = unsafe { render(&console_ms(0, 0, TRI), 1) }.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(empty, clear);
    let one = unsafe { render(&console_ms(67, 1, TRI), 1) }.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(one, [64, 1, 0x77, 0x88]);
}

fn rts0_blob(flags: u32) -> Vec<u8> {
    let body: Vec<u8> = [1u32, 0, 24, 0, 24, flags].iter().flat_map(|x| x.to_le_bytes()).collect();
    let total = (32 + 4 + 8 + body.len()) as u32;
    let mut v = b"DXBC".to_vec();
    v.extend_from_slice(&[0u8; 16]);
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&total.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&36u32.to_le_bytes());
    v.extend_from_slice(b"RTS0");
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(&body);
    v
}

#[test]
fn warp_rejects_the_console_force_memory_based_abi_root_flag() {
    unsafe {
        let d = warp_device();
        let create = |flags: u32| {
            let b = rts0_blob(flags);
            let mut rs: *mut c_void = core::ptr::null_mut();
            let hr = vcall!(d, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, b.as_ptr(), b.len(), &d3d::IID_ROOTSIG, &mut rs);
            if hr >= 0 {
                vcall!(rs, 2, fn() -> u32);
            }
            hr
        };
        assert!(create(0) >= 0, "control root signature");
        let hr = create(0x0100_0000);
        eprintln!("WARP CreateRootSignature with flag 0x01000000 -> {hr:#010x}");
        assert!(hr < 0, "PC accepted the console-only root signature flag bit 24");
    }
}
