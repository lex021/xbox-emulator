use super::{
    get_variant, indirect_stride, mask, slot, Bind, Create, Dispatch, Guid, Indirect, IID_PSO,
};
use crate::gws_scratch::{self, Resource, COPY_DEST, UAV};
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::OnceLock;
use tombox_xshader::gds_abi::TB_GWS_MAX_GROUPS;

const COPY_SOURCE: u32 = 0x800;
const INDIRECT_ARGUMENT: u32 = 0x200;
const MAX_COMMANDS: u32 = 16;
const FUSED_VARIANTS: u32 = 4;

const MAX_PHASED_RECORDS: u32 = 4;
const IID_DEVICE: Guid = Guid {
    d1: 0x189819f1,
    d2: 0x1db6,
    d3: 0x4b57,
    d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7],
};
const IID_ROOT: Guid = Guid {
    d1: 0xc54a6b66,
    d2: 0x72df,
    d3: 0x4ee8,
    d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14],
};

const ROUTE: &str = r#"
    RWByteAddressBuffer args : register(u0);
    cbuffer Control : register(b0) { uint stride; uint maxCommands; uint hasCount; uint phasedMin; uint maxGroups; };
    [RootSignature("UAV(u0), RootConstants(num32BitConstants=5, b0)")]
    [numthreads(16,1,1)]
    void main(uint lane : SV_GroupIndex) {
        if (lane >= maxCommands) return;
        uint3 guest = args.Load3(4 + lane * stride);
        bool active = hasCount == 0 || lane < args.Load(0);
        bool linearGrid = guest.y == 1 && guest.z == 1;
        uint groups = all(guest <= maxGroups) ? guest.x * guest.y * guest.z : 0;
        bool phased = active && phasedMin != 0 && groups != 0 && groups <= maxGroups &&
                      (!linearGrid || guest.x >= phasedMin);
        uint base = 4 + (maxCommands + lane * 4) * stride;
        for (uint x = 1; x <= 4; ++x) {
            uint enabled = active && !phased && linearGrid && guest.x == x ? 1 : 0;
            args.Store3(base + (x - 1) * stride, uint3(enabled, enabled, enabled));
        }
        uint shapeOffset = 4 + maxCommands * 5 * stride + lane * 4;
        args.Store(shapeOffset, phased ? ((guest.y << 24) | (guest.x << 16) | (groups << 8)) : 0);
    }
"#;

