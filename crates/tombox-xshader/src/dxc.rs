//! HLSL → DXIL through Microsoft's `dxcompiler.dll` (Windows only).
//!
//! Why DXC rather than FXC: the HLSL this crate generates (flag-based forward branches,
//! guarded writes, loops) is pathological for FXC — minutes for some compute shaders — and
//! compiles in about a second with DXC. DXC output is DXIL (shader model 6.0), which D3D12
//! accepts when it has been validated and signed; `dxil.dll` next to `dxcompiler.dll` does
//! that as part of the compile.
//!
//! The two DLLs are looked for in: `%TOMBOX_DXC_DIR%`, the directory of the running
//! executable, then the newest `Windows Kits\10\bin\<version>\x64`.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

extern "system" {
    fn LoadLibraryW(n: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const CLSID_DXC_COMPILER: Guid = Guid(0x73e22d93, 0xe6ce, 0x47f3, [0xb5, 0xbf, 0xf0, 0x66, 0x4f, 0x39, 0xc1, 0xb0]);
const IID_IDXC_COMPILER3: Guid = Guid(0x228b4687, 0x5a6a, 0x4730, [0x90, 0x0c, 0x97, 0x02, 0xb2, 0x20, 0x3f, 0x54]);
const IID_IDXC_RESULT: Guid = Guid(0x58346cda, 0xdde7, 0x4497, [0x94, 0x61, 0x6f, 0x87, 0xaf, 0x5e, 0x06, 0x59]);

#[repr(C)]
struct DxcBuffer {
    ptr: *const c_void,
    size: usize,
    encoding: u32,
}

type CreateInstance = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(d) = std::env::var("TOMBOX_DXC_DIR") {
        v.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        v.push(d);
    }
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(rd) = std::fs::read_dir(&kits) {
        let mut vers: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("x64")).filter(|p| p.join("dxcompiler.dll").exists()).collect();
        vers.sort();
        vers.reverse();
        v.extend(vers);
    }
    v
}

/// `DxcCreateInstance`, or why it is unavailable.
fn create_instance() -> Result<CreateInstance, String> {
    static F: OnceLock<Result<usize, String>> = OnceLock::new();
    F.get_or_init(|| {
        for dir in candidate_dirs() {
            let (comp, dxil) = (dir.join("dxcompiler.dll"), dir.join("dxil.dll"));
            if !comp.exists() || !dxil.exists() {
                continue;
            }
            unsafe {
                // dxil.dll first, by full path, so the compiler finds the validator/signer.
                let _ = LoadLibraryW(wide(&dxil.to_string_lossy()).as_ptr());
                let h = LoadLibraryW(wide(&comp.to_string_lossy()).as_ptr());
                if h.is_null() {
                    continue;
                }
                let p = GetProcAddress(h, b"DxcCreateInstance\0".as_ptr());
                if !p.is_null() {
                    return Ok(p as usize);
                }
            }
        }
        Err("dxcompiler.dll + dxil.dll not found (set TOMBOX_DXC_DIR or install the Windows SDK)".to_string())
    })
    .clone()
    .map(|p| unsafe { std::mem::transmute::<usize, CreateInstance>(p) })
}

pub fn available() -> bool {
    if host::exe().is_some() {
        return dll_dir().is_some();
    }
    create_instance().is_ok()
}

fn dll_dir() -> Option<PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| candidate_dirs().into_iter().find(|d| d.join("dxcompiler.dll").exists() && d.join("dxil.dll").exists())).clone()
}

pub fn host_main() -> i32 {
    let t = std::thread::Builder::new().stack_size(64 << 20).spawn(host::serve);
    match t.map(|t| t.join()) {
        Ok(Ok(code)) => code,
        _ => 1,
    }
}

pub mod host {
    use super::*;
    use std::io::{Read, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    pub const EXE: &str = "tombox_dxc_host.exe";
    pub const COMPILE: u8 = 1;
    pub const DISASSEMBLE: u8 = 2;
    pub const FOREGROUND: u8 = 0x80;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const IDLE_CAP: usize = 16;

    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadPriority(t: *mut c_void, p: i32) -> i32;
    }

