#[path = "../../../tombox-xshader/tests/support/mesh_gpu.rs"]
mod mesh_gpu;
#[path = "../../../tombox-xshader/tests/support/mesh_isa.rs"]
mod mesh_isa;

use core::ffi::c_void;
use crate::d3d::{self, Gpu, Guid};
use crate::vcall;
use mesh_gpu::{mesh_gpu, mesh_stream, render_pixel};
use mesh_isa::{console_ms, exp, vop1, vop2, S_ENDPGM, TRI};
use tombox_xshader::{dxc, Dxbc};

fn shader_container(version: u32, words: &[u32], info: &[u8], outputs: &[u8]) -> Vec<u8> {
    let isa: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut fssm = vec![0; 0xe0];
    fssm[..4].copy_from_slice(b"FSSM");
    fssm[0xc0..0xc4].copy_from_slice(&0xe0u32.to_le_bytes());
    fssm[0xc8..0xcc].copy_from_slice(&(isa.len() as u32).to_le_bytes());
    fssm.extend(isa);

    let mut dxil = vec![0; 24];
    dxil[..4].copy_from_slice(&version.to_le_bytes());
    dxil[8..12].copy_from_slice(b"DXIL");
    dxil.extend(b"XDAT");
    dxil.extend(1u32.to_le_bytes());
    dxil.extend(miniz_oxide::deflate::compress_to_vec_zlib(&fssm, 6));

    let chunks = [(b"DXIL", dxil.as_slice()), (b"PSV0", info), (b"OSG1", outputs)];
    let mut dxbc = vec![0; 32 + 4 * chunks.len()];
    dxbc[..4].copy_from_slice(b"DXBC");
    dxbc[20..24].copy_from_slice(&1u32.to_le_bytes());
    dxbc[28..32].copy_from_slice(&(chunks.len() as u32).to_le_bytes());
    for (i, (name, body)) in chunks.into_iter().enumerate() {
        let off = dxbc.len() as u32;
        dxbc[32 + i * 4..36 + i * 4].copy_from_slice(&off.to_le_bytes());
        dxbc.extend(name);
        dxbc.extend((body.len() as u32).to_le_bytes());
        dxbc.extend(body);
        dxbc.resize(dxbc.len().next_multiple_of(4), 0);
    }
    let size = dxbc.len() as u32;
    dxbc[24..28].copy_from_slice(&size.to_le_bytes());
    dxbc
}

fn signature(name: &[u8], semantic: u32, component: u32) -> Vec<u8> {
    let mut bytes: Vec<_> = [1u32, 8, 0, 40, 0, semantic, component, 0, 15, 0]
        .into_iter().flat_map(u32::to_le_bytes).collect();
    bytes.extend(name);
    bytes.push(0);
    bytes
}

fn load_cbv(base: u32, offset: u32) -> [u32; 2] {
    // s_buffer_load_dword s28, s[base:base+3], offset
    [0xf420_0700 | (base >> 1), 0xfa00_0000 | offset]
}

fn mesh_shader() -> Vec<u8> {
    let mut words = console_ms(67, 1, TRI);
    let position_export = words.len() - 7;
    // Load the horizontal shift and add it to the exported vertex X.
    words.splice(position_export..position_export, load_cbv(16, 0).into_iter().chain([vop2(0x03, 1, 28, 1)]));
    let mut psv = vec![0; 60];
    psv[..4].copy_from_slice(&52u32.to_le_bytes());
    psv[16..18].copy_from_slice(&128u16.to_le_bytes());
    psv[18..20].copy_from_slice(&128u16.to_le_bytes());
    psv[28] = 13;
    psv[31] = 2;
    psv[40..44].copy_from_slice(&128u32.to_le_bytes());
    psv[44..48].copy_from_slice(&1u32.to_le_bytes());
    psv[48..52].copy_from_slice(&1u32.to_le_bytes());
    shader_container(0x000d_0065, &words, &psv, &signature(b"SV_Position", 1, 3))
}

fn pixel_shader() -> Vec<u8> {
    let mut words = load_cbv(8, 4).to_vec();
    words.extend([vop1(1, 0, 28), vop1(1, 1, 129), vop1(1, 2, 130), vop1(1, 3, 131)]);
    words.extend(exp(0, 15, true, [0, 1, 2, 3]));
    words.push(S_ENDPGM);
    shader_container(0x0065, &words, &[0; 8], &signature(b"SV_Target", 64, 1))
}

