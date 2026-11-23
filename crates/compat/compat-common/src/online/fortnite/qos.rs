use std::borrow::Cow;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub const PING_TIMEOUT: &str = "1.0";
pub const DATACENTERS: &[(&str, &str)] = &[("VA", "NAE")];

static ECHOED: AtomicU64 = AtomicU64::new(0);

fn responder() -> &'static Option<u16> {
    static P: OnceLock<Option<u16>> = OnceLock::new();
    P.get_or_init(|| {
        let sock = match UdpSocket::bind(("127.0.0.1", 0)) {
            Ok(s) => {
                crate::netview::sockets::report_std(&s);
                s
            }
            Err(e) => {
                crate::record_raw("online", "[qos]", &format!("bind failed: {e}"));
                return None;
            }
        };
        let port = sock.local_addr().ok()?.port();
        let spawned = std::thread::Builder::new().name("tombox-qos-echo".into()).spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        if sock.send_to(&buf[..n], from).is_ok() && ECHOED.fetch_add(1, Ordering::Relaxed) < 8 {
                            crate::record_raw("online", "[qos]", &format!("echo {n} bytes to {from}"));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                    Err(e) => {
                        crate::record_raw("online", "[qos]", &format!("responder stopped: {e}"));
                        return;
                    }
                }
            }
        });
        if spawned.is_err() {
            return None;
        }
        crate::record_raw("online", "[qos]", &format!("echo responder on 127.0.0.1:{port}"));
        Some(port)
    })
}

pub fn port() -> Option<u16> {
    *responder()
}

pub fn echoed() -> u64 {
    ECHOED.load(Ordering::Relaxed)
}

pub fn hotfix_ini_section() -> String {
    let Some(port) = port() else { return String::new() };
    let mut s = format!("\n[/Script/Qos.QosRegionManager]\nPingTimeout={PING_TIMEOUT}\n!DatacenterDefinitions=ClearArray\n");
    for (id, region) in DATACENTERS {
        s.push_str(&format!("+DatacenterDefinitions=(Id=\"{id}\", RegionId=\"{region}\", bEnabled=true, Servers[0]=(Address=\"127.0.0.1\", Port={port}))\n"));
    }
    s
}

pub fn system_file(name: &str, content: &'static str) -> Cow<'static, str> {
    if !name.eq_ignore_ascii_case("DefaultEngine.ini") {
        return Cow::Borrowed(content);
    }
    let section = hotfix_ini_section();
    if section.is_empty() {
        return Cow::Borrowed(content);
    }
    let mut out = String::with_capacity(content.len() + section.len());
    for line in content.lines() {
        if line.trim_start().starts_with("PingTimeout=") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&section);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn echo_roundtrip() {
        let p = port().expect("responder");
        let c = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let payload = [0x01u8, 0x00, 0xde, 0xad, 0xbe, 0xef, 0x42, 0x13, 0x37];
        c.send_to(&payload, ("127.0.0.1", p)).unwrap();
        let mut buf = [0u8; 64];
        let (n, from) = c.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], &payload);
        assert_eq!(from.port(), p);
    }

    #[test]
    fn engine_ini_points_nae_at_responder() {
        let p = port().unwrap();
        let base = "[/Script/Qos.QosRegionManager]\nNumTestsPerRegion=1\nPingTimeout=0.1\n\n[HTTP]\nX=1\n";
        let out = system_file("DefaultEngine.ini", base);
        assert!(!out.contains("PingTimeout=0.1"));
        assert!(out.contains("PingTimeout=1.0"));
        assert!(out.contains("!DatacenterDefinitions=ClearArray"));
        assert!(out.contains(&format!("+DatacenterDefinitions=(Id=\"VA\", RegionId=\"NAE\", bEnabled=true, Servers[0]=(Address=\"127.0.0.1\", Port={p}))")));
        assert!(out.contains("[HTTP]\nX=1"));
        assert_eq!(system_file("DefaultGame.ini", base), base);
        assert_eq!(system_file("DefaultEngine.ini", base), out);
    }
}
