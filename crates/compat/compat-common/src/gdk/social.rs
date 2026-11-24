use super::events::Registry;
use super::{data_root, log, title, E_INVALIDARG, S_OK};
use core::ffi::c_void;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn social_dir() -> PathBuf {
    data_root().join("social")
}

pub fn friends() -> Vec<(u64, String)> {
    let mut v = Vec::new();
    let mut add = |x: u64, n: String| {
        if x != 0 && x != super::user::user().xuid && !v.iter().any(|f: &(u64, String)| f.0 == x) {
            v.push((x, n));
        }
    };
    if let Some(s) = super::setting("friends") {
        for part in s.split(',') {
            let mut it = part.splitn(2, ':');
            if let (Some(x), Some(n)) = (it.next().and_then(|x| x.trim().parse::<u64>().ok()), it.next()) {
                add(x, n.trim().to_string());
            }
        }
    }
    if let Ok(t) = std::fs::read_to_string(social_dir().join("friends.json")) {
        if let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(&t) {
            for f in a {
                if let (Some(x), Some(n)) = (f["xuid"].as_str().and_then(|x| x.parse().ok()).or(f["xuid"].as_u64()), f["gamertag"].as_str()) {
                    add(x, n.to_string());
                }
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(social_dir().join("profiles")) {
        for e in rd.flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                if let Ok(p) = serde_json::from_str::<serde_json::Value>(&t) {
                    if let (Some(x), Some(n)) = (p["xuid"].as_str().and_then(|x| x.parse().ok()), p["gamertag"].as_str()) {
                        add(x, n.to_string());
                    }
                }
            }
        }
    }
    v
}

pub fn publish_profile() {
    let u = super::user::user();
    let d = social_dir().join("profiles");
    let _ = std::fs::create_dir_all(&d);
    let p = serde_json::json!({"xuid": u.xuid.to_string(), "gamertag": u.gamertag, "titleId": title().title_id.map(|t| format!("{t:08X}"))});
    let _ = std::fs::write(d.join(format!("{}.json", u.xuid)), p.to_string());
}

pub fn name_for(xuid: u64) -> String {
    friends().into_iter().find(|f| f.0 == xuid).map(|f| f.1).unwrap_or_else(|| format!("Player {xuid}"))
}

pub fn relationship_text(xuid: u64) -> String {
    if xuid == super::user::user().xuid {
        "This is you.".into()
    } else if friends().iter().any(|f| f.0 == xuid) {
        "Friend".into()
    } else {
        "Not on your friends list".into()
    }
}

pub fn achievements_text(title_id: u32) -> String {
    let p = data_root().join("achievements").join(format!("{title_id:08X}.json"));
    let list: Vec<String> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|a| format!("{}  {}", if a["unlocked"].as_bool().unwrap_or(false) { "[x]" } else { "[ ]" }, a["name"].as_str().unwrap_or("?")))
        .collect();
    if list.is_empty() {
        format!("Title {title_id:08X}\n\nNo achievements unlocked yet.")
    } else {
        format!("Title {title_id:08X}\n\n{}", list.join("\n"))
    }
}

pub fn activity_session() -> String {
    std::fs::read_to_string(social_dir().join("activity").join(format!("{}.txt", super::user::user().xuid))).unwrap_or_default().trim().to_string()
}

fn inbox(xuid: u64) -> PathBuf {
    social_dir().join("invites").join(format!("{xuid}.jsonl"))
}

pub fn invite_uri(sender: u64, session: &str, ctx: &str) -> String {
    let tid = title().title_id.map(|t| format!("{t:08X}")).unwrap_or_default();
    format!("ms-xbl-{}://inviteHandleAccept/?invitedXuid={}&handle={}&senderXuid={sender}&context={}", tid.to_ascii_lowercase(), super::user::user().xuid, urlenc(session), urlenc(ctx))
}

fn urlenc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn send_invites(to: &[u64], session: &str, text: &str, ctx: &str) -> i32 {
    let me = super::user::user();
    let mut hr = S_OK;
    for &x in to {
        let p = inbox(x);
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let rec = serde_json::json!({
            "from": me.xuid.to_string(),
            "fromGamertag": me.gamertag,
            "titleId": title().title_id.map(|t| format!("{t:08X}")),
            "session": session,
            "text": text,
            "context": ctx,
            "time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        });
        match std::fs::OpenOptions::new().create(true).append(true).open(&p).and_then(|mut f| writeln!(f, "{rec}")) {
            Ok(()) => log("XGameUiShowSendGameInviteAsync", &format!("invite to {x} queued in {}", p.display())),
            Err(e) => {
                log("XGameUiShowSendGameInviteAsync", &format!("invite to {x}: {e}"));
                hr = super::win::hr_from_win32(e.raw_os_error().unwrap_or(29) as u32);
            }
        }
    }
    hr
}

static INVITES: Registry = Registry::new();

fn deliver(uri: String) {
    log("XGameInvite", &format!("invite accepted: {uri}"));
    INVITES.fire(None, move |r| unsafe {
        let cb: unsafe extern "system" fn(*mut c_void, *const u8) = core::mem::transmute(r.cb);
        let c = std::ffi::CString::new(uri.clone()).unwrap_or_default();
        cb(r.ctx as *mut c_void, c.as_ptr() as *const u8);
    });
}

pub fn deliver_uri(uri: String) {
    deliver(uri)
}

fn launch_invite() -> Option<String> {
    std::env::args().find(|a| a.to_ascii_lowercase().starts_with("ms-xbl-") && a.contains("invite"))
}

fn start_inbox_watch() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    publish_profile();
    let me = super::user::user().xuid;
    std::thread::spawn(move || {
        let p = inbox(me);
        let mut seen = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if len < seen {
                seen = 0;
            }
            if len == seen {
                continue;
            }
            let Ok(all) = std::fs::read(&p) else { continue };
            let new = String::from_utf8_lossy(&all[seen as usize..]).into_owned();
            seen = len;
            for line in new.lines() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                let from: u64 = v["from"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0);
                let who = v["fromGamertag"].as_str().unwrap_or("A friend").to_string();
                let session = v["session"].as_str().unwrap_or("").to_string();
                let ctx = v["context"].as_str().unwrap_or("").to_string();
                let text = v["text"].as_str().unwrap_or("").to_string();
                std::thread::spawn(move || {
                    let d = super::win::Dialog {
                        title: "Game invite".into(),
                        text: format!("{who} invited you to play.{}", if text.is_empty() { String::new() } else { format!("\n\n{text}") }),
                        buttons: vec!["Accept".into(), "Ignore".into()],
                        default: 0,
                        cancel: Some(1),
                        ..Default::default()
                    };
                    if super::win::show(d, None).button == Some(0) {
                        deliver(invite_uri(from, &session, &ctx));
                    }
                });
            }
        }
    });
}

pub unsafe extern "system" fn register_invite(_t: super::This, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    if cb == 0 {
        return E_INVALIDARG;
    }
    let hr = INVITES.register(0, queue, ctx, cb, token);
    if hr >= 0 {
        start_inbox_watch();
        if let Some(uri) = launch_invite() {
            static ONCE: AtomicBool = AtomicBool::new(false);
            if !ONCE.swap(true, Ordering::AcqRel) {
                deliver(uri);
            }
        }
    }
    hr
}

pub unsafe extern "system" fn unregister_invite(_t: super::This, token: u64, wait: bool) -> bool {
    INVITES.unregister(None, token, wait)
}