    struct Host {
        child: Child,
        inp: ChildStdin,
        out: ChildStdout,
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    static IDLE: Mutex<Vec<Host>> = Mutex::new(Vec::new());
    static OFF: AtomicBool = AtomicBool::new(false);
    static SPAWNED: AtomicUsize = AtomicUsize::new(0);
    static FAILED: AtomicUsize = AtomicUsize::new(0);

    pub fn wanted(env: Option<&str>, module: Option<&Path>) -> Option<PathBuf> {
        match env.map(str::trim) {
            Some("0") => return None,
            Some(p) if p.len() > 1 => return Some(PathBuf::from(p)),
            _ => {}
        }
        let m = module?;
        if !m.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
            return None;
        }
        Some(m.parent()?.join(EXE))
    }

    pub fn exe() -> Option<PathBuf> {
        static H: OnceLock<Option<PathBuf>> = OnceLock::new();
        H.get_or_init(|| {
            let env = std::env::var("TOMBOX_DXC_HOST").ok();
            let module = crate::shader_cache::module_path();
            let p = wanted(env.as_deref(), module.as_deref())?;
            if p.is_file() {
                return Some(p);
            }
            eprintln!("dxc: no compile host at {}, compiles stay in-process", p.display());
            None
        })
        .clone()
    }

    pub fn stats() -> (usize, usize) {
        (SPAWNED.load(Ordering::Relaxed), FAILED.load(Ordering::Relaxed))
    }

    fn spawn(exe: &Path) -> Result<Host, String> {
        let mut c = Command::new(exe);
        if let Some(d) = dll_dir() {
            c.env("TOMBOX_DXC_DIR", d);
        }
        c.env_remove("TOMBOX_DXC_HOST");
        let mut child = c
            .creation_flags(BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
        let inp = child.stdin.take().ok_or("no stdin")?;
        let out = child.stdout.take().ok_or("no stdout")?;
        SPAWNED.fetch_add(1, Ordering::Relaxed);
        Ok(Host { child, inp, out })
    }

    fn put(w: &mut impl Write, b: &[u8]) -> std::io::Result<()> {
        w.write_all(&(b.len() as u32).to_le_bytes())?;
        w.write_all(b)
    }

    fn get(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
        let mut n = [0u8; 4];
        r.read_exact(&mut n)?;
        let mut v = vec![0u8; u32::from_le_bytes(n) as usize];
        r.read_exact(&mut v)?;
        Ok(v)
    }

    pub fn write_request(w: &mut impl Write, op: u8, parts: &[&[u8]]) -> std::io::Result<()> {
        w.write_all(&[op])?;
        w.write_all(&(parts.len() as u32).to_le_bytes())?;
        for p in parts {
            put(w, p)?;
        }
        w.flush()
    }

    pub fn read_request(r: &mut impl Read) -> std::io::Result<(u8, Vec<Vec<u8>>)> {
        let mut op = [0u8; 1];
        r.read_exact(&mut op)?;
        let mut n = [0u8; 4];
        r.read_exact(&mut n)?;
        let n = u32::from_le_bytes(n) as usize;
        let mut parts = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            parts.push(get(r)?);
        }
        Ok((op[0], parts))
    }

    pub fn write_reply(w: &mut impl Write, r: &Result<Vec<u8>, String>) -> std::io::Result<()> {
        match r {
            Ok(b) => {
                w.write_all(&[1])?;
                put(w, b)?;
            }
            Err(e) => {
                w.write_all(&[0])?;
                put(w, e.as_bytes())?;
            }
        }
        w.flush()
    }

    pub fn read_reply(r: &mut impl Read) -> std::io::Result<Result<Vec<u8>, String>> {
        let mut ok = [0u8; 1];
        r.read_exact(&mut ok)?;
        let b = get(r)?;
        Ok(if ok[0] == 1 { Ok(b) } else { Err(String::from_utf8_lossy(&b).into_owned()) })
    }

