//! `time`: one moment as Unix time, ISO 8601, RFC 2822 and in time zones.
//!
//! The input may be empty or `now`; a Unix timestamp, whose unit is guessed
//! from its size (seconds up to 10^11, then milliseconds, microseconds,
//! nanoseconds) unless `--unit` says; RFC 3339 / ISO 8601 with an offset;
//! RFC 9557 with a zone (`…+07:00[Asia/Ho_Chi_Minh]`); RFC 2822; a date and
//! time with no offset, read in `--tz`; or a time relative to now: "in 3
//! days", "2 hours ago", "+90m".

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Unit, Zoned};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Score, Tool, Value, Values, en,
};

pub struct Convert;

static META: Meta = Meta {
    id: "time",
    group: "time",
    action: "",
    action_flag: None,
    title: en!("Time"),
    summary: en!("Unix timestamps, ISO 8601 and RFC dates, in any time zone"),
    category: Category::Time,
    keywords: &[
        "time",
        "date",
        "epoch",
        "unix",
        "timestamp",
        "iso",
        "rfc3339",
        "timezone",
    ],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: Some(Reference {
        name: "GNU date",
        version: "9.7",
        compat: Compat::Output,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "tz",
        label: en!("Time zone"),
        help: en!(
            "The zone for local times and for input with no offset, such as Asia/Ho_Chi_Minh; this computer's when left out"
        ),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "tz",
            short: None,
        },
    },
    Param {
        key: "zones",
        label: en!("Also in"),
        help: en!("More zones to show the time in, separated by commas"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "zones",
            short: None,
        },
    },
    Param {
        key: "unit",
        label: en!("Numbers are"),
        help: en!("The unit of a Unix timestamp; guessed from its size when left out"),
        kind: ParamKind::Choice {
            options: &[
                ("s", "Seconds"),
                ("ms", "Milliseconds"),
                ("us", "Microseconds"),
                ("ns", "Nanoseconds"),
            ],
            flags: false,
            unset: Some("Guess"),
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "unit",
            short: None,
        },
    },
    Param {
        key: "to",
        label: en!("Write"),
        help: en!("Write one form only"),
        kind: ParamKind::Choice {
            options: &[
                ("unix", "Unix seconds"),
                ("ms", "Unix milliseconds"),
                ("iso", "ISO 8601 in UTC"),
                ("local", "RFC 9557 in the zone"),
                ("rfc2822", "RFC 2822"),
                ("relative", "Relative to now"),
            ],
            flags: false,
            unset: Some("All"),
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "to",
            short: None,
        },
    },
    Param {
        key: "format",
        label: en!("Format"),
        help: en!("Write the time in the zone with this strftime format, such as %Y-%m-%d %H:%M"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "format",
            short: Some('f'),
        },
    },
    Param {
        key: "relative_to",
        label: en!("Now is"),
        help: en!("The moment \"now\" and relative times count from; the clock when left out"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "relative-to",
            short: None,
        },
    },
];

pub(crate) fn zone(name: &str) -> Result<TimeZone, Error> {
    if name.is_empty() {
        return Ok(TimeZone::system());
    }
    TimeZone::get(name).map_err(|_| Error::usage(format!("no time zone named {name:?}")))
}

fn zone_name(tz: &TimeZone) -> String {
    tz.iana_name().unwrap_or("local").to_owned()
}

/// Reads `text` as a moment, in `tz` when it has no offset of its own.
pub fn parse(text: &str, tz: &TimeZone, now: &Zoned, unit: &str) -> Result<Zoned, Error> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("now") {
        return Ok(now.clone());
    }
    if let Some(ts) = number(t, unit)? {
        return Ok(ts.to_zoned(tz.clone()));
    }
    if let Some(z) = relative(t, now)? {
        return Ok(z);
    }
    if let Ok(z) = t.parse::<Zoned>() {
        return Ok(z.with_time_zone(tz.clone()));
    }
    if let Ok(ts) = t.parse::<Timestamp>() {
        return Ok(ts.to_zoned(tz.clone()));
    }
    if let Ok(z) = jiff::fmt::rfc2822::parse(t) {
        return Ok(z.with_time_zone(tz.clone()));
    }
    let civil = t.parse::<DateTime>().or_else(|_| {
        t.parse::<Date>()
            .map(|d| d.to_datetime(jiff::civil::Time::midnight()))
    });
    if let Ok(dt) = civil {
        return dt
            .to_zoned(tz.clone())
            .map_err(|e| Error::input(format!("{t:?} in {}: {e}", zone_name(tz))));
    }
    Err(Error::input(format!(
        "cannot read {t:?} as a time: give a Unix timestamp, ISO 8601, RFC 2822, or \"in 3 days\""
    )))
}

