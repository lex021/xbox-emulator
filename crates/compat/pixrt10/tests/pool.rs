use pixrt10::runtime::*;
use std::sync::atomic::Ordering;

#[test]
fn capture_active_event_buffers_cycle_through_the_pool() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pixrt10_pool_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    unsafe {
        let c = context();
        assert_eq!(c.state.load(Ordering::SeqCst), 1);
        assert_eq!(thread_event_buffer(0), 0);
        c.state.store(2, Ordering::SeqCst);
        assert_eq!(thread_event_buffer(0), 0);
        assert_eq!(event_buffer(), [0, 0, usize::MAX]);
        assert_eq!(create_capture_pool(0, 0x20_0000, 0x8000, 0x4000), 0);
        let ts = thread_event_buffer(0);
        assert_ne!(ts, 0);
        let [b, end, cur] = event_buffer();
        assert_ne!(b, 0);
        assert_eq!((end - b, cur - b), (EVENT_BUFFER_BYTES, EVENT_BUFFER_HEADER));
        let p = b as *const u8;
        assert_eq!(*p, 0);
        assert_eq!((p.add(4) as *const u32).read_unaligned(), current_tid());
        assert_eq!((p.add(8) as *const u32).read_unaligned(), 0);
        assert_eq!((p.add(0x10) as *const u64).read_unaligned(), ts);
        assert_eq!((p.add(0x18) as *const u64).read_unaligned(), u64::MAX);
        assert_eq!((cur as *const u64).read_unaligned(), EVENT_BUFFER_FIRST);
        set_event_buffer_cursor(cur + 0x45);
        let ts2 = thread_event_buffer(1);
        assert!(ts2 >= ts);
        assert_eq!((p.add(8) as *const u32).read_unaligned(), 0x40);
        assert_eq!((p.add(0x18) as *const u64).read_unaligned(), ts2);
        let [b2, _, _] = event_buffer();
        assert_ne!(b2, b);
        assert_eq!((b2 as *const u64).add(2).read_unaligned(), ts2);
        {
            let g = c.pool.lock().unwrap();
            let pool = g.as_ref().unwrap();
            assert_eq!((pool.in_use(), pool.full_len(), pool.free_len()), (1, 1, 0));
        }
        let b3 = {
            thread_event_buffer(0);
            event_buffer()[0]
        };
        let pool = c.pool.lock().unwrap();
        let pool = pool.as_ref().unwrap();
        assert!(b3 >= b + 0x8000);
        assert_eq!((pool.in_use(), pool.full_len()), (1, 2));
        assert_eq!(pool.free_len(), (POOL_GROW / 0x4000) - 2);
        assert_eq!(pool.take_full(), Some(b));
        pool.give_back(b);
    }
}

#[test]
fn pool_recycles_the_oldest_full_block_and_signals_when_half_full() {
    unsafe {
        let p = BlockPool::new(0, 0x4000 * 4, 0x4000 * 4, 0x4000).unwrap();
        let a: Vec<usize> = (0..4).map(|_| p.acquire()).collect();
        assert!(a.iter().all(|&x| x != 0));
        assert_eq!(p.in_use(), 4);
        assert_eq!(p.acquire(), 0);
        p.release(a[0]);
        p.release(a[0]);
        assert_eq!((p.in_use(), p.full_len()), (3, 1));
        p.release(a[1]);
        assert_eq!(WaitForSingleObject(p.full_event(), 0), 0x102);
        p.release(a[2]);
        assert_eq!(WaitForSingleObject(p.full_event(), 0), 0);
        assert_eq!(p.acquire(), a[0]);
        assert_eq!((p.in_use(), p.full_len()), (2, 2));
        let q = BlockPool::new(0, POOL_GROW * 2, 0, 0x4000).unwrap();
        let first = q.acquire();
        assert_ne!(first, 0);
        assert_eq!(q.free_len(), POOL_GROW / 0x4000 - 2);
        assert!(BlockPool::new(0, 0x1000, 0x2000, 0x1000).is_none());
    }
}

fn current_tid() -> u32 {
    unsafe { GetCurrentThreadId() }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
}
