//! Explicit release QA: exercise the real updater against the finished signed artifact.
//! On macOS set ALL_IN_ONE_RELEASE_MACOS=<absolute .app.tar.gz path> with the same command below.
//! ALL_IN_ONE_RELEASE_MSI=<absolute MSI path> cargo test -p all-in-one-host --test updater_artifact -- --ignored
#![cfg(any(windows, target_os = "macos"))]

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    time::Duration,
};
use tauri_plugin_updater::UpdaterExt;

#[test]
#[ignore = "requires a signed release artifact and .sig; see file documentation"]
fn signed_release_download_accepts_original_and_rejects_tampering() {
    let variable = if cfg!(target_os = "macos") {
        "ALL_IN_ONE_RELEASE_MACOS"
    } else {
        "ALL_IN_ONE_RELEASE_MSI"
    };
    let path = PathBuf::from(std::env::var_os(variable).expect(variable));
    let artifact = std::fs::read(&path).unwrap();
    let signature = std::fs::read_to_string(format!("{}.sig", path.display())).unwrap();
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let platform_config = if cfg!(target_os = "macos") {
        include_str!("../tauri.macos.conf.json")
    } else {
        include_str!("../tauri.windows.conf.json")
    };
    let platform: serde_json::Value = serde_json::from_str(platform_config).unwrap();
    let public_key = platform["plugins"]["updater"]["pubkey"].as_str().unwrap();
    let target = if cfg!(target_os = "macos") {
        format!("darwin-{}", std::env::consts::ARCH)
    } else {
        "windows-x86_64".into()
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let manifest = serde_json::json!({
        "version": config["version"],
        "platforms": { (target.clone()): {
            "url": format!("http://{address}/installer.msi"),
            "signature": signature.trim()
        }}
    })
    .to_string();
    let served_artifact = artifact.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            let mut request = [0u8; 8192];
            let count = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            let body = if request.starts_with("GET /installer.msi ") {
                served_artifact.as_slice()
            } else {
                assert!(request.starts_with("GET /feed "));
                manifest.as_bytes()
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        }
    });
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), platform["plugins"]["updater"].clone());
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let updater = app
        .updater_builder()
        .pubkey(public_key)
        .endpoints(vec![format!("http://{address}/feed").parse().unwrap()])
        .unwrap()
        .target(target)
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .version_comparator(|_, _| true)
        .restart_after_install(false)
        .build()
        .unwrap();
    tauri::async_runtime::block_on(async {
        let mut update = updater.check().await.unwrap().expect("fixture update");
        let downloaded = update.download(|_, _| {}, || {}).await.unwrap();
        assert_eq!(downloaded, artifact);
        update.verify(&downloaded).unwrap();
        #[cfg(target_os = "macos")]
        {
            let signed_version = update.version.clone();
            update.version = "99.0.0".into();
            assert!(
                update.verify(&downloaded).is_err(),
                "manifest version must match the signed version"
            );
            update.version = signed_version;
        }
        let valid_signature = update.signature.clone();
        update.signature.clear();
        assert!(
            update.verify(&downloaded).is_err(),
            "empty signatures must be rejected"
        );
        update.signature = valid_signature;
        let mut tampered = downloaded;
        tampered[1024] ^= 1;
        assert!(
            update.verify(&tampered).is_err(),
            "modified release artifact must be rejected"
        );
    });
    server.join().unwrap();
}
