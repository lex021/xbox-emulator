use core::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn ReadProcessMemory(h: *mut c_void, addr: *const c_void, buf: *mut c_void, n: usize, read: *mut usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn VirtualQuery(addr: *const c_void, mbi: *mut Mbi, len: usize) -> usize;
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Mbi {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

pub const VAR: &str = "TOMBOX_TM_RACE_PROBE";
pub const KEYS: [&str; 18] = ["race.script_player_vtable", "race.game_time_ms", "race.sp_player", "race.player_race_state", "race.rs_cur", "race.rs_running", "race.player_race", "race.player_lap_waypoints", "race.player_lap", "race.race_start_ms", "race.waypoint_count", "race.player_entity", "race.player_flags", "race.offzone_bit", "race.player_position", "race.player_vehicle_state", "race.vs_surface", "race.surface_water"];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub vtable: usize,
    pub game_time: usize,
    pub sp_player: usize,
    pub race_state: usize,
    pub rs_cur: usize,
    pub rs_running: usize,
    pub race: usize,
    pub lap_waypoints: usize,
    pub lap: usize,
    pub race_start: usize,
    pub waypoint_count: usize,
    pub entity: usize,
    pub flags: usize,
    pub offzone_bit: usize,
    pub position: usize,
    pub vehicle_state: usize,
    pub surface: usize,
    pub water: usize,
}

pub fn layout(o: &crate::title_offsets::Offsets) -> Result<Layout, String> {
    let v = o.require(&KEYS)?;
    Ok(Layout { vtable: v[0], game_time: v[1], sp_player: v[2], race_state: v[3], rs_cur: v[4], rs_running: v[5], race: v[6], lap_waypoints: v[7], lap: v[8], race_start: v[9], waypoint_count: v[10], entity: v[11], flags: v[12], offzone_bit: v[13], position: v[14], vehicle_state: v[15], surface: v[16], water: v[17] })
}

const MAX_LINES: u32 = 4000;
const RUN_MS: u64 = 600_000;
const RESCAN_MS: u64 = 5000;
const CHUNK: usize = 1 << 20;
const MAX_CANDIDATES: usize = 16;

fn read_bytes(addr: usize, out: &mut [u8]) -> bool {
    let mut got = 0usize;
    unsafe { ReadProcessMemory(GetCurrentProcess(), addr as *const c_void, out.as_mut_ptr() as *mut c_void, out.len(), &mut got) != 0 && got == out.len() }
}

fn rd<const N: usize>(addr: usize) -> Option<[u8; N]> {
    let mut b = [0u8; N];
    read_bytes(addr, &mut b).then_some(b)
}

fn u32_at(a: usize) -> Option<u32> {
    rd::<4>(a).map(u32::from_le_bytes)
}

fn i32_at(a: usize) -> Option<i32> {
    rd::<4>(a).map(i32::from_le_bytes)
}

fn ptr_at(a: usize) -> Option<usize> {
    rd::<8>(a).map(|b| u64::from_le_bytes(b) as usize).filter(|&p| p >= 0x10000 && p < 0x7fff_ffff_0000)
}

fn f32_at(a: usize) -> Option<f32> {
    rd::<4>(a).map(f32::from_le_bytes)
}

pub fn find_script_players(vtable: usize, cap: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut p = 0x1_0000usize;
    let want = (vtable as u64).to_le_bytes();
    while p < 0x7fff_ffff_0000 && out.len() < cap {
        let mut m = Mbi::default();
        if unsafe { VirtualQuery(p as *const c_void, &mut m, core::mem::size_of::<Mbi>()) } == 0 {
            break;
        }
        let next = m.base + m.region_size;
        if m.state == 0x1000 && m.kind == 0x2_0000 && m.protect & 0x04 != 0 && m.protect & 0x100 == 0 {
            let mut a = m.base;
            while a < next && out.len() < cap {
                let n = CHUNK.min(next - a);
                if read_bytes(a, &mut buf[..n]) {
                    for (i, c) in buf[..n].chunks_exact(8).enumerate() {
                        if c == want {
                            out.push(a + i * 8);
                            if out.len() >= cap {
                                break;
                            }
                        }
                    }
                }
                a += n;
            }
        }
        if next <= p {
            break;
        }
        p = next;
    }
    out
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Sample {
    pub racing: bool,
    pub race_ms: Option<i64>,
    pub checkpoints: Option<u32>,
    pub lap_checkpoints: Option<u32>,
    pub lap: Option<u32>,
    pub pos: Option<[f32; 3]>,
    pub surface: Option<u32>,
    pub offzone: Option<bool>,
}

pub fn sample(l: &Layout, base: usize, player: usize) -> Sample {
    let mut s = Sample::default();
    let rs = player + l.race_state;
    let running = i32_at(rs + l.rs_running);
    let cur = i32_at(rs + l.rs_cur);
    s.racing = matches!((running, cur), (Some(r), Some(c)) if r != -1 && r == c);
    let race = ptr_at(player + l.race);
    if let (true, Some(r), Some(now)) = (s.racing, race, u32_at(base.wrapping_add(l.game_time))) {
        s.race_ms = u32_at(r + l.race_start).map(|st| now.wrapping_sub(st) as i32 as i64);
    }
    s.checkpoints = race.and_then(|r| u32_at(r + l.waypoint_count));
    s.lap_checkpoints = ptr_at(player + l.lap_waypoints).and_then(|w| u32_at(w + l.waypoint_count));
    s.lap = u32_at(player + l.lap);
    if i32_at(player + l.entity).map(|e| e != -1).unwrap_or(false) {
        let x = f32_at(player + l.position);
        let y = f32_at(player + l.position + 4);
        let z = f32_at(player + l.position + 8);
        if let (Some(x), Some(y), Some(z)) = (x, y, z) {
            if x.is_finite() && y.is_finite() && z.is_finite() {
                s.pos = Some([x, y, z]);
            }
        }
    }
    s.surface = ptr_at(player + l.vehicle_state).and_then(|v| u32_at(v + l.surface));
    s.offzone = u32_at(player + l.flags).map(|f| f >> l.offzone_bit & 1 != 0);
    s
}

pub fn line(l: &Layout, t_ms: u64, sp: usize, player: usize, s: &Sample) -> String {
    let opt = |v: Option<String>| v.unwrap_or_else(|| "?".into());
    format!(
        "t={t_ms}ms sp={sp:#x} player={player:#x} racing={} race_ms={} cp={} lap_cp={} lap={} pos={} surface={} water={} offzone={}",
        s.racing as u8,
        opt(s.race_ms.map(|v| v.to_string())),
        opt(s.checkpoints.map(|v| v.to_string())),
        opt(s.lap_checkpoints.map(|v| v.to_string())),
        opt(s.lap.map(|v| v.to_string())),
        opt(s.pos.map(|p| format!("{:.1},{:.1},{:.1}", p[0], p[1], p[2]))),
        opt(s.surface.map(|v| v.to_string())),
        opt(s.surface.map(|v| ((v as usize == l.water) as u8).to_string())),
        opt(s.offzone.map(|v| (v as u8).to_string())),
    )
}

fn pick(l: &Layout, candidates: &[usize]) -> Option<(usize, usize)> {
    let mut first = None;
    for &sp in candidates {
        let Some(p) = ptr_at(sp + l.sp_player) else { continue };
        if ptr_at(p + l.race).is_some() {
            return Some((sp, p));
        }
        first.get_or_insert((sp, p));
    }
    first
}

pub fn enabled() -> bool {
    std::env::var(VAR).ok().map(|v| v.trim() != "0" && !v.trim().is_empty()).unwrap_or(false)
}

pub fn start() {
    if !enabled() {
        return;
    }
    let base = unsafe { GetModuleHandleW(core::ptr::null()) } as usize;
    if base == 0 {
        return;
    }
    let Some(o) = crate::title_offsets::for_this_title() else {
        crate::record_raw("tm_race", "inert", &crate::title_offsets::status_for_this_title());
        return;
    };
    let l = match layout(o) {
        Ok(l) => l,
        Err(e) => {
            crate::record_raw("tm_race", "inert", &e);
            return;
        }
    };
    let stamp = unsafe { crate::title_offsets::image_timestamp(base as *const u8) };
    if !o.matches_image(stamp) {
        crate::record_raw("tm_race", "inert", &format!("{} was made for another build (image timestamp {stamp:#x})", o.source));
        return;
    }
    let source = o.source.clone();
    let period = std::env::var(VAR).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|v| *v >= 50).unwrap_or(250);
    let _ = std::thread::Builder::new().name("tombox-tm-race-probe".into()).spawn(move || {
        crate::record_raw("tm_race", "start", &format!("base={base:#x} period={period}ms offsets={source}"));
        let t0 = std::time::Instant::now();
        let mut cur: Option<(usize, usize)> = None;
        let mut last_scan: Option<std::time::Instant> = None;
        let mut lines = 0u32;
        let mut last_found = 0usize;
        while (t0.elapsed().as_millis() as u64) < RUN_MS && lines < MAX_LINES {
            std::thread::sleep(std::time::Duration::from_millis(period));
            let valid = cur.map(|(sp, p)| ptr_at(sp).map(|v| v == base + l.vtable).unwrap_or(false) && ptr_at(sp + l.sp_player) == Some(p)).unwrap_or(false);
            if !valid {
                cur = None;
                if last_scan.map(|t| t.elapsed().as_millis() as u64 >= RESCAN_MS).unwrap_or(true) {
                    let st = std::time::Instant::now();
                    let found = find_script_players(base + l.vtable, MAX_CANDIDATES);
                    last_scan = Some(std::time::Instant::now());
                    cur = pick(&l, &found);
                    if found.len() != last_found || cur.is_some() {
                        crate::record_raw("tm_race", "scan", &format!("t={}ms candidates={} picked={} scan={}ms", t0.elapsed().as_millis(), found.len(), cur.map(|(sp, p)| format!("{sp:#x}/{p:#x}")).unwrap_or_else(|| "none".into()), st.elapsed().as_millis()));
                        lines += 1;
                        last_found = found.len();
                    }
                }
            }
            if let Some((sp, p)) = cur {
                let s = sample(&l, base, p);
                crate::record_raw("tm_race", "s", &line(&l, t0.elapsed().as_millis() as u64, sp, p, &s));
                lines += 1;
            }
        }
        crate::record_raw("tm_race", "stop", &format!("lines={lines}"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic() -> Layout {
        Layout { vtable: 0, game_time: 0x40, sp_player: 0x8, race_state: 0x100, rs_cur: 0x4, rs_running: 0x8, race: 0x200, lap_waypoints: 0x208, lap: 0x210, race_start: 0x8, waypoint_count: 0x18, entity: 0x300, flags: 0x304, offzone_bit: 2, position: 0x310, vehicle_state: 0x320, surface: 0x10, water: 6 }
    }

    #[test]
    fn the_layout_comes_from_the_offsets_file() {
        let vals: Vec<String> = KEYS.iter().enumerate().map(|(i, k)| format!("\"{k}\":{}", i + 1)).collect();
        let o = crate::title_offsets::parse(&format!("{{\"exe\":\"T.exe\",\"values\":{{{}}}}}", vals.join(",")), "t").unwrap();
        let l = layout(&o).unwrap();
        assert_eq!((l.vtable, l.game_time, l.water), (1, 2, KEYS.len()));
        let partial = crate::title_offsets::parse(r#"{"exe":"T.exe","values":{"race.script_player_vtable":1}}"#, "t").unwrap();
        assert!(layout(&partial).unwrap_err().contains("race.surface_water"));
    }

    #[test]
    fn sample_decodes_a_synthetic_player() {
        let l = synthetic();
        let mut game = vec![0u8; 0x10];
        let mut player = vec![0u8; 0x400];
        let mut race = vec![0u8; 0x20];
        let mut lapw = vec![0u8; 0x20];
        let mut veh = vec![0u8; 0x20];
        let fake_base = (game.as_mut_ptr() as usize).wrapping_sub(l.game_time);
        game[0..4].copy_from_slice(&5000u32.to_le_bytes());
        race[8..12].copy_from_slice(&1500u32.to_le_bytes());
        race[0x18..0x1c].copy_from_slice(&3u32.to_le_bytes());
        lapw[0x18..0x1c].copy_from_slice(&2u32.to_le_bytes());
        veh[0x10..0x14].copy_from_slice(&6u32.to_le_bytes());
        let put = |b: &mut Vec<u8>, off: usize, v: &[u8]| b[off..off + v.len()].copy_from_slice(v);
        put(&mut player, l.race_state + l.rs_cur, &7i32.to_le_bytes());
        put(&mut player, l.race_state + l.rs_running, &7i32.to_le_bytes());
        put(&mut player, l.race, &(race.as_ptr() as u64).to_le_bytes());
        put(&mut player, l.lap_waypoints, &(lapw.as_ptr() as u64).to_le_bytes());
        put(&mut player, l.lap, &1u32.to_le_bytes());
        put(&mut player, l.entity, &0i32.to_le_bytes());
        put(&mut player, l.flags, &4u32.to_le_bytes());
        put(&mut player, l.position, &1.5f32.to_le_bytes());
        put(&mut player, l.position + 4, &(-2.0f32).to_le_bytes());
        put(&mut player, l.position + 8, &300.25f32.to_le_bytes());
        put(&mut player, l.vehicle_state, &(veh.as_ptr() as u64).to_le_bytes());
        let s = sample(&l, fake_base, player.as_ptr() as usize);
        assert_eq!(s, Sample { racing: true, race_ms: Some(3500), checkpoints: Some(3), lap_checkpoints: Some(2), lap: Some(1), pos: Some([1.5, -2.0, 300.25]), surface: Some(6), offzone: Some(true) });
        let t = line(&l, 10, 1, 2, &s);
        assert!(t.contains("race_ms=3500 cp=3 lap_cp=2 lap=1 pos=1.5,-2.0,300.2") && t.contains("water=1 offzone=1"), "{t}");
        put(&mut player, l.race_state + l.rs_running, &(-1i32).to_le_bytes());
        put(&mut player, l.entity, &(-1i32).to_le_bytes());
        let s = sample(&l, fake_base, player.as_ptr() as usize);
        assert!(!s.racing && s.race_ms.is_none() && s.pos.is_none());
        drop((game, race, lapw, veh));
    }

    #[test]
    fn the_scan_finds_objects_by_their_vtable() {
        let tag = 0x1357_9bdf_2468_ace0usize;
        let mut v = vec![0usize; 4096];
        v[100] = tag;
        v[3000] = tag;
        let got = find_script_players(tag, 1 << 16);
        let a = v.as_ptr() as usize;
        assert!(got.contains(&(a + 800)) && got.contains(&(a + 24000)), "{got:x?}");
        drop(v);
    }
}