unsafe fn root_signature(device: *mut c_void, register: u32, space: u32) -> *mut c_void {
    let text = format!("[RootSignature(\"CBV(b{register}, space={space})\")] [numthreads(1,1,1)] void main() {{}}");
    let shader = dxc::compile(&text, "cs_6_0").expect("compile external root signature");
    let dxbc = Dxbc::parse(&shader).unwrap();
    let rts = dxbc.chunk(b"RTS0").expect("serialized root signature");
    let layout = crate::device_wrap::wrap_root_signature(&shader[rts.offset..rts.offset + rts.size]);
    let mut root = core::ptr::null_mut();
    let hr = vcall!(device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32,
        0, layout.as_ptr(), layout.len(), &d3d::IID_ROOTSIG, &mut root);
    assert!(hr >= 0 && !root.is_null(), "CreateRootSignature hr={hr:#010x}");
    root
}

unsafe fn pipeline(device: *mut c_void, ms: &[u8], ps: &[u8], root: *mut c_void) -> (i32, *mut c_void) {
    let stream = mesh_stream(root, ms, ps);
    let desc = [stream.len() as u64, stream.as_ptr() as u64];
    let mut pso = core::ptr::null_mut();
    let hr = vcall!(device, crate::console_slots_mesh::DEV_CREATE_PIPELINE_STATE,
        fn(*const [u64; 2], *const Guid, *mut *mut c_void) -> i32, &desc, &d3d::IID_PSO, &mut pso);
    (hr, pso)
}

#[test]
#[ignore = "requires DXC and D3D12 Mesh Shader Tier 1"]
fn mesh_pipeline_reads_external_root_buffers() {
    assert!(dxc::available(), "DXC is required for mesh shaders");
    unsafe {
        let (mut gpu, kind) = mesh_gpu();
        vcall!(gpu.device, 1, fn() -> u32);
        let device = crate::device_wrap::wrap_device(gpu.device);
        let ms = mesh_shader();
        let ps = pixel_shader();
        for shader in [&ms, &ps] {
            let extracted = tombox_xshader::extract(shader).expect("extract synthetic shader");
            let error = tombox_xshader::hlsl::translate(&extracted).err().expect("external root required");
            assert!(error.contains("missing root signature"), "{error}");
        }
        let (hr, pso) = pipeline(device, &ms, &ps, core::ptr::null_mut());
        assert_eq!(hr, crate::mesh_pso::E_INVALIDARG);
        assert!(pso.is_null());

        let first = root_signature(device, 7, 2);
        let second = root_signature(device, 3, 5);
        let (hr, first_pso) = pipeline(device, &ms, &ps, first);
        assert!(hr >= 0 && !first_pso.is_null(), "first pipeline on {kind}: hr={hr:#010x}");
        let (hr, second_pso) = pipeline(device, &ms, &ps, second);
        assert!(hr >= 0 && !second_pso.is_null(), "second pipeline on {kind}: hr={hr:#010x}");

        let flat = gpu.buffer(256, 2, d3d::GENERIC_READ, false).unwrap();
        let tinted = gpu.buffer(256, 2, d3d::GENERIC_READ, false).unwrap();
        let shifted = gpu.buffer(256, 2, d3d::GENERIC_READ, false).unwrap();
        let cases = [
            (first_pso, first, flat, 0.0f32, 17u32, [17, 1, 2, 3]),
            (second_pso, second, tinted, 0.0, 29, [29, 1, 2, 3]),
            (first_pso, first, shifted, 4.0, 17, [5, 6, 7, 8]),
        ];
        for (pso, root, buf, shift, color, expected) in cases {
            let data: Vec<_> = [shift.to_bits(), color].into_iter().flat_map(u32::to_le_bytes).collect();
            assert!(Gpu::map_write(buf, &data));
            let pixel = render_pixel(&mut gpu, pso, root, Some(Gpu::gpu_va(buf)), 1).unwrap();
            assert_eq!(pixel, expected, "{kind}, shift={shift}, color={color}");
        }
        for obj in [flat, tinted, shifted, first_pso, second_pso, first, second, device,
                    gpu.list, gpu.alloc, gpu.queue, gpu.fence, gpu.device] {
            d3d::release(obj);
        }
    }
}
