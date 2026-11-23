use core::sync::atomic::{AtomicUsize, Ordering};

pub const CONSOLE_TIMESTAMP_HZ: u64 = 99_750_000;
pub const IOCTL_CLOCK_CALIBRATION: u32 = 0x7900_c213;
pub const CLOCK_CALIBRATION_REQUEST_BYTES: usize = 0x18;
pub const IOCTL_TIMESTAMP_PAIR: u32 = 0x7900_c1e0;
pub const TIMESTAMP_PAIR_BYTES: u32 = 0x10;

pub type ClockSource = unsafe fn() -> Option<u64>;

static SOURCE: AtomicUsize = AtomicUsize::new(0);

extern "system" {
    fn QueryPerformanceCounter(v: *mut i64) -> i32;
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
}

pub fn set_clock_source(f: ClockSource) {
    SOURCE.store(f as usize, Ordering::Release);
}

pub fn qpc() -> (u64, u64) {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    (c as u64, f.max(1) as u64)
}

pub fn ticks_from_qpc(qpc: u64, qpf: u64) -> u64 {
    (qpc as u128 * CONSOLE_TIMESTAMP_HZ as u128 / qpf.max(1) as u128) as u64
}

pub fn gpu_counter() -> u64 {
    let s = SOURCE.load(Ordering::Acquire);
    if s != 0 {
        let f: ClockSource = unsafe { core::mem::transmute(s) };
        if let Some(v) = unsafe { f() } {
            return v;
        }
    }
    let (c, f) = qpc();
    ticks_from_qpc(c, f)
}

fn store(addr: u64, v: u64) -> Result<(), u32> {
    if addr == 0 {
        return Ok(());
    }
    if addr & 7 != 0 || !crate::accessible(addr, 8, true) {
        return Err(crate::ERROR_NOACCESS);
    }
    unsafe { core::ptr::write_unaligned(addr as usize as *mut u64, v) };
    Ok(())
}

pub fn calibration_ioctl(input: &[u8]) -> Result<u32, u32> {
    if input.len() < CLOCK_CALIBRATION_REQUEST_BYTES {
        return Err(crate::ERROR_INVALID_PARAMETER);
    }
    let at = |o: usize| u64::from_le_bytes(input[o..o + 8].try_into().unwrap());
    let (p_tsc, p_gpu, p_qpc) = (at(0), at(8), at(16));
    let gpu = gpu_counter();
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    let (q, _) = qpc();
    store(p_tsc, tsc)?;
    store(p_gpu, gpu)?;
    store(p_qpc, q)?;
    Ok(0)
}

pub fn timestamp_pair_ioctl(output: &mut [u8]) -> Result<u32, u32> {
    if output.len() < TIMESTAMP_PAIR_BYTES as usize {
        return Err(crate::ERROR_INSUFFICIENT_BUFFER);
    }
    let gpu = gpu_counter();
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    output[0..8].copy_from_slice(&tsc.to_le_bytes());
    output[8..16].copy_from_slice(&gpu.to_le_bytes());
    Ok(TIMESTAMP_PAIR_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{close, device_io_control, open, test_serial, ERROR_NOACCESS};

    extern "system" {
        fn GetLastError() -> u32;
    }

    fn req(a: u64, b: u64, c: u64) -> [u8; 0x18] {
        let mut r = [0u8; 0x18];
        r[0..8].copy_from_slice(&a.to_le_bytes());
        r[8..16].copy_from_slice(&b.to_le_bytes());
        r[16..24].copy_from_slice(&c.to_le_bytes());
        r
    }

    #[test]
    fn qpc_fallback_counts_at_99_75_mhz_from_boot() {
        assert_eq!(ticks_from_qpc(10_000_000, 10_000_000), 99_750_000);
        assert_eq!(ticks_from_qpc(3 * 10_000_000 + 1, 10_000_000), 299_250_009);
        assert_eq!(ticks_from_qpc(86_400 * 30 * 10_000_000, 10_000_000), 86_400 * 30 * 99_750_000);
    }

    #[test]
    fn calibration_ioctl_writes_tsc_counter_and_qpc_through_the_device() {
        let _g = test_serial();
        let h = open();
        let (mut tsc, mut gpu, mut cpu) = (0u64, 0u64, 0u64);
        let (q0, f) = qpc();
        let mut r = req(&mut tsc as *mut u64 as u64, &mut gpu as *mut u64 as u64, &mut cpu as *mut u64 as u64);
        let mut ret = 7u32;
        assert!(unsafe { device_io_control(h, IOCTL_CLOCK_CALIBRATION, r.as_mut_ptr() as *const _, 0x18, core::ptr::null_mut(), 0, &mut ret) });
        let (q1, _) = qpc();
        assert_eq!(ret, 0);
        assert!(tsc != 0);
        assert!(cpu >= q0 && cpu <= q1);
        let lo = ticks_from_qpc(q0, f);
        let hi = ticks_from_qpc(q1, f);
        assert!(SOURCE.load(Ordering::Acquire) != 0 || (gpu >= lo && gpu <= hi), "gpu={gpu} window={lo}..{hi}");
        let mut only = 0u64;
        let mut r = req(0, &mut only as *mut u64 as u64, 0);
        assert!(unsafe { device_io_control(h, IOCTL_CLOCK_CALIBRATION, r.as_mut_ptr() as *const _, 0x18, core::ptr::null_mut(), 0, &mut ret) });
        assert!(only != 0);
        let mut bad = [0u64; 2];
        let mut first = 0u64;
        let mut r = req(0, &mut first as *mut u64 as u64, (bad.as_mut_ptr() as u64) + 4);
        assert!(!unsafe { device_io_control(h, IOCTL_CLOCK_CALIBRATION, r.as_mut_ptr() as *const _, 0x18, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, ERROR_NOACCESS);
        assert!(first != 0);
        assert_eq!(bad, [0, 0]);
        assert_eq!(calibration_ioctl(&[0u8; 0x10]), Err(crate::ERROR_INVALID_PARAMETER));
        assert!(close(h));
    }

    #[test]
    fn timestamp_pair_returns_tsc_then_counter_like_kernel() {
        let _g = test_serial();
        let h = open();
        let mut out = [0u64; 3];
        let mut ret = 0u32;
        let (q0, f) = qpc();
        let t0 = unsafe { core::arch::x86_64::_rdtsc() };
        assert!(unsafe { device_io_control(h, IOCTL_TIMESTAMP_PAIR, core::ptr::null(), 0, out.as_mut_ptr().cast(), 0x18, &mut ret) });
        let t1 = unsafe { core::arch::x86_64::_rdtsc() };
        let (q1, _) = qpc();
        assert_eq!(ret, 0x10);
        assert!(out[0] >= t0 && out[0] <= t1);
        assert!(SOURCE.load(Ordering::Acquire) != 0 || (out[1] >= ticks_from_qpc(q0, f) && out[1] <= ticks_from_qpc(q1, f)));
        assert_eq!(out[2], 0);
        let mut small = [0u8; 0xf];
        assert!(!unsafe { device_io_control(h, IOCTL_TIMESTAMP_PAIR, core::ptr::null(), 0, small.as_mut_ptr().cast(), 0xf, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, crate::ERROR_INSUFFICIENT_BUFFER);
        assert_eq!(ret, 0);
        assert!(close(h));
    }
}
