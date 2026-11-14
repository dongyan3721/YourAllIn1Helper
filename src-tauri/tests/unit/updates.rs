use super::*;
#[test]
fn installed_path_match_rejects_portable_and_sibling_copies() {
    assert!(same_install_directory(
        Path::new(r"C:\Users\QA\AppData\Local\Programs\All-in-One Toolbox\all-in-one-desktop.exe"),
        Path::new(r"c:\users\qa\appdata\local\programs\All-in-One Toolbox\")
    ));
    assert!(!same_install_directory(
        Path::new(r"D:\Downloads\all-in-one-desktop.exe"),
        Path::new(r"C:\Users\QA\AppData\Local\Programs\All-in-One Toolbox")
    ));
    assert!(!same_install_directory(
        Path::new(r"C:\Apps\All-in-One Toolbox-old\all-in-one-desktop.exe"),
        Path::new(r"C:\Apps\All-in-One Toolbox")
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_updates_only_apply_to_standard_installations() {
    let home = Some(Path::new("/Users/qa"));
    for root in ["/Applications", "/Users/qa/Applications"] {
        assert!(is_macos_app_bundle_exe(
            &Path::new(root).join("All-in-One Toolbox.app/Contents/MacOS/all-in-one-desktop"),
            home
        ));
    }
    for root in [
        "/Volumes/Toolbox",
        "/Users/qa/Downloads",
        "/work/dist",
        "/work/target/release/bundle/macos",
        "/Applications/nested",
    ] {
        assert!(!is_macos_app_bundle_exe(
            &Path::new(root).join("All-in-One Toolbox.app/Contents/MacOS/all-in-one-desktop"),
            home
        ));
    }
    assert!(!is_macos_app_bundle_exe(
        Path::new("/Applications/All-in-One Toolbox.app/Contents/MacOS/other"),
        home
    ));
    assert!(is_macos_app_bundle_exe(
        Path::new("/Applications/All-in-One Toolbox.app/Contents/MacOS/all-in-one-desktop"),
        None
    ));
}
