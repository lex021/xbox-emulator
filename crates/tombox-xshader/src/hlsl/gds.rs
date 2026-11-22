use super::{mesh, r, Emitter};
use crate::gds_abi::{
    tb_gds_byte_offset, TB_GDS_HLSL_DECL, TB_GDS_TITLE_DWORDS, TB_GWS_COUNTER_MASK,
    TB_GWS_RESOURCES, TB_GWS_TABLE_DWORD,
};
use crate::rdna2::{Enc, Insn};
use crate::Stage;

pub(super) fn is_gds(ins: &Insn) -> bool {
    ins.raw.first().is_some_and(|w| (w >> 17) & 1 != 0) || ins.mnemonic.starts_with("ds_gws_") || ins.mnemonic == "ds_ordered_count"
}

pub(super) fn gds_header(body: &str) -> Option<&'static str> {
    body.contains("tb_gds.").then_some(TB_GDS_HLSL_DECL)
}

impl Emitter<'_> {
    fn emit_gws_counter(&mut self, ins: &Insn) -> Result<(), String> {
        let name = ins.mnemonic.as_str();
        let [_, x, y, z] = self.ex.isa_preamble;
        let has_control_flow = self.insns.iter().enumerate().any(|(i, op)| {
            if op.mnemonic == "s_endpgm" {
                return i + 1 != self.insns.len();
            }
            (op.enc == Enc::Sopp && !matches!(op.mnemonic.as_str(), "s_nop" | "s_waitcnt"))
                || matches!(op.mnemonic.as_str(), "s_setpc_b64" | "s_swappc_b64")
        });
        let threads = x.saturating_mul(y).saturating_mul(z);
        if self.ex.stage != Stage::Compute || !(1..=1024).contains(&threads) || z > 64
            || mesh::wave_size(&self.insns) != 32 || has_control_flow
        {
            return Err(format!("{name}: the GWS counter model requires a straight-line wave32 compute group"));
        }
        if ins.raw[0] & (1 << 17) == 0 || ins.aux.imm & !0x3f != 0 {
            return Err(format!("{name}: invalid GWS resource encoding"));
        }
        let t = self.tmp;
        self.tmp += 1;
        let offset = ins.aux.imm;
        let resource_mask = TB_GWS_RESOURCES - 1;
        let table = tb_gds_byte_offset(TB_GWS_TABLE_DWORD);
        // Elect one active lane per guest wave independently of host wave size.
        self.line(&format!("uint t{t}id = gtid.x + {x}u * (gtid.y + {y}u * gtid.z); uint t{t}wave = t{t}id >> 5u;"));
        self.line(&format!("if ((t{t}id & 31u) == 0u) tb_gws_counter_first[t{t}wave] = 0xffffffffu;"));
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line(&format!("if ((exec_lo & 1u) != 0u) InterlockedMin(tb_gws_counter_first[t{t}wave], t{t}id);"));
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line(&format!("if (t{t}id == tb_gws_counter_first[t{t}wave]) {{ uint t{t}a = {table}u + 4u * (((m0 >> 16) + {offset}u) & {resource_mask}u);"));
        let value = if name == "ds_gws_sema_v" {
            "1u".to_string()
        } else {
            let data = &ins.src[1];
            self.touch(data);
            format!("({}) & {TB_GWS_COUNTER_MASK}u", r(data)?)
        };
        // The logical count is in the low bits, so concurrent posts can use
        // one atomic add even when the underlying word carries out of the field.
        let atomic = if name == "ds_gws_init" { "InterlockedExchange" } else { "InterlockedAdd" };
        self.line(&format!("uint t{t}old; tb_gds.{atomic}(t{t}a, {value}, t{t}old);"));
        self.line("}");
        self.line("AllMemoryBarrierWithGroupSync();");
        Ok(())
    }

    pub(super) fn emit_gds(&mut self, ins: &Insn) -> Result<(), String> {
        let m = ins.mnemonic.clone();
        if matches!(m.as_str(), "ds_gws_init" | "ds_gws_sema_v" | "ds_gws_sema_br") {
            return self.emit_gws_counter(ins);
        }
        if m.starts_with("ds_gws_") {
            return Err(format!("{m}: global wave sync counts hardware waves across thread groups; no lowering in the one-lane-per-thread model"));
        }
        if m == "ds_ordered_count" {
            return Err("ds_ordered_count: dispatch-ordered GDS append is not lowered".into());
        }
        let off = ins.aux.imm as u32 & 0xffff;
        let limit = TB_GDS_TITLE_DWORDS * 4;
        let t = self.tmp;
        self.tmp += 1;
        let d = ins.dst.first().copied();
        let (a, v0, v1) = (ins.src[0], ins.src[1], ins.src[2]);
        let counter = matches!(m.as_str(), "ds_append" | "ds_consume");
        if counter && mesh::wave_size(&self.insns) == 64 {
            return Err(format!("{m}: a wave64 shader spans two host waves, so count_bits(exec) and the shared pre-op value cannot be formed in one host wave"));
        }
        let rel = if counter {
            format!("{off:#x}u")
        } else {
            self.touch(&a);
            format!("({}) + {off:#x}u", r(&a)?)
        };
        self.line(&format!(
            "uint t{t}r = {rel}; uint t{t}a = ((m0 >> 16) + t{t}r) & ~3u; bool t{t}k = (exec_lo & 1u) != 0u && t{t}r < (m0 & 0xffffu) && t{t}a < {limit}u; uint t{t} = 0u;"
        ));
        let rtn = m.contains("_rtn") || counter || m == "ds_read_b32";
        let op = m.replace("_rtn", "");
        self.touch(&v0);
        self.touch(&v1);
        let (x0, x1) = (r(&v0)?, r(&v1)?);
        let body = match op.as_str() {
            "ds_append" | "ds_consume" => {
                self.uses_wave = true;
                let n = if op == "ds_append" { format!("t{t}n") } else { format!("0u - t{t}n") };
                self.line(&format!("uint4 t{t}b = WaveActiveBallot((exec_lo & 1u) != 0u); uint t{t}h = WaveGetLaneIndex() >> 5; uint t{t}m = (t{t}h == 0u) ? t{t}b.x : ((t{t}h == 1u) ? t{t}b.y : ((t{t}h == 2u) ? t{t}b.z : t{t}b.w)); uint t{t}n = countbits(t{t}m); uint t{t}l = (t{t}h << 5) + firstbitlow(t{t}m);"));
                format!("if (WaveGetLaneIndex() == t{t}l) {{ tb_gds.InterlockedAdd(t{t}a, {n}, t{t}); }}")
            }
            "ds_read_b32" => format!("t{t} = tb_gds.Load(t{t}a);"),
            "ds_write_b32" => format!("tb_gds.Store(t{t}a, {x0});"),
            "ds_add_u32" => format!("tb_gds.InterlockedAdd(t{t}a, {x0}, t{t});"),
            "ds_sub_u32" => format!("tb_gds.InterlockedAdd(t{t}a, 0u - ({x0}), t{t});"),
            "ds_min_u32" => format!("tb_gds.InterlockedMin(t{t}a, {x0}, t{t});"),
            "ds_max_u32" => format!("tb_gds.InterlockedMax(t{t}a, {x0}, t{t});"),
            "ds_min_i32" => format!("int t{t}s; tb_gds.InterlockedMin(t{t}a, asint({x0}), t{t}s); t{t} = asuint(t{t}s);"),
            "ds_max_i32" => format!("int t{t}s; tb_gds.InterlockedMax(t{t}a, asint({x0}), t{t}s); t{t} = asuint(t{t}s);"),
            "ds_and_b32" => format!("tb_gds.InterlockedAnd(t{t}a, {x0}, t{t});"),
            "ds_or_b32" => format!("tb_gds.InterlockedOr(t{t}a, {x0}, t{t});"),
            "ds_xor_b32" => format!("tb_gds.InterlockedXor(t{t}a, {x0}, t{t});"),
            "ds_wrxchg_b32" => format!("tb_gds.InterlockedExchange(t{t}a, {x0}, t{t});"),
            "ds_cmpst_b32" => {
                format!("tb_gds.InterlockedCompareExchange(t{t}a, {x0}, {x1}, t{t});")
            }
            "ds_rsub_u32" | "ds_inc_u32" | "ds_dec_u32" => {
                let x = &x0;
                let c = format!("t{t}c");
                let new = match op.as_str() {
                    "ds_rsub_u32" => format!("({x}) - {c}"),
                    "ds_inc_u32" => format!("(({c} >= ({x})) ? 0u : ({c} + 1u))"),
                    _ => format!("((({c} == 0u) || ({c} > ({x}))) ? ({x}) : ({c} - 1u))"),
                };
                format!(
                    "uint {c} = tb_gds.Load(t{t}a); [allow_uav_condition] for (uint t{t}i = 0u; t{t}i < 4096u; t{t}i++) {{ uint t{t}p; tb_gds.InterlockedCompareExchange(t{t}a, {c}, {new}, t{t}p); if (t{t}p == {c}) break; {c} = t{t}p; }} t{t} = {c};"
                )
            }
            other => return Err(format!("{other} with GDS=1 is not lowered")),
        };
        self.line(&format!("if (t{t}k) {{ {body} }}"));
        if counter {
            self.line(&format!("t{t} = WaveReadLaneAt(t{t}, t{t}l);"));
        }
        if rtn {
            let d = d.ok_or("gds: no vdst")?;
            self.wv(&d, &format!("t{t}"))?;
        }
        Ok(())
    }
}

