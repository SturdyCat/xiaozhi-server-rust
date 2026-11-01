use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Duration, FixedOffset, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DAY_MS: i64 = 86_400_000;
const MAX_DURATION_MS: i64 = 366 * DAY_MS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Schedule {
    Once,
    OneShot {
        timezone: String,
    },
    Interval {
        milliseconds: i64,
        #[serde(default)]
        timezone: Option<String>,
    },
    Daily {
        hour: u32,
        minute: u32,
        timezone: String,
    },
    Hourly {
        minute: u32,
        timezone: String,
    },
    Workdays {
        hour: u32,
        minute: u32,
        timezone: String,
    },
    Weekly {
        weekday: u32,
        hour: u32,
        minute: u32,
        timezone: String,
    },
    Monthly {
        day: u32,
        hour: u32,
        minute: u32,
        timezone: String,
    },
    Yearly {
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        timezone: String,
    },
}

impl Schedule {
    pub(super) fn parse(args: &Value, now_ms: i64) -> Result<(Self, i64)> {
        let timezone = args
            .get("timezone")
            .and_then(Value::as_str)
            .unwrap_or("UTC");
        offset(timezone)?;
        if let Some(after) = args.get("after").and_then(Value::as_str) {
            return Ok((
                Self::OneShot {
                    timezone: timezone.into(),
                },
                add_ms(now_ms, duration_ms(after)?)?,
            ));
        }
        if let Some(interval) = args.get("interval").and_then(Value::as_str) {
            let milliseconds = duration_ms(interval)?;
            return Ok((
                Self::Interval {
                    milliseconds,
                    timezone: Some(timezone.into()),
                },
                add_ms(now_ms, milliseconds)?,
            ));
        }
        if let Some(at) = args.get("at").and_then(Value::as_str) {
            return Ok((
                Self::OneShot {
                    timezone: timezone.into(),
                },
                at_ms(at, timezone, now_ms)?,
            ));
        }
        let schedule = super::calendar::parse(args, timezone)?;
        let next = schedule
            .next_after(now_ms, now_ms)?
            .context("calendar schedule has no next time")?;
        Ok((schedule, next))
    }

    /// Strictly after now, skipping missed repetitions; never catch up with a flood.
    pub(super) fn next_after(&self, previous_ms: i64, now_ms: i64) -> Result<Option<i64>> {
        Ok(match self {
            Self::Once | Self::OneShot { .. } => None,
            Self::Interval { milliseconds, .. } => {
                ensure!(
                    *milliseconds >= 1000 && *milliseconds <= MAX_DURATION_MS,
                    "invalid stored interval"
                );
                let missed = now_ms.saturating_sub(previous_ms).max(0) / milliseconds + 1;
                Some(add_ms(
                    previous_ms,
                    missed
                        .checked_mul(*milliseconds)
                        .context("interval overflow")?,
                )?)
            }
            _ => Some(super::calendar::next(self, now_ms)?),
        })
    }
}

fn add_ms(base: i64, delta: i64) -> Result<i64> {
    let value = base
        .checked_add(delta)
        .context("schedule timestamp overflow")?;
    ensure!(
        DateTime::<Utc>::from_timestamp_millis(value).is_some(),
        "schedule timestamp outside supported range"
    );
    Ok(value)
}

pub(super) fn offset(timezone: &str) -> Result<FixedOffset> {
    match timezone {
        "UTC" => Ok(FixedOffset::east_opt(0).unwrap()),
        "Asia/Shanghai" => Ok(FixedOffset::east_opt(8 * 3600).unwrap()),
        _ => bail!("unsupported timezone: {timezone}; use UTC or Asia/Shanghai"),
    }
}

pub(super) fn duration_ms(text: &str) -> Result<i64> {
    let mut text = text.trim();
    for prefix in ["每隔", "every ", "每"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim();
            break;
        }
    }
    for suffix in ["之后", "以后", "后"] {
        if let Some(rest) = text.strip_suffix(suffix) {
            text = rest.trim();
            break;
        }
    }
    if text == "半小时" {
        return Ok(30 * 60_000);
    }
    for (unit, multiplier) in [
        ("milliseconds", 1),
        ("ms", 1),
        ("minutes", 60_000),
        ("minute", 60_000),
        ("分钟", 60_000),
        ("hours", 3_600_000),
        ("hour", 3_600_000),
        ("小时", 3_600_000),
        ("seconds", 1000),
        ("second", 1000),
        ("秒钟", 1000),
        ("秒", 1000),
        ("days", DAY_MS),
        ("day", DAY_MS),
        ("天", DAY_MS),
        ("min", 60_000),
        ("h", 3_600_000),
        ("s", 1000),
    ] {
        if let Some(number) = text.strip_suffix(unit) {
            let milliseconds = number_value(number.trim())?
                .checked_mul(multiplier)
                .context("duration overflow")?;
            ensure!(
                (1000..=MAX_DURATION_MS).contains(&milliseconds),
                "duration must be 1 second..366 days"
            );
            return Ok(milliseconds);
        }
    }
    bail!("unsupported or ambiguous duration: {text}");
}

