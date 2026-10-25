use super::*;

fn utc_ms(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}

#[test]
fn split_interval_crosses_hour_and_day_boundaries() {
    let start = utc_ms("2026-10-25T23:59:30Z");
    let end = utc_ms("2026-10-26T00:00:30Z");
    let slices = split_interval(start, end, |_| 8 * 3600);
    assert_eq!(slices.len(), 2);
    assert_eq!(slices[0].date, "2026-10-26");
    assert_eq!(slices[0].hour, 7);
    assert_eq!(slices[0].millis, 30_000);
    assert_eq!(slices[1].date, "2026-10-26");
    assert_eq!(slices[1].hour, 8);
    assert_eq!(slices[1].millis, 30_000);
}

#[test]
fn split_interval_handles_dst_like_offset_changes_without_losing_time() {
    let start = utc_ms("2026-11-01T05:30:00Z");
    let end = utc_ms("2026-11-01T07:30:00Z");
    // 模拟回拨：第一段 +8，跨过边界后 +7，重复本地小时会合并统计。
    let slices = split_interval(start, end, |at| {
        if at >= utc_ms("2026-11-01T06:00:00Z") {
            7 * 3600
        } else {
            8 * 3600
        }
    });
    assert_eq!(
        slices.iter().map(|slice| slice.millis).sum::<u64>(),
        2 * 3_600_000
    );
    assert_eq!(slices[0].hour, 13);
    assert_eq!(slices[1].hour, 13);
    assert_eq!(slices[2].hour, 14);
}

#[test]
fn split_interval_ignores_reversed_ranges() {
    assert!(split_interval(100, 100, |_| 0).is_empty());
    assert!(split_interval(200, 100, |_| 0).is_empty());
}
