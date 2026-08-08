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

/// 把事件/消费/任务/项目/想法汇总成一段 markdown 报告。
/// - 纯函数：所有输入（已过滤到 [from, to) 范围）由调用方提供
/// - 输出可直接作为 AI 总结的「参考上下文」
/// - 行数随数据量动态变化，但单段超过 50 条会被截断（避免 prompt 爆炸）
pub fn build_report_context(
    period: Period,
    date: NaiveDate,
    events: &[Event],
    expenses: &[Expense],
    tasks: &[crate::models::Task],
    projects: &[crate::models::Project],
    ideas: &[crate::models::Idea],
    now: i64,
) -> String {
    use crate::models::{ProjectStatus, TaskPriority};
    use std::collections::BTreeMap;

    let (from, to) = period_range(period, date);
    let mut out = String::new();
    let title = match period {
        Period::Day => format!("{} 日报", date.format("%Y-%m-%d")),
        Period::Week => {
            let monday = date - Duration::days(date.weekday().num_days_from_monday() as i64);
            format!(
                "{} 周报（第 {} 周）",
                monday.format("%Y-%m-%d"),
                monday.iso_week().week()
            )
        }
        Period::Month => format!("{} 月报", date.format("%Y-%m")),
        Period::Year => format!("{} 年报", date.year()),
    };
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!(
        "数据范围：{} ~ {}（本地时区）\n\n",
        fmt_md_hm(from),
        fmt_md_hm(to - 1)
    ));

    // ---- 时间投入 ----
    let (tag_summaries, total_secs) = summarize_events(events, from, to, now);
    out.push_str("## ⏱ 时间投入\n");
    if total_secs == 0 {
        out.push_str("无事件记录\n");
    } else {
        out.push_str(&format!("总时长 {}\n", fmt_duration(total_secs)));
        for row in &tag_summaries {
            if row.seconds > 0 {
                out.push_str(&format!(
                    "- {}：{}（{:.0}%）\n",
                    row.tag.label(),
                    fmt_duration(row.seconds),
                    row.percent
                ));
            }
        }
    }
    out.push('\n');

    // ---- 消费 ----
    out.push_str("## 💰 消费\n");
    let total_cents = sum_expenses_in_range(expenses, from, to);
    if total_cents == 0 {
        out.push_str("无消费记录\n");
    } else {
        // 按分类汇总
        let mut by_cat: BTreeMap<&'static str, i64> = BTreeMap::new();
        for e in expenses.iter().filter(|e| e.ts >= from && e.ts < to) {
            *by_cat.entry(e.category.label()).or_insert(0) += e.amount_cents;
        }
        out.push_str(&format!("总支出 ¥{}\n", fmt_yuan(total_cents)));
        for (cat, cents) in by_cat.iter() {
            let pct = if total_cents > 0 {
                *cents as f64 / total_cents as f64 * 100.0
            } else {
                0.0
            };
            out.push_str(&format!(
                "- {cat}：¥{}（{pct:.0}%）\n",
                fmt_yuan(*cents)
            ));
        }
    }
    out.push('\n');

    // ---- 任务 ----
    out.push_str("## ☑ 任务\n");
    if tasks.is_empty() {
        out.push_str("无任务\n");
    } else {
        let done: Vec<_> = tasks.iter().filter(|t| t.done).collect();
        let pending: Vec<_> = tasks.iter().filter(|t| !t.done).collect();
        let total_pomo: i32 = tasks.iter().map(|t| t.pomodoro_count).sum();
        out.push_str(&format!(
            "完成 {} / 共 {}（总番茄 {}）\n",
            done.len(),
            tasks.len(),
            total_pomo
        ));
        // 最多列 8 条待办 + 5 条已完成
        if !pending.is_empty() {
            out.push_str("待办：\n");
            for t in pending.iter().take(8) {
                out.push_str(&format!(
                    "- [ ] {}（{}）{}\n",
                    t.title,
                    match t.priority {
                        TaskPriority::High => "高",
                        TaskPriority::Mid => "中",
                        TaskPriority::Low => "低",
                    },
                    if t.project_id.is_some() { "[项目]" } else { "" }
                ));
            }
            if pending.len() > 8 {
                out.push_str(&format!("- …其余 {} 条\n", pending.len() - 8));
            }
        }
        if !done.is_empty() {
            out.push_str("已完成：\n");
            for t in done.iter().take(5) {
                out.push_str(&format!("- [x] {}\n", t.title));
            }
            if done.len() > 5 {
                out.push_str(&format!("- …其余 {} 条\n", done.len() - 5));
            }
        }
    }
    out.push('\n');

    // ---- 项目 ----
    out.push_str("## 📊 项目\n");
    if projects.is_empty() {
        out.push_str("无项目\n");
    } else {
        // 按状态分组
        let mut groups: BTreeMap<&'static str, Vec<&crate::models::Project>> = BTreeMap::new();
        for p in projects {
            let key = match p.status {
                ProjectStatus::Todo => "待启动",
                ProjectStatus::Doing => "进行中",
                ProjectStatus::Done => "已完成",
                ProjectStatus::Paused => "暂停",
                ProjectStatus::Backlog => "待规划",
            };
            groups.entry(key).or_default().push(p);
        }
        for (status, list) in groups.iter() {
            out.push_str(&format!("{status}（{}）：\n", list.len()));
            for p in list.iter().take(6) {
                let deadline = p
                    .deadline_ts
                    .map(|d| format!("（截止 {}）", fmt_md_hm(d)))
                    .unwrap_or_default();
                out.push_str(&format!("- {}{deadline}\n", p.name));
            }
            if list.len() > 6 {
                out.push_str(&format!("- …其余 {} 个\n", list.len() - 6));
            }
        }
    }
    out.push('\n');

    // ---- 想法（最多 8 条置顶 + 5 条最新）----
    out.push_str("## 💡 想法\n");
    if ideas.is_empty() {
        out.push_str("无想法\n");
    } else {
        let pinned: Vec<_> = ideas.iter().filter(|i| i.pinned).take(8).collect();
        let recent: Vec<_> = ideas.iter().take(5).collect();
        if !pinned.is_empty() {
            out.push_str("置顶：\n");
            for i in pinned {
                out.push_str(&format!("- ⭐ [{}] {}\n", i.tag.label(), i.content));
            }
        }
        if !recent.is_empty() {
            out.push_str("最新：\n");
            for i in recent {
                if i.pinned {
                    continue;
                }
                out.push_str(&format!("- [{}] {}\n", i.tag.label(), i.content));
            }
        }
    }
    out.push('\n');

    // 单段超过 50 条（防 prompt 爆炸）— 当前实现各分段已限制条目数，作为兜底
    out
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
    #[test]
    fn build_report_context_smoke_with_seed() {
        use crate::models::{Category, Event, Expense, Idea, IdeaTag, Project, ProjectStatus, Tag, Task, TaskPriority};
        // 范围：2026-08-09 当天 [00:00, 次日 00:00)
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let from = local_midnight(date);
        let to = local_midnight(date + Duration::days(1));

        // 事件：工作 1h + 看书 30m
        let events = vec![
            Event { id: "e1".into(), start_ts: from + 9 * 3600, end_ts: Some(from + 10 * 3600), content: "晨会".into(), tag: Tag::Work, remind: false, task_id: None, notion_page_id: None, dirty: false, deleted: false },
            Event { id: "e2".into(), start_ts: from + 20 * 3600, end_ts: Some(from + 20 * 3600 + 30 * 60), content: "夜读".into(), tag: Tag::Reading, remind: false, task_id: None, notion_page_id: None, dirty: false, deleted: false },
        ];
        // 消费：餐饮 25
        let expenses = vec![
            Expense { id: "x1".into(), item: "午饭".into(), amount_cents: 2500, ts: from + 12 * 3600, category: Category::Food, notion_page_id: None, dirty: false, deleted: false },
        ];
        // 任务：1 完成 + 1 待办
        let tasks = vec![
            Task { id: "t1".into(), date: "2026-08-09".into(), title: "写周报".into(), priority: TaskPriority::High, important: false, urgent: false, pomodoro_count: 2, estimated_minutes: None, notes: "".into(), task_type: "".into(), project_id: None, start_ts: None, done: true, created_ts: 0, updated_ts: 0, notion_page_id: None, dirty: false, deleted: false },
            Task { id: "t2".into(), date: "2026-08-09".into(), title: "回邮件".into(), priority: TaskPriority::Mid, important: false, urgent: false, pomodoro_count: 0, estimated_minutes: None, notes: "".into(), task_type: "".into(), project_id: Some("p1".into()), start_ts: None, done: false, created_ts: 0, updated_ts: 0, notion_page_id: None, dirty: false, deleted: false },
        ];
        let projects = vec![
            Project { id: "p1".into(), name: "体重管理".into(), status: ProjectStatus::Doing, start_ts: Some(from), deadline_ts: Some(from + 30 * 86400), note: "".into(), notion_page_id: None, dirty: false, deleted: false },
        ];
        let ideas = vec![
            Idea { id: "i1".into(), content: "读《系统之美》".into(), tag: IdeaTag::Reading, pinned: true, created_ts: 0, updated_ts: 0, notion_page_id: None, dirty: false, deleted: false },
        ];
        let now = to;
        let out = build_report_context(Period::Day, date, &events, &expenses, &tasks, &projects, &ideas, now);
        // 标题
        assert!(out.contains("# 2026-08-09 日报"), "应包含日报标题：{out}");
        // 时间投入：工作 1h00m + 看书 0h30m
        assert!(out.contains("总时长 1h30m"), "应汇总事件总时长：{out}");
        assert!(out.contains("工作：1h00m"), "工作标签 1h00m：{out}");
        assert!(out.contains("看书：30m00s") || out.contains("看书：0h30m"), "看书标签 30m：{out}");
        // 消费
        assert!(out.contains("总支出 ¥25.00"));
        assert!(out.contains("餐饮：¥25.00（100%）"));
        // 任务
        assert!(out.contains("完成 1 / 共 2"));
        assert!(out.contains("总番茄 2"));
        assert!(out.contains("- [ ] 回邮件（中）[项目]"));
        assert!(out.contains("- [x] 写周报"));
        // 项目
        assert!(out.contains("📊 项目"));
        assert!(out.contains("进行中（1）"));
        assert!(out.contains("- 体重管理"));
        // 想法
        assert!(out.contains("💡 想法"));
        assert!(out.contains("⭐ [读书] 读《系统之美》"));
    }

    #[test]
    fn build_report_context_empty_data_still_writes_headers() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let now = local_midnight(date + Duration::days(1));
        let out = build_report_context(Period::Day, date, &[], &[], &[], &[], &[], now);
        assert!(out.contains("# 2026-08-09 日报"));
        assert!(out.contains("无事件记录"));
        assert!(out.contains("无消费记录"));
        assert!(out.contains("无任务"));
        assert!(out.contains("无项目"));
        assert!(out.contains("无想法"));
    }

    #[test]
    fn build_report_context_week_uses_monday_range() {
        // 2026-08-09 是周日；周报起点应为 2026-08-03（周一）
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let now = local_midnight(date);
        let out = build_report_context(Period::Week, date, &[], &[], &[], &[], &[], now);
        assert!(out.contains("# 2026-08-03 周报"), "周报应从周一开始：{out}");
    }

    #[test]
    fn build_report_context_truncates_long_task_lists() {
        use crate::models::{Task, TaskPriority};
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let now = local_midnight(date);
        let tasks: Vec<Task> = (0..15)
            .map(|i| Task {
                id: format!("t{i}"), date: "2026-08-09".into(),
                title: format!("待办 {i}"), priority: TaskPriority::Mid,
                important: false, urgent: false, pomodoro_count: 0,
                estimated_minutes: None, notes: "".into(), task_type: "".into(),
                project_id: None, start_ts: None, done: false,
                created_ts: 0, updated_ts: 0, notion_page_id: None,
                dirty: false, deleted: false,
            })
            .collect();
        let out = build_report_context(Period::Day, date, &[], &[], &tasks, &[], &[], now);
        assert!(out.contains("- [ ] 待办 0"));
        assert!(out.contains("- [ ] 待办 7"), "应取前 8 条");
        assert!(!out.contains("- [ ] 待办 8"), "第 9 条不应出现");
        assert!(out.contains("…其余 7 条"), "应标注截断");
    }
}
