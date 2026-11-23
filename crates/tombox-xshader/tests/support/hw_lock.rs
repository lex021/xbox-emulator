pub const LOCK_DIR: &str = "D:/TomBox/tools/dev/batch_runs";
pub const LOCKS: [&str; 3] = ["GAME_LOCK", "GAME_LOCK_2", "USER_PLAYING"];

pub fn held_in(dir: &std::path::Path) -> Vec<String> {
    LOCKS.iter().filter(|n| dir.join(n).exists()).map(|n| n.to_string()).collect()
}

pub fn require_free_gpu() {
    let dir = std::env::var("TOMBOX_LOCK_DIR").unwrap_or_else(|_| LOCK_DIR.to_string());
    let held = held_in(std::path::Path::new(&dir));
    if !held.is_empty() {
        eprintln!("refusing to touch the GPU: {} held in {dir}", held.join(", "));
        std::process::exit(3);
    }
}
