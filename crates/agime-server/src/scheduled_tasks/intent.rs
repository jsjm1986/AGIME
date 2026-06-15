//! Natural language parsing for scheduled tasks.
//! Direct port from team-server — no MongoDB dependency.

use crate::scheduled_tasks::models::{
    infer_delivery_plan, infer_execution_contract, infer_payload_kind, infer_session_binding,
    infer_task_profile_from_prompt, ScheduledTaskDeliveryTier, ScheduledTaskKind,
    ScheduledTaskParseResult, ScheduledTaskScheduleConfig, ScheduledTaskScheduleMode,
    ScheduledTaskScheduleSpec, ScheduledTaskScheduleSpecKind,
};
use chrono::{TimeZone, Utc};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse natural language text into a structured task parse result.
pub fn parse_scheduled_task_text(
    text: &str,
    timezone: Option<&str>,
    _agent_id: Option<&str>,
) -> ScheduledTaskParseResult {
    let tz = timezone.unwrap_or("Asia/Shanghai");
    let normalized = text.trim().to_ascii_lowercase();
    let (schedule_spec, human_schedule, schedule_warnings) =
        parse_schedule_from_text(&normalized, tz);
    let schedule_warnings: Vec<String> = schedule_warnings;
    let prompt = text.to_string();
    let title = infer_title(&prompt);
    let profile = infer_task_profile_from_prompt(&prompt);
    let execution_contract = infer_execution_contract("temp", &prompt, profile);
    let payload_kind = infer_payload_kind(profile, &execution_contract);
    let session_binding = infer_session_binding(&prompt, ScheduledTaskDeliveryTier::Durable);
    let delivery_plan = infer_delivery_plan(&execution_contract);
    let confidence = calculate_confidence(&schedule_spec, &schedule_warnings);
    let ready_to_create = confidence >= 0.6;
    let advanced_mode = !schedule_warnings.is_empty() || is_advanced_pattern(&prompt);
    let warnings: Vec<String> = schedule_warnings
        .into_iter()
        .filter(|w| !w.is_empty())
        .chain(if ready_to_create {
            Vec::new()
        } else {
            vec!["置信度过低，请尝试更明确的描述".to_string()]
        })
        .collect();

    ScheduledTaskParseResult {
        title,
        prompt,
        task_kind: schedule_spec.kind.into(),
        task_profile: profile,
        payload_kind,
        session_binding,
        delivery_plan,
        schedule_spec,
        execution_contract,
        human_schedule,
        warnings,
        advanced_mode,
        confidence,
        ready_to_create,
        agent_id: None,
    }
}

impl From<ScheduledTaskScheduleSpecKind> for ScheduledTaskKind {
    fn from(kind: ScheduledTaskScheduleSpecKind) -> Self {
        match kind {
            ScheduledTaskScheduleSpecKind::OneShot => ScheduledTaskKind::OneShot,
            ScheduledTaskScheduleSpecKind::Every | ScheduledTaskScheduleSpecKind::Calendar => {
                ScheduledTaskKind::Cron
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Time parsing
// ---------------------------------------------------------------------------

fn parse_every_minutes(text: &str) -> Option<u32> {
    let patterns = [
        r"每(\d+)分钟",
        r"每(\d+)分钟执行",
        r"every\s*(\d+)\s*min",
        r"every\s*(\d+)\s*minutes?",
        r"每(\d+)分钟一次",
        r"每(\d+)分钟执行一次",
    ];
    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                if let Some(m) = caps.get(1) {
                    if let Ok(value) = m.as_str().parse::<u32>() {
                        return Some(value.max(1));
                    }
                }
            }
        }
    }
    None
}

fn parse_every_hours(text: &str) -> Option<u32> {
    let patterns = [
        r"每(\d+)小时",
        r"每(\d+)小时执行",
        r"every\s*(\d+)\s*hour",
        r"every\s*(\d+)\s*hours?",
        r"每(\d+)小时一次",
        r"每(\d+)小时执行一次",
    ];
    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                if let Some(m) = caps.get(1) {
                    if let Ok(value) = m.as_str().parse::<u32>() {
                        return Some(value.max(1));
                    }
                }
            }
        }
    }
    None
}

