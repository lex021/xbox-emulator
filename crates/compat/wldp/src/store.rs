use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::HResult;

pub const KEY_CI_CONFIG: &str = "System\\CurrentControlSet\\Control\\CI\\Config";
pub const VALUE_BLOCKLIST: &str = "VulnerableDriverBlocklistEnable";
pub const KEY_DEVELOPER_UNLOCK: &str = "SYSTEM\\CurrentControlSet\\Control\\CI\\Developer\\DeveloperUnlock";
pub const DPP_HSTI: &str = "%SystemDrive%\\DPP\\WatermarkHSTITestComplete.bin";
pub const DPP_SB: &str = "%SystemDrive%\\DPP\\WatermarkSBTestComplete.bin";
pub const CI_DATA: &str = "%SystemRoot%\\System32\\CodeIntegrity\\Data";
pub const NON_PRODUCTION_ERRORS: &str = "%SystemDrive%\\Data\\Users\\DefaultAccount\\Documents\\Non-Production Errors.txt";

pub const RETAIL_CI_POLICY_OPTIONS: u32 = 0x2400_0000;

pub const STATE_KEYS: [&str; 5] = ["ci_policy_options", "state_separation", "manufacturing_mode", "blocklist_enable", "developer_unlock"];

pub fn data_root() -> PathBuf {
    if let Ok(p) = std::env::var("TOMBOX_GDK_ROOT") {
        return PathBuf::from(p);
    }
    match std::env::var_os("LOCALAPPDATA") {
        Some(p) => PathBuf::from(p).join("XboxEmulator"),
        None => std::env::temp_dir().join("XboxEmulator"),
    }
}

pub fn log(func: &str, detail: &str) {
    let path = std::env::var("TOMBOX_LOG").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("tombox_compat.log"));
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(format!("{ts} [wldp] {func} {detail}\n").as_bytes());
    }
}

pub fn io_hresult(e: &std::io::Error) -> HResult {
    match e.raw_os_error() {
        Some(c) if c > 0 => ((c as u32 & 0xffff) | 0x8007_0000) as i32,
        Some(c) => c,
        None => 0x8000_4005u32 as i32,
    }
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open() -> Store {
        Store::at(data_root().join("wldp"))
    }

    pub fn at(root: impl Into<PathBuf>) -> Store {
        let s = Store { root: root.into() };
        s.init();
        s
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn state_path(&self) -> PathBuf {
        self.root.join("state.txt")
    }

    fn init(&self) {
        let _ = std::fs::create_dir_all(self.console_path("%SystemDrive%\\DPP"));
        let _ = std::fs::create_dir_all(self.console_path(CI_DATA));
        let _ = std::fs::create_dir_all(self.console_path("%SystemDrive%\\Data\\Users\\DefaultAccount\\Documents"));
        let _ = std::fs::create_dir_all(self.title_cwd());
        if !self.state_path().exists() {
            let _ = self.write_state(&[
                ("ci_policy_options".to_string(), format!("{RETAIL_CI_POLICY_OPTIONS:#x}")),
                ("state_separation".to_string(), "0".to_string()),
                ("manufacturing_mode".to_string(), "0".to_string()),
            ]);
        }
    }

    fn read_state(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(self.state_path())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect()
    }

    fn write_state(&self, kv: &[(String, String)]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let text: String = kv.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        let tmp = self.root.join("state.txt.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, self.state_path())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.read_state().into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn set(&self, key: &str, value: Option<&str>) -> std::io::Result<()> {
        let mut kv: Vec<(String, String)> = self.read_state().into_iter().filter(|(k, _)| k != key).collect();
        if let Some(v) = value {
            kv.push((key.to_string(), v.to_string()));
        }
        self.write_state(&kv)
    }

    fn get_u32(&self, key: &str) -> Option<u32> {
        let v = self.get(key)?;
        match v.strip_prefix("0x") {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => v.parse().ok(),
        }
    }

    pub fn ci_policy_options(&self) -> u32 {
        self.get_u32("ci_policy_options").unwrap_or(RETAIL_CI_POLICY_OPTIONS)
    }

    pub fn state_separation(&self) -> bool {
        self.get_u32("state_separation").unwrap_or(0) != 0
    }

    pub fn manufacturing_mode(&self) -> bool {
        self.get_u32("manufacturing_mode").unwrap_or(0) != 0
    }

    pub fn blocklist_enable(&self) -> Option<u32> {
        self.get_u32("blocklist_enable")
    }

    pub fn set_blocklist_enable(&self, v: u32) -> std::io::Result<()> {
        self.set("blocklist_enable", Some(&v.to_string()))
    }

    pub fn developer_unlock(&self) -> Option<String> {
        self.get("developer_unlock")
    }

    pub fn set_developer_unlock(&self, sddl: Option<&str>) -> std::io::Result<()> {
        self.set("developer_unlock", sddl)
    }

    pub fn console_path(&self, p: &str) -> PathBuf {
        let drive = self.root.join("SystemDrive");
        let lower = p.to_ascii_lowercase();
        let (base, rest) = if lower.starts_with("%systemroot%") {
            (drive.join("Windows"), &p["%SystemRoot%".len()..])
        } else if lower.starts_with("%systemdrive%") {
            (drive, &p["%SystemDrive%".len()..])
        } else {
            (drive, p)
        };
        rest.split('\\').filter(|c| !c.is_empty()).fold(base, |acc, c| acc.join(c))
    }

    pub fn title_cwd(&self) -> PathBuf {
        self.root.join("TitleCwd")
    }

    pub fn title_relative_path(&self, p: &str) -> PathBuf {
        p.split('\\').filter(|c| !c.is_empty()).fold(self.title_cwd(), |acc, c| acc.join(c))
    }

    pub fn read_file(&self, p: &str, buf: &mut [u8]) -> HResult {
        use std::io::Read;
        let mut f = match std::fs::File::open(self.console_path(p)) {
            Ok(f) => f,
            Err(e) => return io_hresult(&e),
        };
        let mut n = 0;
        while n < buf.len() {
            match f.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) => return io_hresult(&e),
            }
        }
        (n != buf.len()) as HResult
    }

    pub fn write_file(&self, p: &str, data: &[u8]) -> HResult {
        let hr = match std::fs::write(self.console_path(p), data) {
            Ok(()) => 0,
            Err(e) => io_hresult(&e),
        };
        if hr < 0 {
            self.delete_file(p);
        }
        hr
    }

    pub fn delete_file(&self, p: &str) -> HResult {
        match std::fs::remove_file(self.console_path(p)) {
            Ok(()) => 0,
            Err(e) => {
                let hr = io_hresult(&e);
                if hr == 0x8007_0002u32 as i32 {
                    0
                } else {
                    hr
                }
            }
        }
    }

    pub fn file_exists(&self, p: &str) -> bool {
        self.console_path(p).is_file()
    }
}
