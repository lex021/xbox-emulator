use super::{dispatch, util, Request, Response, TitleService};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 64 * 1024 * 1024;

pub fn serve(title: Box<dyn TitleService>) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    crate::netview::sockets::report_std(&listener);
    let port = listener.local_addr()?.port();
    let title: Arc<dyn TitleService> = Arc::from(title);
    std::thread::Builder::new().name("tombox-online-accept".into()).spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let t = title.clone();
            let _ = std::thread::Builder::new().name("tombox-online-conn".into()).spawn(move || handle_conn(stream, t));
        }
    })?;
    Ok(port)
}

fn read_line(r: &mut BufReader<TcpStream>, budget: &mut usize) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(*budget as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if n > *budget {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "header too large"));
    }
    *budget -= n;
    while buf.last().map_or(false, |c| *c == b'\n' || *c == b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn read_request(r: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut budget = MAX_HEADER;
    let line = loop {
        match read_line(r, &mut budget)? {
            None => return Ok(None),
            Some(l) if l.is_empty() => continue,
            Some(l) => break l,
        }
    };
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    loop {
        match read_line(r, &mut budget)? {
            None => return Ok(None),
            Some(l) if l.is_empty() => break,
            Some(l) => {
                if let Some((k, v)) = l.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
    }
    let hdr = |k: &str| headers.iter().find(|(n, _): &&(String, String)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let mut body = Vec::new();
    if hdr("transfer-encoding").map_or(false, |v| v.to_ascii_lowercase().contains("chunked")) {
        loop {
            let mut b = MAX_HEADER;
            let size_line = read_line(r, &mut b)?.unwrap_or_default();
            let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
            if size == 0 {
                loop {
                    let mut b2 = MAX_HEADER;
                    match read_line(r, &mut b2)? {
                        Some(l) if !l.is_empty() => continue,
                        _ => break,
                    }
                }
                break;
            }
            if body.len() + size > MAX_BODY {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "body too large"));
            }
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..])?;
            let mut crlf = MAX_HEADER;
            let _ = read_line(r, &mut crlf)?;
        }
    } else if let Some(len) = hdr("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if len > MAX_BODY {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "body too large"));
        }
        body.resize(len, 0);
        r.read_exact(&mut body)?;
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), util::parse_pairs(q)),
        None => (target.clone(), Vec::new()),
    };
    let path = if let Some(rest) = path.strip_prefix("http://").or_else(|| path.strip_prefix("https://")) {
        rest.find('/').map(|i| rest[i..].to_string()).unwrap_or_else(|| "/".into())
    } else {
        path
    };
    Ok(Some(Request { method, path: util::url_decode(&path.replace('+', "%2B")), query, headers, body }))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        416 => "Range Not Satisfiable",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn write_response(w: &mut TcpStream, req_method: &str, resp: &Response, keep_alive: bool) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
    head.push_str(if keep_alive { "Connection: keep-alive\r\n" } else { "Connection: close\r\n" });
    let no_body = resp.status == 204 || resp.status == 304 || (100..200).contains(&resp.status);
    if !no_body {
        if !resp.content_type.is_empty() {
            let ct = if resp.content_type.starts_with("application/json") || resp.content_type.starts_with("text/") {
                format!("{}; charset=utf-8", resp.content_type)
            } else {
                resp.content_type.to_string()
            };
            head.push_str(&format!("Content-Type: {ct}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    }
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    if !no_body && req_method != "HEAD" {
        w.write_all(&resp.body)?;
    }
    w.flush()
}

fn parse_range(spec: &str, len: usize) -> Option<Option<(usize, usize)>> {
    let spec = spec.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    if len == 0 {
        return Some(None);
    }
    let (start, end) = if a.is_empty() {
        let n = b.parse::<usize>().ok()?;
        if n == 0 {
            return Some(None);
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let s = a.parse::<usize>().ok()?;
        let e = if b.is_empty() { len - 1 } else { b.parse::<usize>().ok()?.min(len - 1) };
        (s, e)
    };
    if start >= len || start > end {
        return Some(None);
    }
    Some(Some((start, end)))
}

pub fn apply_range(req: &Request, mut resp: Response) -> Response {
    if resp.status != 200 || resp.content_type.starts_with("application/json") || resp.content_type.is_empty() {
        return resp;
    }
    resp.headers.push(("Accept-Ranges".into(), "bytes".into()));
    let Some(spec) = req.header("range") else { return resp };
    let len = resp.body.len();
    match parse_range(spec, len) {
        None => resp,
        Some(None) => {
            let mut r = Response::empty(416);
            r.headers.push(("Content-Range".into(), format!("bytes */{len}")));
            r
        }
        Some(Some((s, e))) => {
            resp.status = 206;
            resp.body = resp.body[s..=e].to_vec();
            resp.headers.push(("Content-Range".into(), format!("bytes {s}-{e}/{len}")));
            resp
        }
    }
}

fn handle_conn(stream: TcpStream, title: Arc<dyn TitleService>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_nodelay(true);
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let req = match read_request(&mut reader) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::InvalidData {
                    let _ = write_response(&mut writer, "GET", &Response::empty(400), false);
                }
                return;
            }
        };
        if super::ws::is_upgrade(&req) {
            if let Some((session, proto)) = super::websocket_session(title.name(), &req) {
                super::ws::run(reader, writer, &req, session, proto);
                return;
            }
        }
        let close = req.header("connection").map_or(false, |c| c.eq_ignore_ascii_case("close"));
        let resp = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(title.as_ref(), &req))) {
            Ok(r) => r,
            Err(_) => {
                crate::record_raw("online", "panic", &format!("{} {}", req.method, req.path));
                Response::json_status(500, &serde_json::json!({"errorCode": "errors.com.xboxemulator.common.server_error"}))
            }
        };
        let resp = apply_range(&req, resp);
        if write_response(&mut writer, &req.method, &resp, !close).is_err() || close {
            return;
        }
    }
}