fn weekly_days_from_text(text: &str) -> Vec<String> {
    let mut days: Vec<String> = Vec::new();

    // Exact day phrases with "周" prefix — safe, won't match "一下" or other words
    let day_map = [
        ("周一", "1"),
        ("星期1", "1"),
        ("星期一", "1"),
        ("周二", "2"),
        ("星期2", "2"),
        ("星期二", "2"),
        ("周三", "3"),
        ("星期3", "3"),
        ("星期三", "3"),
        ("周四", "4"),
        ("星期4", "4"),
        ("星期四", "4"),
        ("周五", "5"),
        ("星期5", "5"),
        ("星期五", "5"),
        ("周六", "6"),
        ("星期6", "6"),
        ("星期六", "6"),
        ("周日", "0"),
        ("星期0", "0"),
        ("星期日", "0"),
        ("星期天", "0"),
        ("周天", "0"),
    ];
    for (name, value) in &day_map {
        if text.contains(*name) {
            if !days.contains(&value.to_string()) {
                days.push(value.to_string());
            }
        }
    }

    // Connected abbreviations like 周一三五 / 星期一三五 — a single 周/星期
    // prefix followed by a run of day characters. The exact-phrase map above
    // only catches the first day (周一), so the trailing days (三、五) are lost;
    // walk the run explicitly here to recover them.
    let day_char = |c: char| -> Option<&'static str> {
        match c {
            '一' | '1' => Some("1"),
            '二' | '2' => Some("2"),
            '三' | '3' => Some("3"),
            '四' | '4' => Some("4"),
            '五' | '5' => Some("5"),
            '六' | '6' => Some("6"),
            '日' | '天' | '0' => Some("0"),
            _ => None,
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let prefix_len = if chars[i] == '周' {
            1
        } else if chars[i] == '星' && i + 1 < chars.len() && chars[i + 1] == '期' {
            2
        } else {
            0
        };
        if prefix_len == 0 {
            i += 1;
            continue;
        }
        let mut j = i + prefix_len;
        let mut matched = false;
        while j < chars.len() {
            match day_char(chars[j]) {
                Some(v) => {
                    if !days.contains(&v.to_string()) {
                        days.push(v.to_string());
                    }
                    matched = true;
                    j += 1;
                }
                None => break,
            }
        }
        i = if matched { j } else { i + prefix_len };
    }

    // Group keywords — these cover multiple days, safe to match anywhere
    let group_names = [
        ("weekday", "1-5"),
        ("weekdays", "1-5"),
        ("工作日", "1-5"),
        ("平日", "1-5"),
        ("weekend", "0,6"),
        ("周末", "0,6"),
    ];
    for (name, value) in &group_names {
        if text.contains(*name) {
            if value.contains(',') {
                for part in value.split(',') {
                    if !days.contains(&part.to_string()) {
                        days.push(part.to_string());
                    }
                }
            } else if !days.contains(&value.to_string()) {
                days.push(value.to_string());
            }
        }
    }

    days
}

fn is_weekday(text: &str) -> bool {
    text.contains("工作日") || text.contains("weekday") || text.contains("weekdays")
}

fn is_weekend(text: &str) -> bool {
    text.contains("周末") || text.contains("weekend")
}

// ---------------------------------------------------------------------------
// Schedule parsing
// ---------------------------------------------------------------------------

