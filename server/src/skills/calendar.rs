//! Bounded wall-clock recurrence calculations, without DST guesses or invalid-date clamping.
use super::schedule::{clock, number_value, offset, Schedule};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
use serde_json::Value;

pub(super) fn parse(args: &Value, timezone: &str) -> Result<Schedule> {
    let zone = timezone.to_owned();
    if let Some(value) = args.get("hourly") {
        return Ok(Schedule::Hourly {
            minute: integer(value, "minute", 0, 59)?,
            timezone: zone,
        });
    }
    for kind in ["daily", "workdays"] {
        if let Some(text) = args.get(kind).and_then(Value::as_str) {
            let (hour, minute) = clock(text)?;
            return Ok(if kind == "daily" {
                Schedule::Daily {
                    hour,
                    minute,
                    timezone: zone,
                }
            } else {
                Schedule::Workdays {
                    hour,
                    minute,
                    timezone: zone,
                }
            });
        }
    }
    for kind in ["weekly", "monthly", "yearly"] {
        if let Some(value) = args.get(kind) {
            let (hour, minute) = clock(
                value["time"]
                    .as_str()
                    .context("calendar time must be a string")?,
            )?;
            return Ok(match kind {
                "weekly" => Schedule::Weekly {
                    weekday: weekday(&value["weekday"])?,
                    hour,
                    minute,
                    timezone: zone,
                },
                "monthly" => Schedule::Monthly {
                    day: integer(&value["day"], "day", 1, 31)?,
                    hour,
                    minute,
                    timezone: zone,
                },
                _ => {
                    let month = integer(&value["month"], "month", 1, 12)?;
                    let day = integer(&value["day"], "day", 1, 31)?;
                    ensure!(
                        NaiveDate::from_ymd_opt(2000, month, day).is_some(),
                        "yearly month/day never exists"
                    );
                    Schedule::Yearly {
                        month,
                        day,
                        hour,
                        minute,
                        timezone: zone,
                    }
                }
            });
        }
    }
    bail!("missing calendar schedule")
}

pub(super) fn validate_args(args: &Value) -> Result<()> {
    let scalar = |value: &Value| -> Result<()> {
        match value {
            Value::String(text) => ensure!(
                !text.trim().is_empty() && text.len() <= 256 && !text.contains('\0'),
                "calendar field must be a bounded nonempty string"
            ),
            Value::Number(number) => ensure!(
                number.as_u64().is_some(),
                "calendar field must be a nonnegative integer"
            ),
            _ => bail!("calendar field must be integer or string"),
        }
        Ok(())
    };
    if let Some(value) = args.get("hourly") {
        scalar(value)?;
    }
    for (kind, keys) in [
        ("weekly", &["weekday", "time"][..]),
        ("monthly", &["day", "time"][..]),
        ("yearly", &["month", "day", "time"][..]),
    ] {
        if let Some(value) = args.get(kind) {
            let object = value
                .as_object()
                .context("calendar schedule must be an object")?;
            ensure!(
                object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)),
                "invalid {kind} calendar keys"
            );
            for (key, value) in object {
                scalar(value)?;
                if key == "time" {
                    ensure!(value.is_string(), "calendar time must be a string");
                }
            }
        }
    }
    Ok(())
}

pub(super) fn integer(value: &Value, field: &str, min: u32, max: u32) -> Result<u32> {
    let number = match value {
        Value::Number(value) => value
            .as_i64()
            .context("calendar field must be an integer")?,
        Value::String(value) => {
            let value = value.trim();
            let value = match field {
                "minute" => value
                    .strip_suffix("分钟")
                    .or_else(|| value.strip_suffix('分'))
                    .unwrap_or(value),
                "day" => value
                    .strip_suffix('日')
                    .or_else(|| value.strip_suffix('号'))
                    .unwrap_or(value),
                "month" => value.strip_suffix('月').unwrap_or(value),
                _ => value,
            };
            number_value(value)?
        }
        _ => bail!("{field} must be an integer or integer string"),
    };
    ensure!(
        (min as i64..=max as i64).contains(&number),
        "{field} must be {min}..{max}"
    );
    Ok(number as u32)
}

fn weekday(value: &Value) -> Result<u32> {
    if let Some(text) = value.as_str() {
        let text = text
            .trim()
            .strip_prefix("星期")
            .or_else(|| text.trim().strip_prefix("周"))
            .unwrap_or(text.trim());
        if text == "日" || text == "天" {
            return Ok(7);
        }
        return integer(&Value::String(text.into()), "weekday", 1, 7);
    }
    integer(value, "weekday", 1, 7)
}

pub(super) fn timezone(schedule: &Schedule) -> Option<&str> {
    match schedule {
        Schedule::Interval {
            timezone: Some(timezone),
            ..
        } => Some(timezone),
        Schedule::OneShot { timezone }
        | Schedule::Daily { timezone, .. }
        | Schedule::Hourly { timezone, .. }
        | Schedule::Workdays { timezone, .. }
        | Schedule::Weekly { timezone, .. }
        | Schedule::Monthly { timezone, .. }
        | Schedule::Yearly { timezone, .. } => Some(timezone),
        _ => None,
    }
}

