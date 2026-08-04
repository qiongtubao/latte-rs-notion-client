//! 报表聚合：日/周/月/年时间报表、消费汇总、日历日聚合。
//!
//! 所有函数均为纯函数（输入时间戳/事件切片），便于单元测试。

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};

use crate::models::{Event, Expense, Tag};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    pub fn label(self) -> &'static str {
        match self {
            Period::Day => "日",
            Period::Week => "周",
            Period::Month => "月",
            Period::Year => "年",
        }
    }

    pub fn next(self) -> Period {
        match self {
            Period::Day => Period::Week,
            Period::Week => Period::Month,
            Period::Month => Period::Year,
            Period::Year => Period::Day,
        }
    }

    pub fn prev(self) -> Period {
        match self {
            Period::Day => Period::Year,
            Period::Week => Period::Day,
            Period::Month => Period::Week,
            Period::Year => Period::Month,
        }
    }
}

/// 本地时区下某日零点的时间戳
pub fn local_midnight(date: NaiveDate) -> i64 {
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

/// 某日 [零点, 次日零点)
pub fn day_range(date: NaiveDate) -> (i64, i64) {
    (
        local_midnight(date),
        local_midnight(date + Duration::days(1)),
    )
}

/// 某周期 [起点, 终点)；周从周一开始
pub fn period_range(period: Period, date: NaiveDate) -> (i64, i64) {
    match period {
        Period::Day => day_range(date),
        Period::Week => {
            let monday = date - Duration::days(date.weekday().num_days_from_monday() as i64);
            (
                local_midnight(monday),
                local_midnight(monday + Duration::days(7)),
            )
        }
        Period::Month => {
            let first = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap();
            let (ny, nm) = if date.month() == 12 {
                (date.year() + 1, 1)
            } else {
                (date.year(), date.month() + 1)
            };
            let next = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
            (local_midnight(first), local_midnight(next))
        }
        Period::Year => {
            let first = NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap();
            let next = NaiveDate::from_ymd_opt(date.year() + 1, 1, 1).unwrap();
            (local_midnight(first), local_midnight(next))
        }
    }
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    (first_next - Duration::days(1)).day()
}

/// 事件在 [from, to) 内的有效秒数（裁剪到范围内；进行中的事件以 now 为结束）
pub fn event_seconds_in_range(ev: &Event, from: i64, to: i64, now: i64) -> i64 {
    let end = ev.end_ts.unwrap_or(now);
    let start = ev.start_ts.max(from);
    let end = end.min(to);
    (end - start).max(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TagSummary {
    pub tag: Tag,
    pub seconds: i64,
    /// 占总时长的百分比（0.0 - 100.0）
    pub percent: f64,
}

/// 按标签汇总 [from, to) 内的事件时长，返回（各标签汇总, 总秒数）。
/// 只包含时长 > 0 的标签，按 Tag::ALL 顺序排列。
pub fn summarize_events(events: &[Event], from: i64, to: i64, now: i64) -> (Vec<TagSummary>, i64) {
    let mut per_tag = [0i64; Tag::ALL.len()];
    let mut total = 0i64;
    for ev in events {
        let secs = event_seconds_in_range(ev, from, to, now);
        if secs > 0 {
            let idx = Tag::ALL.iter().position(|t| *t == ev.tag).unwrap_or(0);
            per_tag[idx] += secs;
            total += secs;
        }
    }
    let rows = Tag::ALL
        .iter()
        .enumerate()
        .filter(|(i, _)| per_tag[*i] > 0)
        .map(|(i, tag)| TagSummary {
            tag: *tag,
            seconds: per_tag[i],
            percent: if total > 0 {
                per_tag[i] as f64 * 100.0 / total as f64
            } else {
                0.0
            },
        })
        .collect();
    (rows, total)
}

/// [from, to) 内消费总额（分）
pub fn sum_expenses_in_range(expenses: &[Expense], from: i64, to: i64) -> i64 {
    expenses
        .iter()
        .filter(|e| e.ts >= from && e.ts < to)
        .map(|e| e.amount_cents)
        .sum()
}

/// 时长格式化：1h02m / 5m30s / 42s
pub fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// 分 → 元字符串
pub fn fmt_yuan(cents: i64) -> String {
    format!("{:.2}", cents as f64 / 100.0)
}

/// 时间戳 → "HH:MM"（本地时区）
pub fn fmt_hm(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t: DateTime<Local>| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// 时间戳 → "YYYY-MM-DD"（本地时区）
pub fn fmt_date(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t: DateTime<Local>| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// 时间戳 → "MM-dd HH:mm"（本地时区）
pub fn fmt_md_hm(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t: DateTime<Local>| t.format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Tag;

    /// 本地时区构造时间戳
    fn ts(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_event(id: &str, start: i64, end: Option<i64>, tag: Tag) -> Event {
        Event {
            id: id.to_string(),
            start_ts: start,
            end_ts: end,
            content: String::new(),
            tag,
            remind: false,
            task_id: None,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        }
    }

    #[test]
    fn day_range_is_24h() {
        let (from, to) = day_range(date(2026, 7, 31));
        assert_eq!(to - from, 24 * 3600);
        assert_eq!(from, ts(2026, 7, 31, 0, 0));
    }

    #[test]
    fn week_range_starts_monday() {
        // 2026-07-31 是周五
        assert_eq!(date(2026, 7, 31).weekday(), chrono::Weekday::Fri);
        let (from, to) = period_range(Period::Week, date(2026, 7, 31));
        assert_eq!(from, ts(2026, 7, 27, 0, 0)); // 周一
        assert_eq!(to, ts(2026, 8, 3, 0, 0));
        assert_eq!(to - from, 7 * 24 * 3600);
    }

    #[test]
    fn month_range_and_year_range() {
        let (from, to) = period_range(Period::Month, date(2026, 7, 15));
        assert_eq!(from, ts(2026, 7, 1, 0, 0));
        assert_eq!(to, ts(2026, 8, 1, 0, 0));

        let (from, to) = period_range(Period::Month, date(2026, 12, 20));
        assert_eq!(from, ts(2026, 12, 1, 0, 0));
        assert_eq!(to, ts(2027, 1, 1, 0, 0));

        let (from, to) = period_range(Period::Year, date(2026, 7, 31));
        assert_eq!(from, ts(2026, 1, 1, 0, 0));
        assert_eq!(to, ts(2027, 1, 1, 0, 0));
    }

    #[test]
    fn cross_day_event_is_split() {
        // 23:00 -> 次日 01:00，共 2 小时
        let ev = make_event(
            "e1",
            ts(2026, 7, 31, 23, 0),
            Some(ts(2026, 8, 1, 1, 0)),
            Tag::Work,
        );
        let (d1f, d1t) = day_range(date(2026, 7, 31));
        let (d2f, d2t) = day_range(date(2026, 8, 1));
        let now = ts(2026, 8, 2, 0, 0);
        assert_eq!(event_seconds_in_range(&ev, d1f, d1t, now), 3600);
        assert_eq!(event_seconds_in_range(&ev, d2f, d2t, now), 3600);
    }

    #[test]
    fn ongoing_event_uses_now_as_end() {
        let now = ts(2026, 7, 31, 12, 0);
        let ev = make_event("e1", ts(2026, 7, 31, 10, 0), None, Tag::Study);
        let (from, to) = day_range(date(2026, 7, 31));
        // to 是次日零点，now 更早，应裁剪到 now
        assert_eq!(event_seconds_in_range(&ev, from, to, now), 2 * 3600);
    }

    #[test]
    fn summarize_by_tag_with_percent() {
        let now = ts(2026, 8, 2, 0, 0);
        let (from, to) = day_range(date(2026, 8, 1));
        let events = vec![
            make_event(
                "a",
                ts(2026, 8, 1, 9, 0),
                Some(ts(2026, 8, 1, 10, 0)),
                Tag::Work,
            ),
            make_event(
                "b",
                ts(2026, 8, 1, 14, 0),
                Some(ts(2026, 8, 1, 17, 0)),
                Tag::Work,
            ),
            make_event(
                "c",
                ts(2026, 8, 1, 20, 0),
                Some(ts(2026, 8, 1, 21, 0)),
                Tag::Sport,
            ),
            // 完全在范围外的事件不计入
            make_event(
                "d",
                ts(2026, 7, 20, 9, 0),
                Some(ts(2026, 7, 20, 10, 0)),
                Tag::Work,
            ),
        ];
        let (rows, total) = summarize_events(&events, from, to, now);
        assert_eq!(total, 5 * 3600);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tag, Tag::Work);
        assert_eq!(rows[0].seconds, 4 * 3600);
        assert!((rows[0].percent - 80.0).abs() < 1e-9);
        assert_eq!(rows[1].tag, Tag::Sport);
        assert!((rows[1].percent - 20.0).abs() < 1e-9);
    }

    #[test]
    fn weekly_summary_includes_cross_month_days() {
        // 周跨月：2026-07-27(一) ~ 2026-08-03(一)
        let now = ts(2026, 8, 4, 0, 0);
        let (from, to) = period_range(Period::Week, date(2026, 7, 31));
        let events = vec![
            make_event(
                "a",
                ts(2026, 7, 28, 9, 0),
                Some(ts(2026, 7, 28, 10, 0)),
                Tag::Life,
            ),
            make_event(
                "b",
                ts(2026, 8, 1, 9, 0),
                Some(ts(2026, 8, 1, 11, 0)),
                Tag::Life,
            ),
        ];
        let (_, total) = summarize_events(&events, from, to, now);
        assert_eq!(total, 3 * 3600);
    }

    #[test]
    fn expense_sum_in_range() {
        let mk = |cents: i64, ts: i64| Expense {
            id: String::new(),
            item: String::new(),
            amount_cents: cents,
            ts,
            category: crate::models::Category::Food,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        };
        let (from, to) = day_range(date(2026, 7, 31));
        let expenses = vec![
            mk(2500, ts(2026, 7, 31, 12, 0)),
            mk(400, ts(2026, 7, 31, 23, 59)),
            mk(999, ts(2026, 8, 1, 0, 0)), // 次日零点，不在范围
        ];
        assert_eq!(sum_expenses_in_range(&expenses, from, to), 2900);
    }

    #[test]
    fn duration_and_money_format() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(42), "42s");
        assert_eq!(fmt_duration(330), "5m30s");
        assert_eq!(fmt_duration(3720), "1h02m");
        assert_eq!(fmt_yuan(2500), "25.00");
        assert_eq!(fmt_yuan(5), "0.05");
    }

    #[test]
    fn days_in_month_values() {
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 12), 31);
    }
}
