pub const TB_GDS_SPACE: u32 = 4_000_002;
pub const TB_GDS_REGISTER: u32 = 0;
pub const TB_GDS_BYTES: u64 = 0x1_0000;
pub const TB_GDS_TITLE_DWORDS: u32 = 0x3f00;
pub const TB_GDS_HLSL_DECL: &str = "RWByteAddressBuffer tb_gds : register(u0, space4000002);";
pub const TB_GDS_BYTES_PER_TITLE_DWORD: u32 = 4;
pub const TB_GDS_OA_TABLE_DWORD: u32 = TB_GDS_TITLE_DWORDS;
pub const TB_GDS_OA_COUNTERS: u32 = 8;
pub const TB_GDS_OA_STRIDE_DWORDS: u32 = 4;
pub const TB_GDS_OA_FIELD_COUNTER_DWORD: u32 = 0;
pub const TB_GDS_OA_FIELD_CNTL: u32 = 1;
pub const TB_GDS_OA_FIELD_STAGE: u32 = 2;
pub const TB_GDS_OA_FIELD_WAVE_RELEASE: u32 = 3;
pub const TB_GWS_TABLE_DWORD: u32 = TB_GDS_OA_TABLE_DWORD + TB_GDS_OA_COUNTERS * TB_GDS_OA_STRIDE_DWORDS;
pub const TB_GWS_RESOURCES: u32 = 64;
pub const TB_GWS_RELEASE_DWORD: u32 = TB_GWS_TABLE_DWORD + TB_GWS_RESOURCES;
pub const TB_GWS_BARRIER_DWORD: u32 = TB_GWS_RELEASE_DWORD + TB_GWS_RESOURCES;
// GFX103 GDS_GWS_RESOURCE.COUNTER occupies bits 1..=12. Our private
// buffer stores the count starting at bit zero.
pub const TB_GWS_COUNTER_MASK: u32 = (1 << 12) - 1;
pub const TB_GWS_SCRATCH_HLSL_DECL: &str = "RWByteAddressBuffer tb_gws_scratch : register(u1, space4000002);";
pub const TB_GWS_CONTROL_HLSL_DECL: &str = "cbuffer TbGwsControl : register(b0, space4000002) { uint tb_gws_control; };";

pub const TB_GWS_MAX_GROUPS: u32 = 16;
// The per-lane stride covers 256 VGPRs, 128 SGPRs and scalar control registers.
pub const TB_GWS_CHECKPOINT_DWORDS: u32 = 400;

pub fn tb_gws_group_count(groups: [u32; 3]) -> Option<u32> {
    let [x, y, z] = groups;
    x.checked_mul(y)
        .and_then(|xy| xy.checked_mul(z))
        .filter(|&count| (1..=TB_GWS_MAX_GROUPS).contains(&count))
}

pub const fn tb_gws_scratch_bytes(waves: u32) -> u32 {
    4 * (2 + 4 * waves + waves * 32 * TB_GWS_CHECKPOINT_DWORDS)
}

pub const fn tb_gws_phase_word(groups: [u32; 3], waves: u32, selected: u32) -> u32 {
    selected | (waves << 8) | (groups[0] << 16) | (groups[1] << 24)
}

pub const fn tb_gds_byte_offset(title_dword: u32) -> u32 {
    title_dword * TB_GDS_BYTES_PER_TITLE_DWORD
}

pub const fn tb_gds_oa_byte_offset(counter: u32, field: u32) -> u32 {
    tb_gds_byte_offset(TB_GDS_OA_TABLE_DWORD + counter * TB_GDS_OA_STRIDE_DWORDS + field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_append_table_sits_after_title_gds() {
        assert_eq!(tb_gds_byte_offset(5), 20);
        assert_eq!(tb_gds_oa_byte_offset(0, TB_GDS_OA_FIELD_COUNTER_DWORD), TB_GDS_TITLE_DWORDS * 4);
        let end = tb_gds_oa_byte_offset(TB_GDS_OA_COUNTERS, 0) as u64;
        assert_eq!(end, tb_gds_byte_offset(TB_GWS_TABLE_DWORD) as u64);
        assert!(tb_gds_byte_offset(TB_GWS_BARRIER_DWORD + TB_GWS_RESOURCES) as u64 <= TB_GDS_BYTES);
        assert_eq!(tb_gds_oa_byte_offset(1, TB_GDS_OA_FIELD_WAVE_RELEASE) - tb_gds_oa_byte_offset(0, 0), 4 * (TB_GDS_OA_STRIDE_DWORDS + 3));
    }

    #[test]
    fn hlsl_decl_matches_binding_constants() {
        assert_eq!(TB_GDS_HLSL_DECL, format!("RWByteAddressBuffer tb_gds : register(u{TB_GDS_REGISTER}, space{TB_GDS_SPACE});"));
        assert!(TB_GDS_TITLE_DWORDS as u64 * 4 <= TB_GDS_BYTES);
        assert_ne!(TB_GDS_SPACE, crate::hlsl::TB_MIP_SPACE);
        assert_ne!(TB_GDS_SPACE, crate::hlsl::TB_GMEM_SPACE);
    }
}
