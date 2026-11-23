use super::util::{b64, b64_decode, random_uuid};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const HUB_PORT: u16 = 47630;
pub const DISCOVERY_PORT: u16 = 47631;
pub const MAGIC_ASK: &str = "TOMBOX-LAN-DISCOVER 1";
pub const MAGIC_HOST: &str = "TOMBOX-LAN-HOST 1";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

pub type Handler = fn(&Value) -> Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    Off,
    Host { port: u16 },
    Client { hub: SocketAddr },
}

pub fn setting(key: &str) -> Option<String> {
    std::env::var(format!("TOMBOX_{}", key.to_ascii_uppercase())).ok().or_else(|| crate::cfg(key)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn handler(service: &str) -> Option<Handler> {
    match service {
        "playfab.lobby" => Some(super::playfab::lobby::hub_handle),
        "lan.echo" => Some(|v| json!({"echo": v})),
        "lan.devices" => Some(|_| json!({"devices": hub().devices()})),
        _ => None,
    }
}

pub fn parse_hub(s: &str) -> Option<SocketAddr> {
    let s = s.trim();
    let (ip, port) = match s.rsplit_once(':') {
        Some((i, p)) => (i, p.parse::<u16>().ok()?),
        None => (s, HUB_PORT),
    };
    let ip: Ipv4Addr = ip.parse().ok()?;
    Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
}

fn private(ip: Ipv4Addr) -> bool {
    crate::net_guard::is_private_v4(ip.octets())
}

fn discovery_port() -> u16 {
    setting("lan_discovery_port").and_then(|p| p.parse().ok()).unwrap_or(DISCOVERY_PORT)
}

pub fn device_id() -> &'static str {
    static D: OnceLock<String> = OnceLock::new();
    D.get_or_init(|| setting("lan_device_id").unwrap_or_else(random_uuid))
}

pub fn role() -> &'static Role {
    static R: OnceLock<Role> = OnceLock::new();
    R.get_or_init(decide)
}

fn decide() -> Role {
    let mode = setting("lan").unwrap_or_else(|| "off".into()).to_ascii_lowercase();
    let port = setting("lan_port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(HUB_PORT);
    let note = |m: &str| crate::record_raw("online", "lan", m);
    match mode.as_str() {
        "host" => host_role(port),
        "join" | "auto" => {
            let configured = setting("lan_host");
            let found = match &configured {
                Some(h) => match parse_hub(h) {
                    Some(a) => Some(a),
                    None => {
                        note(&format!("lan_host '{h}' is not an IPv4 address[:port]; ignoring it"));
                        None
                    }
                },
                None => discover(Duration::from_millis(1500), &[]),
            };
            match found {
                Some(SocketAddr::V4(a)) if private(*a.ip()) => {
                    crate::net_guard::allow_lan_peer(a.ip().octets());
                    match Client::connect(SocketAddr::V4(a)) {
                        Ok(c) => {
                            let _ = CLIENT.set(c);
                            note(&format!("joined LAN hub {a} as device {}", device_id()));
                            Role::Client { hub: SocketAddr::V4(a) }
                        }
                        Err(e) => {
                            note(&format!("LAN hub {a} did not accept a connection: {e}"));
                            if mode == "auto" { host_role(port) } else { Role::Off }
                        }
                    }
                }
                Some(a) => {
                    note(&format!("LAN hub {a} is not a private, link-local or loopback address; refusing it to stay offline"));
                    Role::Off
                }
                None if mode == "auto" => host_role(port),
                None => {
                    note("no LAN hub found; running private sessions on this instance only");
                    Role::Off
                }
            }
        }
        "off" | "0" | "false" | "no" => Role::Off,
        other => {
            note(&format!("unknown lan mode '{other}'; LAN sharing is off"));
            Role::Off
        }
    }
}

fn host_role(port: u16) -> Role {
    match start_hub(port) {
        Ok(p) => Role::Host { port: p },
        Err(e) => {
            crate::record_raw("online", "lan", &format!("could not host the LAN hub on port {port}: {e}"));
            Role::Off
        }
    }
}

pub fn discover(timeout: Duration, extra: &[SocketAddr]) -> Option<SocketAddr> {
    let dp = discovery_port();
    crate::net_guard::allow_lan_broadcast(dp);
    let sock = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    crate::netview::sockets::report_std(&sock);
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let ask = format!("{MAGIC_ASK} {}", device_id());
    let mut targets: Vec<SocketAddr> = vec![SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, dp)), SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, dp))];
    targets.extend_from_slice(extra);
    let end = Instant::now() + timeout;
    let mut buf = [0u8; 256];
    while Instant::now() < end {
        for t in &targets {
            let _ = sock.send_to(ask.as_bytes(), t);
        }
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            let Ok((n, from)) = sock.recv_from(&mut buf) else { break };
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            let Some(rest) = text.strip_prefix(MAGIC_HOST) else { continue };
            let mut parts = rest.split_whitespace();
            let Some(port) = parts.next().and_then(|p| p.parse::<u16>().ok()) else { continue };
            if parts.next() == Some(device_id()) {
                continue;
            }
            if let SocketAddr::V4(f) = from {
                if private(*f.ip()) {
                    return Some(SocketAddr::V4(SocketAddrV4::new(*f.ip(), port)));
                }
            }
        }
    }
    None
}