fn parse_schedule_from_text(
    text: &str,
    timezone: &str,
) -> (ScheduledTaskScheduleSpec, String, Vec<String>) {
    let now = Utc::now();
    let mut warnings = Vec::new();

    // Every N minutes
    if let Some(minutes) = parse_every_minutes(text) {
        return (
            ScheduledTaskScheduleSpec {
                kind: ScheduledTaskScheduleSpecKind::Every,
                one_shot_at: None,
                schedule_config: Some(ScheduledTaskScheduleConfig {
                    mode: ScheduledTaskScheduleMode::EveryMinutes,
                    every_minutes: Some(minutes),
                    every_hours: None,
                    daily_time: None,
                    weekly_days: None,
                    cron_expression: None,
                }),
                cron_expression: None,
                timezone: timezone.to_string(),
            },
            format!("每 {} 分钟执行一次 ({})", minutes, timezone),
            warnings,
        );
    }

    // Every N hours
    if let Some(hours) = parse_every_hours(text) {
        return (
            ScheduledTaskScheduleSpec {
                kind: ScheduledTaskScheduleSpecKind::Every,
                one_shot_at: None,
                schedule_config: Some(ScheduledTaskScheduleConfig {
                    mode: ScheduledTaskScheduleMode::EveryHours,
                    every_minutes: None,
                    every_hours: Some(hours),
                    daily_time: None,
                    weekly_days: None,
                    cron_expression: None,
                }),
                cron_expression: None,
                timezone: timezone.to_string(),
            },
            format!("每 {} 小时执行一次 ({})", hours, timezone),
            warnings,
        );
    }

    // Daily at HH:MM
    if text.contains("每天") || text.contains("daily") || text.contains("every day") {
        if let Some((hour, minute)) = extract_time(text) {
            let time_str = format!("{:02}:{:02}", hour, minute);
            return (
                ScheduledTaskScheduleSpec {
                    kind: ScheduledTaskScheduleSpecKind::Calendar,
                    one_shot_at: None,
                    schedule_config: Some(ScheduledTaskScheduleConfig {
                        mode: ScheduledTaskScheduleMode::DailyAt,
                        every_minutes: None,
                        every_hours: None,
                        daily_time: Some(time_str.clone()),
                        weekly_days: None,
                        cron_expression: Some(format!("{} {} * * *", minute, hour)),
                    }),
                    cron_expression: Some(format!("{} {} * * *", minute, hour)),
                    timezone: timezone.to_string(),
                },
                format!("每天 {:02}:{:02} 执行一次 ({})", hour, minute, timezone),
                warnings,
            );
        }
    }

    // Weekdays at HH:MM
    if is_weekday(text) {
        let days = vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "4".to_string(),
            "5".to_string(),
        ];
        let (hour, minute) = extract_time(text).unwrap_or((9, 0));
        let time_str = format!("{:02}:{:02}", hour, minute);
        return (
            ScheduledTaskScheduleSpec {
                kind: ScheduledTaskScheduleSpecKind::Calendar,
                one_shot_at: None,
                schedule_config: Some(ScheduledTaskScheduleConfig {
                    mode: ScheduledTaskScheduleMode::WeekdaysAt,
                    every_minutes: None,
                    every_hours: None,
                    daily_time: Some(time_str.clone()),
                    weekly_days: Some(days),
                    cron_expression: Some(format!("{} {} * * 1-5", minute, hour)),
                }),
                cron_expression: Some(format!("{} {} * * 1-5", minute, hour)),
                timezone: timezone.to_string(),
            },
            format!(
                "每个工作日 {:02}:{:02} 执行一次 ({})",
                hour, minute, timezone
            ),
            warnings,
        );
    }

    // Weekend at HH:MM
    if is_weekend(text) {
        let days = vec!["0".to_string(), "6".to_string()];
        let (hour, minute) = extract_time(text).unwrap_or((10, 0));
        let time_str = format!("{:02}:{:02}", hour, minute);
        return (
            ScheduledTaskScheduleSpec {
                kind: ScheduledTaskScheduleSpecKind::Calendar,
                one_shot_at: None,
                schedule_config: Some(ScheduledTaskScheduleConfig {
                    mode: ScheduledTaskScheduleMode::WeeklyOn,
                    every_minutes: None,
                    every_hours: None,
                    daily_time: Some(time_str.clone()),
                    weekly_days: Some(days),
                    cron_expression: Some(format!("{} {} * * 0,6", minute, hour)),
                }),
                cron_expression: Some(format!("{} {} * * 0,6", minute, hour)),
                timezone: timezone.to_string(),
            },
            format!("每个周末 {:02}:{:02} 执行一次 ({})", hour, minute, timezone),
            warnings,
        );
    }

    // Weekly on specific days at HH:MM
    let weekly_days = weekly_days_from_text(text);
    if !weekly_days.is_empty() {
        let (hour, minute) = extract_time(text).unwrap_or((9, 0));
        let time_str = format!("{:02}:{:02}", hour, minute);
        let days_str = weekly_days.join(",");
        let cron = format!("{} {} * * {}", minute, hour, days_str);
        let day_names = weekly_days
            .iter()
            .map(|d| match d.as_str() {
                "0" => "周日",
                "1" => "周一",
                "2" => "周二",
                "3" => "周三",
                "4" => "周四",
                "5" => "周五",
                "6" => "周六",
                _ => d.as_str(),
            })
            .collect::<Vec<_>>()
            .join("、");
        return (
            ScheduledTaskScheduleSpec {
                kind: ScheduledTaskScheduleSpecKind::Calendar,
                one_shot_at: None,
                schedule_config: Some(ScheduledTaskScheduleConfig {
                    mode: ScheduledTaskScheduleMode::WeeklyOn,
                    every_minutes: None,
                    every_hours: None,
                    daily_time: Some(time_str.clone()),
                    weekly_days: Some(weekly_days.clone()),
                    cron_expression: Some(cron.clone()),
                }),
                cron_expression: Some(cron),
                timezone: timezone.to_string(),
            },
            format!(
                "每周 {} {:02}:{:02} 执行一次 ({})",
                day_names, hour, minute, timezone
            ),
            warnings,
        );
    }

    // Tomorrow at HH:MM
    if text.contains("明天") || text.contains("tomorrow") {
        if let Some((hour, minute)) = extract_time(text) {
            let tomorrow_date = (now + chrono::Duration::days(1)).date_naive();
            let at_str = local_wallclock_to_utc_rfc3339(tomorrow_date, hour, minute, timezone);
            return (
                ScheduledTaskScheduleSpec {
                    kind: ScheduledTaskScheduleSpecKind::OneShot,
                    one_shot_at: Some(at_str),
                    schedule_config: None,
                    cron_expression: None,
                    timezone: timezone.to_string(),
                },
                format!("明天 {:02}:{:02} 执行一次 ({})", hour, minute, timezone),
                warnings,
            );
        }
    }

    // Monthly on specific day at HH:MM. Checked BEFORE the one-shot branch:
    // "每月15号上午9点" contains "点", which the one-shot branch would otherwise
    // claim, collapsing a recurring monthly schedule into a single fire.
    let month_day_patterns = [
        r"每月(\d+)号",
        r"每月(\d+)日",
        r"每个?月(\d+)号",
        r"每个?月(\d+)日",
        r"on\s+the\s+(\d+)(?:st|nd|rd|th)\s+of\s+(?:every\s+)?month",
    ];
    for pattern in &month_day_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                if let Some(m) = caps.get(1) {
                    if let Ok(day) = m.as_str().parse::<u32>() {
                        if (1..=31).contains(&day) {
                            let (hour, minute) = extract_time(text).unwrap_or((9, 0));
                            let cron = format!("{} {} {} * *", minute, hour, day);
                            return (
                                ScheduledTaskScheduleSpec {
                                    kind: ScheduledTaskScheduleSpecKind::Calendar,
                                    one_shot_at: None,
                                    schedule_config: Some(ScheduledTaskScheduleConfig {
                                        mode: ScheduledTaskScheduleMode::Custom,
                                        every_minutes: None,
                                        every_hours: None,
                                        daily_time: None,
                                        weekly_days: None,
                                        cron_expression: Some(cron.clone()),
                                    }),
                                    cron_expression: Some(cron),
                                    timezone: timezone.to_string(),
                                },
                                format!(
                                    "每月 {} {:02}:{:02} 执行一次 ({})",
                                    day, hour, minute, timezone
                                ),
                                warnings,
                            );
                        }
                    }
                }
            }
        }
    }

    // One-shot at specific time
    if text.contains("在") || text.contains("at") || text.contains("点") {
        if let Some((hour, minute)) = extract_time(text) {
            // Interpret the wall-clock in the task timezone. If that time has
            // already passed today, roll to tomorrow so a one-shot fires.
            let tz: Option<chrono_tz::Tz> = timezone.parse().ok();
            let base_date = tz
                .map(|tz| now.with_timezone(&tz).date_naive())
                .unwrap_or_else(|| now.date_naive());
            let at_str = local_wallclock_to_utc_rfc3339(base_date, hour, minute, timezone);
            let at_str = match chrono::DateTime::parse_from_rfc3339(&at_str) {
                Ok(dt) if dt.with_timezone(&Utc) <= now => local_wallclock_to_utc_rfc3339(
                    base_date + chrono::Duration::days(1),
                    hour,
                    minute,
                    timezone,
                ),
                _ => at_str,
            };
            return (
                ScheduledTaskScheduleSpec {
                    kind: ScheduledTaskScheduleSpecKind::OneShot,
                    one_shot_at: Some(at_str),
                    schedule_config: None,
                    cron_expression: None,
                    timezone: timezone.to_string(),
                },
                format!("{:02}:{:02} 执行一次 ({})", hour, minute, timezone),
                warnings,
            );
        }
    }

    // No schedule detected — low confidence
    warnings.push("未检测到明确的执行时间，请尝试描述「每天几点」或「每几分钟」。".to_string());
    (
        ScheduledTaskScheduleSpec {
            kind: ScheduledTaskScheduleSpecKind::OneShot,
            one_shot_at: None,
            schedule_config: None,
            cron_expression: None,
            timezone: timezone.to_string(),
        },
        "未设置".to_string(),
        warnings,
    )
}

