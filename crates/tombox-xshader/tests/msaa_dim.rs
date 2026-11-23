use tombox_xshader::{dxc, extract, hlsl};

const MS_READERS: [&str; 14] = [
    "0f7a1eeca785d353", "0ff9ad9a34cd4e65", "22745e92669cb22c", "2a6f23cc7c21a3d5", "345547c8e6b92208", "4f91474ae02fd09e", "5e7153a4cae574b4",
    "77e0c7aa364a3e2f", "7c906a6984c64250", "8f215c725a248afa", "cd8dbd157b656319", "daf86e0c6fe22909", "f861c56a34758d84", "fa51212d2d286562",
];

#[test]
fn durango_texture2dms_reads_become_ms_loads_with_the_fragment_operand() {
    std::env::set_var("TOMBOX_XS_MSAA_DIM", "1");
    assert!(hlsl::msaa_dim_on());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../input/shader_cache/dump/GbxGame");
    if !dir.is_dir() {
        eprintln!("skipped: no shader dump at {}", dir.display());
        return;
    }
    let mut seen = 0;
    for h in MS_READERS {
        for v in ["0200000000000000", "8200000000000000"] {
            let p = dir.join(format!("{h}_{v}.xbox"));
            let Ok(blob) = std::fs::read(&p) else { continue };
            let ex = extract(&blob).unwrap_or_else(|e| panic!("{h}_{v}: extract {e}"));
            let t = hlsl::translate(&ex).unwrap_or_else(|e| panic!("{h}_{v}: translate {e}"));
            let ms: Vec<&str> = t.hlsl.lines().filter(|l| l.starts_with("Texture2DMS<")).collect();
            assert!(!ms.is_empty(), "{h}_{v}: no Texture2DMS declaration");
            for d in &ms {
                let slot = d.split_whitespace().nth(1).unwrap();
                let load = format!("{slot}.Load(int2(");
                assert!(t.hlsl.lines().any(|l| l.contains(&load) && l.contains("), (int)v")), "{h}_{v}: {slot} has no per-sample Load");
            }
            if h == "0ff9ad9a34cd4e65" {
                assert!(t.hlsl.contains("tex0.GetDimensions(") && t.hlsl.contains("0x88888810u"), "{h}_{v}: FMASK read of t1 is not the identity map");
                assert!(!t.hlsl.contains("tex1.Load("), "{h}_{v}: FMASK slot still loaded from the bound view");
            }
            dxc::compile(&t.hlsl, &dxc::profile_for(&t.profile)).unwrap_or_else(|e| panic!("{h}_{v}: dxc {e}"));
            seen += 1;
        }
    }
    eprintln!("{seen} MS-reading shader variants translated and compiled");
}
