use std::path::PathBuf;

const SHADERS: &[&str] = &["fortnite_vs_sample.dxbc", "fortnite_ps_sample.dxbc", "fortnite_cs_clear.dxbc", "xdefiant_vs_swappc_s4.dxbc", "xdefiant_ps_frontface_quad.dxbc"];
const CONSOLE_XSC: &[&str] = &["console_xsc/as2.dxbc", "console_xsc/as3.dxbc", "console_xsc/ms3.dxbc"];

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../analysis/shaders");
    for cfg in ["tombox_fixtures_shaders", "tombox_fixtures_console_xsc", "tombox_fixtures_mesh"] {
        println!("cargo:rustc-check-cfg=cfg({cfg})");
    }
    let present = |names: &[&str]| {
        names.iter().all(|n| {
            let p = root.join(n);
            println!("cargo:rerun-if-changed={}", p.display());
            p.is_file()
        })
    };
    if present(SHADERS) {
        println!("cargo:rustc-cfg=tombox_fixtures_shaders");
    }
    if present(CONSOLE_XSC) {
        println!("cargo:rustc-cfg=tombox_fixtures_console_xsc");
    }
    let mesh = root.join("mesh");
    println!("cargo:rerun-if-changed={}", mesh.display());
    if mesh.is_dir() {
        println!("cargo:rustc-cfg=tombox_fixtures_mesh");
    }
    println!("cargo:rerun-if-changed=build.rs");
    translator_hash();
}

fn collect(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn translator_hash() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let crates = manifest.parent().unwrap_or(&manifest).to_path_buf();
    let d3d = crates.join("compat").join("d3d12_xs").join("src");
    let mut files = Vec::new();
    collect(&manifest.join("src"), &mut files);
    files.push(manifest.join("Cargo.toml"));
    for f in ["shader_xlat.rs", "cbv_raw.rs", "mip_bind.rs", "global_bind.rs", "centroid_bind.rs", "xlat_index.rs"] {
        files.push(d3d.join(f));
    }
    let rel = |f: &PathBuf| f.strip_prefix(&crates).unwrap_or(f).to_string_lossy().replace('\\', "/");
    files.sort_by_key(|f| rel(f));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let Ok(data) = std::fs::read(f) else { continue };
        for b in rel(f).bytes().chain(data.into_iter().filter(|&b| b != b'\r')) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    println!("cargo:rerun-if-changed={}", manifest.join("src").display());
    println!("cargo:rustc-env=TOMBOX_XLAT_SRC_HASH={h:016x}");
}
