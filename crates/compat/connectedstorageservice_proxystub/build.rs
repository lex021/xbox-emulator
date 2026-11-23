fn main() {
    println!("cargo:rerun-if-changed=data/proxy_19041.bin");
    let data = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("data/proxy_19041.bin");
    if !data.is_file() {
        panic!("missing input file {}: it is generated locally from a system binary and is not part of the repository (*.bin is in .gitignore), so this crate does not build without it", data.display());
    }
    xone_exports::emit_link_args("proxy_exports.txt");
}
