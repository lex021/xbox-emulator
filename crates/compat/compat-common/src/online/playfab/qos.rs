use std::net::UdpSocket;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub const QOS_PORT: u16 = 3075;

static PINGS: AtomicU64 = AtomicU64::new(0);

pub fn reply_for(packet: &[u8]) -> Option<Vec<u8>> {
    if packet.len() < 2 || packet[0] != 0xFF || packet[1] != 0xFF {
        return None;
    }
    let mut r = packet.to_vec();
    r[0] = 0;
    r[1] = 0;
    Some(r)
}

pub fn pings_answered() -> u64 {
    PINGS.load(Ordering::Relaxed)
}

pub fn ensure_started() -> Option<u16> {
    static PORT: Mutex<Option<u16>> = Mutex::new(None);
    let mut g = PORT.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return *g;
    }
    *g = (|| {
        let port = std::env::var("TOMBOX_PLAYFAB_QOS_PORT").ok().or_else(|| crate::cfg("playfab_qos_port")).and_then(|p| p.trim().parse().ok()).unwrap_or(QOS_PORT);
        match UdpSocket::bind(("127.0.0.1", port)) {
            Ok(sock) => {
                crate::netview::sockets::report_std(&sock);
                let bound = sock.local_addr().map(|a| a.port()).unwrap_or(port);
                std::thread::Builder::new().name("tombox-playfab-qos".into()).spawn(move || {
                    let mut buf = [0u8; 2048];
                    loop {
                        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                        match reply_for(&buf[..n]) {
                            Some(r) => {
                                let _ = sock.send_to(&r, from);
                                PINGS.fetch_add(1, Ordering::Relaxed);
                            }
                            None => crate::record_raw("online", "playfab", &format!("QoS server got a {n}-byte datagram from {from} without the 0xFFFF probe header; not answered")),
                        }
                    }
                }).ok()?;
                crate::record_raw("online", "playfab", &format!("PlayFab QoS responder listening on udp 127.0.0.1:{bound}"));
                Some(bound)
            }
            Err(e) => {
                crate::record_raw("online", "playfab", &format!("PlayFab QoS responder could not bind udp 127.0.0.1:{port}: {e}; it is retried on the next ListPartyQosServers"));
                None
            }
        }
    })();
    *g
}