type Conn = Arc<Mutex<TcpStream>>;

#[derive(Default)]
pub struct Hub {
    conns: Mutex<HashMap<String, Conn>>,
    names: Mutex<HashMap<String, String>>,
    topics: Mutex<HashMap<String, Vec<String>>>,
    local_topics: Mutex<HashMap<String, Vec<Sender<Value>>>>,
    local_dgrams: Mutex<Option<Sender<(String, Vec<u8>)>>>,
}

fn hub() -> &'static Hub {
    static H: OnceLock<Hub> = OnceLock::new();
    H.get_or_init(Hub::default)
}

fn send_line(c: &Conn, v: &Value) -> bool {
    let mut line = v.to_string();
    line.push('\n');
    c.lock().map(|mut s| s.write_all(line.as_bytes()).is_ok()).unwrap_or(false)
}

impl Hub {
    fn devices(&self) -> Vec<Value> {
        let names = self.names.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = vec![json!({"device": device_id(), "name": crate::gdk::user::user().gamertag, "host": true})];
        for d in self.conns.lock().unwrap_or_else(|e| e.into_inner()).keys() {
            out.push(json!({"device": d, "name": names.get(d), "host": false}));
        }
        out
    }

    fn publish(&self, topic: &str, data: &Value, from: &str) -> usize {
        let frame = json!({"push": topic, "data": data, "from": from});
        let mut n = 0;
        if let Some(subs) = self.local_topics.lock().unwrap_or_else(|e| e.into_inner()).get_mut(topic) {
            subs.retain(|s| s.send(frame.clone()).is_ok());
            n += subs.len();
        }
        let devices = self.topics.lock().unwrap_or_else(|e| e.into_inner()).get(topic).cloned().unwrap_or_default();
        let conns = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        for d in devices {
            if d != from {
                if let Some(c) = conns.get(&d) {
                    if send_line(c, &frame) {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    fn dgram(&self, to: &str, from: &str, data: &[u8]) -> bool {
        if to == device_id() {
            return self.local_dgrams.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(false, |s| s.send((from.to_string(), data.to_vec())).is_ok());
        }
        let c = self.conns.lock().unwrap_or_else(|e| e.into_inner()).get(to).cloned();
        c.map_or(false, |c| send_line(&c, &json!({"dgram": from, "data": b64(data)})))
    }

    fn serve(&'static self, stream: TcpStream) {
        let peer = stream.peer_addr().ok();
        let Ok(w) = stream.try_clone() else { return };
        let conn: Conn = Arc::new(Mutex::new(w));
        let mut me: Option<String> = None;
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let Ok(f) = serde_json::from_str::<Value>(&line) else { continue };
            let id = f["id"].clone();
            let reply = match f["op"].as_str().unwrap_or("") {
                "hello" => {
                    let d = f["device"].as_str().filter(|d| !d.is_empty() && *d != device_id()).map(String::from);
                    match d {
                        Some(d) => {
                            self.conns.lock().unwrap_or_else(|e| e.into_inner()).insert(d.clone(), conn.clone());
                            self.names.lock().unwrap_or_else(|e| e.into_inner()).insert(d.clone(), f["name"].as_str().unwrap_or("").to_string());
                            crate::record_raw("online", "lan", &format!("device {d} ({}) joined from {:?}", f["name"].as_str().unwrap_or(""), peer));
                            me = Some(d);
                            json!({"re": id, "ok": true, "hub": device_id()})
                        }
                        None => json!({"re": id, "ok": false, "error": "a unique device id is required"}),
                    }
                }
                _ if me.is_none() => json!({"re": id, "ok": false, "error": "send hello first"}),
                "call" => {
                    let svc = f["service"].as_str().unwrap_or("");
                    match handler(svc) {
                        Some(h) => json!({"re": id, "ok": true, "result": h(&f["payload"])}),
                        None => json!({"re": id, "ok": false, "error": format!("no LAN service '{svc}'")}),
                    }
                }
                "sub" => {
                    let t = f["topic"].as_str().unwrap_or("").to_string();
                    let d = me.clone().unwrap_or_default();
                    let mut topics = self.topics.lock().unwrap_or_else(|e| e.into_inner());
                    let v = topics.entry(t).or_default();
                    if !v.contains(&d) {
                        v.push(d);
                    }
                    json!({"re": id, "ok": true})
                }
                "unsub" => {
                    let d = me.clone().unwrap_or_default();
                    if let Some(v) = self.topics.lock().unwrap_or_else(|e| e.into_inner()).get_mut(f["topic"].as_str().unwrap_or("")) {
                        v.retain(|x| *x != d);
                    }
                    json!({"re": id, "ok": true})
                }
                "pub" => {
                    let n = self.publish(f["topic"].as_str().unwrap_or(""), &f["data"], me.as_deref().unwrap_or(""));
                    json!({"re": id, "ok": true, "delivered": n})
                }
                "dgram" => {
                    let ok = self.dgram(f["to"].as_str().unwrap_or(""), me.as_deref().unwrap_or(""), &b64_decode(f["data"].as_str().unwrap_or("")));
                    json!({"re": id, "ok": ok})
                }
                other => json!({"re": id, "ok": false, "error": format!("unknown op '{other}'")}),
            };
            if !id.is_null() && !send_line(&conn, &reply) {
                break;
            }
        }
        if let Some(d) = me {
            self.conns.lock().unwrap_or_else(|e| e.into_inner()).remove(&d);
            for v in self.topics.lock().unwrap_or_else(|e| e.into_inner()).values_mut() {
                v.retain(|x| *x != d);
            }
            crate::record_raw("online", "lan", &format!("device {d} left"));
            self.publish("lan.device.left", &json!({"device": d}), "");
        }
    }
}

pub fn start_hub(port: u16) -> std::io::Result<u16> {
    let l = TcpListener::bind(("0.0.0.0", port))?;
    crate::netview::sockets::report_std(&l);
    let bound = l.local_addr()?.port();
    let h = hub();
    std::thread::Builder::new().name("tombox-lan-hub".into()).spawn(move || {
        for s in l.incoming().flatten() {
            let ok = match s.peer_addr() {
                Ok(SocketAddr::V4(a)) => private(*a.ip()),
                _ => false,
            };
            if !ok {
                crate::record_raw("online", "lan", &format!("refused a hub connection from {:?}: not a LAN address", s.peer_addr()));
                continue;
            }
            let _ = s.set_nodelay(true);
            let _ = std::thread::Builder::new().name("tombox-lan-peer".into()).spawn(move || h.serve(s));
        }
    })?;
    let dp = discovery_port();
    match UdpSocket::bind(("0.0.0.0", dp)) {
        Ok(u) => {
            crate::netview::sockets::report_std(&u);
            let _ = std::thread::Builder::new().name("tombox-lan-discovery".into()).spawn(move || {
                let mut buf = [0u8; 256];
                loop {
                    let Ok((n, from)) = u.recv_from(&mut buf) else { continue };
                    let ok = matches!(from, SocketAddr::V4(a) if private(*a.ip()));
                    if ok && buf[..n].starts_with(MAGIC_ASK.as_bytes()) {
                        if let SocketAddr::V4(a) = from {
                            crate::net_guard::allow_lan_peer(a.ip().octets());
                        }
                        let _ = u.send_to(format!("{MAGIC_HOST} {bound} {}", device_id()).as_bytes(), from);
                    }
                }
            });
        }
        Err(e) => crate::record_raw("online", "lan", &format!("LAN discovery port {dp} is busy ({e}); clients must set lan_host"),),
    }
    crate::record_raw("online", "lan", &format!("hosting the LAN hub on tcp {bound}, discovery udp {dp}, device {}", device_id()));
    Ok(bound)
}

pub struct Client {
    w: Conn,
    next: AtomicU64,
    waiting: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    topics: Arc<Mutex<HashMap<String, Vec<Sender<Value>>>>>,
    dgrams: Arc<Mutex<Option<Sender<(String, Vec<u8>)>>>>,
}

static CLIENT: OnceLock<Client> = OnceLock::new();

impl Client {
    pub fn connect(addr: SocketAddr) -> std::io::Result<Client> {
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        let _ = s.set_nodelay(true);
        let r = s.try_clone()?;
        let c = Client { w: Arc::new(Mutex::new(s)), next: AtomicU64::new(1), waiting: Arc::default(), topics: Arc::default(), dgrams: Arc::default() };
        let (waiting, topics, dgrams) = (c.waiting.clone(), c.topics.clone(), c.dgrams.clone());
        std::thread::Builder::new().name("tombox-lan-client".into()).spawn(move || {
            for line in BufReader::new(r).lines() {
                let Ok(line) = line else { break };
                let Ok(f) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = f["re"].as_u64() {
                    if let Some(tx) = waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                        let _ = tx.send(f);
                    }
                } else if let Some(t) = f["push"].as_str() {
                    if let Some(subs) = topics.lock().unwrap_or_else(|e| e.into_inner()).get_mut(t) {
                        subs.retain(|s| s.send(f.clone()).is_ok());
                    }
                } else if let Some(from) = f["dgram"].as_str() {
                    if let Some(tx) = dgrams.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                        let _ = tx.send((from.to_string(), b64_decode(f["data"].as_str().unwrap_or(""))));
                    }
                }
            }
            crate::record_raw("online", "lan", "the LAN hub connection closed");
            waiting.lock().unwrap_or_else(|e| e.into_inner()).clear();
        })?;
        let hello = c.request(json!({"op": "hello", "device": device_id(), "name": crate::gdk::user::user().gamertag}));
        match hello {
            Some(h) if h["ok"] == true => Ok(c),
            Some(h) => Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, h["error"].as_str().unwrap_or("hello refused").to_string())),
            None => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the hub did not answer hello")),
        }
    }