    pub fn handle(op: u8, parts: &[Vec<u8>]) -> Result<Vec<u8>, String> {
        match op & !FOREGROUND {
            COMPILE => {
                let src = std::str::from_utf8(parts.first().ok_or("no source")?).map_err(|e| e.to_string())?;
                let args: Vec<&str> = parts[1..].iter().map(|a| std::str::from_utf8(a).unwrap_or("")).collect();
                compile_local(src, &args)
            }
            DISASSEMBLE => disassemble_local(parts.first().ok_or("no code")?).map(String::into_bytes),
            _ => Err(format!("dxc host: unknown request {op}")),
        }
    }

    pub fn serve() -> i32 {
        let mut i = std::io::stdin().lock();
        let mut o = std::io::stdout().lock();
        let mut fg = None;
        loop {
            let Ok((op, parts)) = read_request(&mut i) else { return 0 };
            let want = op & FOREGROUND != 0;
            if fg != Some(want) {
                unsafe { SetThreadPriority(GetCurrentThread(), if want { 2 } else { 0 }) };
                fg = Some(want);
            }
            let r = handle(op, &parts);
            if write_reply(&mut o, &r).is_err() {
                return 0;
            }
        }
    }

    fn exchange(h: &mut Host, op: u8, parts: &[&[u8]]) -> Result<Result<Vec<u8>, String>, String> {
        write_request(&mut h.inp, op, parts).map_err(|e| format!("send: {e}"))?;
        read_reply(&mut h.out).map_err(|e| match h.child.try_wait() {
            Ok(Some(s)) => format!("host exited ({s}) during the request"),
            _ => format!("receive: {e}"),
        })
    }

    pub fn call(op: u8, parts: &[&[u8]]) -> Option<Result<Vec<u8>, String>> {
        if OFF.load(Ordering::Relaxed) {
            return None;
        }
        let exe = exe()?;
        let op = if speculative_allowed() { op | FOREGROUND } else { op };
        let mut last = String::new();
        for _ in 0..2 {
            let idle = IDLE.lock().unwrap_or_else(|e| e.into_inner()).pop();
            let mut h = match idle.map(Ok).unwrap_or_else(|| spawn(&exe)) {
                Ok(h) => h,
                Err(e) => {
                    if SPAWNED.load(Ordering::Relaxed) == 0 {
                        OFF.store(true, Ordering::Relaxed);
                        eprintln!("dxc: compile host unavailable, compiling in process: {e}");
                        return None;
                    }
                    last = e;
                    continue;
                }
            };
            match exchange(&mut h, op, parts) {
                Ok(r) => {
                    let mut idle = IDLE.lock().unwrap_or_else(|e| e.into_inner());
                    if idle.len() < IDLE_CAP {
                        idle.push(h);
                    }
                    return Some(r);
                }
                Err(e) => {
                    FAILED.fetch_add(1, Ordering::Relaxed);
                    last = e;
                }
            }
        }
        Some(Err(format!("dxc: compile host failed twice: {last}")))
    }
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    *(obj as *const *const usize)
}
unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vt(obj).add(2));
        f(obj);
    }
}
unsafe fn blob_bytes(blob: *mut c_void) -> Vec<u8> {
    if blob.is_null() {
        return Vec::new();
    }
    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = std::mem::transmute(*vt(blob).add(3));
    let len: unsafe extern "system" fn(*mut c_void) -> usize = std::mem::transmute(*vt(blob).add(4));
    let (p, n) = (ptr(blob), len(blob));
    if p.is_null() || n == 0 { Vec::new() } else { std::slice::from_raw_parts(p, n).to_vec() }
}

pub fn profile_for(fxc_profile: &str) -> String {
    let mut it = fxc_profile.split('_');
    let stage = it.next().unwrap_or("ps");
    let minor = if it.next() == Some("6") { it.next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0) } else { 0 };
    format!("{stage}_6_{}", minor.max(2))
}

/// Compile `hlsl` (entry `main`) to signed DXIL. `profile` is e.g. `cs_6_0`.
pub fn compile(hlsl: &str, profile: &str) -> Result<Vec<u8>, String> {
    compile_with(hlsl, profile, &[])
}

