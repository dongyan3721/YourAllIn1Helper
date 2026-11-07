//! 使用测试程序自身作为跨平台子进程，验证版本探测必须检查退出状态。
use super::*;

#[test]
fn successful_version_probe_returns_output() {
    let executable = std::env::current_exe().unwrap();
    let version = EnvProbeService::verify_binary_version(&executable, &["--help"]);
    assert!(version.is_some_and(|text| !text.is_empty()));
}

#[test]
fn failed_version_probe_is_not_reported_as_available() {
    let executable = std::env::current_exe().unwrap();
    assert!(EnvProbeService::verify_binary_version(
        &executable,
        &["--invalid-runtime-probe-option"]
    )
    .is_none());
}
