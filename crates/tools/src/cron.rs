//! `cron`: what a cron expression means, and when it runs next.
//!
//! The crontab(5) form of Vixie cron, which most crons and Kubernetes take:
//! five fields, or six with seconds first; `*`, `?`, numbers, `a-b`, `*/n`,
//! `a-b/n`, lists, month and day names, 7 for Sunday, and `@daily` and its
//! kin. When both the day of the month and the day of the week are
//! restricted, a day matching either runs, as Vixie's does; a field that
//! starts with `*` (such as `*/2`) counts as unrestricted there.
//!
//! Next runs are in a time zone: a time a daylight-saving change skips does
//! not run that day, and a time it repeats runs once.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{ToSpan as _, Zoned};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Span, Tool, Value, Values, en,
};

use crate::time;

pub struct Explain;

static META: Meta = Meta {
    id: "cron",
    group: "cron",
    action: "",
    action_flag: None,
    title: en!("Cron"),
    summary: en!("What a cron expression means, and when it runs next"),
    category: Category::Time,
    keywords: &["cron", "crontab", "schedule", "kubernetes", "cronjob"],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: Some(Reference {
        name: "crontab(5), Vixie cron",
        version: "4.1",
        compat: Compat::Standard,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "next",
        label: en!("Next runs"),
        help: en!("How many of the next runs to list"),
        kind: ParamKind::Int { min: 0, max: 100 },
        default: Value::Int(5),
        cli: Cli {
            long: "next",
            short: Some('n'),
        },
    },
    Param {
        key: "tz",
        label: en!("Time zone"),
        help: en!("The zone the schedule runs in; this computer's when left out"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "tz",
            short: None,
        },
    },
    Param {
        key: "relative_to",
        label: en!("From"),
        help: en!("List the runs after this moment; now when left out"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "relative-to",
            short: None,
        },
    },
];

/// One comma-separated part of a field, as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Value(u32),
    Range(u32, u32),
    /// `*/n`, `a/n` or `a-b/n`.
    Step {
        from: u32,
        to: u32,
        step: u32,
        star: bool,
    },
    All,
}

#[derive(Clone, Copy)]
struct Kind {
    name: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
}

