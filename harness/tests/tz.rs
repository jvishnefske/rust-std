//! The runtime's local time (`linux_none_rt::tz`, behind `localtime_r`):
//! POSIX TZ rules and TZif files.

use linux_none_rt::tz::{Day, Local, civil_from_days, days_from_civil, posix, rule_day, tzif};

fn abbr(l: &Local) -> &str {
    let end = l.abbr.iter().position(|&b| b == 0).unwrap_or(8);
    core::str::from_utf8(&l.abbr[..end]).unwrap()
}

#[test]
fn civil_round_trips() {
    for z in [-800_000, -1, 0, 1, 11_016, 19_675, 2_932_896] {
        let (y, m, d) = civil_from_days(z);
        assert_eq!(days_from_civil(y, m, d), z);
    }
    assert_eq!(civil_from_days(19_675), (2023, 11, 14));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
}

#[test]
fn posix_rules() {
    let us = b"EST5EDT,M3.2.0,M11.1.0";
    // 2023-11-14 22:13:20 UTC, after the end
    let l = posix(us, 1_700_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-18000, false, "EST"));
    // 2023-07-22 04:26:40 UTC
    let l = posix(us, 1_690_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-14400, true, "EDT"));
    // 2023-03-12 06:59:59 and 07:00:00 UTC: the start, 2:00 EST
    assert!(!posix(us, 1_678_604_399).unwrap().isdst);
    assert!(posix(us, 1_678_604_400).unwrap().isdst);
    // 2023-11-05 05:59:59 and 06:00:00 UTC: the end, 2:00 EDT
    assert!(posix(us, 1_699_163_999).unwrap().isdst);
    assert!(!posix(us, 1_699_164_000).unwrap().isdst);
    // the defaults are the US rules
    assert_eq!(posix(b"EST5EDT", 1_690_000_000), posix(us, 1_690_000_000));

    // southern: daylight time spans the new year
    let au = b"AEST-10AEDT,M10.1.0,M4.1.0/3";
    let l = posix(au, 1_700_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (39600, true, "AEDT"));
    assert!(!posix(au, 1_690_000_000).unwrap().isdst);

    let l = posix(b"<+0530>-5:30", 0).unwrap();
    assert_eq!((l.offset, abbr(&l)), (19800, "+0530"));
    assert_eq!(posix(b"UTC0", 5), Some(Local::UTC));
    assert_eq!(posix(b"J", 0), None);
}

#[test]
fn julian_days() {
    // J60 is March 1 even in leap years; 59 is March 1 only in others
    assert_eq!(rule_day(2024, Day::Julian(60)), days_from_civil(2024, 3, 1));
    assert_eq!(rule_day(2024, Day::Zero(59)), days_from_civil(2024, 2, 29));
    assert_eq!(rule_day(2023, Day::Zero(59)), days_from_civil(2023, 3, 1));
    // the last Sunday of October 2023 was the 29th
    assert_eq!(rule_day(2023, Day::Month(10, 5, 0)), days_from_civil(2023, 10, 29));
}

#[test]
fn tzif_files() {
    assert_eq!(tzif(b"", 0), None);
    let mut f = Vec::new();
    let header = |f: &mut Vec<u8>, times: u32, types: u32, chars: u32| {
        f.extend_from_slice(b"TZif2");
        f.extend_from_slice(&[0; 15]);
        for c in [0, 0, 0, times, types, chars] {
            f.extend_from_slice(&c.to_be_bytes());
        }
    };
    // v1: no data
    header(&mut f, 0, 0, 0);
    // v2: LMT until 1883-11-18 18:00 UTC, then CST; footer CST6CDT
    header(&mut f, 1, 2, 8);
    f.extend_from_slice(&(-2_717_647_200i64).to_be_bytes());
    f.push(1);
    f.extend_from_slice(&(-21036i32).to_be_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(&(-21600i32).to_be_bytes());
    f.extend_from_slice(&[0, 4]);
    f.extend_from_slice(b"LMT\0CST\0");
    f.extend_from_slice(b"\nCST6CDT,M3.2.0,M11.1.0\n");

    let l = tzif(&f, -3_000_000_000).unwrap();
    assert_eq!((l.offset, abbr(&l)), (-21036, "LMT"));
    let l = tzif(&f, 1_690_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-18000, true, "CDT"));
    let l = tzif(&f, 1_700_000_000).unwrap();
    assert_eq!((l.offset, abbr(&l)), (-21600, "CST"));
}

/// The system's file for Chicago, if it has one.
#[test]
fn tzif_chicago() {
    let Ok(f) = std::fs::read("/usr/share/zoneinfo/America/Chicago") else { return };
    // July 1960, in the file's transitions; 2023, in them or its footer
    let l = tzif(&f, -299_376_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-18000, true, "CDT"));
    let l = tzif(&f, 1_690_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-18000, true, "CDT"));
    let l = tzif(&f, 1_700_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-21600, false, "CST"));
    // 2100, past any file's transitions
    let l = tzif(&f, 4_118_000_000).unwrap();
    assert_eq!((l.offset, l.isdst, abbr(&l)), (-18000, true, "CDT"));
}
