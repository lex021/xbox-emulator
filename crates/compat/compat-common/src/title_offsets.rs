use std::collections::HashMap;
use std::sync::OnceLock;

pub const VAR: &str = "TOMBOX_PROBE_OFFSETS";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Offsets {
    pub source: String,
    pub exe: String,
    pub pe_timestamp: Option<u32>,
    values: HashMap<String, usize>,
    fields: HashMap<String, Vec<(String, usize, usize)>>,
    bytes: HashMap<String, Vec<u8>>,
}

impl Offsets {
    pub fn get(&self, key: &str) -> Option<usize> {
        self.values.get(key).copied()
    }

    pub fn require(&self, keys: &[&str]) -> Result<Vec<usize>, String> {
        let missing: Vec<&str> = keys.iter().copied().filter(|k| !self.values.contains_key(*k)).collect();
        if missing.is_empty() {
            Ok(keys.iter().map(|k| self.values[*k]).collect())
        } else {
            Err(format!("{} lacks {}", self.source, missing.join(", ")))
        }
    }

    pub fn fields(&self, key: &str) -> &[(String, usize, usize)] {
        self.fields.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        self.bytes.get(key).map(|v| v.as_slice())
    }

    pub fn require_bytes(&self, keys: &[&str]) -> Result<Vec<Vec<u8>>, String> {
        let missing: Vec<&str> = keys.iter().copied().filter(|k| !self.bytes.contains_key(*k)).collect();
        if missing.is_empty() {
            Ok(keys.iter().map(|k| self.bytes[*k].clone()).collect())
        } else {
            Err(format!("{} lacks bytes {}", self.source, missing.join(", ")))
        }
    }

    pub fn matches_image(&self, stamp: u32) -> bool {
        self.pe_timestamp.map(|t| t == stamp).unwrap_or(true)
    }
}

pub fn parse_num(v: &serde_json::Value) -> Option<usize> {
    match v {
        serde_json::Value::Number(n) => n.as_u64().map(|x| x as usize),
        serde_json::Value::String(s) => {
            let t = s.trim().replace('_', "");
            match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                Some(h) => usize::from_str_radix(h, 16).ok(),
                None => t.parse().ok(),
            }
        }
        _ => None,
    }
}

pub fn parse_bytes(v: &str) -> Option<Vec<u8>> {
    let t: String = v.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() || t.len() % 2 != 0 {
        return None;
    }
    (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).ok()).collect()
}

pub fn parse(text: &str, source: &str) -> Result<Offsets, String> {
    let j: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("{source}: {e}"))?;
    let exe = j.get("exe").and_then(|v| v.as_str()).ok_or_else(|| format!("{source}: no \"exe\""))?.to_string();
    let pe_timestamp = match j.get("pe_timestamp") {
        Some(v) => Some(parse_num(v).ok_or_else(|| format!("{source}: bad pe_timestamp"))? as u32),
        None => None,
    };
    let mut values = HashMap::new();
    if let Some(m) = j.get("values").and_then(|v| v.as_object()) {
        for (k, v) in m {
            values.insert(k.clone(), parse_num(v).ok_or_else(|| format!("{source}: bad value for {k}"))?);
        }
    }
    let mut fields = HashMap::new();
    if let Some(m) = j.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in m {
            let arr = v.as_array().ok_or_else(|| format!("{source}: fields.{k} is not a list"))?;
            let mut out = Vec::new();
            for e in arr {
                let t = e.as_array().filter(|t| t.len() == 3).ok_or_else(|| format!("{source}: fields.{k} entries are [name, offset, size]"))?;
                let name = t[0].as_str().ok_or_else(|| format!("{source}: fields.{k} name"))?.to_string();
                let off = parse_num(&t[1]).ok_or_else(|| format!("{source}: fields.{k}.{name} offset"))?;
                let size = parse_num(&t[2]).filter(|s| (1..=8).contains(s)).ok_or_else(|| format!("{source}: fields.{k}.{name} size"))?;
                out.push((name, off, size));
            }
            fields.insert(k.clone(), out);
        }
    }
    let mut bytes = HashMap::new();
    if let Some(m) = j.get("bytes").and_then(|v| v.as_object()) {
        for (k, v) in m {
            let b = v.as_str().and_then(parse_bytes).ok_or_else(|| format!("{source}: bytes.{k} is not a hex byte string"))?;
            bytes.insert(k.clone(), b);
        }
    }
    Ok(Offsets { source: source.to_string(), exe, pe_timestamp, values, fields, bytes })
}

fn dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var(VAR) {
        if !p.trim().is_empty() {
            return Some(std::path::PathBuf::from(p.trim()));
        }
    }
    std::env::var("LOCALAPPDATA").ok().map(|d| std::path::PathBuf::from(d).join("TomBox").join("probe_offsets"))
}