const SECOND: Kind = Kind {
    name: "second",
    min: 0,
    max: 59,
    names: &[],
};
const MINUTE: Kind = Kind {
    name: "minute",
    min: 0,
    max: 59,
    names: &[],
};
const HOUR: Kind = Kind {
    name: "hour",
    min: 0,
    max: 23,
    names: &[],
};
const DAY: Kind = Kind {
    name: "day of month",
    min: 1,
    max: 31,
    names: &[],
};
const MONTH: Kind = Kind {
    name: "month",
    min: 1,
    max: 12,
    names: &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ],
};
const WEEKDAY: Kind = Kind {
    name: "day of week",
    min: 0,
    max: 7,
    names: &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A field: as written, and the values it allows as bits.
#[derive(Clone, Debug)]
struct Part {
    text: String,
    items: Vec<Item>,
    /// Starts with `*` or `?`: unrestricted for Vixie's day rule.
    star: bool,
    bits: u64,
}

impl Part {
    fn has(&self, v: u32) -> bool {
        self.bits & (1 << v) != 0
    }

    fn values(&self) -> Vec<u32> {
        (0..64).filter(|&v| self.has(v)).collect()
    }

    fn single(&self) -> Option<u32> {
        match self.items.as_slice() {
            [Item::Value(v)] => Some(*v),
            _ => None,
        }
    }
}

fn parse_value(text: &str, kind: Kind) -> Option<u32> {
    if let Ok(n) = text.parse::<u32>() {
        return (kind.min..=kind.max).contains(&n).then_some(n);
    }
    let lower = text.to_ascii_lowercase();
    let i = kind.names.iter().position(|n| *n == lower)? as u32;
    Some(if kind.min == 1 { i + 1 } else { i })
}

fn parse_part(text: &str, kind: Kind, at: usize, whole: &str) -> Result<Part, Error> {
    let bad = |why: String| {
        Error::input(format!("the {} field {text:?}: {why}", kind.name))
            .at(Span::at(whole.as_bytes(), at))
    };
    let mut items = Vec::new();
    let mut bits = 0u64;
    for piece in text.split(',') {
        let (range, step) = match piece.split_once('/') {
            Some((r, s)) => {
                let s: u32 = s
                    .parse()
                    .ok()
                    .filter(|&s| s > 0)
                    .ok_or_else(|| bad(format!("{s:?} is not a step")))?;
                (r, Some(s))
            }
            None => (piece, None),
        };
        let star = range == "*" || range == "?";
        let (from, to) = if star {
            (kind.min, kind.max)
        } else if let Some((a, b)) = range.split_once('-') {
            let a = parse_value(a, kind)
                .ok_or_else(|| bad(format!("{a:?} is out of {}–{}", kind.min, kind.max)))?;
            let b = parse_value(b, kind)
                .ok_or_else(|| bad(format!("{b:?} is out of {}–{}", kind.min, kind.max)))?;
            if a > b {
                return Err(bad(format!("{a} is after {b}")));
            }
            (a, b)
        } else {
            let v = parse_value(range, kind)
                .ok_or_else(|| bad(format!("{range:?} is out of {}–{}", kind.min, kind.max)))?;
            // `a/n` runs from a to the end.
            (v, if step.is_some() { kind.max } else { v })
        };
        let item = match (step, star, from == to) {
            (Some(step), _, _) => Item::Step {
                from,
                to,
                step,
                star,
            },
            (None, true, _) => Item::All,
            (None, false, true) => Item::Value(from),
            (None, false, false) => Item::Range(from, to),
        };
        let mut v = from;
        while v <= to {
            bits |= 1 << v;
            v += step.unwrap_or(1);
        }
        items.push(item);
    }
    // 7 is Sunday too.
    if kind.name == WEEKDAY.name && bits & (1 << 7) != 0 {
        bits = (bits & !(1 << 7)) | 1;
    }
    Ok(Part {
        text: text.to_owned(),
        star: text.starts_with('*') || text.starts_with('?'),
        items,
        bits,
    })
}

/// A schedule: seconds (only 0 for five fields), minutes … days of the week.
#[derive(Clone, Debug)]
pub struct Schedule {
    with_seconds: bool,
    second: Part,
    minute: Part,
    hour: Part,
    day: Part,
    month: Part,
    weekday: Part,
}

pub fn parse(expression: &str) -> Result<Schedule, Error> {
    let e = expression.trim();
    let expanded = match e.to_ascii_lowercase().as_str() {
        "@yearly" | "@annually" => "0 0 1 1 *",
        "@monthly" => "0 0 1 * *",
        "@weekly" => "0 0 * * 0",
        "@daily" | "@midnight" => "0 0 * * *",
        "@hourly" => "0 * * * *",
        "@reboot" => return Err(Error::input("@reboot runs when cron starts, not at a time")),
        m if m.starts_with('@') => return Err(Error::input(format!("no macro named {e}"))),
        _ => e,
    };
    let mut offsets = Vec::new();
    let mut fields = Vec::new();
    let mut i = 0;
    for f in expanded.split_whitespace() {
        let at = expanded[i..].find(f).map_or(i, |p| i + p);
        offsets.push(if expanded == e { at } else { 0 });
        fields.push(f);
        i = at + f.len();
    }
    let kinds: &[Kind] = match fields.len() {
        5 => &[MINUTE, HOUR, DAY, MONTH, WEEKDAY],
        6 => &[SECOND, MINUTE, HOUR, DAY, MONTH, WEEKDAY],
        n => {
            return Err(Error::input(format!(
                "a cron expression has 5 fields (minute hour day month weekday), or 6 with seconds first; this has {n}"
            )));
        }
    };
    let parts = fields
        .iter()
        .zip(kinds)
        .zip(&offsets)
        .map(|((f, k), at)| parse_part(f, *k, *at, e))
        .collect::<Result<Vec<_>, _>>()?;
    let with_seconds = parts.len() == 6;
    let mut it = parts.into_iter();
    let second = if with_seconds {
        it.next()
    } else {
        Some(Part {
            text: "0".into(),
            items: vec![Item::Value(0)],
            star: false,
            bits: 1,
        })
    };
    match (
        second,
        it.next(),
        it.next(),
        it.next(),
        it.next(),
        it.next(),
    ) {
        (Some(second), Some(minute), Some(hour), Some(day), Some(month), Some(weekday)) => {
            Ok(Schedule {
                with_seconds,
                second,
                minute,
                hour,
                day,
                month,
                weekday,
            })
        }
        _ => Err(Error::input("not a cron expression")),
    }
}

impl Schedule {
    fn day_matches(&self, d: Date) -> bool {
        if !self.month.has(d.month() as u32) {
            return false;
        }
        let dom = self.day.has(d.day() as u32);
        let dow = self.weekday.has(d.weekday().to_sunday_zero_offset() as u32);
        // Vixie: both restricted, either one; otherwise both.
        if !self.day.star && !self.weekday.star {
            dom || dow
        } else {
            dom && dow
        }
    }

    /// The next `count` runs after `after`, in `tz`.
    pub fn next(&self, after: &Zoned, tz: &TimeZone, count: usize) -> Vec<Zoned> {
        let mut runs = Vec::new();
        let start = after.with_time_zone(tz.clone());
        let mut day = start.date();
        let (hours, minutes, seconds) = (
            self.hour.values(),
            self.minute.values(),
            self.second.values(),
        );
        // Eight years cover a 29 February on a given weekday.
        for _ in 0..(366 * 8) {
            if runs.len() >= count {
                break;
            }
            if self.day_matches(day) {
                'day: for &h in &hours {
                    for &m in &minutes {
                        for &s in &seconds {
                            let civil = DateTime::from_parts(
                                day,
                                Time::constant(h as i8, m as i8, s as i8, 0),
                            );
                            let Ok(z) = civil.to_zoned(tz.clone()) else {
                                continue;
                            };
                            // Skipped by a daylight-saving change: not run that day.
                            if z.datetime() != civil || z.timestamp() <= start.timestamp() {
                                continue;
                            }
                            runs.push(z);
                            if runs.len() >= count {
                                break 'day;
                            }
                        }
                    }
                }
            }
            day = match day.checked_add(1.day()) {
                Ok(d) => d,
                Err(_) => break,
            };
        }
        runs
    }

    /// The schedule in a sentence.
    pub fn describe(&self) -> String {
        let mut sentence = self.describe_time();
        let date = self.describe_date();
        if !date.is_empty() {
            sentence.push_str(", ");
            sentence.push_str(&date);
        }
        let mut chars = sentence.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    }

    fn describe_time(&self) -> String {
        let (s, m, h) = (&self.second, &self.minute, &self.hour);
        let clock = |h: u32, m: u32| {
            if self.with_seconds {
                format!("{h:02}:{m:02}:{:02}", s.single().unwrap_or(0))
            } else {
                format!("{h:02}:{m:02}")
            }
        };
        let seconds = if self.with_seconds && s.single().is_none() {
            format!("{}, ", phrase(s, "second", |v| v.to_string()))
        } else {
            String::new()
        };
        let at_second = |text: String| match (self.with_seconds, s.single()) {
            (true, Some(sec)) if sec != 0 && m.single().is_none() => {
                format!("{text}, at second {sec}")
            }
            _ => text,
        };
        if let (Some(mm), Some(hh)) = (m.single(), h.single()) {
            return format!("{seconds}at {}", clock(hh, mm));
        }
        if let (Some(mm), [Item::Value(_), ..]) = (m.single(), h.items.as_slice())
            && h.items.iter().all(|i| matches!(i, Item::Value(_)))
        {
            let times: Vec<String> = h.values().into_iter().map(|hh| clock(hh, mm)).collect();
            return format!("{seconds}at {}", join(&times));
        }
        let minutes = match m.single() {
            Some(mm) => format!("at minute {mm}"),
            None => phrase(m, "minute", |v| v.to_string()),
        };
        let hours = match h.items.as_slice() {
            [Item::All] => String::new(),
            [Item::Range(a, b)] => {
                let last = m.values().last().copied().unwrap_or(0);
                format!(
                    ", from {} through {}",
                    clock(*a, m.values().first().copied().unwrap_or(0)),
                    clock(*b, last)
                )
            }
            [
                Item::Step {
                    step, star: true, ..
                },
            ] => format!(", every {step} hours"),
            _ => format!(
                ", in hours {}",
                join(
                    &h.values()
                        .iter()
                        .map(|v| format!("{v:02}"))
                        .collect::<Vec<_>>()
                )
            ),
        };
        let every_hour = if hours.is_empty() && m.single().is_some() {
            " of every hour"
        } else {
            ""
        };
        at_second(format!("{seconds}{minutes}{every_hour}{hours}"))
    }

    fn describe_date(&self) -> String {
        let day = match self.day.items.as_slice() {
            [Item::All] => String::new(),
            _ => {
                let days = phrase(&self.day, "day", |v| v.to_string());
                if days.starts_with("every") {
                    format!("{days} of the month")
                } else {
                    format!("on {days} of the month")
                }
            }
        };
        let weekday = match self.weekday.items.as_slice() {
            [Item::All] => String::new(),
            [Item::Range(a, b)] => format!(
                "{} through {}",
                DAYS[*a as usize % 7],
                DAYS[*b as usize % 7]
            ),
            _ => {
                let names: Vec<String> = self
                    .weekday
                    .values()
                    .iter()
                    .map(|v| DAYS[*v as usize % 7].to_owned())
                    .collect();
                format!("on {}", join(&names))
            }
        };
        let month = match self.month.items.as_slice() {
            [Item::All] => String::new(),
            [Item::Range(a, b)] => format!(
                "{} through {}",
                MONTHS[*a as usize - 1],
                MONTHS[*b as usize - 1]
            ),
            [
                Item::Step {
                    step, star: true, ..
                },
            ] => format!("every {step} months"),
            _ => {
                let names: Vec<String> = self
                    .month
                    .values()
                    .iter()
                    .map(|v| MONTHS[*v as usize - 1].to_owned())
                    .collect();
                format!("in {}", join(&names))
            }
        };
        let days = match (day.is_empty(), weekday.is_empty()) {
            (true, true) => String::new(),
            (false, true) => day,
            (true, false) => weekday,
            // Vixie's rule: either.
            (false, false) if !self.day.star && !self.weekday.star => {
                format!("{day}, or {weekday}")
            }
            (false, false) => format!("{day}, if it is {weekday}"),
        };
        [days, month]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn parts(&self) -> Vec<(&'static str, &'static str, &Part)> {
        let mut parts = Vec::new();
        if self.with_seconds {
            parts.push(("second", "Second", &self.second));
        }
        parts.extend([
            ("minute", "Minute", &self.minute),
            ("hour", "Hour", &self.hour),
            ("day", "Day of month", &self.day),
            ("month", "Month", &self.month),
            ("weekday", "Day of week", &self.weekday),
        ]);
        parts
    }
}

/// "every 15 minutes", "every minute from 0 through 30", "at minutes 0, 15 and 45".
fn phrase(p: &Part, unit: &str, show: impl Fn(u32) -> String) -> String {
    match p.items.as_slice() {
        [Item::All] => format!("every {unit}"),
        [
            Item::Step {
                step, star: true, ..
            },
        ] => format!("every {step} {unit}s"),
        [Item::Step { from, to, step, .. }] => format!(
            "every {step} {unit}s from {} through {}",
            show(*from),
            show(*to)
        ),
        [Item::Range(a, b)] => format!("every {unit} from {} through {}", show(*a), show(*b)),
        [Item::Value(v)] => format!("{unit} {}", show(*v)),
        _ => format!(
            "{unit}s {}",
            join(&p.values().into_iter().map(show).collect::<Vec<_>>())
        ),
    }
}

fn join(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a field allows, short: "0, 15, 30 and 45", "9 through 17".
fn allows(p: &Part, names: Option<&[&str]>, offset: u32) -> String {
    if matches!(p.items.as_slice(), [Item::All]) {
        return "every".into();
    }
    let show = |v: u32| {
        names.map_or(v.to_string(), |n| {
            n[(v - offset) as usize % n.len()].chars().take(3).collect()
        })
    };
    let values = p.values();
    if values.len() > 6
        && let (Some(a), Some(b)) = (values.first(), values.last())
    {
        return format!("{} values, {} through {}", values.len(), show(*a), show(*b));
    }
    join(&values.into_iter().map(show).collect::<Vec<_>>())
}

impl Tool for Explain {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        if text.trim().is_empty() {
            return Err(Error::usage(
                "give a cron expression, such as */15 9-17 * * 1-5",
            ));
        }
        let schedule = parse(text)?;
        let tz = time::zone(&values.text(PARAMS, "tz"))?;
        let clock = Zoned::now().with_time_zone(tz.clone());
        let relative_to = values.text(PARAMS, "relative_to");
        let from = if relative_to.is_empty() {
            clock
        } else {
            time::parse(&relative_to, &tz, &clock, "")?
        };
        let mut fields = vec![Field::new("description", "Means", schedule.describe())];
        for (key, label, part) in schedule.parts() {
            let (names, offset): (Option<&[&str]>, u32) = match key {
                "month" => (Some(&MONTHS), 1),
                "weekday" => (Some(&DAYS), 0),
                _ => (None, 0),
            };
            fields.push(
                Field::new(
                    key,
                    label,
                    format!("{} · {}", part.text, allows(part, names, offset)),
                )
                .in_section("Fields"),
            );
        }
        let count = usize::try_from(values.int(PARAMS, "next")).unwrap_or(5);
        ctx.check()?;
        for (i, run) in schedule.next(&from, &tz, count).iter().enumerate() {
            fields.push(
                Field::new(
                    format!("next_{}", i + 1),
                    format!("{}", i + 1),
                    format!("{} · {}", time::local(run), time::describe(&from, run)),
                )
                .in_section(format!(
                    "Next runs in {}",
                    tz.iana_name().unwrap_or("local")
                )),
            );
        }
        Ok(Output::Fields(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str, tz: &str) -> (Zoned, TimeZone) {
        let tz = TimeZone::get(tz).unwrap_or(TimeZone::UTC);
        let z = s
            .parse::<jiff::Timestamp>()
            .map(|t| t.to_zoned(tz.clone()))
            .unwrap_or_else(|_| Zoned::now());
        (z, tz)
    }

    fn runs(expr: &str, from: &str, tz: &str, n: usize) -> Vec<String> {
        let (from, tz) = at(from, tz);
        parse(expr)
            .unwrap_or_else(|e| panic!("{expr}: {e}"))
            .next(&from, &tz, n)
            .iter()
            .map(|z| z.strftime("%a %Y-%m-%d %H:%M:%S %:z").to_string())
            .collect()
    }

    fn describe(expr: &str) -> String {
        parse(expr)
            .map(|s| s.describe())
            .unwrap_or_else(|e| e.message)
    }

    #[test]
    fn sentences() {
        assert_eq!(
            describe("*/15 9-17 * * 1-5"),
            "Every 15 minutes, from 09:00 through 17:45, Monday through Friday"
        );
        assert_eq!(describe("30 2 * * *"), "At 02:30");
        assert_eq!(describe("0 9,12,18 * * *"), "At 09:00, 12:00 and 18:00");
        assert_eq!(describe("* * * * *"), "Every minute");
        assert_eq!(describe("5 * * * *"), "At minute 5 of every hour");
        assert_eq!(describe("0 0 1 * *"), "At 00:00, on day 1 of the month");
        assert_eq!(describe("0 0 * * SUN"), "At 00:00, on Sunday");
        assert_eq!(
            describe("0 0 1,15 * MON"),
            "At 00:00, on days 1 and 15 of the month, or on Monday"
        );
        assert_eq!(
            describe("0 6 * JAN-MAR *"),
            "At 06:00, January through March"
        );
        assert_eq!(describe("0 */2 * * *"), "At minute 0, every 2 hours");
        assert_eq!(describe("*/10 * * * * *"), "Every 10 seconds, every minute");
        assert_eq!(describe("@daily"), "At 00:00");
        assert_eq!(
            describe("0 0 */2 * *"),
            "At 00:00, every 2 days of the month"
        );
    }

    /// From Tuesday 3 November 2026, 13:26 in Ho Chi Minh City.
    #[test]
    fn next_runs_in_working_hours() {
        let r = runs(
            "*/15 9-17 * * 1-5",
            "2026-11-03T06:26:00Z",
            "Asia/Ho_Chi_Minh",
            3,
        );
        assert_eq!(
            r,
            [
                "Tue 2026-11-03 13:30:00 +07:00",
                "Tue 2026-11-03 13:45:00 +07:00",
                "Tue 2026-11-03 14:00:00 +07:00"
            ]
        );
        // Friday 17:50: the next is Monday 09:00.
        let r = runs(
            "*/15 9-17 * * 1-5",
            "2026-11-06T10:50:00Z",
            "Asia/Ho_Chi_Minh",
            1,
        );
        assert_eq!(r, ["Mon 2026-11-09 09:00:00 +07:00"]);
    }

    #[test]
    fn vixie_day_rule() {
        // The 1st, or any Monday: 1 December 2026 is a Tuesday.
        let r = runs("0 0 1 * MON", "2026-11-28T00:00:00Z", "UTC", 3);
        assert_eq!(
            r,
            [
                "Mon 2026-11-30 00:00:00 +00:00",
                "Tue 2026-12-01 00:00:00 +00:00",
                "Mon 2026-12-07 00:00:00 +00:00"
            ]
        );
        // A starred day of the month: both must match, odd days that are
        // Mondays. 30 November is even; 7 December is the next.
        let r = runs("0 0 */2 * MON", "2026-11-28T00:00:00Z", "UTC", 1);
        assert_eq!(r, ["Mon 2026-12-07 00:00:00 +00:00"]);
        assert_eq!(
            runs("0 0 29 2 *", "2026-11-03T00:00:00Z", "UTC", 1),
            ["Tue 2028-02-29 00:00:00 +00:00"]
        );
    }

    #[test]
    fn daylight_saving_changes() {
        // New York springs forward on 14 March 2027: 02:30 is skipped that day.
        let r = runs("30 2 * * *", "2027-03-13T12:00:00Z", "America/New_York", 2);
        assert_eq!(
            r,
            [
                "Mon 2027-03-15 02:30:00 -04:00",
                "Tue 2027-03-16 02:30:00 -04:00"
            ]
        );
        // It falls back on 7 November 2027: 01:30 comes twice, runs once.
        let r = runs("30 1 * * *", "2027-11-06T12:00:00Z", "America/New_York", 2);
        assert_eq!(
            r,
            [
                "Sun 2027-11-07 01:30:00 -04:00",
                "Mon 2027-11-08 01:30:00 -05:00"
            ]
        );
    }

    #[test]
    fn names_sevens_steps_and_mistakes() {
        assert_eq!(
            runs("0 12 * * 7", "2026-11-03T00:00:00Z", "UTC", 1),
            ["Sun 2026-11-08 12:00:00 +00:00"]
        );
        assert_eq!(
            runs("0 12 * * sun", "2026-11-03T00:00:00Z", "UTC", 1),
            ["Sun 2026-11-08 12:00:00 +00:00"]
        );
        assert_eq!(
            runs("10/20 * * * *", "2026-11-03T00:00:00Z", "UTC", 3).len(),
            3
        );
        assert_eq!(
            runs("*/20 0 * * * *", "2026-11-03T00:00:00Z", "UTC", 2),
            [
                "Tue 2026-11-03 00:00:20 +00:00",
                "Tue 2026-11-03 00:00:40 +00:00"
            ]
        );
        let e = parse("*/15 25 * * *").expect_err("hour 25");
        assert!(
            e.message.starts_with("the hour field \"25\""),
            "{}",
            e.message
        );
        assert_eq!(e.span.map(|s| s.column), Some(6));
        assert!(parse("* * * *").is_err());
        assert!(parse("@reboot").is_err());
        assert!(parse("0 0 31-1 * *").is_err());
    }
}
