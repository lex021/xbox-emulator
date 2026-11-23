use tombox_xshader::dxc;

const PS: &str = "float4 main(float4 p : SV_Position) : SV_Target { return p * 0.5; }";

#[test]
fn compiles_and_disassembles_through_the_out_of_process_host() {
    if !std::path::Path::new(r"C:\Windows\System32\d3d12.dll").exists() {
        return;
    }
    std::env::set_var("TOMBOX_DXC_HOST", env!("CARGO_BIN_EXE_tombox_dxc_host"));
    assert!(dxc::host::exe().is_some());
    let code = dxc::compile(PS, "ps_6_2").expect("compile through host");
    assert_eq!(&code[..4], b"DXBC");
    let text = dxc::disassemble(&code).expect("disassemble through host");
    assert!(text.contains("define void @main()"));
    let bad = dxc::compile("float4 main() : SV_Target { return undefined_thing; }", "ps_6_2");
    assert!(bad.is_err());
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| assert_eq!(dxc::compile(PS, "ps_6_2").unwrap(), code));
        }
    });
    let (spawned, failed) = dxc::host::stats();
    assert!(spawned >= 1 && spawned <= 5, "spawned {spawned}");
    assert_eq!(failed, 0);
}