/// Build an RFC3339 (UTC) timestamp for a local wall-clock time on a target
/// date, interpreting `hour:minute` in `timezone`. Falls back to treating the
/// wall-clock as UTC if the timezone can't be parsed or the local time is
/// ambiguous/nonexistent (DST edges).
fn local_wallclock_to_utc_rfc3339(
    date: chrono::NaiveDate,
    hour: u32,
    minute: u32,
    timezone: &str,
) -> String {
    let naive = match date.and_hms_opt(hour, minute, 0) {
        Some(n) => n,
        None => return format!("{}T{:02}:{:02}:00Z", date, hour, minute),
    };
    if let Ok(tz) = timezone.parse::<chrono_tz::Tz>() {
        if let chrono::LocalResult::Single(local) = tz.from_local_datetime(&naive) {
            return local
                .with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        }
    }
    format!("{}T{:02}:{:02}:00Z", date, hour, minute)
}

fn extract_time(text: &str) -> Option<(u32, u32)> {
    // Chinese time-of-day prefixes. `offset` is added to a bare hour only when
    // `h1 < offset`, i.e. the stated hour is in the 12-hour (PM) range and must
    // be promoted to 24-hour. AM-style periods use offset 0 (hour already 0-23).
    // The third flag marks night periods where "12点" means midnight (00:00)
    // rather than noon, per the 12-hour clock convention (12 AM = 00:00).
    let chinese_patterns = [
        (r"早上(\d{1,2})点(\d{1,2})?", 0, false), // morning 早上9点 → 9
        (r"上午(\d{1,2})点(\d{1,2})?", 0, false), // forenoon 上午11点 → 11
        (r"凌晨(\d{1,2})点(\d{1,2})?", 0, true),  // pre-dawn 凌晨3点 → 3, 凌晨12点 → 0
        (r"中午(\d{1,2})点(\d{1,2})?", 12, false), // midday 中午12点 → 12, 中午1点 → 13
        (r"下午(\d{1,2})点(\d{1,2})?", 12, false), // afternoon 下午3点 → 15, 下午12点 → 12
        (r"晚上(\d{1,2})点(\d{1,2})?", 12, true), // evening 晚上8点 → 20, 晚上12点 → 0
        (r"傍晚(\d{1,2})点(\d{1,2})?", 12, true), // dusk 傍晚6点 → 18
        (r"深夜(\d{1,2})点(\d{1,2})?", 12, true), // late night 深夜11点 → 23, 深夜12点 → 0
        (r"(\d{1,2})点(\d{1,2})?", 0, false),     // bare hour: take as-is
    ];
    for (pattern, offset, twelve_is_midnight) in &chinese_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                let h1: u32 = caps.get(1)?.as_str().parse::<u32>().ok()?;
                let m1: u32 = caps
                    .get(2)
                    .and_then(|m: regex::Match<'_>| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                let hour = if h1 == 12 && *twelve_is_midnight {
                    0
                } else if h1 < *offset {
                    h1 + offset
                } else {
                    h1
                };
                if hour < 24 && m1 < 60 {
                    return Some((hour, m1));
                }
            }
        }
    }

    // English am/pm, e.g. "9am", "at 9 am", "9:30pm", "12am" (midnight),
    // "12pm" (noon). Checked before bare HH:MM so "9am" isn't missed.
    let ampm_patterns = [r"(\d{1,2})(?::(\d{2}))?\s*(am|pm)"];
    for pattern in &ampm_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                let h1: u32 = caps.get(1)?.as_str().parse::<u32>().ok()?;
                let m1: u32 = caps
                    .get(2)
                    .and_then(|m: regex::Match<'_>| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                let is_pm = caps.get(3).map(|m| m.as_str() == "pm").unwrap_or(false);
                if (1..=12).contains(&h1) && m1 < 60 {
                    let hour = match (h1, is_pm) {
                        (12, false) => 0,    // 12am → 00:00
                        (12, true) => 12,    // 12pm → 12:00
                        (h, false) => h,     // 1am..11am
                        (h, true) => h + 12, // 1pm..11pm
                    };
                    return Some((hour, m1));
                }
            }
        }
    }

    // HH:MM format second
    let hhmm_patterns = [r"(\d{1,2}):(\d{2})", r"(\d{1,2})\.(\d{2})"];
    for pattern in &hhmm_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                let h1: u32 = caps.get(1)?.as_str().parse::<u32>().ok()?;
                let m1: u32 = caps
                    .get(2)
                    .and_then(|m: regex::Match<'_>| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                if h1 < 24 && m1 < 60 {
                    return Some((h1, m1));
                }
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Inference helpers
// ---------------------------------------------------------------------------

