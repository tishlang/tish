//! `Date` — real constructor + instance methods for the non-JS targets (interpreter, VM, native).
//!
//! Representation is runtime-agnostic: a `Date` instance is a plain `Value::Object` whose methods
//! are per-instance `Value::native` closures that all capture the SAME `VmRef<f64>` epoch-millis
//! cell. Mutators (`setTime`) write the cell; getters recompute calendar fields from it. No new
//! `Value` variant is needed, so the interpreter, bytecode VM and native-compiled code all share
//! this one implementation.
//!
//! **Timezone:** Tish's `Date` runs in **UTC** — `getTimezoneOffset()` is `0` and the local-time
//! getters (`getFullYear`, `getHours`, …) are exact aliases of their `getUTC*` counterparts. There
//! is no timezone database in the core builtins (keeps them lean + wasm-friendly). `getTime`,
//! `valueOf`, the `getUTC*` family and `toISOString` are therefore fully deterministic everywhere.

use std::sync::Arc;
use tishlang_core::{ObjectMap, Value, VmRef};

const CONSTRUCT: &str = "__construct";
const MS_PER_DAY: i64 = 86_400_000;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Current wall-clock time as epoch milliseconds.
fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// A `(method name, field extractor)` pair for the numeric `Date` getters. The explicit
/// `fn(&Civil) -> i64` coerces the (non-capturing) extractor closures to a single fn-pointer type so
/// they can live in one array.
type DateGetter = (&'static str, fn(&Civil) -> i64);

/// Broken-down UTC calendar fields for an epoch-millis instant.
struct Civil {
    year: i64,
    /// 1..=12 (callers convert to JS's 0-based month where needed).
    month: i64,
    day: i64,
    hours: i64,
    minutes: i64,
    seconds: i64,
    millis: i64,
    /// 0 = Sunday … 6 = Saturday.
    weekday: i64,
}

/// Days since 1970-01-01 for a proleptic-Gregorian (y, m∈1..=12, d) — Howard Hinnant's algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`]: days-since-epoch → (year, month∈1..=12, day) — Hinnant.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Decompose epoch milliseconds into UTC calendar fields. Uses Euclidean div/rem so pre-1970
/// (negative) instants decompose correctly.
fn civil_from_ms(ms: f64) -> Civil {
    let ms_i = ms.floor() as i64;
    let days = ms_i.div_euclid(MS_PER_DAY);
    let tod = ms_i.rem_euclid(MS_PER_DAY); // [0, 86_399_999]
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday (weekday index 4).
    let weekday = (days.rem_euclid(7) + 4).rem_euclid(7);
    Civil {
        year,
        month,
        day,
        hours: tod / 3_600_000,
        minutes: (tod / 60_000) % 60,
        seconds: (tod / 1000) % 60,
        millis: tod % 1000,
        weekday,
    }
}

/// Build epoch millis from UTC components (month is 0-based, JS-style).
#[allow(clippy::too_many_arguments)]
fn ms_from_utc(
    year: i64,
    month0: i64,
    day: i64,
    hours: i64,
    minutes: i64,
    seconds: i64,
    millis: i64,
) -> f64 {
    // Normalize the 0-based month into a year/month carry so `Date.UTC(2020, 13, 1)` works.
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) + 1; // 1..=12
    let days = days_from_civil(y, m, day);
    (days * MS_PER_DAY
        + hours * 3_600_000
        + minutes * 60_000
        + seconds * 1000
        + millis) as f64
}

/// ISO-8601 string for an epoch-millis instant (always UTC, `…Z`). `None` when the instant is NaN
/// (JS would throw `RangeError` from `toISOString` on an invalid date).
fn to_iso(ms: f64) -> Option<String> {
    if !ms.is_finite() {
        return None;
    }
    let c = civil_from_ms(ms);
    Some(if (0..=9999).contains(&c.year) {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            c.year, c.month, c.day, c.hours, c.minutes, c.seconds, c.millis
        )
    } else {
        // Expanded-year form (JS uses ±YYYYYY for years outside 0..=9999).
        format!(
            "{}{:06}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            if c.year < 0 { '-' } else { '+' },
            c.year.abs(),
            c.month,
            c.day,
            c.hours,
            c.minutes,
            c.seconds,
            c.millis
        )
    })
}

