use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use super::Hook;

const DLL: &str = "Bink2Scarlett.dll";
const MATCH: &[&str] = &["bink2scarlett.dll"];

macro_rules! trace {
    ($thunk:ident, $target:ident, $export:literal) => {
        #[unsafe(naked)]
        unsafe extern "system" fn $thunk() {
            core::arch::naked_asm!(
                "sub rsp, 0x48",
                "mov [rsp+0x20], rcx", "mov [rsp+0x28], rdx",
                "mov [rsp+0x30], r8", "mov [rsp+0x38], r9",
                "call {resolve}",
                "mov rcx, [rsp+0x20]", "mov rdx, [rsp+0x28]",
                "mov r8, [rsp+0x30]", "mov r9, [rsp+0x38]",
                "add rsp, 0x48",
                "jmp rax",
                resolve = sym $target,
            );
        }
        unsafe extern "system" fn $target() -> usize {
            static ENTRY: OnceLock<usize> = OnceLock::new();
            static HITS: AtomicUsize = AtomicUsize::new(0);
            let entry = *ENTRY.get_or_init(|| unsafe { super::resolve(DLL, $export) as usize });
            if HITS.fetch_add(1, Ordering::Relaxed) == 0 {
                crate::record_raw("bink_probe", "entered", $export);
            }
            entry
        }
    };
}

// These exports take integer or pointer arguments. Preserve the inbound argument
// registers and all stack arguments before jumping to the original DLL entry.
#[unsafe(naked)]
unsafe extern "system" fn open() {
    core::arch::naked_asm!("mov r9, [rsp]", "jmp {target}", target = sym open_logged);
}

unsafe extern "system" fn open_logged(input: usize, options: usize, flags: u32, caller: usize) -> usize {
    type Open = unsafe extern "system" fn(usize, usize, u32) -> usize;
    static ENTRY: OnceLock<usize> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| unsafe { super::resolve(DLL, "BinkOpenWithOptions") as usize });
    let forward: Open = unsafe { core::mem::transmute(entry) };
    let exe = unsafe { super::GetModuleHandleW(core::ptr::null()) } as usize;
    let rva = caller.checked_sub(exe).filter(|&rva| rva < 0x0c00_0000);
    crate::record_raw("bink_probe", "open", &format!("input={input:#x} flags={flags:#010x} caller={caller:#x} title_rva={rva:x?}"));
    let handle = unsafe { forward(input, options, flags) };
    let error = if handle == 0 {
        let proc = unsafe { super::resolve(DLL, "BinkGetError") };
        if proc.is_null() { String::new() } else {
            let call: unsafe extern "system" fn() -> *const i8 = unsafe { core::mem::transmute(proc) };
            let message = unsafe { call() };
            if message.is_null() { String::new() } else {
                unsafe { std::ffi::CStr::from_ptr(message) }.to_string_lossy().into_owned()
            }
        }
    } else { String::new() };
    crate::record_raw("bink_probe", "open_result", &format!("handle={handle:#x} error={error:?}"));
    handle
}

trace!(frame, frame_target, "BinkDoFrame");
trace!(frame_plane, frame_plane_target, "BinkDoFramePlane");
trace!(async_frame, async_frame_target, "BinkDoFrameAsyncMulti");
trace!(next, next_target, "BinkNextFrame");
trace!(callbacks, callbacks_target, "BinkSetOSFileCallbacks");
trace!(get_buffers, get_buffers_target, "BinkGetFrameBuffersInfo");
trace!(register_buffers, register_buffers_target, "BinkRegisterFrameBuffers");
trace!(start_async, start_async_target, "BinkStartAsyncThread");
trace!(async_wait, async_wait_target, "BinkDoFrameAsyncWait");
trace!(wait_frame, wait_frame_target, "BinkWait");
trace!(close, close_target, "BinkClose");
trace!(audio, audio_target, "BinkSetSoundSystem");
trace!(pause, pause_target, "BinkPause");
trace!(memory, memory_target, "BinkSetMemory");

pub(super) fn install() {
    if std::env::var_os("TOMBOX_BINK_TRACE").is_none() {
        return;
    }
    let name: Vec<u16> = DLL.encode_utf16().chain(Some(0)).collect();
    let module = unsafe { super::GetModuleHandleW(name.as_ptr()) };
    if module.is_null() {
        return;
    }
    let mut hooks = Vec::new();
    for (export, hook) in [
        ("BinkOpenWithOptions", open as *const () as usize),
        ("BinkDoFrame", frame as *const () as usize),
        ("BinkDoFramePlane", frame_plane as *const () as usize),
        ("BinkDoFrameAsyncMulti", async_frame as *const () as usize),
        ("BinkNextFrame", next as *const () as usize),
        ("BinkSetOSFileCallbacks", callbacks as *const () as usize),
        ("BinkGetFrameBuffersInfo", get_buffers as *const () as usize),
        ("BinkRegisterFrameBuffers", register_buffers as *const () as usize),
        ("BinkStartAsyncThread", start_async as *const () as usize),
        ("BinkDoFrameAsyncWait", async_wait as *const () as usize),
        ("BinkWait", wait_frame as *const () as usize),
        ("BinkClose", close as *const () as usize),
        ("BinkSetSoundSystem", audio as *const () as usize),
        ("BinkPause", pause as *const () as usize),
        ("BinkSetMemory", memory as *const () as usize),
    ] {
        let encoded = format!("{export}\0");
        if !unsafe { super::GetProcAddress(module.cast(), encoded.as_ptr()) }.is_null() {
            hooks.push(Hook { name: export, dlls: MATCH, hook });
        }
    }
    let exe = unsafe { super::GetModuleHandleW(core::ptr::null()) };
    let mut hits = Vec::new();
    unsafe { super::patch::patch_image(exe, &hooks, &mut hits) };
    if !hits.is_empty() {
        crate::record_raw("bink_probe", "armed", &hits.join(", "));
    }
}
