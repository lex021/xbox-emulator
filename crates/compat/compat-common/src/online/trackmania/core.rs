use super::super::util::{b64, b64url, iso_millis, now_millis, now_secs, random_hex32, random_uuid, sha256};
use super::super::xdefiant::{is_uuid, uuid_for};
use super::super::{Request, Response};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

const ROUTES: &str = include_str!("../../../backend_data/trackmania/core_routes.json");
const ACCESS_SECS: u64 = 3600;
const REFRESH_SECS: u64 = 86400;
const WAITING_REFRESH_SECS: u64 = 30;
const AUDIENCES: &[&str] = &["NadeoServices", "NadeoLiveServices", "NadeoClubServices"];

pub fn routes() -> &'static Vec<(String, String, Vec<String>)> {
    static R: OnceLock<Vec<(String, String, Vec<String>)>> = OnceLock::new();
    R.get_or_init(|| {
        let v: Value = serde_json::from_str(ROUTES).unwrap_or(Value::Null);
        v.as_object().map(|m| m.iter().map(|(id, r)| {
            let path = r["path"].as_str().unwrap_or("/").to_string();
            let segs = path.split('?').next().unwrap_or("").split('/').filter(|s| !s.is_empty()).map(String::from).collect();
            (id.clone(), r["method"].as_str().unwrap_or("GET").to_string(), segs)
        }).collect()).unwrap_or_default()
    })
}

pub const CORE_PORT: u16 = 443;

pub fn route_table(host: &str) -> Value {
    let mut v: Value = serde_json::from_str(ROUTES).unwrap_or(Value::Null);
    if let Some(m) = v.as_object_mut() {
        for r in m.values_mut() {
            if let Some(o) = r.as_object_mut() {
                o.insert("host".into(), json!(host));
                o.insert("port".into(), json!(CORE_PORT));
                o.insert("timeout".into(), json!(0));
            }
        }
    }
    v
}