pub(super) fn next(schedule: &Schedule, now_ms: i64) -> Result<i64> {
    let tz = offset(timezone(schedule).context("not a calendar recurrence")?)?;
    let local = DateTime::<Utc>::from_timestamp_millis(now_ms)
        .context("invalid clock")?
        .with_timezone(&tz);
    let date = local.date_naive();
    let candidate = |date: NaiveDate, hour: u32, minute: u32| -> Result<Option<i64>> {
        let naive = date
            .and_hms_opt(hour, minute, 0)
            .context("invalid stored clock")?;
        let at = tz
            .from_local_datetime(&naive)
            .single()
            .context("ambiguous clock")?
            .timestamp_millis();
        Ok((at > now_ms).then_some(at))
    };
    match schedule {
        Schedule::Hourly { minute, .. } => {
            ensure!(*minute < 60, "invalid stored minute");
            if let Some(at) = candidate(date, local.hour(), *minute)? {
                return Ok(at);
            }
            let next_hour = local
                .checked_add_signed(Duration::hours(1))
                .context("date overflow")?;
            return candidate(next_hour.date_naive(), next_hour.hour(), *minute)?
                .context("hourly time not future");
        }
        Schedule::Daily { hour, minute, .. }
        | Schedule::Workdays { hour, minute, .. }
        | Schedule::Weekly { hour, minute, .. } => {
            for days in 0..=7 {
                let day = date
                    .checked_add_signed(Duration::days(days))
                    .context("date overflow")?;
                let eligible = match schedule {
                    Schedule::Workdays { .. } => day.weekday().number_from_monday() <= 5,
                    Schedule::Weekly { weekday, .. } => {
                        day.weekday().number_from_monday() == *weekday
                    }
                    _ => true,
                };
                if eligible {
                    if let Some(at) = candidate(day, *hour, *minute)? {
                        return Ok(at);
                    }
                }
            }
        }
        Schedule::Monthly {
            day, hour, minute, ..
        } => {
            ensure!((1..=31).contains(day), "invalid stored month day");
            // At most twelve months needed; invalid dates are skipped, never clamped.
            for delta in 0..=12 {
                let month_index = date
                    .year()
                    .checked_mul(12)
                    .and_then(|year| year.checked_add(date.month0() as i32 + delta))
                    .context("date overflow")?;
                if let Some(day) = NaiveDate::from_ymd_opt(
                    month_index.div_euclid(12),
                    (month_index.rem_euclid(12) + 1) as u32,
                    *day,
                ) {
                    if let Some(at) = candidate(day, *hour, *minute)? {
                        return Ok(at);
                    }
                }
            }
        }
        Schedule::Yearly {
            month,
            day,
            hour,
            minute,
            ..
        } => {
            ensure!(
                NaiveDate::from_ymd_opt(2000, *month, *day).is_some(),
                "invalid stored yearly date"
            );
            // Gregorian February 29 can skip eight years across a non-leap century.
            for delta in 0..=8 {
                let year = date.year().checked_add(delta).context("date overflow")?;
                if let Some(day) = NaiveDate::from_ymd_opt(year, *month, *day) {
                    if let Some(at) = candidate(day, *hour, *minute)? {
                        return Ok(at);
                    }
                }
            }
        }
        _ => bail!("not a calendar recurrence"),
    }
    bail!("no valid future calendar occurrence in bounded search")
}

pub(super) fn frequency(schedule: &Schedule, args: Option<&Value>) -> String {
    match schedule {
        Schedule::Once | Schedule::OneShot { .. } => match args
            .and_then(|args| args.get("after"))
            .and_then(Value::as_str)
        {
            Some(delay) => format!("倒计时{delay}，一次性"),
            None => "一次性".into(),
        },
        Schedule::Interval { milliseconds, .. } => format!("每隔{}秒", milliseconds / 1000),
        Schedule::Daily { hour, minute, .. } => format!("每日{hour:02}:{minute:02}"),
        Schedule::Hourly { minute, .. } => format!("每小时第{minute}分"),
        Schedule::Workdays { hour, minute, .. } => {
            format!("周一至周五{hour:02}:{minute:02}，不排除法定节假日")
        }
        Schedule::Weekly {
            weekday,
            hour,
            minute,
            ..
        } => format!(
            "每周{} {hour:02}:{minute:02}",
            ["一", "二", "三", "四", "五", "六", "日"][weekday.saturating_sub(1).min(6) as usize]
        ),
        Schedule::Monthly {
            day, hour, minute, ..
        } => format!("每月{day}日{hour:02}:{minute:02}，不存在的日期跳过"),
        Schedule::Yearly {
            month,
            day,
            hour,
            minute,
            ..
        } => format!("每年{month}月{day}日{hour:02}:{minute:02}，不存在的日期跳过"),
    }
}

pub(super) fn local_datetime(at_ms: i64, timezone: &str) -> Result<String> {
    let local = DateTime::<Utc>::from_timestamp_millis(at_ms)
        .context("invalid clock")?
        .with_timezone(&offset(timezone)?);
    Ok(format!(
        "{}（{timezone}）",
        local.format("%Y年%m月%d日%H时%M分%S秒")
    ))
}