pub(super) fn number_value(text: &str) -> Result<i64> {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        return text.parse::<i64>().context("invalid duration number");
    }
    if text == "半" {
        bail!("fractional duration unsupported except 半小时");
    }
    let mut total: i64 = 0;
    let mut digit: i64 = 0;
    let mut has_digit = false;
    let mut previous_unit: i64 = 10000;
    for ch in text.chars() {
        let value = match ch {
            '零' | '〇' => 0,
            '一' => 1,
            '二' | '两' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            '十' | '百' | '千' => {
                let unit = match ch {
                    '十' => 10,
                    '百' => 100,
                    _ => 1000,
                };
                ensure!(unit < previous_unit, "ambiguous Chinese number: {text}");
                total += if has_digit { digit } else { 1 } * unit;
                previous_unit = unit;
                digit = 0;
                has_digit = false;
                continue;
            }
            _ => bail!("unsupported number: {text}"),
        };
        ensure!(!has_digit || digit == 0, "ambiguous Chinese number: {text}");
        digit = value;
        has_digit = true;
    }
    ensure!(!text.is_empty(), "number is empty");
    Ok(total + digit)
}

pub(super) fn clock(text: &str) -> Result<(u32, u32)> {
    let text = text.trim();
    if text.len() == 5 && text.as_bytes()[2] == b':' {
        let time =
            NaiveTime::parse_from_str(text, "%H:%M").context("daily clock requires HH:MM")?;
        return Ok((time.hour(), time.minute()));
    }
    let (mut text, mut period) = (text, "");
    for prefix in ["凌晨", "早上", "上午", "中午", "下午", "晚上"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest;
            period = prefix;
            break;
        }
    }
    let (hour, minutes) = if let Some(parts) = text.split_once('点') {
        parts
    } else if !period.is_empty() {
        (text, "")
    } else {
        bail!("clock requires HH:MM or explicit Chinese 点 time");
    };
    let mut hour = u32::try_from(number_value(hour)?).context("invalid hour")?;
    let minute = match minutes {
        "" | "整" => 0,
        "半" => 30,
        "一刻" => 15,
        "三刻" => 45,
        other => u32::try_from(number_value(other.strip_suffix('分').unwrap_or(other))?)
            .context("invalid minute")?,
    };
    if !period.is_empty() {
        ensure!((1..=12).contains(&hour), "period requires hour 1..12");
        ensure!(
            hour != 12 || period == "中午",
            "ambiguous period at twelve; use explicit 00:00 or 12:00"
        );
        if matches!(period, "下午" | "晚上") && hour < 12 {
            hour += 12;
        }
        if period == "中午" {
            ensure!((11..=12).contains(&hour), "ambiguous noon clock");
        }
    }
    NaiveTime::from_hms_opt(hour, minute, 0).context("invalid hour or minute")?;
    Ok((hour, minute))
}

pub(super) enum AtSpec {
    Absolute(i64),
    Clock {
        days: Option<i64>,
        hour: u32,
        minute: u32,
    },
}

/// Syntax only: no current clock, storage, or future-time validation.
pub(super) fn parse_at_spec(text: &str) -> Result<AtSpec> {
    let text = text.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Ok(AtSpec::Absolute(at.timestamp_millis()));
    }
    let explicit = [("大后天", 3), ("后天", 2), ("明天", 1), ("今天", 0)]
        .into_iter()
        .find_map(|(prefix, days)| text.strip_prefix(prefix).map(|time| (days, time)));
    let (hour, minute) = clock(explicit.map(|(_, time)| time).unwrap_or(text))?;
    Ok(AtSpec::Clock {
        days: explicit.map(|(days, _)| days),
        hour,
        minute,
    })
}

fn at_ms(text: &str, timezone: &str, now_ms: i64) -> Result<i64> {
    let (days, hour, minute) = match parse_at_spec(text)? {
        AtSpec::Absolute(at) => {
            ensure!(at > now_ms, "scheduled time must be in the future");
            return Ok(at);
        }
        AtSpec::Clock { days, hour, minute } => (days, hour, minute),
    };
    let tz = offset(timezone)?;
    let today = DateTime::<Utc>::from_timestamp_millis(now_ms)
        .context("invalid clock")?
        .with_timezone(&tz)
        .date_naive();
    let date = today
        .checked_add_signed(Duration::days(days.unwrap_or(0)))
        .context("date overflow")?;
    let datetime = date.and_hms_opt(hour, minute, 0).context("invalid clock")?;
    let mut at = tz
        .from_local_datetime(&datetime)
        .single()
        .context("ambiguous clock")?
        .timestamp_millis();
    if at <= now_ms && days.is_none() {
        let tomorrow = date
            .succ_opt()
            .context("date overflow")?
            .and_hms_opt(hour, minute, 0)
            .context("invalid clock")?;
        at = tz
            .from_local_datetime(&tomorrow)
            .single()
            .context("ambiguous clock")?
            .timestamp_millis();
    }
    ensure!(at > now_ms, "scheduled time must be in the future");
    Ok(at)
}