/// Human form, e.g. `Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)`.
fn to_string_full(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".to_string();
    }
    let c = civil_from_ms(ms);
    format!(
        "{} {} {:02} {:04} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        WEEKDAYS[c.weekday as usize],
        MONTHS[(c.month - 1) as usize],
        c.day,
        c.year,
        c.hours,
        c.minutes,
        c.seconds
    )
}

fn to_date_string(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".to_string();
    }
    let c = civil_from_ms(ms);
    format!(
        "{} {} {:02} {:04}",
        WEEKDAYS[c.weekday as usize],
        MONTHS[(c.month - 1) as usize],
        c.day,
        c.year
    )
}

/// Parse a subset of the formats `Date.parse` accepts: ISO-8601 date (`YYYY-MM-DD`) and date-time
/// (`YYYY-MM-DDTHH:MM[:SS[.sss]]`) with an optional `Z` or `±HH:MM` offset. No offset ⇒ UTC
/// (Tish runs Dates in UTC). Returns NaN on anything it cannot parse.
fn parse_date(s: &str) -> f64 {
    let s = s.trim();
    // Split date and (optional) time on 'T' or a space.
    let (date_part, time_part) = match s.find(['T', ' ']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let mut dit = date_part.split('-');
    // Leading '-' (negative year) is not supported here; the common cases are positive years.
    let year: i64 = match dit.next().and_then(|x| x.parse().ok()) {
        Some(y) => y,
        None => return f64::NAN,
    };
    let month: i64 = dit.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    let day: i64 = dit.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return f64::NAN;
    }

    let (mut hh, mut mm, mut ss, mut ms, mut offset_min) = (0i64, 0i64, 0i64, 0i64, 0i64);
    if let Some(tp) = time_part {
        let tp = tp.trim();
        // Strip a trailing timezone designator.
        let (clock, tz): (&str, Option<&str>) = if let Some(stripped) = tp.strip_suffix('Z') {
            (stripped, Some("Z"))
        } else if let Some(pos) = tp.rfind(['+', '-']) {
            (&tp[..pos], Some(&tp[pos..]))
        } else {
            (tp, None)
        };
        let mut cit = clock.split(':');
        hh = cit.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        mm = cit.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        if let Some(sec) = cit.next() {
            let mut sp = sec.split('.');
            ss = sp.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            if let Some(frac) = sp.next() {
                // milliseconds = first 3 fractional digits, right-padded.
                let mut f = frac.to_string();
                f.truncate(3);
                while f.len() < 3 {
                    f.push('0');
                }
                ms = f.parse().unwrap_or(0);
            }
        }
        if let Some(tz) = tz {
            if tz != "Z" && tz.len() >= 3 {
                let sign = if tz.starts_with('-') { -1 } else { 1 };
                let body = &tz[1..];
                let mut oit = body.split(':');
                let oh: i64 = oit.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                let om: i64 = oit.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                offset_min = sign * (oh * 60 + om);
            }
        }
    }
    if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) || !(0..=60).contains(&ss) {
        return f64::NAN;
    }
    ms_from_utc(year, month - 1, day, hh, mm, ss, ms) - (offset_min * 60_000) as f64
}

/// `args[i]` as f64, defaulting to `dflt` when absent.
fn arg_num(args: &[Value], i: usize, dflt: f64) -> f64 {
    args.get(i).and_then(Value::as_number).unwrap_or(dflt)
}

/// A numeric getter over the instance's current epoch-millis (NaN-safe).
fn num_getter(store: &VmRef<f64>, f: fn(&Civil) -> i64) -> Value {
    let s = store.clone();
    Value::native(move |_args: &[Value]| {
        let ms = *s.borrow();
        if ms.is_nan() {
            Value::Number(f64::NAN)
        } else {
            Value::Number(f(&civil_from_ms(ms)) as f64)
        }
    })
}

/// A string getter over the instance's current epoch-millis.
fn str_getter(store: &VmRef<f64>, f: fn(f64) -> String) -> Value {
    let s = store.clone();
    Value::native(move |_args: &[Value]| Value::String(f(*s.borrow()).into()))
}