pub fn match_route(method: &str, seg: &[&str]) -> Option<(&'static str, HashMap<String, String>)> {
    let mut best: Option<(&'static str, HashMap<String, String>, usize)> = None;
    for (id, m, tpl) in routes() {
        if !m.eq_ignore_ascii_case(method) || tpl.len() != seg.len() {
            continue;
        }
        let mut params = HashMap::new();
        let mut literal = 0;
        let ok = tpl.iter().zip(seg).all(|(t, s)| {
            if t.starts_with('{') && t.ends_with('}') {
                params.insert(t[1..t.len() - 1].to_string(), s.to_string());
                true
            } else if t.eq_ignore_ascii_case(s) {
                literal += 1;
                true
            } else {
                false
            }
        });
        if ok && best.as_ref().map_or(true, |b| literal > b.2) {
            best = Some((id.as_str(), params, literal));
        }
    }
    best.map(|(i, p, _)| (i, p))
}

use super::super::util::hmac_sha256 as hmac;

pub fn ts(ms: u64) -> String {
    let s = iso_millis(ms);
    let base = s.split('.').next().unwrap_or(&s).trim_end_matches('Z').to_string();
    format!("{base}+00:00")
}

pub fn now_ts() -> String {
    ts(now_millis())
}

pub fn nadeo_error(status: u16, code: &str, message: &str, info: Value) -> Response {
    Response::json_status(status, &json!({"code": code, "correlation_id": random_hex32(), "message": message, "info": info}))
}

fn invalid(field: &str, why: &str) -> Response {
    nadeo_error(400, "C-AA-00-03", "There was a validation error.", json!({ field: why }))
}

fn not_found(what: &str) -> Response {
    nadeo_error(404, "C-AA-00-04", &format!("{what} not found."), Value::Null)
}

#[derive(Clone)]
pub struct Claims {
    pub audience: String,
    pub account_id: String,
    pub refresh: bool,
    pub refresh_audience: String,
    pub expires: u64,
    pub session: String,
}

struct Storage {
    content_type: String,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct State {
    tokens: HashMap<String, Claims>,
    zone_of: HashMap<String, String>,
    club_tag: HashMap<String, Value>,
    presence: HashMap<String, Value>,
    prestige_current: HashMap<String, Value>,
    chunks: BTreeMap<String, Value>,
    equipped: BTreeMap<(String, String), Value>,
    skins: BTreeMap<String, Value>,
    skin_favorites: BTreeSet<String>,
    stations: BTreeMap<String, Value>,
    subscriptions: BTreeMap<String, Value>,
    collections: BTreeMap<String, Value>,
    collection_favorites: BTreeSet<String>,
    maps: BTreeMap<String, Value>,
    records: Vec<Value>,
    attempts: Vec<Value>,
    secure_attempts: BTreeMap<String, Value>,
    seasons: BTreeMap<String, Value>,
    squads: BTreeMap<String, Value>,
    matches: BTreeMap<String, Value>,
    packages: BTreeMap<String, Value>,
    package_versions: BTreeMap<String, Value>,
    bots: Vec<Value>,
    titles: BTreeMap<String, Value>,
    trophy_gains: Vec<Value>,
    uploads: BTreeMap<String, (Value, Vec<u8>)>,
    storage: HashMap<String, Storage>,
    waiting: BTreeMap<String, Value>,
    logs: Vec<Value>,
    accounts: BTreeMap<String, Value>,
}

pub struct Core {
    pub account_id: String,
    pub name: String,
    pub ubi_profile: String,
    secret: Vec<u8>,
    st: Mutex<State>,
}

fn zones_data() -> &'static Vec<Value> {
    static Z: OnceLock<Vec<Value>> = OnceLock::new();
    Z.get_or_init(|| {
        let tree: &[(&str, &[&str])] = &[
            ("Africa", &["Algeria", "Egypt", "Kenya", "Morocco", "Nigeria", "South Africa", "Tunisia"]),
            ("Asia", &["China", "India", "Indonesia", "Japan", "Korea", "Malaysia", "Philippines", "Singapore", "Taiwan", "Thailand", "Vietnam"]),
            ("Europe", &["Austria", "Belgium", "Czechia", "Denmark", "Finland", "France", "Germany", "Greece", "Hungary", "Ireland", "Italy", "Netherlands", "Norway", "Poland", "Portugal", "Romania", "Russia", "Slovakia", "Spain", "Sweden", "Switzerland", "Turkey", "Ukraine", "United Kingdom"]),
            ("Middle East", &["Israel", "Saudi Arabia", "United Arab Emirates"]),
            ("North America", &["Canada", "Mexico", "United States"]),
            ("Oceania", &["Australia", "New Zealand"]),
            ("South America", &["Argentina", "Brazil", "Chile", "Colombia", "Peru"]),
        ];
        let stamp = "2020-07-01T00:00:00+00:00";
        let world = uuid_for("nadeo-zone:World");
        let mut out = vec![json!({"icon": "file://Media/Flags/WOR.dds", "name": "World", "parentId": Value::Null, "timestamp": stamp, "zoneId": world})];
        for (cont, countries) in tree {
            let cid = uuid_for(&format!("nadeo-zone:{cont}"));
            out.push(json!({"icon": "file://Media/Flags/WOR.dds", "name": cont, "parentId": world, "timestamp": stamp, "zoneId": cid}));
            for c in *countries {
                out.push(json!({"icon": format!("file://Media/Flags/{}.dds", c.chars().filter(|ch| ch.is_ascii_alphabetic()).take(3).collect::<String>().to_ascii_uppercase()), "name": c, "parentId": cid, "timestamp": stamp, "zoneId": uuid_for(&format!("nadeo-zone:{c}"))}));
            }
        }
        out
    })
}

fn zone_by_name(name: &str) -> Option<String> {
    zones_data().iter().find(|z| z["name"].as_str().map_or(false, |n| n.eq_ignore_ascii_case(name))).and_then(|z| z["zoneId"].as_str().map(String::from))
}

fn id_list(req: &Request, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in &req.query {
        if k == key || k == &format!("{key}[]") {
            out.extend(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
        }
    }
    out
}

fn uuid_list(req: &Request, key: &str) -> Result<Vec<String>, Response> {
    let ids = id_list(req, key);
    if ids.iter().any(|i| !is_uuid(i)) {
        return Err(invalid(key, "Invalid account id."));
    }
    Ok(ids.into_iter().map(|s| s.to_ascii_lowercase()).collect())
}

fn page(req: &Request, items: Vec<Value>) -> Vec<Value> {
    let offset = req.query("offset").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let len = req.query("length").or(req.query("count")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(usize::MAX);
    items.into_iter().skip(offset).take(len).collect()
}

pub struct Multipart {
    pub fields: BTreeMap<String, (Option<String>, Option<String>, Vec<u8>)>,
}

pub fn multipart(req: &Request) -> Option<Multipart> {
    let ct = req.header("content-type")?;
    let boundary = ct.split(';').map(str::trim).find_map(|p| p.strip_prefix("boundary="))?.trim_matches('"').to_string();
    let delim = format!("--{boundary}").into_bytes();
    let body = &req.body;
    let mut fields = BTreeMap::new();
    let mut pos = find(body, &delim, 0)?;
    loop {
        let start = pos + delim.len();
        if body.get(start..start + 2) == Some(b"--") {
            break;
        }
        let head_start = start + 2;
        let head_end = find(body, b"\r\n\r\n", head_start)?;
        let head = String::from_utf8_lossy(&body[head_start..head_end]).to_string();
        let next = find(body, &delim, head_end + 4)?;
        let data = body[head_end + 4..next.saturating_sub(2)].to_vec();
        let mut name = None;
        let mut filename = None;
        let mut ctype = None;
        for line in head.split("\r\n") {
            let l = line.to_ascii_lowercase();
            if l.starts_with("content-disposition:") {
                for part in line.split(';').map(str::trim) {
                    if let Some(v) = part.strip_prefix("name=") {
                        name = Some(v.trim_matches('"').to_string());
                    }
                    if let Some(v) = part.strip_prefix("filename=") {
                        filename = Some(v.trim_matches('"').to_string());
                    }
                }
            } else if l.starts_with("content-type:") {
                ctype = Some(line[13..].trim().to_string());
            }
        }
        if let Some(n) = name {
            fields.insert(n, (filename, ctype, data));
        }
        pos = next;
    }
    Some(Multipart { fields })
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn str_field(b: &Value, mp: &Option<Multipart>, k: &str) -> Option<String> {
    b.get(k).and_then(|v| v.as_str().map(String::from).or_else(|| (!v.is_null()).then(|| v.to_string())))
        .or_else(|| mp.as_ref().and_then(|m| m.fields.get(k)).map(|(_, _, d)| String::from_utf8_lossy(d).to_string()))
}

impl Core {
    pub fn new(name: &str, ubi_profile: &str) -> Core {
        let secret = sha256(format!("tombox-nadeo:{name}:{}", random_uuid()).as_bytes()).to_vec();
        Core { account_id: uuid_for(&format!("nadeo-account:{name}")), name: name.to_string(), ubi_profile: ubi_profile.to_string(), secret, st: Mutex::new(State::default()) }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn base_url(&self, host: &str) -> String {
        format!("https://{host}")
    }

    fn jwt(&self, claims: &Value) -> String {
        let head = b64url(br#"{"alg":"HS256","env":"trackmania-prod","ver":"1"}"#);
        let body = b64url(claims.to_string().as_bytes());
        let sig = b64url(&hmac(&self.secret, format!("{head}.{body}").as_bytes()));
        format!("{head}.{body}.{sig}")
    }

    fn issue(&self, audience: &str, session: &str) -> Value {
        let now = now_secs();
        let base = json!({"iss": "NadeoServices", "iat": now, "rat": now + ACCESS_SECS / 2, "sub": self.account_id, "aun": self.name, "sid": session, "usg": "Client", "rtk": false, "pce": false, "ubiservices_uid": self.ubi_profile});
        let mut access = base.clone();
        access["jti"] = json!(random_uuid());
        access["aud"] = json!(audience);
        access["exp"] = json!(now + ACCESS_SECS);
        let mut refresh = base;
        refresh["jti"] = json!(random_uuid());
        refresh["aud"] = json!("NadeoServices");
        refresh["refresh_aud"] = json!(audience);
        refresh["rtk"] = json!(true);
        refresh["exp"] = json!(now + REFRESH_SECS);
        let at = self.jwt(&access);
        let rt = self.jwt(&refresh);
        let mut st = self.st();
        st.tokens.retain(|_, c| c.expires > now);
        st.tokens.insert(at.clone(), Claims { audience: audience.into(), account_id: self.account_id.clone(), refresh: false, refresh_audience: audience.into(), expires: now + ACCESS_SECS, session: session.into() });
        st.tokens.insert(rt.clone(), Claims { audience: "NadeoServices".into(), account_id: self.account_id.clone(), refresh: true, refresh_audience: audience.into(), expires: now + REFRESH_SECS, session: session.into() });
        json!({"accessToken": at, "refreshToken": rt})
    }

    pub fn bearer(&self, req: &Request) -> Option<Claims> {
        let h = req.header("authorization")?;
        let t = h.trim().strip_prefix("nadeo_v1")?.trim().strip_prefix("t=")?.trim();
        let c = self.st().tokens.get(t).cloned()?;
        (c.expires > now_secs()).then_some(c)
    }

    fn audience_of(body: &Value) -> Result<String, Response> {
        match body.get("audience").and_then(Value::as_str) {
            None => Ok("NadeoServices".into()),
            Some(a) if AUDIENCES.contains(&a) => Ok(a.into()),
            Some(_) => Err(invalid("audience", "Unknown audience.")),
        }
    }

    pub fn token(&self, kind: &str, req: &Request, ubi_ticket: Option<String>) -> Response {
        let body = req.json();
        let auth = req.header("authorization").unwrap_or("").trim().to_string();
        match kind {
            "ubiservices" => {
                let Some(t) = auth.strip_prefix("ubi_v1").map(str::trim).and_then(|s| s.strip_prefix("t=")) else {
                    return nadeo_error(401, "C-AA-00-01", "An ubi_v1 ticket is required.", Value::Null);
                };
                if ubi_ticket.as_deref() != Some(t.trim()) {
                    return nadeo_error(401, "C-AA-00-01", "The UbiServices ticket is not valid.", Value::Null);
                }
                match Self::audience_of(&body) {
                    Ok(a) => Response::json(&self.issue(&a, &random_uuid())),
                    Err(e) => e,
                }
            }
            "nadeoservices" => {
                let Some(c) = self.bearer(req).filter(|c| !c.refresh) else {
                    return nadeo_error(401, "C-AA-00-01", "A valid nadeo_v1 access token is required.", Value::Null);
                };
                match Self::audience_of(&body) {
                    Ok(a) => Response::json(&self.issue(&a, &c.session)),
                    Err(e) => e,
                }
            }
            "refresh" => match self.bearer(req).filter(|c| c.refresh) {
                Some(c) => {
                    let mut st = self.st();
                    if let Some(t) = auth.strip_prefix("nadeo_v1").map(str::trim).and_then(|s| s.strip_prefix("t=")) {
                        st.tokens.remove(t.trim());
                    }
                    drop(st);
                    Response::json(&self.issue(&c.refresh_audience, &c.session))
                }
                None => nadeo_error(401, "C-AA-00-01", "A valid refresh token is required.", Value::Null),
            },
            "basic" => nadeo_error(403, "C-AA-00-02", "Dedicated server accounts are not available to a console client.", Value::Null),
            _ => not_found("Authentication method"),
        }
    }

    fn me(&self, id: &str) -> bool {
        id.eq_ignore_ascii_case(&self.account_id)
    }

    pub fn maps(&self) -> Vec<Value> {
        self.st().maps.values().cloned().collect()
    }

    pub fn seed_map(&self, meta: &Value, file: Vec<u8>) -> Option<Value> {
        let uid = meta["mapUid"].as_str()?.to_string();
        if let Some(m) = self.map_by_uid(&uid) {
            return Some(m);
        }
        let host = "prod.trackmania.core.nadeo.online";
        let (_, url) = self.store_object(host, "application/octet-stream", file);
        let mid = meta["mapId"].as_str().filter(|m| is_uuid(m)).map(String::from).unwrap_or_else(|| uuid_for(&format!("nadeo-map:{uid}")));
        let num = |k: &str| meta[k].as_i64().unwrap_or(-1);
        let author = meta["author"].as_str().filter(|a| is_uuid(a)).map(String::from).unwrap_or_else(|| uuid_for("nadeo-account:Nadeo"));
        let v = json!({
            "author": author, "authorScore": num("authorScore"), "bronzeScore": num("bronzeScore"), "silverScore": num("silverScore"), "goldScore": num("goldScore"),
            "collectionName": meta["collectionName"].as_str().unwrap_or("Stadium"), "createdWithGamepadEditor": false, "createdWithSimpleEditor": false,
            "filename": meta["filename"].as_str().map(String::from).unwrap_or_else(|| format!("{uid}.Map.Gbx")), "hasClones": false, "isPlayable": true, "mapId": mid,
            "mapStyle": meta["mapStyle"].as_str().unwrap_or(""), "mapType": meta["mapType"].as_str().unwrap_or("TrackMania\\TM_Race"), "mapUid": uid,
            "name": meta["name"].as_str().unwrap_or(""), "submitter": author, "timestamp": meta["timestamp"].as_str().map(String::from).unwrap_or_else(now_ts), "fileUrl": url, "thumbnailUrl": Value::Null,
        });
        self.st().maps.insert(mid, v.clone());
        Some(v)
    }

    pub fn map_by_uid(&self, uid: &str) -> Option<Value> {
        self.st().maps.values().find(|m| m["mapUid"] == json!(uid)).cloned()
    }

    pub fn records(&self) -> Vec<Value> {
        self.st().records.clone()
    }

    pub fn trophy_points(&self) -> i64 {
        let st = self.st();
        st.trophy_gains.iter().map(|g| (1..=9).map(|i| g[format!("t{i}Count")].as_i64().unwrap_or(0) * 10i64.pow(i as u32)).sum::<i64>()).sum()
    }

    pub fn zone_chain(&self, account: &str) -> Vec<(String, String)> {
        let mut id = self.zone_id(account);
        let mut out = Vec::new();
        while let Some(z) = zones_data().iter().find(|z| z["zoneId"] == json!(id)) {
            out.push((id.clone(), z["name"].as_str().unwrap_or("").to_string()));
            match z["parentId"].as_str() {
                Some(p) if out.len() < 8 => id = p.to_string(),
                _ => break,
            }
        }
        out.reverse();
        out
    }

    fn mine_or_403(&self, id: &str) -> Option<Response> {
        (!self.me(id)).then(|| nadeo_error(403, "C-AA-00-02", "You can only change your own account.", Value::Null))
    }

    fn zone_id(&self, account: &str) -> String {
        if let Some(z) = self.st().zone_of.get(&account.to_ascii_lowercase()) {
            return z.clone();
        }
        let wanted = crate::cfg("trackmania_zone").or_else(|| std::env::var("TOMBOX_TRACKMANIA_ZONE").ok()).unwrap_or_else(|| "World".into());
        zone_by_name(&wanted).unwrap_or_else(|| uuid_for("nadeo-zone:World"))
    }

    fn store_object(&self, host: &str, content_type: &str, bytes: Vec<u8>) -> (String, String) {
        let id = random_uuid();
        let url = format!("{}/storageObjects/{id}", self.base_url(host));
        self.st().storage.insert(id.clone(), Storage { content_type: content_type.into(), bytes });
        (id, url)
    }

    pub fn handle(&self, id: &str, p: &HashMap<String, String>, req: &Request, host: &str) -> Response {
        let body = req.json();
        let me = self.account_id.clone();
        let arg = |k: &str| p.get(k).cloned().unwrap_or_default();
        let now = now_ts();
        let short = id.trim_start_matches("nadeoservices_");
        if short != "api_routes_get" && short != "client_debug_info_add" && short != "client_config_get" && short != "waiting_queue_client_add" && short != "storage_object_url_redirect" && short != "storage_object_content_url_redirect" && self.bearer(req).filter(|c| !c.refresh).is_none() {
            return nadeo_error(401, "C-AA-00-01", "A valid nadeo_v1 access token is required.", Value::Null);
        }
        if let Some(a) = p.get("accountId") {
            if !is_uuid(a) {
                return invalid("accountId", "Invalid uuid.");
            }
        }
        match short {
            "api_routes_get" => match req.query("usage") {
                Some(u) if u.eq_ignore_ascii_case("Client") || u.eq_ignore_ascii_case("Server") => Response::json(&route_table(host)),
                _ => invalid("usage", "The usage must be Client or Server."),
            },
            "storage_object_url_redirect" | "storage_object_content_url_redirect" => {
                let sid = arg("storageObjectId");
                let key = sid.split('.').next().unwrap_or("").to_string();
                match self.st().storage.get(&key) {
                    Some(s) => Response { status: 200, content_type: "application/octet-stream", headers: vec![("Content-Type".into(), s.content_type.clone())], body: s.bytes.clone() },
                    None => not_found("Storage object"),
                }
            }
            "client_config_get" => Response::json(&json!({"timestamp": now, "settings": {}})),
            "title_client_config_get" => Response::json(&json!({"titleId": arg("titleId"), "timestamp": now, "settings": {}})),
            "account_client_config_get" => Response::json(&json!({"accountId": arg("accountId"), "timestamp": now, "settings": {}})),
            "account_client_plugins_get" => Response::json(&json!({"accountId": arg("accountId"), "plugins": []})),
            "account_client_signature_get" => Response::json(&json!({"accountId": arg("accountId"), "signature": b64(&sha256(format!("{}:{}", arg("accountId"), self.name).as_bytes()))})),
            "account_client_url_config_get" => Response::json(&json!({
                "accountId": arg("accountId"),
                "urls": {"core": "https://prod.trackmania.core.nadeo.online", "live": "https://live-services.trackmania.nadeo.live", "meet": "https://meet.trackmania.nadeo.club"},
            })),
            "client_file_list_get" => Response::json(&json!([])),
            "client_log_add" | "client_debug_info_add" => {
                if short == "client_debug_info_add" {
                    let text: String = String::from_utf8_lossy(&req.body).chars().take(2000).collect();
                    crate::record_raw("online", "trackmania", &format!("client debug info: {text}"));
                }
                let mut st = self.st();
                st.logs.push(json!({"kind": short, "received": now, "body": if body.is_null() { json!(String::from_utf8_lossy(&req.body)) } else { body.clone() }}));
                if st.logs.len() > 2000 {
                    st.logs.remove(0);
                }
                Response::empty(200)
            }
            "zone_list_get" => Response::json(&json!(zones_data())),
            "account_zone_get" => Response::json(&json!({"accountId": arg("accountId"), "timestamp": now, "zoneId": self.zone_id(&arg("accountId"))})),
            "account_zone_list_get" => match uuid_list(req, "accountIdList") {
                Ok(ids) => Response::json(&json!(ids.iter().map(|a| json!({"accountId": a, "timestamp": now, "zoneId": self.zone_id(a)})).collect::<Vec<_>>())),
                Err(e) => e,
            },
            "account_zone_set" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let Some(z) = body["zoneId"].as_str().filter(|z| zones_data().iter().any(|d| d["zoneId"] == json!(z))) else { return invalid("zoneId", "Unknown zone.") };
                self.st().zone_of.insert(me.to_ascii_lowercase(), z.to_string());
                Response::json(&json!({"accountId": me, "timestamp": now, "zoneId": z}))
            }
            "account_club_tag_get" => match self.st().club_tag.get(&arg("accountId").to_ascii_lowercase()) {
                Some(t) => Response::json(t),
                None => Response::json(&json!({"accountId": arg("accountId"), "clubTag": "", "timestamp": now})),
            },
            "account_club_tag_list_get" => match uuid_list(req, "accountIdList") {
                Ok(ids) => {
                    let st = self.st();
                    Response::json(&json!(ids.iter().filter_map(|a| st.club_tag.get(a).cloned()).collect::<Vec<_>>()))
                }
                Err(e) => e,
            },
            "account_club_tag_set" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let Some(tag) = body["clubTag"].as_str() else { return invalid("clubTag", "Missing club tag.") };
                if tag.chars().filter(|c| *c != '$').count() > 32 {
                    return invalid("clubTag", "The club tag is too long.");
                }
                let v = json!({"accountId": me, "clubTag": tag, "timestamp": now});
                self.st().club_tag.insert(me.to_ascii_lowercase(), v.clone());
                Response::json(&v)
            }
            "account_club_tag_delete" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                self.st().club_tag.remove(&me.to_ascii_lowercase());
                Response::empty(204)
            }
            "account_presence_set" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let v = json!({"accountId": me, "activity": body.get("activity").cloned().unwrap_or(Value::Null), "status": body.get("status").cloned().unwrap_or(json!("Online")), "timestamp": now, "data": body});
                self.st().presence.insert(me.clone(), v.clone());
                Response::json(&v)
            }
            "account_presence_delete" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                self.st().presence.remove(&me);
                Response::empty(204)
            }
            "prestige_list_get" => Response::json(&json!([])),
            "account_prestige_list_get" => Response::json(&json!([])),
            "account_prestige_current_get" | "current_account_prestige_get" => match self.st().prestige_current.get(&arg("accountId").to_ascii_lowercase()) {
                Some(v) => Response::json(v),
                None => not_found("Current prestige"),
            },
            "current_account_prestige_list_get" => match uuid_list(req, "accountIdList") {
                Ok(ids) => {
                    let st = self.st();
                    Response::json(&json!(ids.iter().filter_map(|a| st.prestige_current.get(a).cloned()).collect::<Vec<_>>()))
                }
                Err(e) => e,
            },
            "account_prestige_current_set" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let Some(pid) = body["prestigeId"].as_str() else { return invalid("prestigeId", "Missing prestige id.") };
                let v = json!({"accountId": me, "prestigeId": pid, "timestamp": now});
                self.st().prestige_current.insert(me.to_ascii_lowercase(), v.clone());
                Response::json(&v)
            }
            "account_prestige_current_delete" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                self.st().prestige_current.remove(&me.to_ascii_lowercase());
                Response::empty(204)
            }
            "account_profile_chunk_list_get" => {
                let st = self.st();
                Response::json(&json!(st.chunks.values().cloned().collect::<Vec<_>>()))
            }
            "account_profile_chunk_create" | "account_profile_chunk_set" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let cid = if short.ends_with("create") { random_uuid() } else { arg("chunkId") };
                if short.ends_with("set") && !self.st().chunks.contains_key(&cid) {
                    return not_found("Profile chunk");
                }
                let content = body.get("content").cloned().or_else(|| body.get("data").cloned()).unwrap_or_else(|| json!(b64(&req.body)));
                let v = json!({"accountId": me, "chunkId": cid, "name": body.get("name").cloned().unwrap_or(Value::Null), "content": content, "timestamp": now});
                self.st().chunks.insert(cid, v.clone());
                Response::json(&v)
            }
            "account_profile_chunk_delete" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                match self.st().chunks.remove(&arg("chunkId")) {
                    Some(_) => Response::empty(204),
                    None => not_found("Profile chunk"),
                }
            }
            "skin_create" => {
                let mp = multipart(req);
                let file = mp.as_ref().and_then(|m| m.fields.values().find(|f| f.0.is_some()).map(|f| f.2.clone())).unwrap_or_default();
                let checksum = super::super::util::hex(&sha256(&file)).to_ascii_uppercase();
                if let Some(existing) = self.st().skins.values().find(|s| s["checksum"] == json!(checksum)).cloned() {
                    return Response::json(&existing);
                }
                let (_, url) = self.store_object(host, "application/zip", file);
                let name = str_field(&body, &mp, "name").unwrap_or_else(|| "Skin.zip".into());
                let skin_type = str_field(&body, &mp, "type").unwrap_or_else(|| "Models/CarSport".into());
                let sid = random_uuid();
                let v = json!({"creatorId": me, "checksum": checksum, "filename": name.rsplit(['\\', '/']).next().unwrap_or(&name), "fileOptimizedUrl": url, "fileUrl": url, "skinDisplayName": str_field(&body, &mp, "displayName").unwrap_or_else(|| name.trim_end_matches(".zip").to_string()), "skinId": sid, "skinName": name, "skinType": skin_type, "thumbnailUrl": Value::Null, "timestamp": now});
                self.st().skins.insert(sid, v.clone());
                Response::json(&v)
            }
            "skin_list_get" => {
                let ids = id_list(req, "skinIdList");
                let sums = id_list(req, "skinChecksumList");
                let st = self.st();
                Response::json(&json!(st.skins.values().filter(|s| ids.iter().any(|i| s["skinId"] == json!(i)) || sums.iter().any(|c| s["checksum"].as_str().map_or(false, |x| x.eq_ignore_ascii_case(c)))).cloned().collect::<Vec<_>>()))
            }
            "account_skin_list_get" => {
                let a = arg("accountId").to_ascii_lowercase();
                let st = self.st();
                Response::json(&json!(st.equipped.iter().filter(|((acc, _), _)| *acc == a).map(|(_, v)| v.clone()).collect::<Vec<_>>()))
            }
            "account_skin_list_account_list_get" => match uuid_list(req, "accountIdList") {
                Ok(ids) => {
                    let st = self.st();
                    Response::json(&json!(st.equipped.iter().filter(|((acc, _), _)| ids.contains(acc)).map(|(_, v)| v.clone()).collect::<Vec<_>>()))
                }
                Err(e) => e,
            },
            "account_skin_set" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let Some(sid) = body["skinId"].as_str().filter(|s| self.st().skins.contains_key(*s)) else { return invalid("skinId", "Unknown skin.") };
                let v = json!({"accountId": me, "skinId": sid, "skinType": arg("skinType"), "timestamp": now});
                self.st().equipped.insert((me.to_ascii_lowercase(), arg("skinType")), v.clone());
                Response::json(&v)
            }
            "account_skin_delete" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                self.st().equipped.remove(&(me.to_ascii_lowercase(), arg("skinType")));
                Response::empty(204)
            }
            "account_skin_favorite_list_get" => {
                let st = self.st();
                Response::json(&json!(st.skin_favorites.iter().filter_map(|s| st.skins.get(s).cloned()).collect::<Vec<_>>()))
            }
            "account_skin_favorite_add" | "account_skin_favorite_remove" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let sid = arg("skinId");
                let mut st = self.st();
                if short.ends_with("add") {
                    if !st.skins.contains_key(&sid) {
                        return not_found("Skin");
                    }
                    st.skin_favorites.insert(sid);
                } else {
                    st.skin_favorites.remove(&sid);
                }
                Response::empty(204)
            }
            "account_station_set" | "account_station_delete" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let pid = arg("encryptedPackageId");
                let mut st = self.st();
                if short.ends_with("set") {
                    if !st.packages.contains_key(&pid) {
                        return not_found("Encrypted package");
                    }
                    let v = json!({"accountId": me, "encryptedPackageId": pid, "timestamp": now});
                    st.stations.insert(pid, v.clone());
                    Response::json(&v)
                } else {
                    st.stations.remove(&pid);
                    Response::empty(204)
                }
            }
            "account_subscription_list_get_v2" => {
                let st = self.st();
                Response::json(&json!(st.subscriptions.values().cloned().collect::<Vec<_>>()))
            }
            "account_subscription_add_v2" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let name = arg("subscriptionName");
                if !matches!(name.as_str(), "Standard" | "Club" | "Trackmania_Standard" | "Trackmania_Club") && !name.starts_with("Trackmania") {
                    return invalid("subscriptionName", "Unknown subscription.");
                }
                let v = json!({"accountId": me, "subscriptionName": name, "platform": body.get("platform").cloned().unwrap_or(json!("XboxOne")), "startTimestamp": now, "endTimestamp": ts(now_millis() + 365 * 86_400_000), "receipt": body.get("receipt").cloned().unwrap_or(Value::Null)});
                self.st().subscriptions.insert(name, v.clone());
                Response::json(&v)
            }
            "account_title_list_get" => {
                let st = self.st();
                Response::json(&json!(st.titles.values().cloned().collect::<Vec<_>>()))
            }
            "account_global_policy_rule_list_get" | "account_title_policy_rule_list_get" => Response::json(&json!([])),
            "account_trophy_gain_list_get" => {
                let st = self.st();
                let all: Vec<Value> = st.trophy_gains.iter().rev().cloned().collect();
                let count = all.len();
                Response::json(&json!({"count": count, "data": page(req, all)}))
            }
            "account_trophy_last_year_summary_get" => {
                let st = self.st();
                let mut t = [0i64; 9];
                for g in &st.trophy_gains {
                    for (i, c) in t.iter_mut().enumerate() {
                        *c += g[format!("t{}Count", i + 1)].as_i64().unwrap_or(0);
                    }
                }
                let points: i64 = t.iter().enumerate().map(|(i, c)| c * 10i64.pow(i as u32 + 1)).sum();
                let mut v = json!({"accountId": arg("accountId"), "points": points, "timestamp": now});
                for (i, c) in t.iter().enumerate() {
                    v[format!("t{}Count", i + 1)] = json!(c);
                }
                Response::json(&v)
            }
            "trophy_settings_get" => Response::json(&json!({"soloMedal": {"bronze": 1, "silver": 2, "gold": 3, "author": 4}, "competitionMatch": {}, "liveTimeAttack": {}})),
            "trophy_competition_match_achievement_add" | "trophy_live_time_attack_achievement_add" => {
                let mut g = json!({"accountId": me, "timestamp": now, "trophyAchievementInfo": {"trophyAchievementId": random_uuid(), "trophyAchievementType": if short.contains("competition") { "CompetitionMatch" } else { "LiveTimeAttack" }}, "trophyGainDetails": body.clone()});
                for i in 1..=9 {
                    g[format!("t{i}Count")] = body.get(format!("t{i}Count")).cloned().unwrap_or(json!(0));
                }
                self.st().trophy_gains.push(g.clone());
                Response::json(&g)
            }
            "account_xp_get" => Response::json(&json!({"accountId": arg("accountId"), "xp": 0, "level": 1, "timestamp": now})),
            "account_ladder_info_get" => Response::json(&json!({"accountId": arg("accountId"), "zoneId": self.zone_id(&arg("accountId")), "rank": Value::Null, "score": 0, "timestamp": now})),
            "webidentity_list_get" => match uuid_list(req, "accountIdList") {
                Ok(ids) => Response::json(&json!(ids.iter().filter(|a| self.me(a)).flat_map(|a| [
                    json!({"accountId": a, "provider": "ubiServices", "uid": self.ubi_profile, "timestamp": "2020-07-01T00:00:00+00:00"}),
                    json!({"accountId": a, "provider": "xbox", "uid": crate::gdk::user::user().xuid.to_string(), "timestamp": "2020-07-01T00:00:00+00:00"}),
                ]).collect::<Vec<_>>())),
                Err(e) => e,
            },
            "user_account_create" => {
                let v = json!({"accountId": me, "displayName": self.name, "timestamp": now});
                self.st().accounts.insert(me.clone(), v.clone());
                Response::json(&v)
            }
            "account_password_reset_add" | "account_password_reset_set_password" => nadeo_error(403, "C-AA-00-02", "Ubisoft-linked console accounts have no Nadeo password.", Value::Null),
            "club_list_get" => Response::json(&json!([])),
            "server_get" => not_found("Server"),
            "upload_create" => {
                let uid = random_uuid();
                let v = json!({"uploadId": uid, "accountId": me, "size": 0, "timestamp": now});
                self.st().uploads.insert(uid, (v.clone(), Vec::new()));
                Response::json_status(201, &v)
            }
            "upload_data_add" => {
                let mut st = self.st();
                match st.uploads.get_mut(&arg("uploadId")) {
                    Some((meta, data)) => {
                        data.extend_from_slice(&req.body);
                        meta["size"] = json!(data.len());
                        let size = data.len();
                        let mut r = Response::empty(204);
                        r.headers.push(("Upload-Offset".into(), size.to_string()));
                        r
                    }
                    None => not_found("Upload"),
                }
            }
            "upload_info_get" => match self.st().uploads.get(&arg("uploadId")) {
                Some((_, data)) => {
                    let mut r = Response::empty(200);
                    r.headers.push(("Upload-Offset".into(), data.len().to_string()));
                    r
                }
                None => not_found("Upload"),
            },
            "map_create" | "map_set" => self.map_write(short, &arg("mapId"), req, host),
            "map_list_get" => {
                let ids = id_list(req, "mapIdList");
                let uids = id_list(req, "mapUidList");
                let st = self.st();
                Response::json(&json!(st.maps.values().filter(|m| ids.iter().any(|i| m["mapId"] == json!(i)) || uids.iter().any(|u| m["mapUid"] == json!(u))).cloned().collect::<Vec<_>>()))
            }
            "maprecord_set" => self.record_write(req, host),
            "maprecord_list_get" => {
                let accounts = id_list(req, "accountIdList");
                let maps = id_list(req, "mapIdList");
                let season = req.query("seasonId").map(String::from);
                let st = self.st();
                Response::json(&json!(st.records.iter().filter(|r| {
                    (accounts.is_empty() || accounts.iter().any(|a| r["accountId"].as_str().map_or(false, |x| x.eq_ignore_ascii_case(a))))
                        && (maps.is_empty() || maps.iter().any(|m| r["mapId"] == json!(m)))
                        && season.as_ref().map_or(true, |s| r["scopeId"] == json!(s))
                }).cloned().collect::<Vec<_>>()))
            }
            "maprecordattempt_set" => {
                let v = json!({"accountId": me, "mapId": body.get("mapId"), "attemptId": random_uuid(), "timestamp": now, "data": body});
                self.st().attempts.push(v.clone());
                Response::json(&v)
            }
            "maprecordsecureattempt_create" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let cid = random_uuid();
                let v = json!({"accountId": me, "creatorId": cid, "mapId": body.get("mapId"), "status": "Created", "timestamp": now, "data": body});
                self.st().secure_attempts.insert(cid, v.clone());
                Response::json(&v)
            }
            "maprecordsecureattempt_patch" => {
                let mut st = self.st();
                match st.secure_attempts.get_mut(&arg("creatorId")) {
                    Some(a) => {
                        if let (Some(o), Some(n)) = (a.as_object_mut(), body.as_object()) {
                            for (k, v) in n {
                                o.insert(k.clone(), v.clone());
                            }
                            o.insert("timestamp".into(), json!(now));
                        }
                        Response::json(a)
                    }
                    None => not_found("Secure attempt"),
                }
            }
            "season_get" => match self.st().seasons.get(&arg("seasonId")) {
                Some(s) => Response::json(s),
                None => not_found("Season"),
            },
            "season_list_get" => {
                let ids = id_list(req, "seasonIdList");
                let st = self.st();
                Response::json(&json!(st.seasons.values().filter(|s| ids.is_empty() || ids.iter().any(|i| s["seasonId"] == json!(i))).cloned().collect::<Vec<_>>()))
            }
            "playable_season_list_get" => {
                let now_ms = now_millis() as i64 / 1000;
                let st = self.st();
                Response::json(&json!(st.seasons.values().filter(|s| s["startTimestamp"].as_i64().unwrap_or(0) <= now_ms && s["endTimestamp"].as_i64().map_or(true, |e| e == 0 || e > now_ms)).cloned().collect::<Vec<_>>()))
            }
            "season_create" | "season_set" => {
                let sid = if short == "season_create" { random_uuid() } else { arg("seasonId") };
                let mut st = self.st();
                let prev = st.seasons.get(&sid).cloned();
                if short == "season_set" && prev.is_none() {
                    return not_found("Season");
                }
                let mut v = prev.unwrap_or_else(|| json!({"seasonId": sid, "creatorId": me, "seasonMapList": [], "creationTimestamp": now}));
                for k in ["name", "gameMode", "gameModeCustomData", "isOfficial", "startTimestamp", "endTimestamp"] {
                    if let Some(x) = body.get(k) {
                        v[k] = x.clone();
                    }
                }
                st.seasons.insert(sid, v.clone());
                Response::json(&v)
            }
            "season_map_list_add" | "season_map_list_remove" => {
                let mut st = self.st();
                let Some(s) = st.seasons.get_mut(&arg("seasonId")) else { return not_found("Season") };
                let incoming: Vec<Value> = body["seasonMapList"].as_array().or(body["mapList"].as_array()).cloned().unwrap_or_default();
                let list = s["seasonMapList"].as_array_mut().expect("seasonMapList");
                for m in incoming {
                    let mid = m.get("mapId").cloned().unwrap_or(m.clone());
                    list.retain(|x| x.get("mapId") != Some(&mid));
                    if short.ends_with("add") {
                        list.push(json!({"mapId": mid, "timestamp": now}));
                    }
                }
                Response::json(s)
            }
            "squad_create" => {
                let sid = random_uuid();
                let v = json!({"squadId": sid, "leaderAccountId": me, "members": [{"accountId": me, "timestamp": now}], "invitations": [], "creationTimestamp": now, "type": body.get("type").cloned().unwrap_or(Value::Null)});
                self.st().squads.insert(sid, v.clone());
                Response::json(&v)
            }
            "squad_get" => match self.st().squads.get(&arg("squadId")) {
                Some(s) => Response::json(s),
                None => not_found("Squad"),
            },
            "squad_invitation_add" | "squad_invitation_remove" | "squad_invitation_accept" | "squad_member_remove" | "squad_leader_set" => {
                let mut st = self.st();
                let Some(s) = st.squads.get_mut(&arg("squadId")) else { return not_found("Squad") };
                match short {
                    "squad_invitation_add" => {
                        let Some(a) = body["accountId"].as_str().filter(|a| is_uuid(a)) else { return invalid("accountId", "Invalid uuid.") };
                        s["invitations"].as_array_mut().expect("invitations").push(json!({"accountId": a, "timestamp": now}));
                    }
                    "squad_invitation_remove" => {
                        let a = arg("invitedAccountId");
                        s["invitations"].as_array_mut().expect("invitations").retain(|i| i["accountId"] != json!(a));
                    }
                    "squad_invitation_accept" => {
                        let a = arg("accountId");
                        s["invitations"].as_array_mut().expect("invitations").retain(|i| i["accountId"] != json!(a));
                        let members = s["members"].as_array_mut().expect("members");
                        if !members.iter().any(|m| m["accountId"] == json!(a)) {
                            members.push(json!({"accountId": a, "timestamp": now}));
                        }
                    }
                    "squad_member_remove" => {
                        let a = arg("memberAccountId");
                        s["members"].as_array_mut().expect("members").retain(|m| m["accountId"] != json!(a));
                    }
                    _ => {
                        let Some(a) = body["accountId"].as_str() else { return invalid("accountId", "Missing account id.") };
                        if !s["members"].as_array().map_or(false, |m| m.iter().any(|x| x["accountId"] == json!(a))) {
                            return invalid("accountId", "The new leader must be a squad member.");
                        }
                        s["leaderAccountId"] = json!(a);
                    }
                }
                Response::json(s)
            }
            "activity_match_create" => {
                let mid = random_uuid();
                let v = json!({"matchId": mid, "creatorId": me, "status": "Pending", "creationTimestamp": now, "data": body, "results": []});
                self.st().matches.insert(mid, v.clone());
                Response::json(&v)
            }
            "activity_match_update" | "activity_match_result_set" => {
                let mut st = self.st();
                let Some(m) = st.matches.get_mut(&arg("matchId")) else { return not_found("Match") };
                if short == "activity_match_update" {
                    if let Some(s) = body.get("status") {
                        m["status"] = s.clone();
                    }
                    m["data"] = body.clone();
                } else {
                    m["results"] = body.get("results").cloned().unwrap_or(body.clone());
                    m["status"] = json!("Completed");
                }
                m["updateTimestamp"] = json!(now);
                Response::json(m)
            }
            "driver_bot_list_get" => {
                let st = self.st();
                Response::json(&json!(page(req, st.bots.clone())))
            }
            "driver_bot_index_list_get" => {
                let st = self.st();
                Response::json(&json!(st.bots.iter().enumerate().map(|(i, b)| json!({"index": i, "botId": b["botId"]})).collect::<Vec<_>>()))
            }
            "driver_bot_list_add" => {
                let incoming = body.as_array().cloned().unwrap_or_else(|| vec![body.clone()]);
                let mut st = self.st();
                let added: Vec<Value> = incoming.into_iter().map(|mut b| {
                    b["botId"] = json!(random_uuid());
                    b["timestamp"] = json!(now);
                    b
                }).collect();
                st.bots.extend(added.iter().cloned());
                Response::json(&json!(added))
            }
            "encrypted_package_create" | "encrypted_package_set" => {
                let pid = if short.ends_with("create") { random_uuid() } else { arg("encryptedPackageId") };
                let mut st = self.st();
                let prev = st.packages.get(&pid).cloned();
                if short.ends_with("set") && prev.is_none() {
                    return not_found("Encrypted package");
                }
                let mut v = prev.unwrap_or_else(|| json!({"encryptedPackageId": pid, "creatorId": me, "versions": [], "creationTimestamp": now}));
                if let Some(o) = body.as_object() {
                    for (k, x) in o {
                        v[k] = x.clone();
                    }
                }
                st.packages.insert(pid, v.clone());
                Response::json(&v)
            }
            "encrypted_package_list_get" | "account_encrypted_package_list_get" => {
                let st = self.st();
                Response::json(&json!(st.packages.values().filter(|p| short.starts_with("encrypted") || p["creatorId"] == json!(arg("accountId"))).cloned().collect::<Vec<_>>()))
            }
            "encrypted_package_version_create" => {
                let pid = arg("encryptedPackageId");
                let mp = multipart(req);
                let data = mp.as_ref().and_then(|m| m.fields.values().find(|f| f.0.is_some()).map(|f| f.2.clone())).unwrap_or_else(|| req.body.clone());
                let (_, url) = self.store_object(host, "application/octet-stream", data);
                let vid = random_uuid();
                let key = b64(&sha256(format!("{pid}:{vid}:{}", random_uuid()).as_bytes()));
                let mut st = self.st();
                let Some(p) = st.packages.get_mut(&pid) else { return not_found("Encrypted package") };
                let v = json!({"encryptedPackageVersionId": vid, "encryptedPackageId": pid, "fileUrl": url, "timestamp": now});
                p["versions"].as_array_mut().expect("versions").push(v.clone());
                let mut stored = v.clone();
                stored["cryptKey"] = json!(key);
                st.package_versions.insert(vid, stored);
                Response::json(&v)
            }
            "encrypted_package_version_crypt_key_get" => match self.st().package_versions.get(&arg("encryptedPackageVersionId")) {
                Some(v) => Response::json(&json!({"encryptedPackageVersionId": v["encryptedPackageVersionId"], "cryptKey": v["cryptKey"]})),
                None => not_found("Encrypted package version"),
            },
            "account_encrypted_package_key_get" => {
                let st = self.st();
                Response::json(&json!(st.stations.keys().filter_map(|pid| st.packages.get(pid)).flat_map(|p| p["versions"].as_array().cloned().unwrap_or_default()).filter_map(|v| v["encryptedPackageVersionId"].as_str().and_then(|id| st.package_versions.get(id)).map(|s| json!({"encryptedPackageVersionId": s["encryptedPackageVersionId"], "cryptKey": s["cryptKey"]}))).collect::<Vec<_>>()))
            }
            "item_collection_create" | "item_collection_set" => {
                let cid = if short.ends_with("create") { random_uuid() } else { arg("itemCollectionId") };
                let mut st = self.st();
                let prev = st.collections.get(&cid).cloned();
                if short.ends_with("set") && prev.is_none() {
                    return not_found("Item collection");
                }
                let mut v = prev.unwrap_or_else(|| json!({"itemCollectionId": cid, "creatorId": me, "versions": [], "activityId": Value::Null, "creationTimestamp": now}));
                if let Some(o) = body.as_object() {
                    for (k, x) in o {
                        v[k] = x.clone();
                    }
                }
                v["updateTimestamp"] = json!(now);
                st.collections.insert(cid, v.clone());
                Response::json(&v)
            }
            "item_collection_get" => match self.st().collections.get(&arg("itemCollectionId")) {
                Some(c) => Response::json(c),
                None => not_found("Item collection"),
            },
            "item_collection_list_get" => {
                let ids = id_list(req, "itemCollectionIdList");
                let st = self.st();
                Response::json(&json!(st.collections.values().filter(|c| ids.is_empty() || ids.iter().any(|i| c["itemCollectionId"] == json!(i))).cloned().collect::<Vec<_>>()))
            }
            "item_collection_activity_set" => {
                let mut st = self.st();
                let Some(c) = st.collections.get_mut(&arg("itemCollectionId")) else { return not_found("Item collection") };
                c["activityId"] = body.get("activityId").cloned().unwrap_or(Value::Null);
                Response::json(c)
            }
            "item_collection_version_create" => {
                let cid = arg("itemCollectionId");
                let mp = multipart(req);
                let data = mp.as_ref().and_then(|m| m.fields.values().find(|f| f.0.is_some()).map(|f| f.2.clone())).unwrap_or_else(|| req.body.clone());
                let (_, url) = self.store_object(host, "application/zip", data);
                let mut st = self.st();
                let Some(c) = st.collections.get_mut(&cid) else { return not_found("Item collection") };
                let v = json!({"itemCollectionVersionId": random_uuid(), "itemCollectionId": cid, "url": url, "timestamp": now});
                c["versions"].as_array_mut().expect("versions").push(v.clone());
                Response::json(&v)
            }
            "account_item_collection_favorite_list_get" => {
                let st = self.st();
                Response::json(&json!(st.collection_favorites.iter().filter_map(|c| st.collections.get(c).cloned()).collect::<Vec<_>>()))
            }
            "account_item_collection_favorite_add" | "account_item_collection_favorite_remove" => {
                if let Some(r) = self.mine_or_403(&arg("accountId")) {
                    return r;
                }
                let cid = arg("itemCollectionId");
                let mut st = self.st();
                if short.ends_with("add") {
                    if !st.collections.contains_key(&cid) {
                        return not_found("Item collection");
                    }
                    st.collection_favorites.insert(cid);
                } else {
                    st.collection_favorites.remove(&cid);
                }
                Response::empty(204)
            }
            "title_create" | "title_set" => {
                let tid = if short.ends_with("create") { body["titleId"].as_str().map(String::from).unwrap_or_else(random_uuid) } else { arg("titleId") };
                let mut st = self.st();
                if short.ends_with("set") && !st.titles.contains_key(&tid) {
                    return not_found("Title");
                }
                let mut v = st.titles.get(&tid).cloned().unwrap_or_else(|| json!({"titleId": tid, "creatorId": me, "creationTimestamp": now}));
                if let Some(o) = body.as_object() {
                    for (k, x) in o {
                        v[k] = x.clone();
                    }
                }
                st.titles.insert(tid, v.clone());
                Response::json(&v)
            }
            "waiting_queue_client_add" => {
                let wid = arg("waitingClientId");
                if wid.len() < 3 || !(wid.starts_with("u_") || wid.starts_with("r_")) {
                    return invalid("waitingClientId", "Invalid waiting client id.");
                }
                let mut st = self.st();
                let v = json!({"rank": 0, "refreshTime": WAITING_REFRESH_SECS, "waitingTime": 0, "timestamp": now});
                st.waiting.insert(wid, v.clone());
                Response::json(&v)
            }
            _ => {
                crate::record_raw("online", "unhandled", &format!("[trackmania] core route {id} {} {}", req.method, req.path));
                nadeo_error(501, "C-AA-00-05", &format!("TomBox has no implementation of {id} yet"), Value::Null)
            }
        }
    }

    fn map_write(&self, kind: &str, map_id: &str, req: &Request, host: &str) -> Response {
        let mp = multipart(req);
        let body = req.json();
        let me = self.account_id.clone();
        let now = now_ts();
        let file = mp.as_ref().and_then(|m| m.fields.values().find(|f| f.0.as_deref().map_or(false, |n| n.to_ascii_lowercase().ends_with(".gbx"))).map(|f| (f.0.clone().unwrap_or_default(), f.2.clone())));
        let thumb = mp.as_ref().and_then(|m| m.fields.values().find(|f| f.0.as_deref().map_or(false, |n| n.to_ascii_lowercase().ends_with(".jpg"))).map(|f| f.2.clone()));
        let uid = str_field(&body, &mp, "mapUid");
        if kind == "map_create" {
            let Some(uid) = uid else { return invalid("mapUid", "The map uid is required.") };
            if let Some(m) = self.st().maps.values().find(|m| m["mapUid"] == json!(uid)).cloned() {
                return nadeo_error(400, "C-AA-00-03", "There was a validation error.", json!({"mapUid": "A map with this uid already exists.", "mapId": m["mapId"]}));
            }
            let Some((filename, data)) = file else { return invalid("data", "The map file is required.") };
            let (_, url) = self.store_object(host, "application/octet-stream", data);
            let thumb_url = thumb.map(|t| self.store_object(host, "image/jpeg", t).1);
            let mid = random_uuid();
            let num = |k: &str| str_field(&body, &mp, k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(-1);
            let flag = |k: &str| str_field(&body, &mp, k).map_or(false, |v| v == "true" || v == "1");
            let v = json!({
                "author": str_field(&body, &mp, "author").unwrap_or_else(|| me.clone()), "authorScore": num("authorScore"), "bronzeScore": num("bronzeScore"),
                "silverScore": num("silverScore"), "goldScore": num("goldScore"), "collectionName": str_field(&body, &mp, "collectionName").unwrap_or_else(|| "Stadium".into()),
                "createdWithGamepadEditor": flag("createdWithGamepadEditor"), "createdWithSimpleEditor": flag("createdWithSimpleEditor"), "filename": filename,
                "hasClones": false, "isPlayable": flag("isPlayable"), "mapId": mid, "mapStyle": str_field(&body, &mp, "mapStyle").unwrap_or_default(),
                "mapType": str_field(&body, &mp, "mapType").unwrap_or_else(|| "TrackMania\\TM_Race".into()), "mapUid": uid,
                "name": str_field(&body, &mp, "name").unwrap_or_default(), "submitter": me, "timestamp": now, "fileUrl": url, "thumbnailUrl": thumb_url,
            });
            self.st().maps.insert(mid, v.clone());
            Response::json(&v)
        } else {
            let mut st = self.st();
            let Some(m) = st.maps.get(map_id).cloned() else { return not_found("Map") };
            drop(st);
            let mut m = m;
            if let Some((filename, data)) = file {
                m["fileUrl"] = json!(self.store_object(host, "application/octet-stream", data).1);
                m["filename"] = json!(filename);
            }
            if let Some(t) = thumb {
                m["thumbnailUrl"] = json!(self.store_object(host, "image/jpeg", t).1);
            }
            for k in ["name", "mapStyle", "mapType", "collectionName"] {
                if let Some(v) = str_field(&body, &mp, k) {
                    m[k] = json!(v);
                }
            }
            for k in ["authorScore", "bronzeScore", "silverScore", "goldScore"] {
                if let Some(v) = str_field(&body, &mp, k).and_then(|v| v.parse::<i64>().ok()) {
                    m[k] = json!(v);
                }
            }
            m["timestamp"] = json!(now);
            st = self.st();
            st.maps.insert(map_id.to_string(), m.clone());
            Response::json(&m)
        }
    }

    fn record_write(&self, req: &Request, host: &str) -> Response {
        let mp = multipart(req);
        let body = req.json();
        let me = self.account_id.clone();
        let now = now_ts();
        let Some(map_id) = str_field(&body, &mp, "mapId").filter(|m| is_uuid(m)) else { return invalid("mapId", "Invalid uuid.") };
        let Some(map) = self.st().maps.get(&map_id).cloned() else { return not_found("Map") };
        let score: Value = str_field(&body, &mp, "recordScore").and_then(|s| serde_json::from_str(&s).ok()).or_else(|| body.get("recordScore").cloned()).unwrap_or(Value::Null);
        let Some(time) = score["time"].as_i64().or_else(|| str_field(&body, &mp, "time").and_then(|t| t.parse().ok())) else { return invalid("recordScore", "The record time is required.") };
        let mode = str_field(&body, &mp, "gameMode").unwrap_or_else(|| "TimeAttack".into());
        let scope = str_field(&body, &mp, "seasonId");
        let medal = [("authorScore", 4), ("goldScore", 3), ("silverScore", 2), ("bronzeScore", 1)].iter().find(|(k, _)| map[*k].as_i64().map_or(false, |s| s >= 0 && time <= s)).map(|(_, m)| *m).unwrap_or(0);
        let ghost = mp.as_ref().and_then(|m| m.fields.values().find(|f| f.0.is_some()).map(|f| f.2.clone()));
        let url = ghost.map(|g| self.store_object(host, "application/octet-stream", g).1);
        let mut st = self.st();
        let existing = st.records.iter().position(|r| r["accountId"] == json!(me) && r["mapId"] == json!(map_id) && r["gameMode"] == json!(mode) && r["scopeId"] == json!(scope));
        if let Some(i) = existing {
            if st.records[i]["recordScore"]["time"].as_i64().map_or(false, |t| t <= time) {
                return Response::json(&st.records[i]);
            }
            st.records.remove(i);
        }
        let v = json!({
            "accountId": me, "filename": format!("Replays\\Downloaded\\{map_id}_{me}.replay.gbx"), "gameMode": mode, "gameModeCustomData": str_field(&body, &mp, "gameModeCustomData").unwrap_or_default(),
            "mapId": map_id, "mapRecordId": random_uuid(), "medal": medal, "recordScore": {"respawnCount": score["respawnCount"].as_u64().unwrap_or(0), "score": score["score"].as_i64().unwrap_or(0), "time": time},
            "removed": false, "scopeId": scope, "scopeType": if scope.is_some() { "Season" } else { "PersonalBest" }, "timestamp": now, "url": url,
        });
        st.records.push(v.clone());
        Response::json(&v)
    }
}

pub fn audience_for_host(host: &str) -> &'static str {
    let h = host.to_ascii_lowercase();
    if h.ends_with(".nadeo.live") || h.ends_with(".nadeo.club") {
        "NadeoLiveServices"
    } else {
        "NadeoServices"
    }
}

pub fn segments_of(req: &Request) -> Vec<&str> {
    req.path.split('/').filter(|s| !s.is_empty()).collect()
}
