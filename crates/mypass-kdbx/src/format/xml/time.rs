use super::decode::corrupt;
use crate::{Result, Timestamp};
use base64::{prelude::BASE64_STANDARD, Engine};
pub(super) fn parse(s: &str) -> Result<Timestamp> {
    if s.len() == 20 && s.ends_with('Z') && s.as_bytes().get(4) == Some(&b'-') {
        return iso(s).ok_or_else(|| corrupt("ISO-8601 time"));
    }
    let bytes = BASE64_STANDARD
        .decode(s)
        .map_err(|_| corrupt("time base64"))?;
    let raw: i64 = i64::from_le_bytes(
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| corrupt("time length"))?,
    );
    Timestamp::from_kdbx_seconds(raw).ok_or_else(|| corrupt("time overflow"))
}
pub(crate) fn iso(s: &str) -> Option<Timestamp> {
    if s.len() != 20
        || s.get(4..5)? != "-"
        || s.get(7..8)? != "-"
        || s.get(10..11)? != "T"
        || s.get(13..14)? != ":"
        || s.get(16..17)? != ":"
        || s.get(19..20)? != "Z"
    {
        return None;
    }
    let number = |a, b| -> Option<i64> {
        let part = s.get(a..b)?;
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (mut y, m, d, h, min, sec) = (
        number(0, 4)?,
        number(5, 7)?,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
    );
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if y == 0 || d < 1 || d > days || h >= 24 || min >= 60 || sec > 60 {
        return None;
    }
    // Gregorian civil-date conversion, with the year bounded to four ASCII digits above.
    y -= i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let day = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d - 1 - 719468;
    Some(Timestamp(day * 86400 + h * 3600 + min * 60 + sec))
}
/// The inverse of [`iso`]: `YYYY-MM-DDTHH:MM:SSZ`, or `None` outside years 1 to 9999 (the only
/// years `iso` reads back).
pub(crate) fn iso_string(t: Timestamp) -> Option<String> {
    let days = t.0.div_euclid(86400);
    let secs = t.0.rem_euclid(86400);
    // Gregorian civil date from days since 1970-01-01 (the inverse of the conversion in `iso`).
    let z = days.checked_add(719468)?;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe.checked_add(era.checked_mul(400)?)? + i64::from(m <= 2);
    if !(1..=9999).contains(&y) {
        return None;
    }
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    ))
}