pub fn compile_with(hlsl: &str, profile: &str, extra: &[&str]) -> Result<Vec<u8>, String> {
    let minor = profile.rsplit('_').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
    if minor < 2 || extra.contains(&"-denorm") {
        return compile_args(hlsl, profile, &[], extra);
    }
    let bf = branch_free_source(hlsl);
    let code = compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "preserve"], extra)?;
    if profile.starts_with("cs_") && disassemble(&code).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
        return compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "any"], extra);
    }
    Ok(code)
}

pub fn fma_defines(fma: crate::shader_cache::Fma, any_mode: bool) -> Vec<&'static str> {
    let mut d = Vec::new();
    if if any_mode { fma.any } else { fma.preserve } {
        d.extend(["-D", "TB_FAST_FMA"]);
    } else if any_mode && fma.hybrid {
        d.extend(["-D", "TB_HYBRID_FMA"]);
    }
    if fma.f2h {
        d.extend(["-D", "TB_FAST_F2H", "-enable-16bit-types"]);
    }
    d
}

pub fn compile_fma(hlsl: &str, profile: &str, fma: crate::shader_cache::Fma, extra: &[&str]) -> Result<Vec<u8>, String> {
    let minor = profile.rsplit('_').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
    let fma = crate::shader_cache::Fma { f2h: fma.f2h && minor >= 2, ..fma };
    let with = |any_mode: bool| -> Vec<&str> { extra.iter().copied().chain(fma_defines(fma, any_mode)).collect() };
    let bf = branch_free_source(hlsl);
    if minor < 2 {
        return compile_bf(hlsl, bf.as_deref(), profile, &[], &with(true));
    }
    let early = (profile.starts_with("cs_") && cs_parallel_enabled() && speculative_allowed() && may_need_denorm_any(hlsl)).then(|| {
        let (h, b, p) = (hlsl.to_string(), bf.clone(), profile.to_string());
        let args: Vec<String> = with(true).into_iter().map(String::from).collect();
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                compile_bf(&h, b.as_deref(), &p, &["-denorm", "any"], &a)
            })
            .ok()
    }).flatten();
    let code = compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "preserve"], &with(false))?;
    if profile.starts_with("cs_") && disassemble(&code).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
        if let Some(r) = early.and_then(|t| t.join().ok()) {
            return r;
        }
        return compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "any"], &with(true));
    }
    Ok(code)
}

thread_local! {
    static BACKGROUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_background(on: bool) {
    BACKGROUND.with(|b| b.set(on));
}

pub fn speculative_allowed() -> bool {
    speculative_for(BACKGROUND.with(|b| b.get()), std::thread::current().name())
}

fn speculative_for(background: bool, thread_name: Option<&str>) -> bool {
    !background && !thread_name.is_some_and(|n| n.starts_with("tombox"))
}

pub fn cs_parallel_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_DXC_CS_PARALLEL").map(|v| v != "0").unwrap_or(true))
}

pub fn may_need_denorm_any(hlsl: &str) -> bool {
    let r = crate::shader_cache::reduced(hlsl);
    let b = r.as_bytes();
    ["log", "log2", "log10", "sqrt", "pow", "length", "distance", "refract"].iter().any(|n| {
        r.match_indices(n).any(|(i, _)| {
            let before = i.checked_sub(1).map(|k| b[k]).unwrap_or(b' ');
            let after = &r[i + n.len()..];
            !(before.is_ascii_alphanumeric() || before == b'_' || before == b'.') && after.trim_start().starts_with('(')
        })
    })
}

pub fn branch_free_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_DXC_BRANCH_FREE").map(|v| v != "0").unwrap_or(true))
}

fn branch_free_source(hlsl: &str) -> Option<String> {
    if !branch_free_enabled() {
        return None;
    }
    let r = crate::branch_free::rewrite(hlsl);
    (r.len() != hlsl.len() + crate::branch_free::PRELUDE.len() || r[crate::branch_free::PRELUDE.len()..] != *hlsl).then_some(r)
}

fn compile_bf(hlsl: &str, bf: Option<&str>, profile: &str, denorm: &[&str], extra: &[&str]) -> Result<Vec<u8>, String> {
    if let Some(src) = bf {
        match compile_args(src, profile, denorm, extra) {
            Ok(c) => return Ok(c),
            Err(e) => {
                static WARNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                if WARNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
                    eprintln!("dxc: branch-free source failed, compiling the original: {}", e.lines().next().unwrap_or(""));
                }
            }
        }
    }
    compile_args(hlsl, profile, denorm, extra)
}