unsafe fn router(list: *mut c_void) -> Option<(Resource, Resource)> {
    static CODE: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    let code = CODE
        .get_or_init(|| tombox_xshader::dxc::compile(ROUTE, "cs_6_0"))
        .as_ref()
        .ok()?;
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(list, 7)) };
    let mut device = null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut device) } < 0 || device.is_null() {
        return None;
    }
    let _device = Resource(device as usize);
    let create_root: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const u8,
        usize,
        *const Guid,
        *mut *mut c_void,
    ) -> i32 = unsafe { core::mem::transmute(slot(device, 16)) };
    let mut root = null_mut();
    if unsafe { create_root(device, 0, code.as_ptr(), code.len(), &IID_ROOT, &mut root) } < 0
        || root.is_null()
    {
        return None;
    }
    let root = Resource(root as usize);
    let mut desc = [0; 56];
    desc[..8].copy_from_slice(&(root.0 as u64).to_le_bytes());
    desc[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    desc[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let create_pso: Create = unsafe { core::mem::transmute(slot(device, 11)) };
    let mut pso = null_mut();
    if unsafe { create_pso(device, desc.as_ptr(), &IID_PSO, &mut pso) } < 0 || pso.is_null() {
        return None;
    }
    Some((root, Resource(pso as usize)))
}

struct Source {
    resource: usize,
    offset: u64,
    len: u64,
    state: u32,
}

impl Source {
    unsafe fn new(resource: *mut c_void, offset: u64, len: u64) -> Option<Self> {
        if resource.is_null() || offset % 4 != 0 {
            return None;
        }
        let state = crate::gpu_va::state(resource as usize);
        let width = crate::placed_offset::with_true_desc(|| unsafe {
            crate::placed_offset::buffer_width(resource)
        })?;
        if state & INDIRECT_ARGUMENT == 0 || offset.checked_add(len)? > width {
            return None;
        }
        // A rebased placement can expose wider backing storage than the guest buffer.
        let offset = match crate::placed_offset::of(resource as usize) {
            Some(placement) if offset + len <= placement.width => {
                offset.checked_add(placement.bytes)?
            }
            Some(_) => return None,
            None => offset,
        };
        (offset.checked_add(len)? <= width).then_some(Self {
            resource: resource as usize,
            offset,
            len,
            state,
        })
    }

    unsafe fn copy(&self, list: *mut c_void, scratch: usize, offset: u64) {
        let copy: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) =
            unsafe { core::mem::transmute(slot(list, 15)) };
        unsafe {
            if self.state & COPY_SOURCE == 0 {
                crate::fill::barrier_pub(list, self.resource, self.state, COPY_SOURCE);
            }
            copy(list, scratch, offset, self.resource, self.offset, self.len);
            if self.state & COPY_SOURCE == 0 {
                crate::fill::barrier_pub(list, self.resource, COPY_SOURCE, self.state);
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct RoutedPhases {
    rows: Vec<super::Phase>,
}

impl RoutedPhases {
    pub(super) fn owns_scratch(&self, resource: usize) -> bool {
        self.rows.iter().any(|phase| phase.scratch == resource)
    }

    unsafe fn execute_row(&self, row: usize, list: *mut c_void, bind: Bind) {
        if let Some(&phase) = self.rows.get(row) {
            unsafe { phase.run(list, bind) };
        }
    }

    pub(super) unsafe fn resume(&self, list: *mut c_void, bind: Bind) {
        for &phase in &self.rows {
            unsafe { phase.run(list, bind) };
        }
    }
}

pub(super) unsafe fn run(
    list: *mut c_void,
    carrier: *mut c_void,
    signature: *mut c_void,
    count: u32,
    args: *mut c_void,
    offset: u64,
    count_buf: *mut c_void,
    count_offset: u64,
) -> Result<(), &'static str> {
    if !super::phased_enabled() || !(1..=MAX_COMMANDS).contains(&count) {
        return Err("this path requires TOMBOX_GWS_PHASED and a maximum count from 1 to 16");
    }
    if !matches!(unsafe { crate::fill::list_type(list) }, 0 | 2)
        || crate::cl2_state::predication_of(list).is_some()
        || crate::cl2_state::compute_root(list).is_none()
    {
        return Err("the list needs a compute root signature and no active predication");
    }
    let stride = unsafe { indirect_stride(signature) }.ok_or("dispatch-only signature required")?;
    let shape_offset = stride
        .checked_mul(count * (1 + FUSED_VARIANTS))
        .and_then(|n| n.checked_add(4))
        .filter(|&n| n <= 0x10_0000 - count * 4)
        .ok_or("unsupported command stride")?;
    let size = shape_offset + count * 4;
    let arguments = unsafe { Source::new(args, offset, u64::from(count * stride)) }
        .ok_or("invalid or untracked argument buffer")?;
    let count_source = if count_buf.is_null() {
        None
    } else {
        Some(
            unsafe { Source::new(count_buf, count_offset, 4) }
                .ok_or("invalid or untracked count buffer")?,
        )
    };
    let variants = unsafe { mask(carrier) };
    let (root, route) =
        unsafe { router(list) }.ok_or("could not create the GPU argument router")?;
    let (scratch, va) = unsafe { gws_scratch::begin(list, route.0 as *mut c_void, size) }
        .ok_or("could not allocate indirect scratch")?;
    let mut routed = RoutedPhases { rows: Vec::new() };
    let mut resume_mode = 0;
    if count <= MAX_PHASED_RECORDS {
        let variant = unsafe { get_variant(carrier, super::PHASED_VARIANT) };
        if !variant.is_null() {
            let _variant = Resource(variant as usize);
            let allocated = (0..count)
                .map(|_| {
                    let (phase, mode) = unsafe {
                        super::allocate_phase(list, variant, [TB_GWS_MAX_GROUPS, 1, 1], true)
                    }?;
                    resume_mode = mode;
                    Some(phase)
                })
                .collect::<Option<Vec<_>>>();
            routed.rows = allocated.unwrap_or_default();
        }
    }
    let phased_min = if routed.rows.is_empty() {
        resume_mode = 0;
        0
    } else if super::resume::enabled() {
        1
    } else {
        FUSED_VARIANTS + 1
    };
    let candidates: Vec<_> = (1..=FUSED_VARIANTS)
        .filter(|x| variants & (1 << x) != 0 && (phased_min == 0 || *x < phased_min))
        .filter_map(|x| {
            let variant = unsafe { get_variant(carrier, x) };
            (!variant.is_null()).then_some((x, Resource(variant as usize)))
        })
        .collect();
    if candidates.is_empty() && routed.rows.is_empty() {
        return Err("no compatible GWS variants are available");
    }
    let bind: Bind = unsafe { core::mem::transmute(slot(list, 25)) };
    let bind_root: Bind = unsafe { core::mem::transmute(slot(list, 29)) };
    let bind_uav: unsafe extern "system" fn(*mut c_void, u32, u64) =
        unsafe { core::mem::transmute(slot(list, 41)) };
    let constant: unsafe extern "system" fn(*mut c_void, u32, u32, u32) =
        unsafe { core::mem::transmute(slot(list, 33)) };
    let dispatch: Dispatch = unsafe { core::mem::transmute(slot(list, 14)) };
    let indirect: Indirect = unsafe { core::mem::transmute(slot(list, 59)) };
    let barrier: unsafe extern "system" fn(*mut c_void, u32, *const u64) =
        unsafe { core::mem::transmute(slot(list, 26)) };
    let gds_ordering = [2u64, 0, 0, 0];
    unsafe {
        arguments.copy(list, scratch, 4);
        if let Some(source) = &count_source {
            source.copy(list, scratch, 0);
        }
        crate::fill::barrier_pub(list, scratch, COPY_DEST, UAV);
        bind_root(list, root.0 as *mut c_void);
        bind_uav(list, 0, va);
        constant(list, 1, stride, 0);
        constant(list, 1, count, 1);
        constant(list, 1, u32::from(count_source.is_some()), 2);
        constant(list, 1, phased_min, 3);
        constant(list, 1, TB_GWS_MAX_GROUPS, 4);
        bind(list, route.0 as *mut c_void);
        dispatch(list, 1, 1, 1);
        if routed.rows.is_empty() {
            crate::fill::barrier_pub(list, scratch, UAV, INDIRECT_ARGUMENT);
        } else {
            let copy: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) =
                core::mem::transmute(slot(list, 15));
            crate::fill::barrier_pub(list, scratch, UAV, COPY_SOURCE);
            for (row, phase) in routed.rows.iter().enumerate() {
                copy(list, phase.scratch, 0, scratch, u64::from(shape_offset) + row as u64 * 4, 4);
            }
            crate::fill::barrier_pub(list, scratch, COPY_SOURCE, INDIRECT_ARGUMENT);
        }
        crate::cl2_state::reapply_bindings(list);
        use super::resume::Kind;
        let resume_kind = Kind::from_mode(resume_mode)
            .filter(|kind| super::resume::enabled() && (*kind != Kind::Semaphore || count == 1));
        if resume_kind == Some(Kind::OrderedSemaphore) {
            super::resume::before_ordered_wait(list, bind);
        }
        for row in 0..count {
            if row != 0 {
                barrier(list, 1, gds_ordering.as_ptr());
            }
            for (x, variant) in &candidates {
                bind(list, variant.0 as *mut c_void);
                let offset = 4 + u64::from((count + row * FUSED_VARIANTS + x - 1) * stride);
                indirect(
                    list,
                    signature,
                    1,
                    scratch as *mut c_void,
                    offset,
                    null_mut(),
                    0,
                );
            }
            routed.execute_row(row as usize, list, bind);
            if resume_kind.is_some_and(Kind::is_barrier) {
                for earlier in 0..row {
                    routed.execute_row(earlier as usize, list, bind);
                }
            }
            if resume_kind == Some(Kind::BarrierAndRelease) && row + 1 < count {
                super::resume::between_release_rows(list, bind);
            }
        }
        crate::fill::barrier_pub(list, scratch, INDIRECT_ARGUMENT, COPY_DEST);
        if let Some(kind) = resume_kind {
            super::resume::remember_indirect(list, routed, kind, bind);
        } else {
            super::resume::forget(list);
        }
        bind(list, carrier);
    }
    let mut objects = vec![root, route];
    objects.extend(candidates.into_iter().map(|(_, pso)| pso));
    gws_scratch::retain(list, objects);
    Ok(())
}
