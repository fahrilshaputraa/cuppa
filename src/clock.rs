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
