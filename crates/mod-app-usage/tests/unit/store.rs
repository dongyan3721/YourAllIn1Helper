use super::*;

#[test]
fn date_ranges_are_validated_and_filled() {
    let dates = enumerate_dates(
        NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
    )
    .unwrap();
    assert_eq!(dates, vec!["2026-10-31", "2026-11-01", "2026-11-02"]);
    assert!(enumerate_dates(
        NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
    )
    .is_err());
    assert!(parse_date("2026-13-01").is_err());
}
