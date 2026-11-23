// SPDX-License-Identifier: AGPL-3.0-or-later
use time::{OffsetDateTime, UtcOffset, format_description::FormatItem, macros::format_description};

const DAY: &[FormatItem<'static>] = format_description!("[year]-[month]-[day]");
const HM: &[FormatItem<'static>] = format_description!("[hour]:[minute]");

pub fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

fn local(ts: i64, tz: i8) -> OffsetDateTime {
    let off = UtcOffset::from_hms(tz, 0, 0).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::from_unix_timestamp(ts)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(off)
}

/// Business day (outlet-local) as `YYYY-MM-DD`.
pub fn day(ts: i64, tz: i8) -> String {
    local(ts, tz).format(DAY).unwrap_or_default()
}

pub fn hm(ts: i64, tz: i8) -> String {
    local(ts, tz).format(HM).unwrap_or_default()
}

/// Parse local `YYYY-MM-DD` + `HH:MM` into a UTC unix timestamp for an outlet offset.
pub fn local_to_ts(day: &str, hm: &str, tz: i8) -> Option<i64> {
    let d: Vec<&str> = day.split('-').collect();
    let t: Vec<&str> = hm.split(':').collect();
    if d.len() != 3 || t.len() != 2 {
        return None;
    }
    let date = time::Date::from_calendar_date(
        d[0].parse().ok()?,
        time::Month::try_from(d[1].parse::<u8>().ok()?).ok()?,
        d[2].parse().ok()?,
    )
    .ok()?;
    let tm = time::Time::from_hms(t[0].parse().ok()?, t[1].parse().ok()?, 0).ok()?;
    let off = UtcOffset::from_hms(tz, 0, 0).ok()?;
    Some(
        time::PrimitiveDateTime::new(date, tm)
            .assume_offset(off)
            .unix_timestamp(),
    )
}
