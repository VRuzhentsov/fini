use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

/// Compute the UTC fire time from quest due fields using the local wall-clock timezone.
/// If `due_time` is None, defaults to 09:00 local.
pub fn compute_fire_utc(due: &str, due_time: Option<&str>) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
    let time = match due_time {
        Some(t) => NaiveTime::parse_from_str(t, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(t, "%H:%M:%S"))
            .ok()?,
        None => NaiveTime::from_hms_opt(9, 0, 0)?,
    };
    let naive = NaiveDateTime::new(date, time);
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
}