pub fn path_for(exe: &str) -> Option<std::path::PathBuf> {
    let d = dir()?;
    if d.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
        return Some(d);
    }
    let stem = std::path::Path::new(exe).file_stem()?.to_string_lossy().to_string();
    Some(d.join(format!("{stem}.json")))
}

pub fn load_for(exe: &str) -> Result<Offsets, String> {
    let p = path_for(exe).ok_or_else(|| "no offsets directory (LOCALAPPDATA unset)".to_string())?;
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let o = parse(&text, &p.display().to_string())?;
    if !o.exe.eq_ignore_ascii_case(exe) {
        return Err(format!("{} is for {}, not {exe}", o.source, o.exe));
    }
    Ok(o)
}

pub fn current_exe_name() -> Option<String> {
    std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

pub fn for_this_title() -> Option<&'static Offsets> {
    static O: OnceLock<Option<Offsets>> = OnceLock::new();
    O.get_or_init(|| current_exe_name().and_then(|n| load_for(&n).ok())).as_ref()
}

pub fn status_for_this_title() -> String {
    match current_exe_name() {
        Some(n) => match load_for(&n) {
            Ok(o) => format!("offsets from {}", o.source),
            Err(e) => e,
        },
        None => "no executable name".into(),
    }
}

pub unsafe fn image_timestamp(base: *const u8) -> u32 {
    let pe = core::ptr::read_unaligned(base.add(0x3c) as *const u32) as usize;
    core::ptr::read_unaligned(base.add(pe + 8) as *const u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values_fields_and_timestamp() {
        let o = parse(r#"{"exe":"Game.exe","pe_timestamp":"0x12345678","values":{"a":"0x10","b":32,"c":"1_000"},"fields":{"rec":[["x","0x8",4],["y",12,8]]}}"#, "t").unwrap();
        assert_eq!(o.exe, "Game.exe");
        assert_eq!(o.pe_timestamp, Some(0x1234_5678));
        assert_eq!((o.get("a"), o.get("b"), o.get("c"), o.get("d")), (Some(0x10), Some(32), Some(1000), None));
        assert_eq!(o.fields("rec"), &[("x".to_string(), 8, 4), ("y".to_string(), 12, 8)]);
        assert!(o.fields("none").is_empty());
        assert_eq!(o.require(&["a", "b"]), Ok(vec![0x10, 32]));
        assert!(o.require(&["a", "zz"]).unwrap_err().contains("zz"));
        assert!(o.matches_image(0x1234_5678) && !o.matches_image(1));
    }

    #[test]
    fn parses_byte_strings() {
        let o = parse(r#"{"exe":"a","bytes":{"p":"4C 8d3D","q":"00"}}"#, "t").unwrap();
        assert_eq!(o.bytes("p"), Some(&[0x4c, 0x8d, 0x3d][..]));
        assert_eq!(o.require_bytes(&["q"]), Ok(vec![vec![0]]));
        assert!(o.require_bytes(&["z"]).unwrap_err().contains("z"));
        assert!(parse(r#"{"exe":"a","bytes":{"p":"4C 8"}}"#, "t").unwrap_err().contains("bytes.p"));
        assert!(parse(r#"{"exe":"a","bytes":{"p":"zz"}}"#, "t").is_err());
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse("{", "t").is_err());
        assert!(parse(r#"{"values":{}}"#, "t").unwrap_err().contains("exe"));
        assert!(parse(r#"{"exe":"a","values":{"k":"zz"}}"#, "t").unwrap_err().contains("k"));
        assert!(parse(r#"{"exe":"a","fields":{"r":[["x",1,9]]}}"#, "t").unwrap_err().contains("size"));
        let o = parse(r#"{"exe":"a"}"#, "t").unwrap();
        assert!(o.matches_image(7));
    }

    #[test]
    fn loads_by_exe_stem_from_the_override_directory() {
        let d = std::env::temp_dir().join(format!("tombox_offsets_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Sample.json"), r#"{"exe":"Sample.exe","values":{"k":"0x40"}}"#).unwrap();
        std::env::set_var(VAR, &d);
        let o = load_for("Sample.exe").unwrap();
        assert_eq!(o.get("k"), Some(0x40));
        assert!(load_for("Other.exe").is_err());
        std::fs::write(d.join("Wrong.json"), r#"{"exe":"NotWrong.exe"}"#).unwrap();
        assert!(load_for("Wrong.exe").unwrap_err().contains("not Wrong.exe"));
        std::env::remove_var(VAR);
        let _ = std::fs::remove_dir_all(&d);
    }
}