/// A Unix timestamp, its unit given or guessed from its size.
fn number(t: &str, unit: &str) -> Result<Option<Timestamp>, Error> {
    let (negative, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let is_number = !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.bytes().all(|b| b.is_ascii_digit());
    if !is_number {
        return Ok(None);
    }
    let too_large = || Error::input(format!("{t} is out of range for a time"));
    let n: i128 = whole.parse().map_err(|_| too_large())?;
    let unit = match unit {
        "" if n < 100_000_000_000 => "s",
        "" if n < 100_000_000_000_000 => "ms",
        "" if n < 100_000_000_000_000_000 => "us",
        "" => "ns",
        u => u,
    };
    let per_unit: i128 = match unit {
        "s" => 1_000_000_000,
        "ms" => 1_000_000,
        "us" => 1_000,
        "ns" => 1,
        other => return Err(Error::usage(format!("no unit named {other:?}"))),
    };
    let mut frac = fraction.to_owned();
    frac.truncate(18);
    let frac_ns: i128 = if frac.is_empty() {
        0
    } else {
        let scale = 10i128.pow(frac.len() as u32);
        frac.parse::<i128>().map_err(|_| too_large())? * per_unit / scale
    };
    let mut nanos = n.checked_mul(per_unit).ok_or_else(too_large)? + frac_ns;
    if negative {
        nanos = -nanos;
    }
    Timestamp::from_nanosecond(nanos)
        .map(Some)
        .map_err(|_| too_large())
}

/// "in 3 days", "2 hours ago", "+90m", "-1 week".
fn relative(t: &str, now: &Zoned) -> Result<Option<Zoned>, Error> {
    let (forward, span) = if let Some(rest) = t.strip_prefix("in ") {
        (true, rest)
    } else if let Some(rest) = t.strip_suffix(" ago") {
        (false, rest)
    } else if let Some(rest) = t.strip_prefix('+') {
        (true, rest)
    } else if let Some(rest) = t.strip_prefix('-').filter(|r| {
        r.starts_with(|c: char| c.is_ascii_digit()) && r.contains(|c: char| c.is_alphabetic())
    }) {
        (false, rest)
    } else {
        return Ok(None);
    };
    let span: Span = span.trim().parse().map_err(|_| {
        Error::input(format!(
            "cannot read {span:?} as a length of time, such as 3 days or 2h 30m"
        ))
    })?;
    let span = if forward { span } else { span.negate() };
    now.checked_add(span)
        .map(Some)
        .map_err(|e| Error::input(format!("{t:?}: {e}")))
}

/// "in 1 hour 34 minutes", "2 days 3 hours ago", "now": the two largest units.
pub fn describe(from: &Zoned, to: &Zoned) -> String {
    let span = match from.until((Unit::Year, to)) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let past = span.is_negative();
    let span = span.abs();
    let parts = [
        (span.get_years() as i64, "year"),
        (span.get_months() as i64, "month"),
        (span.get_weeks() as i64, "week"),
        (span.get_days() as i64, "day"),
        (span.get_hours() as i64, "hour"),
        (span.get_minutes(), "minute"),
        (span.get_seconds(), "second"),
    ];
    let words: Vec<String> = parts
        .iter()
        .filter(|(n, _)| *n != 0)
        .take(2)
        .map(|(n, unit)| format!("{n} {unit}{}", if *n == 1 { "" } else { "s" }))
        .collect();
    if words.is_empty() {
        return "now".into();
    }
    let words = words.join(" ");
    if past {
        format!("{words} ago")
    } else {
        format!("in {words}")
    }
}

pub(crate) fn local(z: &Zoned) -> String {
    z.strftime("%a %Y-%m-%d %H:%M:%S %:z").to_string()
}

impl Tool for Convert {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        let tz = zone(&values.text(PARAMS, "tz"))?;
        let clock = Zoned::now().with_time_zone(tz.clone());
        let relative_to = values.text(PARAMS, "relative_to");
        let now = if relative_to.is_empty() {
            clock
        } else {
            parse(&relative_to, &tz, &clock, "")?
        };
        let z = parse(text, &tz, &now, &values.text(PARAMS, "unit"))?;
        let ts = z.timestamp();
        let format = values.text(PARAMS, "format");
        if !format.is_empty() {
            let mut out = String::new();
            jiff::fmt::strtime::format(format.as_str(), &z)
                .map(|s| out.push_str(&s))
                .map_err(|e| Error::usage(format!("format {format:?}: {e}")))?;
            return Ok(Output::Text(out));
        }
        // As GNU's `date -u -R` writes it: the day with its zero.
        let rfc2822 = z
            .with_time_zone(TimeZone::UTC)
            .strftime("%a, %d %b %Y %H:%M:%S %z")
            .to_string();
        let one = match values.text(PARAMS, "to").as_str() {
            "" => None,
            "unix" => Some(ts.as_second().to_string()),
            "ms" => Some(ts.as_millisecond().to_string()),
            "iso" => Some(ts.to_string()),
            "local" => Some(z.to_string()),
            "rfc2822" => Some(rfc2822.clone()),
            "relative" => Some(describe(&now, &z)),
            other => return Err(Error::usage(format!("no form named {other:?}"))),
        };
        if let Some(one) = one {
            return Ok(Output::Text(one));
        }
        let date = z.date();
        let week = date.iso_week_date();
        let mut fields = vec![
            Field::new("unix", "Unix seconds", ts.as_second().to_string()),
            Field::new("unix_ms", "Unix ms", ts.as_millisecond().to_string()),
            Field::new("iso", "ISO 8601", ts.to_string()),
            Field::new("rfc2822", "RFC 2822", rfc2822),
            Field::new("local", format!("In {}", zone_name(&tz)), local(&z)),
            Field::new("relative", "Relative", describe(&now, &z)),
            Field::new(
                "calendar",
                "Calendar",
                format!(
                    "{:?} · ISO week {} · day {}",
                    date.weekday(),
                    week.week(),
                    date.day_of_year()
                ),
            ),
        ];
        for name in values
            .text(PARAMS, "zones")
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            let other = zone(name)?;
            fields.push(Field::new(
                name,
                format!("In {name}"),
                local(&z.with_time_zone(other)),
            ));
        }
        Ok(Output::Fields(fields))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        let digits = s.len() >= 9 && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit());
        (digits || s.parse::<Timestamp>().is_ok()).then_some(Score(70))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str, set: &[(&str, &str)]) -> Result<Output, Error> {
        let mut v = Values::new();
        v.set("tz", Value::Text("Asia/Ho_Chi_Minh".into()));
        v.set("relative_to", Value::Text("2026-11-03T06:26:00Z".into()));
        for (k, x) in set {
            v.set(k, Value::Text((*x).into()));
        }
        Convert.run(&Ctx::new(), input.as_bytes(), &v)
    }

    fn field(input: &str, key: &str) -> String {
        match run(input, &[]) {
            Ok(Output::Fields(f)) => f
                .into_iter()
                .find(|f| f.key == key)
                .map(|f| f.value)
                .unwrap_or_default(),
            other => panic!("{input}: {other:?}"),
        }
    }

    fn one(input: &str, to: &str) -> String {
        match run(input, &[("to", to)]) {
            Ok(Output::Text(s)) => s,
            other => panic!("{input}: {other:?}"),
        }
    }

    /// Checked against GNU date 9.7: `date -u -d @1793692800 -R` gives
    /// "Tue, 03 Nov 2026 08:00:00 +0000".
    #[test]
    fn a_unix_timestamp_in_every_form() {
        assert_eq!(field("1793692800", "iso"), "2026-11-03T08:00:00Z");
        assert_eq!(
            field("1793692800", "rfc2822"),
            "Tue, 03 Nov 2026 08:00:00 +0000"
        );
        assert_eq!(
            field("1793692800", "local"),
            "Tue 2026-11-03 15:00:00 +07:00"
        );
        assert_eq!(field("1793692800", "unix_ms"), "1793692800000");
        assert_eq!(field("1793692800", "relative"), "in 1 hour 34 minutes");
        assert_eq!(
            field("1793692800", "calendar"),
            "Tuesday · ISO week 45 · day 307"
        );
    }

    #[test]
    fn the_unit_is_guessed_from_the_size() {
        for input in [
            "1793692800",
            "1793692800000",
            "1793692800000000",
            "1793692800000000000",
        ] {
            assert_eq!(one(input, "iso"), "2026-11-03T08:00:00Z", "{input}");
        }
        assert_eq!(one("1793692800.5", "iso"), "2026-11-03T08:00:00.5Z");
        assert_eq!(one("0", "iso"), "1970-01-01T00:00:00Z");
        assert_eq!(one("-86400", "iso"), "1969-12-31T00:00:00Z");
        match run("1793692800000", &[("unit", "s")]) {
            Err(_) => {}
            other => panic!("ms read as seconds is out of range: {other:?}"),
        }
    }

    #[test]
    fn dates_in_their_usual_forms() {
        assert_eq!(one("2026-11-03T08:00:00Z", "unix"), "1793692800");
        assert_eq!(one("2026-11-03T15:00:00+07:00", "unix"), "1793692800");
        assert_eq!(
            one("2026-11-03T15:00:00+07:00[Asia/Ho_Chi_Minh]", "unix"),
            "1793692800"
        );
        assert_eq!(one("Tue, 03 Nov 2026 08:00:00 +0000", "unix"), "1793692800");
        // No offset: read in --tz.
        assert_eq!(one("2026-11-03 15:00", "unix"), "1793692800");
        assert_eq!(one("2026-11-03", "iso"), "2026-11-02T17:00:00Z");
        assert!(run("yesterday-ish", &[]).is_err());
    }

    #[test]
    fn times_relative_to_now() {
        assert_eq!(one("now", "iso"), "2026-11-03T06:26:00Z");
        assert_eq!(one("", "iso"), "2026-11-03T06:26:00Z");
        assert_eq!(one("in 3 days", "iso"), "2026-11-06T06:26:00Z");
        assert_eq!(one("2 hours ago", "iso"), "2026-11-03T04:26:00Z");
        assert_eq!(one("+90m", "iso"), "2026-11-03T07:56:00Z");
        assert_eq!(one("-1 week", "iso"), "2026-10-27T06:26:00Z");
        assert_eq!(one("2026-11-01T06:26:00Z", "relative"), "2 days ago");
        assert_eq!(one("now", "relative"), "now");
    }

    #[test]
    fn zones_formats_and_mistakes() {
        match run(
            "1793692800",
            &[("zones", "UTC, America/New_York,Europe/Berlin")],
        ) {
            Ok(Output::Fields(f)) => {
                let get = |k: &str| f.iter().find(|x| x.key == k).map(|x| x.value.clone());
                assert_eq!(
                    get("UTC").as_deref(),
                    Some("Tue 2026-11-03 08:00:00 +00:00")
                );
                // New York has left daylight saving time by 3 November 2026.
                assert_eq!(
                    get("America/New_York").as_deref(),
                    Some("Tue 2026-11-03 03:00:00 -05:00")
                );
                assert_eq!(
                    get("Europe/Berlin").as_deref(),
                    Some("Tue 2026-11-03 09:00:00 +01:00")
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            run("1793692800", &[("format", "%Y-%m-%d %H:%M %Z")]),
            Ok(Output::Text("2026-11-03 15:00 +07".into()))
        );
        assert!(run("1793692800", &[("tz", "Mars/Olympus")]).is_err());
    }
}