    fn request(&self, mut frame: Value) -> Option<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        frame["id"] = json!(id);
        let (tx, rx) = channel();
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        if !send_line(&self.w, &frame) {
            self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return None;
        }
        let r = rx.recv_timeout(CALL_TIMEOUT).ok();
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        r
    }
}

fn client() -> Option<&'static Client> {
    match role() {
        Role::Client { .. } => CLIENT.get(),
        _ => None,
    }
}

pub fn invoke(service: &str, payload: &Value) -> Result<Value, String> {
    match client() {
        Some(c) => match c.request(json!({"op": "call", "service": service, "payload": payload})) {
            Some(r) if r["ok"] == true => Ok(r["result"].clone()),
            Some(r) => Err(r["error"].as_str().unwrap_or("the hub refused the call").to_string()),
            None => Err("the LAN hub did not answer in time".into()),
        },
        None => handler(service).map(|h| h(payload)).ok_or_else(|| format!("no LAN service '{service}'")),
    }
}

pub fn subscribe(topic: &str) -> Receiver<Value> {
    let (tx, rx) = channel();
    match client() {
        Some(c) => {
            c.topics.lock().unwrap_or_else(|e| e.into_inner()).entry(topic.to_string()).or_default().push(tx);
            c.request(json!({"op": "sub", "topic": topic}));
        }
        None => hub().local_topics.lock().unwrap_or_else(|e| e.into_inner()).entry(topic.to_string()).or_default().push(tx),
    }
    rx
}

