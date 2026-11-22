use compat_common::record;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Req {
    pub stem: String,
    pub out: PathBuf,
    pub done: PathBuf,
}

static QUEUE: Mutex<VecDeque<Req>> = Mutex::new(VecDeque::new());
static WANTED: AtomicBool = AtomicBool::new(false);

pub fn dir() -> Option<PathBuf> {
    static D: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    D.get_or_init(|| std::env::var("TOMBOX_CAPTURE_DIR").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v != "0").map(PathBuf::from)).clone()
}

pub fn every_secs(v: Option<&str>) -> Option<f64> {
    v.and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s > 0.0).map(|s| s.max(0.25))
}

pub fn parse_request(dir: &Path, stem: &str, body: &str) -> Req {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let out = if line.is_empty() { dir.join(format!("{stem}.bmp")) } else { PathBuf::from(line) };
    Req { stem: stem.to_string(), out, done: dir.join(format!("{stem}.done")) }
}

pub fn request_stem(name: &str) -> Option<&str> {
    name.strip_suffix(".req").filter(|s| !s.is_empty() && !s.contains('.'))
}

fn push(r: Req) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push_back(r);
    WANTED.store(true, Ordering::Release);
}

fn poll(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let pid = std::process::id();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(stem) = request_stem(&name) else { continue };
        let claimed = dir.join(format!("{stem}.claimed{pid}"));
        if std::fs::rename(e.path(), &claimed).is_err() {
            continue;
        }
        let body = std::fs::read_to_string(&claimed).unwrap_or_default();
        let _ = std::fs::remove_file(&claimed);
        push(parse_request(dir, stem, &body));
    }
}

pub fn start() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(d) = dir() else { return };
    ONCE.call_once(move || {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join(format!("alive.{}", std::process::id())), b"presenting");
        let every = every_secs(std::env::var("TOMBOX_CAPTURE_EVERY").ok().as_deref());
        record("d3d12_xs", "capture", &format!("backbuffer capture armed: requests {}\\<name>.req{}", d.display(), every.map(|s| format!(", automatic every {s} s")).unwrap_or_default()));
        std::thread::Builder::new()
            .name("tombox-capture".into())
            .spawn(move || {
                let t0 = std::time::Instant::now();
                let mut next = every;
                loop {
                    poll(&d);
                    if let (Some(n), Some(e)) = (next, every) {
                        let el = t0.elapsed().as_secs_f64();
                        if el >= n {
                            let stem = format!("auto_{:07}ms", (el * 1000.0) as u64);
                            push(parse_request(&d, &stem, ""));
                            next = Some(n + e);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .ok();
    });
}

pub fn take() -> Option<Req> {
    if !WANTED.load(Ordering::Acquire) {
        return None;
    }
    let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let r = q.pop_front();
    if q.is_empty() {
        WANTED.store(false, Ordering::Release);
    }
    r
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, data)?;
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path)?;
    }
    Ok(())
}

pub fn done_line(ok: Result<(&Path, u32, u32, u64, u32), &str>) -> String {
    match ok {
        Ok((p, w, h, frame, fmt)) => format!("ok {w} {h} {frame} {fmt} {}\n", p.display()),
        Err(e) => format!("error {e}\n"),
    }
}

pub fn fail(r: &Req, why: &str) {
    let _ = write_atomic(&r.done, done_line(Err(why)).as_bytes());
    record("d3d12_xs", "capture", &format!("{} failed: {why}", r.stem));
}

pub fn finish(r: Req, data: Vec<u8>, width: u32, height: u32, format: u32, frame: u64) {
    std::thread::spawn(move || {
        let pitch = crate::present_snap::pitch_for(width, format) as usize;
        if data.len() < pitch * height as usize {
            fail(&r, "readback shorter than the back buffer");
            return;
        }
        let img = crate::present_snap::bmp(&data, pitch, width as usize, height as usize, format);
        if let Some(p) = r.out.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        match write_atomic(&r.out, &img) {
            Ok(()) => {
                let _ = write_atomic(&r.done, done_line(Ok((&r.out, width, height, frame, format))).as_bytes());
            }
            Err(e) => fail(&r, &format!("write {}: {e}", r.out.display())),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_names_and_bodies() {
        assert_eq!(request_stem("shot1.req"), Some("shot1"));
        assert_eq!(request_stem("shot1.claimed42"), None);
        assert_eq!(request_stem("a.tmp.req"), None);
        assert_eq!(request_stem(".req"), None);
        assert_eq!(request_stem("shot.done"), None);
        let d = Path::new("C:\\cap");
        let r = parse_request(d, "s1", "  \r\nD:\\runs\\frame_0020s.bmp\r\n");
        assert_eq!(r.out, PathBuf::from("D:\\runs\\frame_0020s.bmp"));
        assert_eq!(r.done, d.join("s1.done"));
        assert_eq!(parse_request(d, "s2", "").out, d.join("s2.bmp"));
    }

    #[test]
    fn interval_parsing() {
        assert_eq!(every_secs(Some("2")), Some(2.0));
        assert_eq!(every_secs(Some("0.01")), Some(0.25));
        assert_eq!(every_secs(Some("0")), None);
        assert_eq!(every_secs(Some("x")), None);
        assert_eq!(every_secs(None), None);
    }

    #[test]
    fn done_lines() {
        assert_eq!(done_line(Ok((Path::new("D:\\a.bmp"), 1920, 1080, 77, 87))), "ok 1920 1080 77 87 D:\\a.bmp\n");
        assert_eq!(done_line(Err("boom")), "error boom\n");
    }

    #[test]
    fn queue_and_files_round_trip() {
        let d = std::env::temp_dir().join(format!("tombox_capture_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join("pic.bmp");
        std::fs::write(d.join("r1.req"), format!("{}\n", out.display())).unwrap();
        std::fs::write(d.join("ignored.txt"), "x").unwrap();
        poll(&d);
        assert!(!d.join("r1.req").exists());
        let r = take().expect("request queued");
        assert_eq!(r.out, out);
        assert!(take().is_none());
        let pitch = crate::present_snap::pitch_for(2, 87) as usize;
        let mut px = vec![0u8; pitch * 2];
        px[..4].copy_from_slice(&[10, 20, 30, 255]);
        finish(r.clone(), px, 2, 2, 87, 5);
        let t0 = std::time::Instant::now();
        while !r.done.exists() && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let line = std::fs::read_to_string(&r.done).unwrap();
        assert!(line.starts_with("ok 2 2 5 87 "), "{line}");
        let b = std::fs::read(&out).unwrap();
        assert_eq!(&b[0..2], b"BM");
        assert_eq!(&b[62..65], &[10, 20, 30]);
        finish(r.clone(), vec![0u8; 4], 2, 2, 87, 6);
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_secs() < 5 && !std::fs::read_to_string(&r.done).unwrap_or_default().starts_with("error") {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(std::fs::read_to_string(&r.done).unwrap().starts_with("error readback shorter"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