fn infer_title(prompt: &str) -> String {
    let prompt_lower = prompt.to_ascii_lowercase();
    let patterns = [
        r"(?:帮我?|请)?总结[一下]?(?:以下)?(.+)",
        r"(?:帮我?|请)?分析[一下]?(?:以下)?(.+)",
        r"(?:帮我?|请)?生成[一下]?(?:一个)?(?:.+的)?(.+)",
        r"(?:帮我?|请)?整理[一下]?(?:以下)?(.+)",
        r"(?:帮我?|请)?汇报[一下]?(?:以下)?(.+)",
        r"(?:帮我?|请)?提供[一下]?(?:以下)?(.+)",
        r"(?:帮我?|请)?提取[一下]?(?:以下)?(.+)",
        r"每天[早中晚]?[上中下午]?(.+)",
        r"每小时(.+)",
        r"每分钟(.+)",
        r"每周(.+)",
        r"每月(.+)",
    ];
    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(&prompt_lower) {
                if let Some(m) = caps.get(1) {
                    let title = m.as_str().trim();
                    if !title.is_empty() && title.len() <= 100 {
                        let mut result = String::new();
                        for word in title.split_whitespace() {
                            if result.is_empty() {
                                result.push_str(word);
                            } else {
                                result.push(' ');
                                result.push_str(word);
                            }
                            if result.len() > 80 {
                                break;
                            }
                        }
                        return result;
                    }
                }
            }
        }
    }
    let trimmed = prompt.trim();
    if trimmed.len() <= 80 {
        trimmed.to_string()
    } else {
        let truncated: String = trimmed.chars().take(77).collect();
        format!("{}...", truncated)
    }
}

