use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub const DETILE: u8 = 0;
pub const COPY: u8 = 1;
pub const CLEAR: u8 = 2;
pub const RESOLVE: u8 = 3;
pub const BARRIER: u8 = 4;
pub const DRAW: u8 = 5;
pub const QUERY: u8 = 6;
pub const KINDS: usize = 7;
pub const NAMES: [&str; KINDS] = ["detile", "copy", "clear", "resolve", "barrier", "draw", "query"];

static FRAME: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
static TOTAL: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];

#[inline]
pub fn note(kind: u8) {
    note_n(kind, 1);
}

#[inline]
pub fn note_n(kind: u8, n: u64) {
    let k = kind as usize;
    if k < KINDS && n != 0 {
        FRAME[k].fetch_add(n, Relaxed);
        TOTAL[k].fetch_add(n, Relaxed);
    }
}

thread_local! {
    static DEPTH: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

pub struct Scope {
    list: *mut core::ffi::c_void,
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.list.is_null() {
            unsafe { crate::gpuprof::pass_end(self.list) };
        }
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

pub fn depth() -> u32 {
    DEPTH.with(|d| d.get())
}

pub unsafe fn scope(list: *mut core::ffi::c_void, kind: u8) -> Scope {
    note(kind);
    let outer = DEPTH.with(|d| {
        let v = d.get();
        d.set(v + 1);
        v == 0
    });
    let stamp = outer && !list.is_null() && (kind as usize) < KINDS;
    if stamp {
        unsafe { crate::gpuprof::pass_begin(list, crate::gpuprof::TAG_INJECT0 + kind) };
    }
    Scope { list: if stamp { list } else { core::ptr::null_mut() } }
}

pub fn mark(kind: u8) -> Scope {
    note(kind);
    DEPTH.with(|d| d.set(d.get() + 1));
    Scope { list: core::ptr::null_mut() }
}

pub fn take() -> [u64; KINDS] {
    core::array::from_fn(|k| FRAME[k].swap(0, Relaxed))
}

pub fn totals() -> [u64; KINDS] {
    core::array::from_fn(|k| TOTAL[k].load(Relaxed))
}

pub fn summary(c: &[u64; KINDS]) -> String {
    NAMES.iter().zip(c.iter()).map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_accumulate_per_kind_and_take_drains_only_the_frame() {
        let _ = take();
        let t0 = totals();
        note(COPY);
        note(COPY);
        note_n(BARRIER, 3);
        note(KINDS as u8);
        note_n(DRAW, 0);
        let f = take();
        assert!(f[COPY as usize] >= 2 && f[BARRIER as usize] >= 3);
        assert_eq!(f[CLEAR as usize], 0);
        let t1 = totals();
        assert!(t1[COPY as usize] >= t0[COPY as usize] + 2);
        assert_eq!(summary(&[1, 2, 0, 0, 5, 0, 0]), "detile=1 copy=2 clear=0 resolve=0 barrier=5 draw=0 query=0");
        assert_eq!(NAMES.len(), KINDS);
    }

    #[test]
    fn scopes_nest_and_only_the_outermost_may_stamp() {
        assert_eq!(depth(), 0);
        {
            let _a = mark(DRAW);
            assert_eq!(depth(), 1);
            let _b = unsafe { scope(core::ptr::null_mut(), BARRIER) };
            assert_eq!(depth(), 2);
            assert!(_b.list.is_null());
        }
        assert_eq!(depth(), 0);
        let s = unsafe { scope(core::ptr::null_mut(), COPY) };
        assert!(s.list.is_null());
        drop(s);
        assert_eq!(depth(), 0);
    }
}
