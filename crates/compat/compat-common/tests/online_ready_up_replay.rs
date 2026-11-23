#![cfg(tombox_backend_data)]

use compat_common::online::fortnite::party::captured;
use compat_common::os_host::fortnite_match as fm;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const ACCT: &str = "XboxEmulator";
const GAMESERVER: &str = "127.0.0.1:7777";
const FN_HOST: &str = "fortnite-public-service-prod11.ol.epicgames.com";
const PARTY_HOST: &str = "party-service-prod.ol.epicgames.com";
const CONN_ID: &str = "XboxEmulator@prod.ol.epicgames.com/V2:Fortnite:XSX:2533274790395905:E4C796FB43D6456A192D3D8602827420";
const RUN42_TICKET: &str = "/fortnite/api/game/v2/matchmakingservice/ticket/player/XboxEmulator?partyPlayerIds=XboxEmulator&bucketId=14457890%3A0%3ANONE%3Aplaylist_defaultsquad&player.platform=XboxOne&player.subregions=&player.option.fillTeam=true&player.option.crossplayOptOut=false&player.option.partyId={party}&player.option.splitScreen=false&party.XSX=true&input.KBM=true&player.input=KBM&player.option.uiLanguage=en";

const RUN42_LOBBY: &[(&str, &str, &str)] = &[
    ("POST", "/party/api/v1/Fortnite/parties", r#"{"config":{"discoverability":"INVITED_ONLY","join_confirmation":true,"joinability":"INVITE_AND_FORMER","max_size":16},"join_info":{"connection":{"id":"XboxEmulator@prod.ol.epicgames.com/V2:Fortnite:XSX:2533274790395905:E4C796FB43D6456A192D3D8602827420","meta":{"urn:epic:conn:platform_s":"XSX","urn:epic:conn:type_s":"game","urn:epic:conn:platform:dn_s":"XboxEmulator"},"yield_leadership":false},"meta":{"urn:epic:member:dn_s":"XboxEmulator"}},"meta":{"urn:epic:cfg:party-type-id_s":"default","urn:epic:cfg:build-id_s":"1:3:14457890","urn:epic:cfg:can-join_b":"true","urn:epic:cfg:join-request-action_s":"Manual","urn:epic:cfg:presence-perm_s":"Noone","urn:epic:cfg:invite-perm_s":"Noone","urn:epic:cfg:chat-enabled_b":"true","urn:epic:cfg:accepting-members_b":"false","urn:epic:cfg:not-accepting-members-reason_i":"0"}}"#),
    ("PATCH", "/party/api/v1/Fortnite/parties/{party}", r#"{"config":{"discoverability":"ALL","joinability":"OPEN"},"meta":{"delete":[],"update":{"urn:epic:cfg:presence-perm_s":"Anyone","urn:epic:cfg:not-accepting-members-reason_i":"33","VoiceChat:implementation_s":"EOSVoiceChat"}},"revision":0}"#),
    ("PATCH", "/party/api/v1/Fortnite/parties/{party}/members/XboxEmulator/meta", r#"{"delete":[],"revision":0,"update":{"VoiceChat:required-implementation_s":"EOSVoiceChat"}}"#),
    ("PATCH", "/party/api/v1/Fortnite/parties/{party}", r#"{"config":{"discoverability":"ALL","joinability":"OPEN"},"meta":{"delete":["urn:epic:cfg:not-accepting-members-reason_i"],"update":{"urn:epic:cfg:invite-perm_s":"Anyone","urn:epic:cfg:accepting-members_b":"true","Default:PrimaryGameSessionId_s":"","Default:PartyState_s":"BattleRoyaleView","Default:LobbyConnectionStarted_b":"false","Default:MatchmakingResult_s":"NoResults","Default:MatchmakingState_s":"NotMatchmaking","Default:SessionIsCriticalMission_b":"false","Default:ZoneTileIndex_U":"-1","Default:ZoneInstanceId_s":"","Default:TheaterId_s":"","Default:TileStates_j":"{\"TileStates\":[]}","Default:PartyMatchmakingInfo_j":"{\"PartyMatchmakingInfo\":{\"buildId\":-1,\"hotfixVersion\":-1,\"regionId\":\"\",\"playlistName\":\"None\",\"playlistRevision\":0,\"tournamentId\":\"\",\"eventWindowId\":\"\",\"linkCode\":\"\"}}","Default:CustomMatchKey_s":"","Default:PlaylistData_j":"{\"PlaylistData\":{\"playlistName\":\"Playlist_DefaultSquad\",\"tournamentId\":\"\",\"eventWindowId\":\"\",\"regionId\":\"NONE\"}}","Default:AllowJoinInProgress_b":"false","Default:LFGTime_s":"0001-01-01T00:00:00.000Z","Default:AthenaSquadFill_b":"true","Default:PartyIsJoinedInProgress_b":"false","Default:GameSessionKey_s":"","Default:RawSquadAssignments_j":"{\"RawSquadAssignments\":[{\"memberId\":\"XboxEmulator\",\"absoluteMemberIdx\":0}]}","Default:PrivacySettings_j":"{\"PrivacySettings\":{\"partyType\":\"Public\",\"partyInviteRestriction\":\"AnyMember\",\"bOnlyLeaderFriendsCanJoin\":false}}","Default:PlatformSessions_j":"{\"PlatformSessions\":[{\"sessionType\":\"MPSD\",\"sessionId\":\"\",\"ownerPrimaryId\":\"XboxEmulator\"}]}"}},"revision":1}"#),
    ("PATCH", "/party/api/v1/Fortnite/parties/{party}", r#"{"config":{"discoverability":"ALL","joinability":"OPEN"},"meta":{"delete":[],"update":{"urn:epic:cfg:accepting-members_b":"false","urn:epic:cfg:not-accepting-members-reason_i":"2","Default:PartyState_s":"BattleRoyalePreloading"}},"revision":2}"#),
];

fn port() -> u16 {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    std::env::set_var("TOMBOX_ONLINE_ACCOUNT", ACCT);
    std::env::set_var("TOMBOX_FN_GAMESERVER", GAMESERVER);
    std::env::remove_var("TOMBOX_FN_REGION");
    compat_common::online::start_for_config().expect("server")
}

struct Http {
    status: u16,
    body: Vec<u8>,
}

impl Http {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn http(method: &str, host: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> Http {
    let mut s = TcpStream::connect(("127.0.0.1", port())).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let data = body.unwrap_or("");
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-Host: {host}\r\nConnection: close\r\nContent-Length: {}\r\n", data.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(data.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let text = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = text.lines().next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    Http { status, body: raw[split + 4..].to_vec() }
}

fn send_text(s: &mut TcpStream, text: &str) {
    let p = text.as_bytes();
    let mask = [0x21u8, 0x7c, 0x93, 0x05];
    let mut f = vec![0x81u8];
    if p.len() < 126 {
        f.push(0x80 | p.len() as u8);
    } else {
        f.push(0x80 | 126);
        f.extend_from_slice(&(p.len() as u16).to_be_bytes());
    }
    f.extend_from_slice(&mask);
    f.extend(p.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    s.write_all(&f).unwrap();
}

fn frame(r: &mut BufReader<TcpStream>) -> (u8, Vec<u8>) {
    let (_, op, data) = compat_common::online::ws::read_frame(r).unwrap();
    (op, data)
}

fn upgrade(host: &str, extra: &[(&str, &str)], proto: Option<&str>) -> (TcpStream, BufReader<TcpStream>, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port())).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut head = format!("GET / HTTP/1.1\r\nHost: {host}\r\nX-Forwarded-Host: {host}:443\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n");
    if let Some(p) = proto {
        head.push_str(&format!("Sec-WebSocket-Protocol: {p}\r\n"));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut first = String::new();
    loop {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        if first.is_empty() {
            first = l.trim_end().to_string();
        }
        if l == "\r\n" || l.is_empty() {
            break;
        }
    }
    (s, r, first)
}

fn xmpp_login() -> (TcpStream, BufReader<TcpStream>) {
    let (mut s, mut r, first) = upgrade("xmpp-service-prod.ol.epicgames.com", &[], Some("xmpp"));
    assert!(first.starts_with("HTTP/1.1 101"), "{first}");
    let open = "<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"prod.ol.epicgames.com\" version=\"1.0\"/>";
    let text = |r: &mut BufReader<TcpStream>| String::from_utf8_lossy(&frame(r).1).into_owned();
    send_text(&mut s, open);
    assert!(text(&mut r).starts_with("<open "));
    assert!(text(&mut r).contains("PLAIN"));
    let token = compat_common::online::util::b64(format!("\0{ACCT}\0eg1~token").as_bytes());
    send_text(&mut s, &format!("<auth xmlns=\"urn:ietf:params:xml:ns:xmpp-sasl\" mechanism=\"PLAIN\">{token}</auth>"));
    assert!(text(&mut r).contains("<success"));
    send_text(&mut s, open);
    assert!(text(&mut r).starts_with("<open "));
    assert!(text(&mut r).contains("xmpp-bind"));
    send_text(&mut s, "<iq type=\"set\" id=\"_xmpp_bind1\" xmlns=\"jabber:client\"><bind xmlns=\"urn:ietf:params:xml:ns:xmpp-bind\"><resource>V2:Fortnite:XSX:2533274790395905:E4C796FB43D6456A192D3D8602827420</resource></bind></iq>");
    assert!(text(&mut r).contains(&format!("<jid>{CONN_ID}</jid>")));
    send_text(&mut s, "<iq type=\"set\" id=\"_xmpp_session1\"><session xmlns=\"urn:ietf:params:xml:ns:xmpp-session\"/></iq>");
    assert!(text(&mut r).contains("type=\"result\""));
    (s, r)
}

fn push(r: &mut BufReader<TcpStream>, ty: &str, party: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let (op, data) = frame(r);
        let text = String::from_utf8_lossy(&data).into_owned();
        if op != 1 || !text.starts_with("<message") {
            continue;
        }
        let a = text.find("<body>").unwrap() + 6;
        let b = text.find("</body>").unwrap();
        let v: Value = serde_json::from_str(&compat_common::online::xmpp::unescape(&text[a..b])).unwrap();
        if v["type"] == json!(format!("com.epicgames.social.party.notification.v0.{ty}")) && v["party_id"] == json!(party) {
            assert!(text.contains("from=\"xmpp-admin@prod.ol.epicgames.com\""));
            return v;
        }
    }
    panic!("no {ty} push for party {party}");
}

fn missing_in_order(log: &[String], wanted: &[String]) -> Option<String> {
    let mut at = 0;
    for w in wanted {
        match log[at..].iter().position(|l| l.starts_with(w.as_str())) {
            Some(i) => at += i + 1,
            None => return Some(format!("backend log is missing '{w}' after entry {at}:\n{}", log.join("\n"))),
        }
    }
    None
}

fn in_order(wanted: &[String]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match missing_in_order(&captured(), wanted) {
            None => return,
            Some(e) if Instant::now() >= deadline => panic!("{e}"),
            Some(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[test]
fn ready_up_replay_lobby_to_local_match() {
    assert_eq!(fm::parse_mode(None), fm::Mode::Play);
    let (_x, mut xr) = xmpp_login();
    let before = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/user/{ACCT}"), &[], None);
    assert_eq!(before.status, 200);

    let (m, path, body) = RUN42_LOBBY[0];
    assert_eq!((m, path), ("POST", "/party/api/v1/Fortnite/parties"));
    let created = http(m, PARTY_HOST, path, &[("Content-Type", "application/json")], Some(body));
    assert_eq!(created.status, 200, "{}", created.text());
    let party = created.json()["id"].as_str().unwrap().to_string();
    let joined = push(&mut xr, "MEMBER_JOINED", &party);
    assert_eq!(joined["account_id"], ACCT);
    assert_eq!(joined["connection"]["id"], CONN_ID);
    assert_eq!(created.json()["meta"]["urn:epic:cfg:build-id_s"], "1:3:14457890");

    let mut party_rev = 0;
    let mut member_rev = 0;
    for (m, path, body) in &RUN42_LOBBY[1..] {
        let path = path.replace("{party}", &party);
        let r = http(m, PARTY_HOST, &path, &[("Content-Type", "application/json")], Some(body));
        assert_eq!(r.status, 204, "{m} {path}: {}", r.text());
        if path.ends_with("/meta") {
            member_rev += 1;
            let v = push(&mut xr, "MEMBER_STATE_UPDATED", &party);
            assert_eq!(v["revision"], member_rev);
            assert_eq!(v["account_id"], ACCT);
        } else {
            party_rev += 1;
            let v = push(&mut xr, "PARTY_UPDATED", &party);
            assert_eq!(v["revision"], party_rev);
            assert_eq!(v["captain_id"], ACCT);
        }
    }
    assert_eq!((party_rev, member_rev), (3, 1));
    let p = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], None).json();
    assert_eq!(p["meta"]["Default:PartyState_s"], "BattleRoyalePreloading");
    assert!(p["meta"]["Default:PlaylistData_j"].as_str().unwrap().contains("Playlist_DefaultSquad"));
    assert_eq!(p["members"][0]["meta"]["VoiceChat:required-implementation_s"], "EOSVoiceChat");

    let engine = http("GET", FN_HOST, "/fortnite/api/cloudstorage/system/DefaultEngine.ini", &[], None);
    assert_eq!(engine.status, 200);
    let ini = engine.text();
    let cv = ini.find("[ConsoleVariables]").expect("ConsoleVariables section");
    let section = &ini[cv..ini[cv + 1..].find("\n[").map(|i| cv + 1 + i).unwrap_or(ini.len())];
    assert!(section.lines().any(|l| l.trim() == "FortMatchmakingV2.EnableContentBeacon=0"), "{section}");
    let listing = http("GET", FN_HOST, "/fortnite/api/cloudstorage/system", &[], None).json();
    let entry = listing.as_array().unwrap().iter().find(|e| e["filename"] == "DefaultEngine.ini").unwrap().clone();
    assert_eq!(entry["length"].as_u64().unwrap() as usize, engine.body.len());

    let vc = http("GET", FN_HOST, "/fortnite/api/v2/versioncheck/XboxOne?version=%2B%2BFortnite%2BRelease-14.50-CL-14457890-XSX", &[], None);
    assert_eq!(vc.json()["type"], "NO_UPDATE");

    let t = http("GET", FN_HOST, &RUN42_TICKET.replace("{party}", &party), &[], None);
    assert_eq!(t.status, 200, "{}", t.text());
    let t = t.json();
    assert_eq!(t["ticketType"], "mms-player");
    let url = t["serviceUrl"].as_str().unwrap().to_string();
    assert_eq!(url, "wss://fortnite-matchmaking-public-service-live-nae.ol.epicgames.com:443");
    let mms_host = url.trim_start_matches("wss://").split(':').next().unwrap().to_string();
    let auth = format!("Epic-Signed mms-player {} {} 7a9c3e1d0b4f", t["payload"].as_str().unwrap(), t["signature"].as_str().unwrap());
    let (mut ws, mut wr, first) = upgrade(&mms_host, &[("Authorization", &auth)], None);
    assert!(first.starts_with("HTTP/1.1 101"), "{first}");
    send_text(&mut ws, "ping");
    let mut frames: Vec<Value> = Vec::new();
    let mut pong = false;
    let close_code = loop {
        let (op, data) = frame(&mut wr);
        if op == 8 {
            break u16::from_be_bytes([data[0], data[1]]);
        }
        let text = String::from_utf8_lossy(&data).into_owned();
        if text == "pong" {
            pong = true;
            continue;
        }
        frames.push(serde_json::from_str(&text).unwrap());
    };
    assert!(pong);
    assert_eq!(close_code, 1000);
    let names: Vec<String> = frames.iter().map(|f| match f["name"].as_str().unwrap() {
        "StatusUpdate" => f["payload"]["state"].as_str().unwrap().to_string(),
        n => n.to_string(),
    }).collect();
    assert_eq!(names, ["Connecting", "Waiting", "Queued", "SessionAssignment", "Play"]);
    let ticket_id = frames[2]["payload"]["ticketId"].as_str().unwrap().to_string();
    let match_id = frames[3]["payload"]["matchId"].as_str().unwrap().to_string();
    let play = &frames[4]["payload"];
    assert_eq!(play["matchId"], json!(match_id));
    assert_eq!(play["joinDelaySec"], 1);
    let sid = play["sessionId"].as_str().unwrap().to_string();

    let sess = http("GET", FN_HOST, &format!("/fortnite/api/matchmaking/session/{sid}"), &[], None);
    assert_eq!(sess.status, 200);
    let s = sess.json();
    assert_eq!(s["id"], json!(sid));
    assert_eq!(s["attributes"]["PLAYLISTNAME_s"], "playlist_defaultsquad");
    assert_eq!(s["attributes"]["REGION_s"], "NAE");
    assert_eq!(s["buildUniqueId"], "14457890");
    assert_eq!(s["started"], false);
    let addr = s["serverAddress"].as_str().unwrap().to_string();
    let sport = s["serverPort"].as_i64().unwrap() as i32;
    assert_eq!(format!("{addr}:{sport}"), GAMESERVER);

    let key = http("GET", FN_HOST, &format!("/fortnite/api/game/v2/matchmaking/account/{ACCT}/session/{sid}"), &[], None);
    assert_eq!(key.status, 200);
    assert_eq!(compat_common::online::util::b64_decode(key.json()["key"].as_str().unwrap()).len(), 32);
    let join = http("POST", FN_HOST, &format!("/fortnite/api/matchmaking/session/{sid}/join?accountId={ACCT}"), &[], None);
    assert_eq!(join.status, 204);
    let found = http("GET", FN_HOST, &format!("/fortnite/api/matchmaking/session/findPlayer/{ACCT}"), &[], None).json();
    assert!(found.as_array().unwrap().iter().any(|x| x["id"] == json!(sid)));

    let after = format!(r#"{{"config":{{"discoverability":"ALL","joinability":"OPEN"}},"meta":{{"delete":[],"update":{{"Default:PartyState_s":"BattleRoyaleView","Default:PrimaryGameSessionId_s":"{sid}","Default:MatchmakingState_s":"NotMatchmaking"}}}},"revision":3}}"#);
    assert_eq!(http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], Some(&after)).status, 204);
    assert_eq!(push(&mut xr, "PARTY_UPDATED", &party)["party_state_updated"]["Default:PrimaryGameSessionId_s"], json!(sid));

    assert_eq!(fm::playlist(), "playlist_defaultsquad");
    let sentinel = fm::sentinel();
    let travel = fm::UrlView { protocol: "unreal".into(), host: addr, port: sport, map: "".into(), ops: vec!["Name=Player".into(), format!("EncryptionToken={sid}")] };
    let local = fm::rewrite_text(&travel, &sentinel, &fm::map(), &fm::gamemode(), &fm::playlist()).expect("sentinel travel is rewritten");
    assert_eq!(local, "/Game/Athena/Apollo/Maps/Apollo_Terrain?game=/Game/Athena/Athena_GameMode.Athena_GameMode_C?Playlist=playlist_defaultsquad");
    let frontend = fm::UrlView { protocol: "unreal".into(), host: "".into(), port: 7777, map: "/Game/Maps/Frontend".into(), ops: vec!["Name=Player".into()] };
    assert!(fm::rewrite_text(&frontend, &sentinel, &fm::map(), &fm::gamemode(), &fm::playlist()).is_none());

    let p = format!("/party/api/v1/Fortnite/parties/{party}");
    in_order(&[
        "[party] POST /party/api/v1/Fortnite/parties body=".into(),
        format!("[party] POST /party/api/v1/Fortnite/parties create party {party} captain {ACCT} published 1"),
        format!("[party] PATCH {p} rev 0->1 published 1"),
        format!("[party] PATCH {p}/members/{ACCT}/meta rev 0->1 published 1"),
        format!("[party] PATCH {p} rev 1->2 published 1"),
        format!("[party] PATCH {p} body="),
        format!("[party] PATCH {p} rev 2->3 published 1"),
        format!("[mms] GET /fortnite/api/game/v2/matchmakingservice/ticket/player/{ACCT} body="),
        format!("[mms] ticket {ticket_id} for {ACCT} bucket 14457890:0:NONE:playlist_defaultsquad playlist playlist_defaultsquad players 1"),
        format!("[mms] websocket ticket {ticket_id} accepted"),
        format!("[session] create {sid} match {match_id} playlist playlist_defaultsquad build 14457890 server {GAMESERVER}"),
        format!("[mms] ticket {ticket_id} -> match {match_id} session {sid}"),
        format!("[session] POST /fortnite/api/matchmaking/session/{sid}/join body="),
        format!("[session] join {sid} {ACCT} -> 1 players"),
        format!("[party] PATCH {p} rev 3->4 published 1"),
    ]);
}
