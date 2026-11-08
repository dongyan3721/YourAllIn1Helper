//! 裁剪成图、标注层叠加与防覆盖保存的单元测试（仅使用自制图像与临时目录）。
use super::*;
use image::Rgba;

fn gradient_frame(w: u32, h: u32) -> CapturedFrame {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[x as u8, y as u8, 100, 255]);
        }
    }
    CapturedFrame {
        width: w,
        height: h,
        scale_factor_milli: 2000,
        rgba_bytes: rgba,
    }
}

fn png_of(image: &RgbaImage) -> Vec<u8> {
    encode_png(image).unwrap()
}

#[test]
fn render_without_overlay_is_exact_crop() {
    let frame = gradient_frame(16, 8);
    let sel = SelectionBox {
        x: 4,
        y: 2,
        width: 3,
        height: 2,
    };
    let out = render_selection(&frame, &sel, None).unwrap();
    assert_eq!(out.dimensions(), (3, 2));
    assert_eq!(out.get_pixel(0, 0), &Rgba([4, 2, 100, 255]));
    assert_eq!(out.get_pixel(2, 1), &Rgba([6, 3, 100, 255]));
    // 空标注层等同无标注。
    let same = render_selection(&frame, &sel, Some(&[])).unwrap();
    assert_eq!(same, out);
}

#[test]
fn overlay_is_alpha_blended_and_transparent_pixels_keep_base() {
    let frame = gradient_frame(8, 8);
    let sel = SelectionBox {
        x: 0,
        y: 0,
        width: 2,
        height: 1,
    };
    let mut layer = RgbaImage::new(2, 1);
    layer.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
    let out = render_selection(&frame, &sel, Some(&png_of(&layer))).unwrap();
    assert_eq!(out.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
    assert_eq!(out.get_pixel(1, 0), &Rgba([1, 0, 100, 255]));
}

#[test]
fn overlay_with_wrong_size_or_garbage_is_rejected() {
    let frame = gradient_frame(8, 8);
    let sel = SelectionBox {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    let wrong = png_of(&RgbaImage::new(2, 2));
    assert!(render_selection(&frame, &sel, Some(&wrong)).is_err());
    assert!(render_selection(&frame, &sel, Some(b"not a png")).is_err());
}

#[test]
fn encoded_png_round_trips_pixels() {
    let frame = gradient_frame(5, 5);
    let sel = SelectionBox {
        x: 1,
        y: 1,
        width: 3,
        height: 3,
    };
    let image = render_selection(&frame, &sel, None).unwrap();
    let decoded = image::load_from_memory(&encode_png(&image).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(decoded, image);
}

#[test]
fn write_exclusive_never_overwrites_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("shot.png");
    std::fs::write(&existing, b"user file").unwrap();

    let first = write_exclusive(dir.path(), "shot", b"new-1").unwrap();
    let second = write_exclusive(dir.path(), "shot", b"new-2").unwrap();
    assert_eq!(first.file_name().unwrap(), "shot_1.png");
    assert_eq!(second.file_name().unwrap(), "shot_2.png");
    assert_eq!(std::fs::read(&existing).unwrap(), b"user file");
    assert_eq!(std::fs::read(&first).unwrap(), b"new-1");
    assert_eq!(std::fs::read(&second).unwrap(), b"new-2");
}

#[test]
fn write_exclusive_creates_missing_directory_and_reports_unwritable_target() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b");
    let path = write_exclusive(&nested, "x", b"ok").unwrap();
    assert!(path.starts_with(&nested));
    // 目标「目录」实际是文件时必须返回错误，而不是静默成功。
    let file_as_dir = dir.path().join("plain");
    std::fs::write(&file_as_dir, b"f").unwrap();
    assert!(write_exclusive(&file_as_dir, "x", b"ok").is_err());
}

#[test]
fn timestamped_stem_uses_sortable_local_time() {
    use chrono::TimeZone;
    let now = chrono::Local
        .with_ymd_and_hms(2026, 10, 25, 9, 8, 7)
        .single()
        .unwrap();
    assert_eq!(
        timestamped_stem("screenshot", now),
        "screenshot_20261025_090807"
    );
}

#[test]
fn resolve_save_path_appends_missing_extension_and_keeps_explicit_one() {
    // 缺省 / 空扩展名补全为目标格式。
    assert_eq!(
        resolve_save_path(Path::new("/tmp/截图 文件"), "png"),
        PathBuf::from("/tmp/截图 文件.png")
    );
    assert_eq!(
        resolve_save_path(Path::new("/tmp/anim."), "gif"),
        PathBuf::from("/tmp/anim.gif")
    );
    // 用户明确书写的扩展名（即使与目标格式不同）原样保留，与系统保存对话框一致。
    assert_eq!(
        resolve_save_path(Path::new("/tmp/截图 文件.PNG"), "gif"),
        PathBuf::from("/tmp/截图 文件.PNG")
    );
    assert_eq!(
        resolve_save_path(Path::new("C:\\媒体 文件\\一 二.jpg"), "png"),
        PathBuf::from("C:\\媒体 文件\\一 二.jpg")
    );
    // 无目录的纯文件名同样适用。
    assert_eq!(
        resolve_save_path(Path::new("shot"), "png"),
        PathBuf::from("shot.png")
    );
}

#[test]
fn write_exact_writes_and_overwrites_chinese_space_paths() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("媒体 文件 01.png");

    let first = write_exact(&target, b"first").unwrap();
    assert_eq!(first, target);
    assert_eq!(std::fs::read(&target).unwrap(), b"first");

    // 「另存为」路径的覆盖语义：用户已在保存对话框确认覆盖。
    let second = write_exact(&target, b"second-bytes").unwrap();
    assert_eq!(second, target);
    assert_eq!(std::fs::read(&target).unwrap(), b"second-bytes");
}

#[test]
fn write_exact_creates_missing_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("新建 文件夹").join("子目录").join("shot.png");
    let path = write_exact(&nested, b"ok").unwrap();
    assert_eq!(path, nested);
    assert_eq!(std::fs::read(&nested).unwrap(), b"ok");
}

#[test]
fn write_exact_failure_reports_error_without_half_written_files() {
    let dir = tempfile::tempdir().unwrap();
    // 父「目录」实际是文件：建目录失败、报错，既有文件与目录都不受影响。
    let file_as_parent = dir.path().join("plain");
    std::fs::write(&file_as_parent, b"f").unwrap();
    let err = write_exact(&file_as_parent.join("x.png"), b"data").unwrap_err();
    assert!(err.to_string().contains("无法创建"), "{err}");
    assert_eq!(std::fs::read(&file_as_parent).unwrap(), b"f");
    assert!(!dir.path().join("x.png").exists());

    // 目标本身是已存在目录：写入失败必须报错而不是静默成功，目录不被破坏。
    let target_dir = dir.path().join("目标目录.png");
    std::fs::create_dir(&target_dir).unwrap();
    let err = write_exact(&target_dir, b"data").unwrap_err();
    assert!(err.to_string().contains("失败"), "{err}");
    assert!(target_dir.is_dir());
}
