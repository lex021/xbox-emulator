use tombox_xshader::dxc;

#[test]
fn a_host_that_dies_fails_the_compile_instead_of_the_caller() {
    let fake = r"C:\Windows\System32\whoami.exe";
    if !std::path::Path::new(fake).exists() {
        return;
    }
    std::env::set_var("TOMBOX_DXC_HOST", fake);
    let r = dxc::compile("float4 main() : SV_Target { return 1; }", "ps_6_2");
    let e = r.expect_err("a dead host cannot produce DXIL");
    assert!(e.contains("compile host failed twice"), "{e}");
    let (spawned, failed) = dxc::host::stats();
    assert_eq!((spawned, failed), (2, 2));
}
