pub const FW_PLUMBER_PREFIX: [u8; 14] = [0x20, 0xed, 0xdb, 0xc3, 0xb6, 0x0b, 0xf3, 0x4b, 0x82, 0x8d, 0x96, 0x73, 0x2e, 0x1e];

pub const FW_PLUMBER_POSITION: usize = 9;

pub use compat_common::netview::wfp_names::NAMES;

fn fw_plumber(g: &[u8; 16]) -> Option<String> {
    if g[..14] != FW_PLUMBER_PREFIX {
        return None;
    }
    let (kind, layer) = (g[14], g[15]);
    Some(match kind {
        0 => format!("FwPlumberCalloutQueryUser_layer_{}", layer),
        1 => format!("FwPlumberCalloutLogging_layer_{}", layer),
        2 => format!("FwPlumberCalloutSecondaryConnections_layer_{}", layer),
        3 => format!("FwPlumberCalloutFlowEstablished_layer_{}", layer),
        4 => format!("FwPlumberCalloutStreamFlowAnalysis_layer_{}", layer),
        _ => format!("FwPlumberCallout_type_{}_layer_{}", kind, layer),
    })
}

pub fn guid_text(g: Option<&[u8; 16]>) -> String {
    let Some(g) = g else {
        return "(nullptr)".to_string();
    };
    if g.iter().all(|b| *b == 0) {
        return "GUID_NULL".to_string();
    }
    for (i, (k, name)) in NAMES.iter().enumerate() {
        if i == FW_PLUMBER_POSITION {
            if let Some(s) = fw_plumber(g) {
                return s;
            }
        }
        if k == g {
            return name.to_string();
        }
    }
    let d1 = u32::from_le_bytes(g[0..4].try_into().unwrap());
    let d2 = u16::from_le_bytes(g[4..6].try_into().unwrap());
    let d3 = u16::from_le_bytes(g[6..8].try_into().unwrap());
    format!("{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}", d1, d2, d3, g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15])
}