pub fn compile_denorm(hlsl: &str, profile: &str, denorm: &str, extra: &[&str]) -> Result<Vec<u8>, String> {
    compile_args(hlsl, profile, &["-denorm", denorm], extra)
}

pub fn identity() -> &'static str {
    static I: OnceLock<String> = OnceLock::new();
    I.get_or_init(|| {
        for dir in candidate_dirs() {
            let (comp, dxil) = (dir.join("dxcompiler.dll"), dir.join("dxil.dll"));
            if let (Ok(a), Ok(b)) = (std::fs::metadata(&comp), std::fs::metadata(&dxil)) {
                return format!("dxcompiler {} dxil {}", a.len(), b.len());
            }
        }
        String::from("none")
    })
}

pub const COMPILE_FLAVOR: &str = "denorm-preserve; cs log/sqrt any";

pub fn has_preserve_unselectable(disasm: &str) -> bool {
    disasm.contains("@dx.op.unary.f32(i32 23,") || disasm.contains("@dx.op.unary.f32(i32 24,")
}

pub fn disassemble(code: &[u8]) -> Result<String, String> {
    if let Some(r) = host::call(host::DISASSEMBLE, &[code]) {
        return r.map(|b| String::from_utf8_lossy(&b).into_owned());
    }
    disassemble_local(code)
}

fn disassemble_local(code: &[u8]) -> Result<String, String> {
    let create = create_instance()?;
    unsafe {
        let mut compiler: *mut c_void = std::ptr::null_mut();
        let hr = create(&CLSID_DXC_COMPILER, &IID_IDXC_COMPILER3, &mut compiler);
        if hr < 0 || compiler.is_null() {
            return Err(format!("dxc: DxcCreateInstance(IDxcCompiler3) hr={hr:#010x}"));
        }
        let buf = DxcBuffer { ptr: code.as_ptr() as *const c_void, size: code.len(), encoding: 0 };
        let f: unsafe extern "system" fn(*mut c_void, *const DxcBuffer, *const Guid, *mut *mut c_void) -> i32 = std::mem::transmute(*vt(compiler).add(4));
        let mut result: *mut c_void = std::ptr::null_mut();
        let hr = f(compiler, &buf, &IID_IDXC_RESULT, &mut result);
        release(compiler);
        if hr < 0 || result.is_null() {
            return Err(format!("dxc: Disassemble hr={hr:#010x}"));
        }
        let mut status = 0i32;
        let get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = std::mem::transmute(*vt(result).add(3));
        get_status(result, &mut status);
        let get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = std::mem::transmute(*vt(result).add(4));
        let mut b: *mut c_void = std::ptr::null_mut();
        get_result(result, &mut b);
        let text = String::from_utf8_lossy(&blob_bytes(b)).into_owned();
        release(b);
        release(result);
        if status < 0 || text.is_empty() {
            return Err(format!("dxc: Disassemble status={status:#010x}"));
        }
        Ok(text)
    }
}

fn compile_args(hlsl: &str, profile: &str, denorm: &[&str], extra: &[&str]) -> Result<Vec<u8>, String> {
    let all: Vec<&str> = ["-T", profile, "-E", "main"].iter().chain(denorm.iter()).chain(extra.iter()).copied().collect();
    let mut parts: Vec<&[u8]> = vec![hlsl.as_bytes()];
    parts.extend(all.iter().map(|a| a.as_bytes()));
    if let Some(r) = host::call(host::COMPILE, &parts) {
        return r;
    }
    compile_local(hlsl, &all)
}