pub(super) const COOPERATIVE_STATE: &str = r#"
groupshared uint tb_gws_pc[TB_GWS_WAVES];
groupshared uint tb_gws_pending[TB_GWS_WAVES];
groupshared uint tb_gws_ticket[TB_GWS_WAVES];
groupshared uint tb_gws_next_ticket;
groupshared uint tb_gws_progress;
"#;

impl Emitter<'_> {
    pub(super) fn emit_cooperative_gws(&mut self, mode: Mode) -> Result<(), String> {
        let [_, x, y, z] = self.ex.isa_preamble;
        let waves = x.saturating_mul(mode.groups()) / 32;
        if self.ex.stage != Stage::Compute
            || !matches!([x, y, z], [32 | 64, 1, 1])
            || waves < 2
            || mesh::wave_size(&self.insns) != 32
        {
            return Err("cooperative GWS requires at least two wave32s in 32x1x1 or 64x1x1 compute groups".into());
        }
        if mode.phased() && x != 32 {
            return Err("phased GWS requires exactly one wave32 per workgroup".into());
        }
        let instructions = self.insns.clone();
        let barrier = instructions.iter().any(|ins| ins.mnemonic == "ds_gws_barrier");
        if !barrier && !instructions.iter().any(|ins| ins.mnemonic == "ds_gws_sema_p") {
            return Err("cooperative GWS requires a semaphore wait or a barrier".into());
        }
        if barrier && instructions.iter().any(|ins| ins.mnemonic.starts_with("ds_gws_sema_")) {
            return Err("mixing GWS barriers and semaphore operations is unsupported".into());
        }
        let end = instructions.len();
        let end_offset = instructions.last().map_or(0, |ins| ins.offset + ins.dwords * 4);
        let counter_mask = TB_GWS_COUNTER_MASK;
        let resource_mask = TB_GWS_RESOURCES - 1;
        let base = tb_gds_byte_offset(TB_GWS_TABLE_DWORD);
        if mode.phased() {
            self.line("uint tb_gws_wave = gid.x;");
            self.line("uint tb_gws_turn = gid.x;");
            self.line("GroupMemoryBarrierWithGroupSync();");
        } else {
            self.line("uint tb_gws_wave = gtid.x >> 5u;");
            self.line("if (gtid.x < TB_GWS_WAVES) { tb_gws_pc[gtid.x] = 0u; tb_gws_pending[gtid.x] = 0u; tb_gws_ticket[gtid.x] = 0u; }");
            self.line("if (gtid.x == 0u) tb_gws_next_ticket = 0u;");
            self.line("GroupMemoryBarrierWithGroupSync();");
            // Every productive round retires an instruction or wakes a wave.
            let rounds = 2 * end * waves as usize + 1;
            self.line(&format!("[loop] for (uint tb_gws_round = 0u; tb_gws_round < {rounds}u; ++tb_gws_round) {{"));
            self.line("if (gtid.x == 0u) tb_gws_progress = 0u;");
            self.line("GroupMemoryBarrierWithGroupSync();");
            self.line("[loop] for (uint tb_gws_turn = 0u; tb_gws_turn < TB_GWS_WAVES; ++tb_gws_turn) {");
        }
        self.line("if (gtid.x == 0u) {");
        self.line("tb_gws_first = 0xffffffffu;");
        if !barrier {
            self.line("uint wait_addr = tb_gws_pending[tb_gws_turn];");
            self.line("if (wait_addr != 0u) {");
            self.line("uint earlier = 0u;");
            self.line("for (uint p = 0u; p < TB_GWS_WAVES; ++p) earlier |= (tb_gws_pending[p] == wait_addr && tb_gws_ticket[p] < tb_gws_ticket[tb_gws_turn]) ? 1u : 0u;");
            self.line("uint count;");
            self.line("tb_gds.InterlockedOr(wait_addr, 0u, count);");
            self.line(&format!("if (earlier == 0u && (count & {counter_mask}u) != 0u) {{"));
            self.line("uint old; tb_gds.InterlockedAdd(wait_addr, 0xffffffffu, old);");
            self.line("tb_gws_pending[tb_gws_turn] = 0u; tb_gws_progress = 1u;");
            self.line("} }");
        }
        self.line("}");
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line("if (tb_gws_wave == tb_gws_turn && (exec_lo & 1u) != 0u) InterlockedMin(tb_gws_first, gtid.x);");
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line("uint tb_gws_step = tb_gws_pc[tb_gws_turn]; uint tb_gws_waiting = tb_gws_pending[tb_gws_turn];");
        // All lanes must finish reading the shared PC before its owner can change it.
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line("if (tb_gws_wave == tb_gws_turn && tb_gws_waiting == 0u) {");
        for (i, ins) in instructions.iter().enumerate() {
            let next = i + 1;
            self.line(&format!("if (tb_gws_step == {i}u) {{"));
            let mut target = format!("{next}u");
            match ins.mnemonic.as_str() {
                "s_endpgm" => target = format!("{end}u"),
                "s_branch" | "s_cbranch_execz" | "s_cbranch_execnz" => {
                    let offset = ins.offset as i64 + 4 + ins.aux.imm * 4;
                    let branch = if offset == end_offset as i64 {
                        end
                    } else {
                        instructions.iter().position(|op| op.offset as i64 == offset)
                            .ok_or_else(|| format!("invalid cooperative GWS target {offset:#x}"))?
                    };
                    if branch <= i {
                        return Err("cooperative GWS supports forward branches only".into());
                    }
                    target = match ins.mnemonic.as_str() {
                        "s_cbranch_execz" => format!("tb_gws_first == 0xffffffffu ? {branch}u : {next}u"),
                        "s_cbranch_execnz" => format!("tb_gws_first != 0xffffffffu ? {branch}u : {next}u"),
                        _ => format!("{branch}u"),
                    };
                }
                "ds_gws_init" | "ds_gws_sema_v" | "ds_gws_sema_br" | "ds_gws_sema_p"
                | "ds_gws_sema_release_all" | "ds_gws_barrier" => {
                    if ins.raw[0] & (1 << 17) == 0 || ins.aux.imm & !0x3f != 0 {
                        return Err("invalid cooperative GWS resource encoding".into());
                    }
                    let offset = ins.aux.imm;
                    self.line(&format!("if (gtid.x == tb_gws_first) {{ uint addr = {base}u + 4u * (((m0 >> 16) + {offset}u) & {resource_mask}u);"));
                    match ins.mnemonic.as_str() {
                        "ds_gws_barrier" => {
                            let data = &ins.src[0];
                            self.touch(data);
                            // The counter holds the number of arrivals before the releasing wave.
                            self.line("uint count; tb_gds.InterlockedOr(addr, 0u, count);");
                            self.line(&format!("if ((count & {counter_mask}u) == 0u) {{"));
                            self.line(&format!("uint old; tb_gds.InterlockedExchange(addr, ({}) & {counter_mask}u, old);", r(data)?));
                            self.line("for (uint p = 0u; p < TB_GWS_WAVES; ++p) { if (tb_gws_pending[p] == addr) tb_gws_pending[p] = 0u; }");
                            self.line("} else {");
                            self.line("uint old; tb_gds.InterlockedAdd(addr, 0xffffffffu, old);");
                            self.line("tb_gws_pending[tb_gws_turn] = addr;");
                            self.line("}");
                        }
                        "ds_gws_sema_p" => {
                            self.line("uint count; tb_gds.InterlockedOr(addr, 0u, count);");
                            self.line("uint older = 0u;");
                            self.line("for (uint p = 0u; p < TB_GWS_WAVES; ++p) older |= (tb_gws_pending[p] == addr) ? 1u : 0u;");
                            self.line(&format!("if (older == 0u && (count & {counter_mask}u) != 0u) {{"));
                            self.line("uint old; tb_gds.InterlockedAdd(addr, 0xffffffffu, old);");
                            self.line("} else { tb_gws_pending[tb_gws_turn] = addr; tb_gws_ticket[tb_gws_turn] = tb_gws_next_ticket++; }");
                        }
                        "ds_gws_sema_release_all" => {
                            // A previous post may not have been consumed yet. Apply its
                            // credits to the existing queue before releasing the rest.
                            self.line("uint waiting = 0u;");
                            self.line("for (uint p = 0u; p < TB_GWS_WAVES; ++p) { if (tb_gws_pending[p] == addr) { tb_gws_pending[p] = 0u; ++waiting; } }");
                            self.line("uint count; tb_gds.InterlockedOr(addr, 0u, count);");
                            self.line(&format!("uint used = min(waiting, count & {counter_mask}u);"));
                            self.line("if (used != 0u) { uint old; tb_gds.InterlockedAdd(addr, 0u - used, old); }");
                        }
                        "ds_gws_sema_v" => self.line("uint old; tb_gds.InterlockedAdd(addr, 1u, old);"),
                        name => {
                            let data = &ins.src[1];
                            self.touch(data);
                            let atomic = if name == "ds_gws_init" { "InterlockedExchange" } else { "InterlockedAdd" };
                            self.line(&format!("uint old; tb_gds.{atomic}(addr, ({}) & {counter_mask}u, old);", r(data)?));
                        }
                    }
                    self.line("}");
                }
                "s_waitcnt" | "s_nop" | "v_nop" => {}
                "s_mov_b32" | "v_mov_b32" | "v_lshlrev_b32" | "v_add_nc_u32" | "s_load_dwordx4" | "buffer_store_dword" => {
                    for operand in ins.src.iter().chain(&ins.dst) {
                        self.touch(operand);
                    }
                    self.emit_insn(i, ins, false, None)?;
                }
                op if op.starts_with("v_cmpx_") => {
                    for operand in ins.src.iter().chain(&ins.dst) {
                        self.touch(operand);
                    }
                    self.emit_insn(i, ins, false, None)?;
                }
                op => return Err(format!("{op} is outside the cooperative GWS instruction subset")),
            }
            self.line(&format!("if ((gtid.x & 31u) == 0u) {{ tb_gws_pc[tb_gws_turn] = {target}; tb_gws_progress = 1u; }} }}"));
        }
        self.line("}");
        self.line("AllMemoryBarrierWithGroupSync();");
        if !mode.phased() {
            self.line("}");
            self.line("if (tb_gws_progress == 0u) break;");
            self.line("GroupMemoryBarrierWithGroupSync();");
            self.line("}");
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Fused(u32),
    Phased(u32),
}

impl Mode {
    pub(super) fn groups(self) -> u32 {
        match self {
            Self::Fused(x) | Self::Phased(x) => x,
        }
    }

    pub(super) fn phased(self) -> bool {
        matches!(self, Self::Phased(_))
    }
}

// The fixed per-lane stride covers 256 VGPRs, 128 SGPRs and scalar control registers.
const CHECKPOINT_DWORDS: u32 = 400;

pub(super) fn scratch_bytes(groups: u32) -> u32 {
    4 * (1 + 3 * groups + groups * 32 * CHECKPOINT_DWORDS)
}

impl Emitter<'_> {
    /// One physical workgroup advances by one instruction per phase. Only the group
    /// named in the immutable root constant may touch shared dispatch state.
    pub(super) fn checkpoint_phased_gws(&self, groups: u32) -> Result<String, String> {
        use core::fmt::Write;
        let registers = ["exec_lo", "exec_hi", "vcc_lo", "vcc_hi", "scc", "m0"]
            .into_iter()
            .chain(self.regs.iter().map(String::as_str))
            .collect::<Vec<_>>();
        if self.vreg_array || registers.len() > CHECKPOINT_DWORDS as usize {
            return Err("phased GWS register checkpoint exceeds its supported layout".into());
        }
        let mut h = String::new();
        let _ = writeln!(h, "if (gid.x != tb_gws_selected || gid.x >= {groups}u) return;");
        let _ = writeln!(h, "uint tb_gws_slot = 4u * ({}u + (gid.x * 32u + gtid.x) * {CHECKPOINT_DWORDS}u);", 1 + 3 * groups);
        h.push_str("if (gtid.x == 0u) tb_gws_next_ticket = tb_gws_scratch.Load(0u);\n");
        let arrays = ["tb_gws_pc", "tb_gws_pending", "tb_gws_ticket"];
        let _ = writeln!(h, "if (gtid.x < {groups}u) {{");
        for (i, array) in arrays.iter().enumerate() {
            let offset = 4 * (1 + i as u32 * groups);
            let _ = writeln!(h, "{array}[gtid.x] = tb_gws_scratch.Load({offset}u + 4u * gtid.x);");
        }
        h.push_str("}\nif (tb_gws_scratch.Load(4u + 4u * gid.x) != 0u) {\n");
        for (i, register) in registers.iter().enumerate() {
            let _ = writeln!(h, "{register} = tb_gws_scratch.Load(tb_gws_slot + {}u);", 4 * i);
        }
        h.push_str("}\n");
        h.push_str(&self.body);
        h.push_str("if (gtid.x == 0u) tb_gws_scratch.Store(0u, tb_gws_next_ticket);\n");
        let _ = writeln!(h, "if (gtid.x < {groups}u) {{");
        for (i, array) in arrays.iter().enumerate() {
            let offset = 4 * (1 + i as u32 * groups);
            let _ = writeln!(h, "tb_gws_scratch.Store({offset}u + 4u * gtid.x, {array}[gtid.x]);");
        }
        h.push_str("}\n");
        for (i, register) in registers.iter().enumerate() {
            let _ = writeln!(h, "tb_gws_scratch.Store(tb_gws_slot + {}u, {register});", 4 * i);
        }
        Ok(h)
    }
}