/// Construct a `Date` instance object backing onto a shared `VmRef<f64>` epoch-millis cell.
pub fn date_instance(ms: f64) -> Value {
    let store: VmRef<f64> = VmRef::new(ms);
    let mut m = ObjectMap::default();

    // Identity / numeric value.
    {
        let s = store.clone();
        m.insert(
            Arc::from("getTime"),
            Value::native(move |_| Value::Number(*s.borrow())),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("valueOf"),
            Value::native(move |_| Value::Number(*s.borrow())),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("setTime"),
            Value::native(move |args: &[Value]| {
                let ms = arg_num(args, 0, f64::NAN);
                *s.borrow_mut() = ms;
                Value::Number(ms)
            }),
        );
    }

    // UTC field getters (the canonical, deterministic family).
    let utc: [DateGetter; 8] = [
        ("getUTCFullYear", |c| c.year),
        ("getUTCMonth", |c| c.month - 1), // JS months are 0-based
        ("getUTCDate", |c| c.day),
        ("getUTCDay", |c| c.weekday),
        ("getUTCHours", |c| c.hours),
        ("getUTCMinutes", |c| c.minutes),
        ("getUTCSeconds", |c| c.seconds),
        ("getUTCMilliseconds", |c| c.millis),
    ];
    for (name, f) in utc {
        m.insert(Arc::from(name), num_getter(&store, f));
    }
    // Local-time getters: Tish runs Dates in UTC, so these alias the UTC family.
    let local: [DateGetter; 8] = [
        ("getFullYear", |c| c.year),
        ("getMonth", |c| c.month - 1),
        ("getDate", |c| c.day),
        ("getDay", |c| c.weekday),
        ("getHours", |c| c.hours),
        ("getMinutes", |c| c.minutes),
        ("getSeconds", |c| c.seconds),
        ("getMilliseconds", |c| c.millis),
    ];
    for (name, f) in local {
        m.insert(Arc::from(name), num_getter(&store, f));
    }
    m.insert(
        Arc::from("getTimezoneOffset"),
        Value::native(|_| Value::Number(0.0)),
    );

    // String renderings.
    {
        let s = store.clone();
        m.insert(
            Arc::from("toISOString"),
            Value::native(move |_| match to_iso(*s.borrow()) {
                Some(iso) => Value::String(iso.into()),
                None => Value::Null,
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("toJSON"),
            Value::native(move |_| match to_iso(*s.borrow()) {
                Some(iso) => Value::String(iso.into()),
                None => Value::Null,
            }),
        );
    }
    m.insert(Arc::from("toString"), str_getter(&store, to_string_full));
    m.insert(
        Arc::from("toUTCString"),
        str_getter(&store, |ms| {
            if !ms.is_finite() {
                return "Invalid Date".to_string();
            }
            let c = civil_from_ms(ms);
            format!(
                "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
                WEEKDAYS[c.weekday as usize],
                c.day,
                MONTHS[(c.month - 1) as usize],
                c.year,
                c.hours,
                c.minutes,
                c.seconds
            )
        }),
    );
    m.insert(Arc::from("toDateString"), str_getter(&store, to_date_string));
    m.insert(
        Arc::from("toTimeString"),
        str_getter(&store, |ms| {
            if !ms.is_finite() {
                return "Invalid Date".to_string();
            }
            let c = civil_from_ms(ms);
            format!(
                "{:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
                c.hours, c.minutes, c.seconds
            )
        }),
    );
    // Locale variants map to the UTC renderings (no locale/ICU data in core builtins).
    m.insert(
        Arc::from("toLocaleDateString"),
        str_getter(&store, to_date_string),
    );
    m.insert(
        Arc::from("toLocaleTimeString"),
        str_getter(&store, |ms| {
            if !ms.is_finite() {
                return "Invalid Date".to_string();
            }
            let c = civil_from_ms(ms);
            format!("{:02}:{:02}:{:02}", c.hours, c.minutes, c.seconds)
        }),
    );
    m.insert(Arc::from("toLocaleString"), str_getter(&store, to_string_full));

    Value::object(m)
}

/// Interpret constructor arguments (`new Date(...)`) into an epoch-millis instant.
fn ms_from_args(args: &[Value]) -> f64 {
    match args.len() {
        0 => now_ms(),
        1 => match &args[0] {
            Value::Number(n) => *n,
            Value::String(s) => parse_date(s),
            other => other.as_number().unwrap_or(f64::NAN),
        },
        _ => {
            // (year, month0, day=1, hours=0, minutes=0, seconds=0, ms=0) — UTC semantics.
            let year = arg_num(args, 0, f64::NAN);
            if !year.is_finite() {
                return f64::NAN;
            }
            ms_from_utc(
                year as i64,
                arg_num(args, 1, 0.0) as i64,
                arg_num(args, 2, 1.0) as i64,
                arg_num(args, 3, 0.0) as i64,
                arg_num(args, 4, 0.0) as i64,
                arg_num(args, 5, 0.0) as i64,
                arg_num(args, 6, 0.0) as i64,
            )
        }
    }
}

/// The global `Date`: callable as a constructor (`new Date(...)`) and carrying the statics
/// `Date.now()`, `Date.parse(str)` and `Date.UTC(...)`. Backwards-compatible with the previous
/// `Date.now()`-only object.
pub fn date_constructor_value() -> Value {
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(CONSTRUCT),
        Value::native(|args: &[Value]| date_instance(ms_from_args(args))),
    );
    m.insert(
        Arc::from("now"),
        Value::native(|_| Value::Number(now_ms())),
    );
    m.insert(
        Arc::from("parse"),
        Value::native(|args: &[Value]| {
            let ms = match args.first() {
                Some(Value::String(s)) => parse_date(s),
                Some(v) => v.as_number().unwrap_or(f64::NAN),
                None => f64::NAN,
            };
            Value::Number(ms)
        }),
    );
    m.insert(
        Arc::from("UTC"),
        Value::native(|args: &[Value]| {
            if args.is_empty() {
                return Value::Number(f64::NAN);
            }
            let year = arg_num(args, 0, f64::NAN);
            if !year.is_finite() {
                return Value::Number(f64::NAN);
            }
            Value::Number(ms_from_utc(
                year as i64,
                arg_num(args, 1, 0.0) as i64,
                arg_num(args, 2, 1.0) as i64,
                arg_num(args, 3, 0.0) as i64,
                arg_num(args, 4, 0.0) as i64,
                arg_num(args, 5, 0.0) as i64,
                arg_num(args, 6, 0.0) as i64,
            ))
        }),
    );
    Value::object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(ms: f64) -> String {
        to_iso(ms).unwrap()
    }

    #[test]
    fn epoch_is_unix_zero() {
        assert_eq!(iso(0.0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn known_instant_roundtrips() {
        // 2021-06-09T12:34:56.789Z
        let ms = ms_from_utc(2021, 5, 9, 12, 34, 56, 789);
        assert_eq!(iso(ms), "2021-06-09T12:34:56.789Z");
        let c = civil_from_ms(ms);
        assert_eq!((c.year, c.month, c.day), (2021, 6, 9));
        assert_eq!((c.hours, c.minutes, c.seconds, c.millis), (12, 34, 56, 789));
    }

    #[test]
    fn weekday_anchor() {
        // 1970-01-01 = Thursday(4); 2021-06-09 = Wednesday(3).
        assert_eq!(civil_from_ms(0.0).weekday, 4);
        assert_eq!(civil_from_ms(ms_from_utc(2021, 5, 9, 0, 0, 0, 0)).weekday, 3);
    }

    #[test]
    fn pre_epoch_negative() {
        // 1969-12-31T00:00:00Z = -86_400_000 ms, a Wednesday(3).
        let ms = ms_from_utc(1969, 11, 31, 0, 0, 0, 0);
        assert_eq!(ms, -86_400_000.0);
        assert_eq!(iso(ms), "1969-12-31T00:00:00.000Z");
        assert_eq!(civil_from_ms(ms).weekday, 3);
    }

    #[test]
    fn parse_iso_forms() {
        assert_eq!(parse_date("1970-01-01"), 0.0);
        assert_eq!(parse_date("1970-01-01T00:00:00.000Z"), 0.0);
        assert_eq!(parse_date("2021-06-09T12:34:56.789Z"), ms_from_utc(2021, 5, 9, 12, 34, 56, 789));
        // +01:00 offset pulls the UTC instant back one hour.
        assert_eq!(parse_date("1970-01-01T01:00:00+01:00"), 0.0);
        assert!(parse_date("not a date").is_nan());
    }

    #[test]
    fn leap_day() {
        assert_eq!(iso(ms_from_utc(2020, 1, 29, 0, 0, 0, 0)), "2020-02-29T00:00:00.000Z");
    }
}
