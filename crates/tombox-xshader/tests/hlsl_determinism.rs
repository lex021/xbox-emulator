use tombox_xshader::{extract, hlsl, runtime_hlsl};

#[test]
#[ignore]
fn dumped_shaders_translate_to_identical_hlsl_twice() {
    let Ok(dir) = std::env::var("TB_DET_DIR") else { return };
    let limit: usize = std::env::var("TB_DET_N").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "xbox")).collect();
    files.sort();
    let (mut same, mut differ, mut fail) = (0usize, Vec::new(), 0usize);
    for p in files.iter().take(limit) {
        let blob = std::fs::read(p).unwrap();
        let once = || -> Option<String> {
            let ex = extract(&blob).ok()?;
            let t = hlsl::translate(&ex).ok()?;
            Some(runtime_hlsl::finalize(&t.hlsl, &[0; 8], true))
        };
        match (once(), once()) {
            (Some(a), Some(b)) if a == b => same += 1,
            (Some(a), Some(b)) => {
                let line = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(0);
                differ.push(format!("{} first differing line {}: {:?} vs {:?}", p.display(), line, a.lines().nth(line), b.lines().nth(line)));
            }
            _ => fail += 1,
        }
    }
    for d in differ.iter().take(5) {
        eprintln!("{d}");
    }
    eprintln!("same {same} differ {} fail {fail}", differ.len());
    assert!(differ.is_empty());
}
