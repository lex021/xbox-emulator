use super::{next_token, E_INVALIDARG, S_OK};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread::ThreadId;

#[derive(Clone, Copy)]
pub struct Reg {
    pub token: u64,
    pub owner: usize,
    pub queue: usize,
    pub ctx: usize,
    pub cb: usize,
}

struct Inner {
    regs: Vec<Reg>,
    active: Vec<(u64, ThreadId)>,
}

pub struct Registry {
    inner: Mutex<Inner>,
    idle: Condvar,
}

struct Running {
    reg: &'static Registry,
    token: u64,
}

impl Drop for Running {
    fn drop(&mut self) {
        let me = std::thread::current().id();
        let mut g = self.reg.lock();
        if let Some(i) = g.active.iter().position(|&(t, th)| t == self.token && th == me) {
            g.active.swap_remove(i);
        }
        drop(g);
        self.reg.idle.notify_all();
    }
}

impl Registry {
    pub const fn new() -> Registry {
        Registry { inner: Mutex::new(Inner { regs: Vec::new(), active: Vec::new() }), idle: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub unsafe fn register(&self, owner: usize, queue: usize, ctx: *mut core::ffi::c_void, cb: usize, token: *mut u64) -> i32 {
        if token.is_null() || cb == 0 {
            return E_INVALIDARG;
        }
        let t = next_token();
        self.lock().regs.push(Reg { token: t, owner, queue, ctx: ctx as usize, cb });
        *token = t;
        S_OK
    }

    pub fn unregister(&self, owner: Option<usize>, token: u64, wait: bool) -> bool {
        let me = std::thread::current().id();
        let mut g = self.lock();
        let before = g.regs.len();
        g.regs.retain(|r| !(r.token == token && owner.map_or(true, |o| o == r.owner)));
        let removed = g.regs.len() != before;
        if removed && wait {
            while g.active.iter().any(|&(t, th)| t == token && th != me) {
                g = self.idle.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        }
        removed
    }

    pub fn all(&self) -> Vec<Reg> {
        self.lock().regs.clone()
    }

    pub fn len(&self) -> usize {
        self.lock().regs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, token: u64) -> bool {
        self.lock().regs.iter().any(|r| r.token == token)
    }

    pub fn in_flight(&self, token: u64) -> usize {
        self.lock().active.iter().filter(|a| a.0 == token).count()
    }

    fn enter(&'static self, token: u64) -> Option<Running> {
        let mut g = self.lock();
        if !g.regs.iter().any(|r| r.token == token) {
            return None;
        }
        g.active.push((token, std::thread::current().id()));
        Some(Running { reg: self, token })
    }

    pub fn fire_token<F>(&'static self, token: u64, f: F) -> i32
    where
        F: FnOnce(&Reg) + Send + 'static,
    {
        let found = self.lock().regs.iter().find(|r| r.token == token).copied();
        let Some(reg) = found else { return E_INVALIDARG };
        unsafe {
            super::xasync::submit(reg.queue, 1, Box::new(move |canceled| {
                if canceled {
                    return;
                }
                if let Some(_run) = self.enter(reg.token) {
                    f(&reg)
                }
            }))
        }
    }

    pub fn fire<F>(&'static self, owner: Option<usize>, f: F)
    where
        F: Fn(&Reg) + Send + Sync + Clone + 'static,
    {
        for r in self.all() {
            if owner.map_or(true, |o| o == r.owner) {
                let f = f.clone();
                let reg = r;
                unsafe {
                    super::xasync::submit(r.queue, 1, Box::new(move |canceled| {
                        if canceled {
                            return;
                        }
                        if let Some(_run) = self.enter(reg.token) {
                            f(&reg)
                        }
                    }));
                }
            }
        }
    }
}