pub fn publish(topic: &str, data: &Value) -> usize {
    match client() {
        Some(c) => c.request(json!({"op": "pub", "topic": topic, "data": data})).and_then(|r| r["delivered"].as_u64()).unwrap_or(0) as usize,
        None => hub().publish(topic, data, device_id()),
    }
}

pub fn datagrams() -> Receiver<(String, Vec<u8>)> {
    let (tx, rx) = channel();
    match client() {
        Some(c) => *c.dgrams.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx),
        None => *hub().local_dgrams.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx),
    }
    rx
}

pub fn send_datagram(to: &str, data: &[u8]) -> bool {
    match client() {
        Some(c) => c.request(json!({"op": "dgram", "to": to, "data": b64(data)})).map_or(false, |r| r["ok"] == true),
        None => hub().dgram(to, device_id(), data),
    }
}

pub fn hub_port() -> Option<u16> {
    match role() {
        Role::Host { port } => Some(*port),
        _ => None,
    }
}

pub fn status() -> Value {
    json!({"role": format!("{:?}", role()), "device": device_id(), "hubPort": hub_port()})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped(a: [u8; 4]) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[10] = 0xff;
        o[11] = 0xff;
        o[12..].copy_from_slice(&a);
        o
    }

    #[test]
    fn guard_allows_only_lan_peers_and_discovery_broadcast() {
        assert!(!crate::net_guard::allow_lan_peer([8, 8, 8, 8]));
        assert!(!crate::net_guard::lan_dest_ok(&mapped([8, 8, 8, 8]), 443));
        assert!(crate::net_guard::allow_lan_peer([192, 168, 77, 20]));
        assert!(crate::net_guard::lan_dest_ok(&mapped([192, 168, 77, 20]), 1234));
        assert!(!crate::net_guard::lan_dest_ok(&mapped([192, 168, 77, 21]), 1234));
        crate::net_guard::allow_lan_broadcast(47999);
        assert!(crate::net_guard::lan_dest_ok(&mapped([255, 255, 255, 255]), 47999));
        assert!(crate::net_guard::lan_dest_ok(&mapped([10, 0, 0, 255]), 47999));
        assert!(!crate::net_guard::lan_dest_ok(&mapped([255, 255, 255, 255]), 80));
        assert!(!crate::net_guard::lan_dest_ok(&mapped([8, 8, 8, 255]), 47999));
    }

    #[test]
    fn hub_addresses_parse_and_default_the_port() {
        assert_eq!(parse_hub("192.168.1.5"), Some("192.168.1.5:47630".parse().unwrap()));
        assert_eq!(parse_hub("10.0.0.2:5000"), Some("10.0.0.2:5000".parse().unwrap()));
        assert_eq!(parse_hub("host.example"), None);
    }
}
