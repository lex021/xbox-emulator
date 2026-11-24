use super::{Emitter, BINDLESS};
use crate::meta::RootParamKind;

impl Emitter<'_> {
    pub(super) fn scarlett_packed_table_kind(&self, table: u8, off: u32) -> Option<u32> {
        let slots = crate::meta::scarlett_graphics_table_slots(self.ex.root_sig.as_ref()?)?;
        if table != 8 || !matches!(self.ex.stage, crate::Stage::Vertex | crate::Stage::Pixel)
            || off / 32 >= slots
        {
            return None;
        }
        crate::meta::scarlett_fssm_table_kind(&self.ex.fssm, table, off)
    }

    pub(super) fn scarlett_media_sampler(&self, at: usize, descriptor: [Option<u32>; 4]) -> Option<u32> {
        if self.ex.stage != crate::Stage::Pixel
            || !self.ex.root_sig.as_ref().is_some_and(crate::meta::scarlett_media_table)
            || self.insns[at].op & 8 != 0
        {
            return None;
        }
        let [Some(w0), Some(w1), Some(w2), Some(w3)] = descriptor else { return None };
        let words = [w0, w1, w2, w3];
        crate::meta::SCARLETT_MEDIA_SAMPLERS.iter()
            .find(|(_, descriptor)| *descriptor == words)
            .map(|(register, _)| (1 << 16) | register)
    }

    pub(super) fn mixed_range_kind(&self, table: u8, off: u32) -> Option<u32> {
        if off & BINDLESS != 0 || self.rm_table_kind(table).is_some() {
            return None;
        }
        if let Some(kind) = self.scarlett_packed_table_kind(table, off) {
            return Some(kind);
        }
        let (p, 0) = self.root_param_at(table, self.user_base())? else { return None };
        let RootParamKind::Table(ranges) = &self.ex.root_sig.as_ref()?.params.get(p)?.kind else { return None };
        if ranges.is_empty() || ranges.iter().all(|r| r.range_type == ranges[0].range_type) {
            return None;
        }
        let idx = off / 32;
        let mut next = 0u32;
        for r in ranges {
            let start = if r.offset_in_table == u32::MAX { next } else { r.offset_in_table };
            next = start.saturating_add(r.num);
            if idx >= start && (r.num == u32::MAX || idx - start < r.num) {
                return Some(r.range_type);
            }
        }
        None
    }

    pub(super) fn kind_at(&self, table: u8, off: u32, writes: bool) -> Result<Option<u32>, String> {
        match self.mixed_range_kind(table, off) {
            Some(0) if writes => Err(format!("descriptor at table s{table} offset {off:#x} is in an SRV range but is written")),
            Some(k) => Ok(Some(k)),
            None => Ok(self.table_kind_for(table, writes)),
        }
    }
}
