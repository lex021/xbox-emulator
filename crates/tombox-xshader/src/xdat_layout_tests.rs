use super::*;

fn xdat(entries: &[(u32, &[u8])]) -> Vec<u8> {
    let table = entries.len() * 12;
    let mut x = b"XDAT".to_vec();
    x.extend(3u32.to_le_bytes());
    x.extend((table as u32).to_le_bytes());
    let mut at = 12 + table;
    let mut body = Vec::new();
    for (kind, data) in entries {
        x.extend(kind.to_le_bytes());
        x.extend((at as u32).to_le_bytes());
        x.extend((data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        at += data.len();
    }
    x.extend(body);
    x
}

#[test]
fn bitcode_section_is_found_by_the_xdat_table() {
    let bc = b"BC\xC0\xDE\x21\x0c\x00\x00";
    let x = xdat(&[(7, bc), (16, &[1, 0, 0, 0]), (8, &[0, 0x31])]);
    assert_eq!(xdat_bitcode_section(&x), Some((48, bc.len())));
}

#[test]
fn nobc_placeholder_is_not_a_bitcode_section() {
    let x = xdat(&[(0, &[0x78, 0x9c, 0, 0]), (3, &[0; 8]), (7, b"NOBC")]);
    assert_eq!(xdat_bitcode_section(&x), None);
}

#[test]
fn bitcode_section_past_the_end_is_rejected() {
    let mut x = xdat(&[(7, b"BC\xC0\xDE")]);
    x[16..20].copy_from_slice(&0x1000u32.to_le_bytes());
    assert_eq!(xdat_bitcode_section(&x), None);
    let mut y = xdat(&[(7, b"BC\xC0\xDE")]);
    y[8..12].copy_from_slice(&13u32.to_le_bytes());
    assert_eq!(xdat_bitcode_section(&y), None);
}

#[test]
fn title_jit_dxil_blobs_report_their_layout() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../input/shader_cache/titles/GbxGame/game_scan");
    let cases = [("0032d78918638e93", "XDAT holds an LLVM DXIL bitcode section"), ("25884d77a3975b74", "XDAT holds an LLVM DXIL bitcode section"), ("1c8bf1744097e36e", "DXIL+24 is LLVM DXIL bitcode")];
    let mut seen = 0;
    for (hash, layout) in cases {
        let Ok(b) = std::fs::read(dir.join(format!("{hash}_0000000000000000.xbox"))) else { continue };
        let e = extract(&b).unwrap_err();
        assert!(e.starts_with(layout), "{hash}: {e}");
        assert!(e.contains("runtime-JIT DXIL"), "{hash}: {e}");
        seen += 1;
    }
    if seen == 0 {
        eprintln!("no GbxGame game_scan blobs; skipped");
    }
}
