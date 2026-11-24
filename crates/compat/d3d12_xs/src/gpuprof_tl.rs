use core::ffi::c_void;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct SubT {
    pub ty: u32,
    pub queue: usize,
    pub head: u64,
    pub tail: u64,
    pub lists: Vec<(u64, u64)>,
    pub n_lists: u32,
    pub missing: u32,
    pub cpu: [i64; 4],
    pub head_qpc: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Calib {
    pub gpu: u64,
    pub cpu: i64,
}

pub fn gpu_to_qpc(pts: &[Calib], gfreq: u64, qpf: i64, g: u64) -> Option<i64> {
    let a = *pts.first()?;
    let (num, den) = match pts.last() {
        Some(b) if b.gpu > a.gpu + gfreq / 100 && b.cpu > a.cpu => ((b.cpu - a.cpu) as f64, (b.gpu - a.gpu) as f64),
        _ if gfreq != 0 => (qpf as f64, gfreq as f64),
        _ => return None,
    };
    Some(a.cpu + ((g as f64 - a.gpu as f64) * num / den).round() as i64)
}

pub unsafe fn calibrate(queue: *mut c_void) -> Option<Calib> {
    let f = crate::present::real_queue_slot(17);
    if f == 0 || queue.is_null() {
        return None;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64) -> i32 = unsafe { core::mem::transmute(f) };
    let (mut g, mut c) = (0u64, 0u64);
    if unsafe { f(queue, &mut g, &mut c) } < 0 || g == 0 || c == 0 {
        return None;
    }
    Some(Calib { gpu: g, cpu: c as i64 })
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[derive(Default)]
struct Acc {
    subs: u64,
    lists: u64,
    max_lists: u32,
    missing: u64,
    in_lists: u64,
    pre: u64,
    between: u64,
    post: u64,
    idle: u64,
    starved: u64,
    late: u64,
    unknown: u64,
    lat: Vec<f64>,
    hook_pre: i64,
    hook_call: i64,
    hook_after: i64,
    title_gap: i64,
    worst: Vec<(u64, String)>,
}

pub fn analyse(subs: &mut [SubT], tick_ms: f64, qpf: i64, nf: u64) -> String {
    let nf = nf.max(1) as f64;
    let qms = |t: i64| t as f64 * 1000.0 / qpf.max(1) as f64;
    subs.sort_by_key(|s| (s.queue, s.head));
    let mut by_ty: Vec<(u32, Acc)> = Vec::new();
    let mut prev: HashMap<usize, (u64, i64)> = HashMap::new();
    for (i, s) in subs.iter().enumerate() {
        let k = match by_ty.iter().position(|e| e.0 == s.ty) {
            Some(k) => k,
            None => {
                by_ty.push((s.ty, Acc::default()));
                by_ty.len() - 1
            }
        };
        let a = &mut by_ty[k].1;
        a.subs += 1;
        a.lists += s.n_lists as u64;
        a.max_lists = a.max_lists.max(s.n_lists);
        a.missing += s.missing as u64;
        let mut l: Vec<(u64, u64)> = s.lists.iter().copied().filter(|x| x.1 >= x.0 && x.0 >= s.head && x.1 <= s.tail).collect();
        l.sort_unstable();
        let (mut pre, mut between, mut post, mut inl) = (0u64, 0u64, 0u64, 0u64);
        if let (Some(f), Some(z)) = (l.first(), l.last()) {
            pre = f.0 - s.head;
            let mut end = f.1;
            for x in &l {
                inl += x.1 - x.0;
                if x.0 > end {
                    between += x.0 - end;
                }
                end = end.max(x.1);
            }
            post = s.tail.saturating_sub(end.max(z.1));
        }
        a.pre += pre;
        a.between += between;
        a.post += post;
        a.in_lists += inl;
        if s.cpu[1] > s.cpu[0] { a.hook_pre += s.cpu[1] - s.cpu[0]; }
        if s.cpu[2] > s.cpu[1] { a.hook_call += s.cpu[2] - s.cpu[1]; }
        if s.cpu[3] > s.cpu[2] { a.hook_after += s.cpu[3] - s.cpu[2]; }
        let lat = s.head_qpc.map(|h| qms(h - s.cpu[1]));
        if let Some(v) = lat {
            a.lat.push(v);
        }
        let mut idle_ms = 0.0;
        if let Some(&(ptail, pexit)) = prev.get(&s.queue) {
            if s.head > ptail {
                let idle = s.head - ptail;
                a.idle += idle;
                idle_ms = idle as f64 * tick_ms;
                match lat {
                    Some(v) if v < 1.0 => a.starved += idle,
                    Some(_) => a.late += idle,
                    None => a.unknown += idle,
                }
            }
            if pexit > 0 && s.cpu[0] > pexit {
                a.title_gap += s.cpu[0] - pexit;
            }
        }
        prev.insert(s.queue, (s.tail, if s.cpu[3] > 0 { s.cpu[3] } else { s.cpu[2] }));
        let bound = pre + between + post;
        a.worst.push((bound, format!("sub {i} lists={} pre {:.2} between {:.2} post {:.2} in-lists {:.2} idle-before {:.2} ms, submit-to-start {} | cpu pre-submit {:.2} call {:.2} ms",
            s.n_lists, pre as f64 * tick_ms, between as f64 * tick_ms, post as f64 * tick_ms, inl as f64 * tick_ms, idle_ms,
            lat.map_or("?".to_string(), |v| format!("{v:.2} ms")), qms(s.cpu[1] - s.cpu[0]).max(0.0), qms(s.cpu[2] - s.cpu[1]).max(0.0))));
        a.worst.sort_by(|x, y| y.0.cmp(&x.0));
        a.worst.truncate(3);
    }
    if by_ty.is_empty() {
        return "timeline: no title submissions recorded".to_string();
    }
    by_ty.sort_by_key(|e| e.0);
    let mut out = String::new();
    for (ty, mut a) in by_ty {
        let per = |t: u64| t as f64 * tick_ms / nf;
        let lat50 = pct(&mut a.lat, 0.5);
        let lat90 = pct(&mut a.lat, 0.9);
        out.push_str(&format!(
            "timeline {}: {:.1} submissions/f, {:.1} lists each (max {}, {} lists without stamps; a list starts at its first stamped command) | GPU inside title lists {:.3} ms/f, before first list {:.3}, between lists {:.3}, after last list {:.3} | GPU idle between submissions {:.3} ms/f (GPU began <1 ms after CPU submit {:.3}, began later {:.3}, uncalibrated {:.3}) | submit-to-start median {:.2} ms p90 {:.2} ms over {} | CPU hook pre-submit {:.3} ms/f, submit call {:.3} ms/f, after {:.3} ms/f, title gap between submits {:.3} ms/f\n",
            crate::gpuprof_q::type_name(ty), a.subs as f64 / nf, a.lists as f64 / a.subs.max(1) as f64, a.max_lists, a.missing,
            per(a.in_lists), per(a.pre), per(a.between), per(a.post),
            per(a.idle), per(a.starved), per(a.late), per(a.unknown),
            lat50, lat90, a.lat.len(),
            qms(a.hook_pre) / nf, qms(a.hook_call) / nf, qms(a.hook_after) / nf, qms(a.title_gap) / nf));
        for (_, w) in &a.worst {
            out.push_str(&format!("  worst boundary: {w}\n"));
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyse_splits_boundaries_idle_and_latency() {
        let mut subs = vec![
            SubT { ty: 0, queue: 1, head: 1000, tail: 5000, lists: vec![(1500, 2000), (3000, 4000)], n_lists: 2, missing: 0, cpu: [0, 100, 200, 300], head_qpc: Some(100) },
            SubT { ty: 0, queue: 1, head: 9000, tail: 10000, lists: vec![(9000, 10000)], n_lists: 1, missing: 1, cpu: [5000, 5100, 5200, 5300], head_qpc: Some(5100 + 5000) },
        ];
        let s = analyse(&mut subs, 0.001, 1_000_000, 1);
        assert!(s.starts_with("timeline direct: 2.0 submissions/f, 1.5 lists each (max 2, 1 lists without stamps; a list starts at its first stamped command)"), "{s}");
        assert!(s.contains("GPU inside title lists 2.500 ms/f, before first list 0.500, between lists 1.000, after last list 1.000"), "{s}");
        assert!(s.contains("GPU idle between submissions 4.000 ms/f (GPU began <1 ms after CPU submit 0.000, began later 4.000, uncalibrated 0.000)"), "{s}");
        assert!(s.contains("CPU hook pre-submit 0.200 ms/f, submit call 0.200 ms/f, after 0.200 ms/f, title gap between submits 4.700 ms/f"), "{s}");
        assert!(s.contains("worst boundary: sub 0 lists=2 pre 0.50 between 1.00 post 1.00"), "{s}");
    }

    #[test]
    fn calibration_maps_gpu_ticks_with_the_two_point_slope() {
        let pts = [Calib { gpu: 1_000_000, cpu: 50_000 }, Calib { gpu: 3_000_000, cpu: 70_000 }];
        assert_eq!(gpu_to_qpc(&pts, 100_000_000, 10_000_000, 1_500_000), Some(55_000));
        assert_eq!(gpu_to_qpc(&pts[..1], 1_000_000, 10_000_000, 1_100_000), Some(1_050_000));
        assert_eq!(gpu_to_qpc(&[], 1, 1, 1), None);
        assert_eq!(analyse(&mut [], 1.0, 1, 1), "timeline: no title submissions recorded");
    }
}