fn compile_local(hlsl: &str, all: &[&str]) -> Result<Vec<u8>, String> {
    let create = create_instance()?;
    unsafe {
        let mut compiler: *mut c_void = std::ptr::null_mut();
        let hr = create(&CLSID_DXC_COMPILER, &IID_IDXC_COMPILER3, &mut compiler);
        if hr < 0 || compiler.is_null() {
            return Err(format!("dxc: DxcCreateInstance(IDxcCompiler3) hr={hr:#010x}"));
        }
        let args_w: Vec<Vec<u16>> = all.iter().map(|a| wide(a)).collect();
        let args: Vec<*const u16> = args_w.iter().map(|a| a.as_ptr()).collect();
        let src = DxcBuffer { ptr: hlsl.as_ptr() as *const c_void, size: hlsl.len(), encoding: 65001 };
        let mut result: *mut c_void = std::ptr::null_mut();
        // IDxcCompiler3::Compile (slot 3)
        let f: unsafe extern "system" fn(*mut c_void, *const DxcBuffer, *const *const u16, u32, *mut c_void, *const Guid, *mut *mut c_void) -> i32 =
            std::mem::transmute(*vt(compiler).add(3));
        let hr = f(compiler, &src, args.as_ptr(), args.len() as u32, std::ptr::null_mut(), &IID_IDXC_RESULT, &mut result);
        release(compiler);
        if hr < 0 || result.is_null() {
            return Err(format!("dxc: Compile hr={hr:#010x}"));
        }
        // IDxcOperationResult: 3 GetStatus, 4 GetResult, 5 GetErrorBuffer.
        let mut status = 0i32;
        let get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = std::mem::transmute(*vt(result).add(3));
        get_status(result, &mut status);
        let get_blob = |slot: usize| -> *mut c_void {
            let g: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = std::mem::transmute(*vt(result).add(slot));
            let mut b: *mut c_void = std::ptr::null_mut();
            g(result, &mut b);
            b
        };
        let out = if status >= 0 {
            let b = get_blob(4);
            let v = blob_bytes(b);
            release(b);
            if v.is_empty() { Err("dxc: empty result".to_string()) } else { Ok(v) }
        } else {
            let b = get_blob(5);
            let text = String::from_utf8_lossy(&blob_bytes(b)).into_owned();
            release(b);
            let line = text.lines().find(|l| l.contains("error")).unwrap_or(text.lines().next().unwrap_or("")).to_string();
            // "hlsl.hlsl:12:5: error: message" -> "message"
            Err(format!("dxc: {}", line.split("error: ").nth(1).unwrap_or(&line)))
        };
        release(result);
        out
    }
}

/// Location of a compiled shader in the on-disk cache shared by the offline sweep
/// (`xscan --compile`) and the runtime (`d3d12_xs`). Keyed by the generated HLSL and the
/// profile, so any change to a shader's translation misses the cache.
pub fn cache_path(hlsl: &str, profile: &str) -> PathBuf {
    cache_file(cache_key(hlsl, profile))
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn cache_key(hlsl: &str, profile: &str) -> u64 {
    fnv1a(hlsl.as_bytes()) ^ fnv1a(profile.as_bytes()).rotate_left(17)
}

fn cache_file(key: u64) -> PathBuf {
    std::env::temp_dir().join("tombox_shader_cache").join("by_hlsl").join(format!("{key:016x}.cso"))
}

fn flavored_cache_path(hlsl: &str, profile: &str) -> PathBuf {
    cache_file(cache_key(hlsl, profile) ^ fnv1a(COMPILE_FLAVOR.as_bytes()).rotate_left(31))
}

fn flags_differ_for(profile: &str) -> bool {
    profile.starts_with("cs_") && profile.rsplit('_').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0) >= 2
}

pub fn cached_matches_flags(code: &[u8], profile: &str) -> bool {
    if !flags_differ_for(profile) {
        return true;
    }
    match disassemble(code) {
        Ok(t) => !(has_preserve_unselectable(&t) && t.contains("\"fp32-denorm-mode\"=\"preserve\"")),
        Err(_) => false,
    }
}

