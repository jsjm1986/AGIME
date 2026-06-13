//! Natural language parsing for scheduled tasks.
//! Direct port from team-server — no MongoDB dependency.

use crate::scheduled_tasks::models::{
    infer_delivery_plan, infer_execution_contract, infer_payload_kind, infer_session_binding,
    infer_task_profile_from_prompt, ScheduledTaskDeliveryTier, ScheduledTaskKind,
    ScheduledTaskParseResult, ScheduledTaskScheduleConfig, ScheduledTaskScheduleMode,
    ScheduledTaskScheduleSpec, ScheduledTaskScheduleSpecKind,
};
use chrono::Utc;

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

fn _parse_hhmm(text: &str) -> Option<(u32, u32)> {
    let text = text.trim();
    for pattern in &["HH:MM", "H:MM", "HHMM", "HMM"] {
        let digit_count = pattern.chars().filter(|&c| c == 'H').count();
        let _sep_count = pattern.chars().filter(|c| *c != 'H').count();
        let total = digit_count + pattern.len() - digit_count;
        if text.len() != total {
            continue;
        }
        let mut found = true;
        let mut pos = 0_usize;
        let chars: Vec<char> = pattern.chars().collect();
        let text_chars: Vec<char> = text.chars().collect();
        for (i, expected) in chars.iter().enumerate() {
            let ch = text_chars[i];
            match expected {
                'H' => {
                    if !ch.is_ascii_digit() {
                        found = false;
                        break;
                    }
                    pos += 1;
                }
                c => {
                    if ch != *c {
                        found = false;
                        break;
                    }
                    pos += 1;
                }
            }
        }
        if found && text.len() == pos {
            if digit_count == 4 {
                let h1 = text_chars[0].to_digit(10)?;
                let h2 = text_chars[1].to_digit(10)?;
                let m1 = text_chars[2].to_digit(10)?;
                let m2 = text_chars[3].to_digit(10)?;
                let hour = h1 * 10 + h2;
                let minute = m1 * 10 + m2;
                if hour < 24 && minute < 60 {
                    return Some((hour, minute));
                }
            } else if digit_count == 3 {
                let h1 = text_chars[0].to_digit(10)?;
                let h2 = text_chars[1].to_digit(10)?;
                let m1 = text_chars[2].to_digit(10)?;
                let hour = h1 * 10 + h2;
                let minute = m1 * 10;
                if hour < 24 && minute < 60 {
                    return Some((hour, minute));
                }
            } else if digit_count == 2 {
                let h1 = text_chars[0].to_digit(10)?;
                let h2 = text_chars[1].to_digit(10)?;
                let hour = h1;
                let minute = h2 * 10;
                if hour < 24 && minute < 60 {
                    return Some((hour, minute));
                }
            }
        }
    }
    None
}

fn parse_every_minutes(text: &str) -> Option<u32> {
    let patterns = [
        r"每(\d+)分钟",
        r"每(\d+)分钟执行",
        r"every\s*(\d+)\s*min",
        r"every\s*(\d+)\s*minutes?",
        r"(\d+)\s*min",
        r"(\d+)\s*minute",
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
        r"(\d+)\s*hour",
        r"(\d+)\s*hours",
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
    let day_names = [
        ("一", "1"),
        ("二", "2"),
        ("三", "3"),
        ("四", "4"),
        ("五", "5"),
        ("六", "6"),
        ("日", "0"),
        ("天", "0"),
        ("weekday", "1-5"),
        ("weekdays", "1-5"),
        ("工作日", "1-5"),
        ("平日", "1-5"),
        ("weekend", "0,6"),
        ("周末", "0,6"),
    ];
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
    for (name, value) in &day_names {
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
                        cron_expression: Some(format!("0 {} * * *", hour)),
                    }),
                    cron_expression: Some(format!("0 {} * * *", hour)),
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
                    cron_expression: Some(format!("0 {} * * 1-5", hour)),
                }),
                cron_expression: Some(format!("0 {} * * 1-5", hour)),
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
                    cron_expression: Some(format!("0 {} * * 0,6", hour)),
                }),
                cron_expression: Some(format!("0 {} * * 0,6", hour)),
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
        let cron = format!("0 {} * * {}", hour, days_str);
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
            let tomorrow = now + chrono::Duration::days(1);
            let fmt_str = format!("%Y-%m-%dT{hour:02}:{minute:02}:00Z");
            let at_str = tomorrow.format(&fmt_str);
            return (
                ScheduledTaskScheduleSpec {
                    kind: ScheduledTaskScheduleSpecKind::OneShot,
                    one_shot_at: Some(at_str.to_string()),
                    schedule_config: None,
                    cron_expression: None,
                    timezone: timezone.to_string(),
                },
                format!("明天 {:02}:{:02} 执行一次 ({})", hour, minute, timezone),
                warnings,
            );
        }
    }

    // One-shot at specific time
    if text.contains("在") || text.contains("at") || text.contains("点") {
        if let Some((hour, minute)) = extract_time(text) {
            let fmt_str = format!("%Y-%m-%dT{hour:02}:{minute:02}:00Z");
            let at_str = now.format(&fmt_str);
            return (
                ScheduledTaskScheduleSpec {
                    kind: ScheduledTaskScheduleSpecKind::OneShot,
                    one_shot_at: Some(at_str.to_string()),
                    schedule_config: None,
                    cron_expression: None,
                    timezone: timezone.to_string(),
                },
                format!("{:02}:{:02} 执行一次 ({})", hour, minute, timezone),
                warnings,
            );
        }
    }

    // Monthly on specific day at HH:MM
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
                        if day >= 1 && day <= 31 {
                            let (hour, minute) = extract_time(text).unwrap_or((9, 0));
                            let cron = format!("0 {} {} * *", hour, day);
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

fn extract_time(text: &str) -> Option<(u32, u32)> {
    // HH:MM format first
    let hhmm_patterns = [
        r"(\d{1,2}):(\d{2})",
        r"(\d{1,2})点(\d{1,2})?",
        r"(\d{1,2})\.(\d{2})",
    ];
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

    // Chinese patterns
    let chinese_patterns = [
        (r"早上(\d{1,2})点(\d{1,2})?", 8),
        (r"上午(\d{1,2})点(\d{1,2})?", 10),
        (r"中午(\d{1,2})点(\d{1,2})?", 12),
        (r"下午(\d{1,2})点(\d{1,2})?", 14),
        (r"晚上(\d{1,2})点(\d{1,2})?", 20),
        (r"凌晨(\d{1,2})点(\d{1,2})?", 3),
        (r"傍晚(\d{1,2})点(\d{1,2})?", 18),
        (r"深夜(\d{1,2})点(\d{1,2})?", 22),
        (r"(\d{1,2})点(\d{1,2})?", 0),
    ];
    for (pattern, offset) in &chinese_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                let h1: u32 = caps.get(1)?.as_str().parse::<u32>().ok()?;
                let m1: u32 = caps
                    .get(2)
                    .and_then(|m: regex::Match<'_>| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                let hour = if offset == &0 { h1 } else { h1 + offset };
                if hour < 24 && m1 < 60 {
                    return Some((hour, m1));
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
        r"每天[早中晚]?(?:上中下午)?(.+)",
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
        assert!(result.confidence < 0.6);
        assert!(!result.warnings.is_empty());
        assert!(!result.ready_to_create);
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
        assert_eq!(config.mode, ScheduledTaskScheduleMode::WeeklyOn);
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
}
