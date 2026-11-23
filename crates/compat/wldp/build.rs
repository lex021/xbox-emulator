const DATA_FILES: [&str; 16] = [
    "activity_provider_guid.bin",
    "activity_provider_meta.bin",
    "blocklist_event_desc.bin",
    "blocklist_event_meta.bin",
    "hsti_failure_event_desc.bin",
    "hsti_failure_event_meta.bin",
    "production_event_desc.bin",
    "production_event_meta.bin",
    "retail_unlocked_event_desc.bin",
    "retail_unlocked_event_meta.bin",
    "state_provider_guid.bin",
    "state_provider_meta.bin",
    "watermark_provider_guid.bin",
    "watermark_provider_meta.bin",
    "wdac_event_desc.bin",
    "wdac_event_meta.bin",
];

fn main() {
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("data");
    for f in DATA_FILES {
        println!("cargo:rerun-if-changed=data/{f}");
    }
    let missing: Vec<&str> = DATA_FILES.into_iter().filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        panic!("missing input files in {}: {}; they are generated locally from a system binary and are not part of the repository (*.bin is in .gitignore), so this crate does not build without them", dir.display(), missing.join(", "));
    }
    println!("cargo:rerun-if-changed=exports.txt");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if !msvc {
        return;
    }
    let text = std::fs::read_to_string("exports.txt").expect("exports.txt");
    for e in xone_exports::parse(&text) {
        if e.status == "missing" {
            continue;
        }
        match e.note.strip_prefix("PE forwarder to ").and_then(|r| r.split(';').next()) {
            Some(target) => println!("cargo:rustc-cdylib-link-arg=/EXPORT:{}={},@{}", e.name, target.trim(), e.ordinal),
            None => println!("cargo:rustc-cdylib-link-arg=/EXPORT:{},@{}", e.name, e.ordinal),
        }
    }
}