pub fn cache_load(hlsl: &str, profile: &str) -> Option<Vec<u8>> {
    let p = cache_path(hlsl, profile);
    if let Ok(v) = std::fs::read(&p) {
        if !v.is_empty() && cached_matches_flags(&v, profile) {
            return Some(v);
        }
    }
    let v = std::fs::read(flavored_cache_path(hlsl, profile)).ok().filter(|v| !v.is_empty())?;
    let _ = std::fs::write(&p, &v);
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_host_is_used_only_inside_a_dll_unless_named() {
        let dll = Path::new("D:/x/d3d12_xs.dll");
        let exe = Path::new("D:/x/xshader.exe");
        assert_eq!(host::wanted(None, Some(dll)), Some(PathBuf::from("D:/x").join(host::EXE)));
        assert_eq!(host::wanted(None, Some(exe)), None);
        assert_eq!(host::wanted(Some("0"), Some(dll)), None);
        assert_eq!(host::wanted(Some("C:/h.exe"), Some(exe)), Some(PathBuf::from("C:/h.exe")));
        assert_eq!(host::wanted(None, None), None);
    }

    #[test]
    fn compile_host_requests_and_replies_round_trip() {
        let mut buf = Vec::new();
        host::write_request(&mut buf, host::COMPILE | host::FOREGROUND, &[b"float4 main() : SV_Target { return 0; }", b"-T", b"ps_6_2", b""]).unwrap();
        let (op, parts) = host::read_request(&mut &buf[..]).unwrap();
        assert_eq!(op, host::COMPILE | host::FOREGROUND);
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[2], b"ps_6_2");
        assert!(parts[3].is_empty());
        for r in [Ok(vec![1u8, 2, 3]), Err("dxc: bad".to_string())] {
            let mut b = Vec::new();
            host::write_reply(&mut b, &r).unwrap();
            assert_eq!(host::read_reply(&mut &b[..]).unwrap(), r);
        }
        assert!(host::read_reply(&mut &[1u8, 9, 0][..]).is_err());
        assert!(host::handle(0x33, &[]).is_err());
    }

    #[test]
    fn speculative_denorm_any_compile_runs_only_on_title_threads() {
        assert!(speculative_for(false, None));
        assert!(speculative_for(false, Some("main")));
        assert!(!speculative_for(true, None));
        assert!(!speculative_for(false, Some("tombox-shader-prewarm")));
        assert!(!speculative_for(false, Some("tombox-pso-xrec")));
        set_background(true);
        assert!(!speculative_allowed());
        set_background(false);
    }

    const PRESERVE: &str = "\"fp32-denorm-mode\"=\"preserve\"";

    fn cs(body: &str) -> String {
        format!("RWTexture2D<float4> u : register(u0);\n[numthreads(8, 8, 1)]\nvoid main(uint3 t : SV_DispatchThreadID) {{ float x = asfloat(t.x); float y = asfloat(t.y); u[t.xy] = float4({body}, 0, 0, 0); }}\n")
    }

    fn denorm_of(src: &str, profile: &str) -> (bool, String) {
        let code = compile(src, profile).expect("dxc compile");
        let text = disassemble(&code).expect("dxc disassemble");
        (text.contains(PRESERVE), text)
    }

    #[test]
    fn compute_log_and_sqrt_leave_denorm_preserve() {
        for body in ["log2(x)", "sqrt(x)", "pow(x, y)", "log(abs(x)) + y"] {
            let (preserve, text) = denorm_of(&cs(body), "cs_6_2");
            assert!(has_preserve_unselectable(&text), "{body}: expected a Log/Sqrt op");
            assert!(!preserve, "{body}: compute shader with Log/Sqrt still compiled with fp32 denorm preserve");
        }
    }

    #[test]
    fn denorm_any_prediction_follows_reachable_intrinsics() {
        assert!(may_need_denorm_any(&cs("log2(x)")));
        assert!(may_need_denorm_any(&cs("length(float2(x, y))")));
        assert!(!may_need_denorm_any(&cs("exp2(x) * rsqrt(y)")));
        assert!(!may_need_denorm_any(&cs("tb_log_x + x.length")));
    }

    #[test]
    fn parallel_compute_compile_matches_the_sequential_flow_byte_for_byte() {
        let fma = crate::shader_cache::Fma { preserve: true, any: false, hybrid: false, f2h: false };
        for body in ["log2(x)", "sqrt(x) + y", "pow(x, y)", "length(float2(x, y))", "exp2(x) * rsqrt(y)"] {
            let src = cs(body);
            let par = compile_fma(&src, "cs_6_2", fma, &[]).expect("parallel flow");
            let pre = compile_args(&src, "cs_6_2", &["-denorm", "preserve"], &fma_defines(fma, false)).expect("preserve");
            let seq = if disassemble(&pre).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
                compile_args(&src, "cs_6_2", &["-denorm", "any"], &fma_defines(fma, true)).expect("any")
            } else {
                pre
            };
            let bf = branch_free_source(&src);
            let seq_bf = match &bf {
                Some(b) => {
                    let pre = compile_args(b, "cs_6_2", &["-denorm", "preserve"], &fma_defines(fma, false)).expect("preserve bf");
                    if disassemble(&pre).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
                        compile_args(b, "cs_6_2", &["-denorm", "any"], &fma_defines(fma, true)).expect("any bf")
                    } else {
                        pre
                    }
                }
                None => seq,
            };
            assert!(par == seq_bf, "{body}: parallel compute compile differs from the sequential flow");
        }
    }

    #[test]
    fn other_shaders_keep_denorm_preserve() {
        let (preserve, text) = denorm_of(&cs("exp2(x) * rsqrt(y)"), "cs_6_2");
        assert!(!has_preserve_unselectable(&text));
        assert!(preserve, "compute shader without Log/Sqrt lost fp32 denorm preserve");
        let ps = "float4 main(float4 p : SV_Position) : SV_Target { return float4(log2(p.x), sqrt(p.y), 0, 1); }";
        let (preserve, text) = denorm_of(ps, "ps_6_2");
        assert!(has_preserve_unselectable(&text));
        assert!(preserve, "pixel shader lost fp32 denorm preserve");
        let (preserve, _) = denorm_of(&cs("log2(x)"), "cs_6_0");
        assert!(!preserve, "shader model 6.0 has no denorm mode");
    }

    #[test]
    fn explicit_denorm_mode_is_respected() {
        let code = compile_with(&cs("log2(x)"), "cs_6_2", &["-denorm", "ftz"]).expect("dxc compile");
        let text = disassemble(&code).expect("dxc disassemble");
        assert!(text.contains("\"fp32-denorm-mode\"=\"ftz\""));
    }

    #[test]
    fn cache_key_unchanged_for_shaders_whose_flags_did_not_change() {
        let p = cache_path("float4 main() : SV_Target { return 0; }", "ps_6_2");
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        let k = u64::from_str_radix(&name, 16).unwrap();
        let f = |b: &[u8]| b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3));
        let plain = f(b"float4 main() : SV_Target { return 0; }") ^ f(b"ps_6_2").rotate_left(17);
        assert_eq!(k, plain);
    }

    #[test]
    fn stale_compute_entries_compiled_with_preserve_are_rejected() {
        let src = cs("log2(x)");
        let stale = compile_args(&src, "cs_6_2", &["-denorm", "preserve"], &[]).expect("dxc compile");
        assert!(!cached_matches_flags(&stale, "cs_6_2"));
        let fresh = compile(&src, "cs_6_2").expect("dxc compile");
        assert!(cached_matches_flags(&fresh, "cs_6_2"));
        let plain = compile(&cs("exp2(x)"), "cs_6_2").expect("dxc compile");
        assert!(cached_matches_flags(&plain, "cs_6_2"));
        assert!(cached_matches_flags(&stale, "ps_6_2"));
        let tag = format!("float4 main() : SV_Target {{ return {}; }}", std::process::id() as f32 + 0.25);
        let p = cache_path(&tag, "cs_6_2");
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        std::fs::write(&p, &stale).unwrap();
        assert!(cache_load(&tag, "cs_6_2").is_none());
        std::fs::write(&p, &fresh).unwrap();
        assert_eq!(cache_load(&tag, "cs_6_2").as_deref(), Some(&fresh[..]));
        let _ = std::fs::remove_file(&p);
        let q = flavored_cache_path(&tag, "ps_6_2");
        std::fs::write(&q, &plain).unwrap();
        assert_eq!(cache_load(&tag, "ps_6_2").as_deref(), Some(&plain[..]));
        let _ = std::fs::remove_file(&q);
        let _ = std::fs::remove_file(cache_path(&tag, "ps_6_2"));
    }
}