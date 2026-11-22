use std::path::{Path, PathBuf};

fn checked_in(dir: &Path, why: &str) -> Vec<u8> {
    let dxil = dir.join("query_accum.dxil");
    match std::fs::read(&dxil) {
        Ok(code) => {
            println!("cargo:warning=query_accum.hlsl not recompiled ({why}); using the checked-in shaders/query_accum.dxil");
            code
        }
        Err(e) => panic!("query_accum.hlsl could not be compiled ({why}) and {} is unavailable ({e}); install the Windows SDK, set TOMBOX_DXC_DIR to the folder with dxcompiler.dll and dxil.dll, or set TOMBOX_DXC to a DXC executable", dxil.display()),
    }
}

#[cfg(windows)]
fn in_process(hlsl: &str) -> Result<Vec<u8>, String> {
    tombox_xshader::dxc::compile(hlsl, "cs_6_0")
}

#[cfg(not(windows))]
fn in_process(_: &str) -> Result<Vec<u8>, String> {
    Err("in-process DXC needs Windows".into())
}

fn external(dxc: &std::ffi::OsStr, source: &Path, out: &Path) -> Result<Vec<u8>, String> {
    let status = std::process::Command::new(dxc)
        .args(["-T", "cs_6_0", "-E", "main", "-Fo"])
        .arg(out)
        .arg(source)
        .status()
        .map_err(|e| format!("could not run {dxc:?}: {e}"))?;
    if !status.success() {
        return Err(format!("{dxc:?} failed ({status})"));
    }
    std::fs::read(out).map_err(|e| format!("read {}: {e}", out.display()))
}

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("shaders");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("query_accum.dxil");
    println!("cargo:rerun-if-changed=shaders/query_accum.hlsl");
    println!("cargo:rerun-if-changed=shaders/query_accum.dxil");
    println!("cargo:rerun-if-env-changed=TOMBOX_DXC");
    println!("cargo:rerun-if-env-changed=TOMBOX_DXC_DIR");
    let source = dir.join("query_accum.hlsl");
    let hlsl = std::fs::read_to_string(&source).expect("shaders/query_accum.hlsl");
    let compiled = match std::env::var_os("TOMBOX_DXC") {
        Some(dxc) => external(&dxc, &source, &out),
        None if cfg!(windows) => in_process(&hlsl),
        None => external(std::ffi::OsStr::new("dxc"), &source, &out),
    };
    let code = compiled.unwrap_or_else(|e| checked_in(&dir, &e));
    std::fs::write(&out, code).expect("write query_accum.dxil");
    translator_hash();
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn translator_hash() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let xshader = manifest.join("..").join("..").join("tombox-xshader");
    let mut files = Vec::new();
    collect(&xshader.join("src"), &mut files);
    files.push(xshader.join("Cargo.toml"));
    for f in ["shader_xlat.rs", "cbv_raw.rs", "mip_bind.rs", "global_bind.rs", "centroid_bind.rs", "xlat_index.rs"] {
        files.push(manifest.join("src").join(f));
    }
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let Ok(data) = std::fs::read(f) else { continue };
        let name = f.strip_prefix(&manifest.join("..").join("..")).unwrap_or(f).to_string_lossy().replace('\\', "/");
        for b in name.bytes().chain(data.into_iter().filter(|&b| b != b'\r')) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    println!("cargo:rerun-if-changed={}", xshader.join("src").display());
    println!("cargo:rustc-env=TOMBOX_XLAT_SRC_HASH={h:016x}");
}