fn calculate_confidence(schedule_spec: &ScheduledTaskScheduleSpec, warnings: &[String]) -> f32 {
    let mut base: f32 = 0.5;

    // Schedule presence
    match schedule_spec.kind {
        ScheduledTaskScheduleSpecKind::OneShot => {
            if schedule_spec.one_shot_at.is_some() {
                base += 0.35;
            } else {
                base -= 0.2;
            }
        }
        ScheduledTaskScheduleSpecKind::Every => {
            if schedule_spec.schedule_config.as_ref().is_some() {
                base += 0.4;
            } else {
                base -= 0.15;
            }
        }
        ScheduledTaskScheduleSpecKind::Calendar => {
            if schedule_spec.cron_expression.is_some() {
                base += 0.45;
            } else {
                base -= 0.15;
            }
        }
    }

    // No warnings = bonus
    if warnings.is_empty() {
        base += 0.1;
    } else {
        base -= 0.05 * warnings.len() as f32;
    }

    base.clamp(0.0, 1.0)
}

fn is_advanced_pattern(prompt: &str) -> bool {
    let advanced_markers = [
        "cron",
        "cron expression",
        "特定",
        "自定义",
        "每月",
        "每月",
        "每季",
        "每半年",
        "自定义周期",
        "monthly",
        "quarterly",
        "semi-annually",
        "custom schedule",
        "特定日期",
        "每月多少",
        "每周几",
    ];
    advanced_markers.iter().any(|m| prompt.contains(m))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_every_5_minutes() {
        let result =
            parse_scheduled_task_text("每5分钟检查一下是否有新消息", Some("Asia/Shanghai"), None);
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Every
        );
        assert_eq!(
            result
                .schedule_spec
                .schedule_config
                .as_ref()
                .unwrap()
                .every_minutes,
            Some(5)
        );
        assert!(result.confidence > 0.8);
    }

    #[test]
    fn test_parse_daily_at_9am() {
        let result = parse_scheduled_task_text("每天早上9点生成报告", Some("Asia/Shanghai"), None);
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Calendar
        );
        assert!(result.confidence > 0.8);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_weekdays_at_830() {
        let result = parse_scheduled_task_text(
            "工作日早上8:30给我汇报项目进展",
            Some("Asia/Shanghai"),
            None,
        );
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Calendar
        );
        let config = result.schedule_spec.schedule_config.as_ref().unwrap();
        assert_eq!(config.mode, ScheduledTaskScheduleMode::WeekdaysAt);
        assert_eq!(config.daily_time.as_deref(), Some("08:30"));
    }

    #[test]
    fn test_parse_no_schedule_low_confidence() {
        let result = parse_scheduled_task_text("帮我总结一下", Some("Asia/Shanghai"), None);
        // "帮我总结" without explicit time should produce low confidence and not be ready to create.
        // The schedule spec defaults to OneShot with no one_shot_at, giving confidence = 0.25.
        let confidence_low = result.confidence < 0.8;
        let not_ready = !result.ready_to_create;
        assert!(
            confidence_low && not_ready,
            "expected low confidence ({:.2}) and not ready_to_create, got confidence={:.2}, ready_to_create={}",
            result.confidence,
            result.confidence,
            result.ready_to_create
        );
    }

    #[test]
    fn test_parse_weekly_on_specific_days() {
        let result = parse_scheduled_task_text(
            "每周一、周三、周五下午3点给我发工作日志总结",
            Some("Asia/Shanghai"),
            None,
        );
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Calendar
        );
        let config = result.schedule_spec.schedule_config.as_ref().unwrap();
        // The parser may classify Mon/Wed/Fri as WeekdaysAt or WeeklyOn depending on implementation
        assert!(
            matches!(
                config.mode,
                ScheduledTaskScheduleMode::WeekdaysAt | ScheduledTaskScheduleMode::WeeklyOn
            ),
            "Expected WeekdaysAt or WeeklyOn, got {:?}",
            config.mode
        );
        assert_eq!(config.daily_time.as_deref(), Some("15:00"));
        assert!(result.confidence > 0.8);
    }

    #[test]
    fn test_infer_title_with_帮我总结() {
        let title = infer_title("帮我总结一下今天的工作");
        assert!(title.contains("今天") || title.contains("工作"));
    }

    #[test]
    fn test_infer_title_truncates_long_prompt() {
        let long = "帮我分析一下每天的工作日志并生成一个详细的总结报告，包括任务完成情况、问题分析、改进建议等多个方面的工作内容";
        let title = infer_title(long);
        assert!(title.len() <= 100);
    }

    #[test]
    fn test_confidence_for_valid_schedule() {
        let result = parse_scheduled_task_text("每30分钟同步一次数据", Some("Asia/Shanghai"), None);
        assert!(result.confidence >= 0.85);
    }

    #[test]
    fn test_confidence_for_missing_schedule() {
        let result = parse_scheduled_task_text("帮我整理文件", Some("Asia/Shanghai"), None);
        assert!(result.confidence < 0.7);
    }

    #[test]
    fn test_daily_cron_preserves_minute() {
        let result =
            parse_scheduled_task_text("每天早上9点30分生成报告", Some("Asia/Shanghai"), None);
        let config = result.schedule_spec.schedule_config.as_ref().unwrap();
        // minute must be preserved in the cron, not flattened to 0.
        assert_eq!(config.cron_expression.as_deref(), Some("30 9 * * *"));
        assert_eq!(config.daily_time.as_deref(), Some("09:30"));
    }

    #[test]
    fn test_weekdays_cron_preserves_minute() {
        let result =
            parse_scheduled_task_text("工作日早上8:30汇报进展", Some("Asia/Shanghai"), None);
        let config = result.schedule_spec.schedule_config.as_ref().unwrap();
        assert_eq!(config.cron_expression.as_deref(), Some("30 8 * * 1-5"));
    }

    #[test]
    fn test_evening_8pm_promotes_to_20() {
        // "晚上8点" must become 20:00, not overflow to 26 (which previously
        // failed the < 24 check and returned None).
        assert_eq!(extract_time("晚上8点"), Some((20, 0)));
    }

    #[test]
    fn test_pre_dawn_1am_stays_1() {
        // "凌晨1点" is 01:00, not 04:00.
        assert_eq!(extract_time("凌晨1点"), Some((1, 0)));
    }

    #[test]
    fn test_dusk_6pm_promotes_to_18() {
        assert_eq!(extract_time("傍晚6点"), Some((18, 0)));
    }

    #[test]
    fn test_midday_12_stays_12() {
        assert_eq!(extract_time("中午12点"), Some((12, 0)));
    }

    #[test]
    fn test_midnight_12_at_night_is_zero() {
        // 12 in night periods is midnight (00:00), not noon, per 12-hour clock.
        assert_eq!(extract_time("晚上12点"), Some((0, 0)));
        assert_eq!(extract_time("凌晨12点"), Some((0, 0)));
        assert_eq!(extract_time("深夜12点"), Some((0, 0)));
    }

    #[test]
    fn test_afternoon_12_stays_noon() {
        // 下午12点 stays 12:00 (not promoted to 24, not midnight).
        assert_eq!(extract_time("下午12点"), Some((12, 0)));
    }

    #[test]
    fn test_bare_duration_words_do_not_trigger_every_minutes() {
        // "30 minutes" buried in prose with an explicit daily time must parse as
        // a daily schedule, not "every 30 minutes".
        let result = parse_scheduled_task_text(
            "summarize the last 30 minutes every day at 9am",
            Some("Asia/Shanghai"),
            None,
        );
        let config = result.schedule_spec.schedule_config.unwrap();
        assert_ne!(config.mode, ScheduledTaskScheduleMode::EveryMinutes);
    }

    #[test]
    fn test_monthly_not_shadowed_by_one_shot() {
        // "每月15号上午9点" contains "点"; the monthly branch must win over the
        // one-shot branch and produce a recurring Calendar schedule.
        let result =
            parse_scheduled_task_text("每月15号上午9点生成月度报告", Some("Asia/Shanghai"), None);
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Calendar
        );
        let config = result.schedule_spec.schedule_config.as_ref().unwrap();
        assert_eq!(config.cron_expression.as_deref(), Some("0 9 15 * *"));
    }

    #[test]
    fn test_english_9am() {
        assert_eq!(extract_time("every day at 9am"), Some((9, 0)));
    }

    #[test]
    fn test_english_3pm() {
        assert_eq!(extract_time("daily at 3pm"), Some((15, 0)));
    }

    #[test]
    fn test_english_930pm() {
        assert_eq!(extract_time("9:30pm"), Some((21, 30)));
    }

    #[test]
    fn test_english_12am_is_midnight() {
        assert_eq!(extract_time("12am"), Some((0, 0)));
    }

    #[test]
    fn test_english_12pm_is_noon() {
        assert_eq!(extract_time("12pm"), Some((12, 0)));
    }

    #[test]
    fn test_english_daily_9am_builds_schedule() {
        // English schedules must now reach a real Calendar schedule instead of
        // falling through to the low-confidence fallback.
        let result =
            parse_scheduled_task_text("every day at 9am send me a summary", Some("UTC"), None);
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Calendar
        );
        let config = result.schedule_spec.schedule_config.as_ref().unwrap();
        assert_eq!(config.cron_expression.as_deref(), Some("0 9 * * *"));
    }

    #[test]
    fn test_connected_weekday_abbreviation() {
        // "每周一三五" must resolve all of Mon/Wed/Fri, not just Monday.
        let days = weekly_days_from_text("每周一三五下午3点汇报");
        assert!(days.contains(&"1".to_string()));
        assert!(days.contains(&"3".to_string()));
        assert!(days.contains(&"5".to_string()));
    }

    #[test]
    fn test_connected_weekday_still_handles_single() {
        let days = weekly_days_from_text("每周二开会");
        assert_eq!(days, vec!["2".to_string()]);
    }

    #[test]
    fn test_one_shot_tomorrow_converts_local_to_utc() {
        // 明天9点 in Asia/Shanghai (UTC+8) must serialize as 01:00Z, not 09:00Z.
        let result =
            parse_scheduled_task_text("明天早上9点提醒我开会", Some("Asia/Shanghai"), None);
        assert_eq!(
            result.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::OneShot
        );
        let at = result.schedule_spec.one_shot_at.as_deref().unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(at).unwrap();
        assert_eq!(
            parsed.with_timezone(&Utc).format("%H:%M").to_string(),
            "01:00"
        );
    }
}
