use chrono::{Datelike, Duration, LocalResult, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

use super::models::{
    infer_execution_contract, infer_task_profile_from_prompt, ScheduledTaskDeliveryPlanKind,
    ScheduledTaskExecutionContract, ScheduledTaskKind, ScheduledTaskParseResult,
    ScheduledTaskPayloadKind, ScheduledTaskProfile, ScheduledTaskScheduleConfig,
    ScheduledTaskScheduleMode, ScheduledTaskScheduleSpec, ScheduledTaskScheduleSpecKind,
    ScheduledTaskSessionBinding,
};

fn trim_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_task_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn has_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}

fn parse_hhmm(text: &str) -> Option<(u32, u32)> {
    let chars = text.chars().collect::<Vec<_>>();
    for i in 0..chars.len() {
        if !chars[i].is_ascii_digit() {
            continue;
        }
        for j in (i + 1)..chars.len() {
            let segment = chars[i..=j].iter().collect::<String>();
            if let Some((left, right)) = segment.split_once(':') {
                let hour = left.parse::<u32>().ok()?;
                let minute = right.parse::<u32>().ok()?;
                if hour <= 23 && minute <= 59 {
                    return Some((hour, minute));
                }
            }
        }
    }

    let chinese = [
        ("早上", 9_u32),
        ("上午", 9_u32),
        ("中午", 12_u32),
        ("下午", 15_u32),
        ("晚上", 20_u32),
        ("今晚", 20_u32),
    ];
    let base_hour = chinese
        .iter()
        .find_map(|(token, default_hour)| text.contains(token).then_some(*default_hour))
        .unwrap_or(9);

    if let Some(index) = text.find('点') {
        let prefix = &text[..index];
        let digits = prefix
            .chars()
            .rev()
            .take_while(|item| item.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        if let Ok(mut hour) = digits.parse::<u32>() {
            if (text.contains("下午") || text.contains("晚上") || text.contains("今晚"))
                && hour < 12
            {
                hour += 12;
            }
            let suffix = &text[index + '点'.len_utf8()..];
            let minute_digits = suffix
                .chars()
                .take_while(|item| item.is_ascii_digit())
                .collect::<String>();
            let minute = minute_digits.parse::<u32>().unwrap_or(0);
            if hour <= 23 && minute <= 59 {
                return Some((hour, minute));
            }
        }
    }

    Some((base_hour, 0))
}

fn parse_every_minutes(text: &str) -> Option<u32> {
    let markers = ["每", "every "];
    for marker in markers {
        if let Some(idx) = text.find(marker) {
            let tail = &text[idx + marker.len()..];
            let digits = tail
                .chars()
                .take_while(|item| item.is_ascii_digit())
                .collect::<String>();
            if let Ok(value) = digits.parse::<u32>() {
                if text[idx..].contains("分钟") || text[idx..].contains("minutes") {
                    return Some(value.max(1));
                }
            }
        }
    }
    None
}

fn parse_every_hours(text: &str) -> Option<u32> {
    let markers = ["每", "every "];
    for marker in markers {
        if let Some(idx) = text.find(marker) {
            let tail = &text[idx + marker.len()..];
            let digits = tail
                .chars()
                .take_while(|item| item.is_ascii_digit())
                .collect::<String>();
            if let Ok(value) = digits.parse::<u32>() {
                if text[idx..].contains("小时")
                    || text[idx..].contains("hours")
                    || text[idx..].contains("hour")
                {
                    return Some(value.max(1));
                }
            }
        }
    }
    None
}

fn weekly_days_from_text(text: &str) -> Option<Vec<String>> {
    let mut days = Vec::new();
    let mappings = [
        ("周一", "1"),
        ("星期一", "1"),
        ("周二", "2"),
        ("星期二", "2"),
        ("周三", "3"),
        ("星期三", "3"),
        ("周四", "4"),
        ("星期四", "4"),
        ("周五", "5"),
        ("星期五", "5"),
        ("周六", "6"),
        ("星期六", "6"),
        ("周日", "0"),
        ("星期日", "0"),
        ("周天", "0"),
        ("星期天", "0"),
        ("monday", "1"),
        ("tuesday", "2"),
        ("wednesday", "3"),
        ("thursday", "4"),
        ("friday", "5"),
        ("saturday", "6"),
        ("sunday", "0"),
    ];
    for (token, value) in mappings {
        if text.contains(token) && !days.contains(&value.to_string()) {
            days.push(value.to_string());
        }
    }
    if days.is_empty() {
        None
    } else {
        Some(days)
    }
}

fn resolve_datetime_for_day(
    timezone: Tz,
    base_days_from_now: i64,
    hour: u32,
    minute: u32,
) -> Option<chrono::DateTime<Utc>> {
    let base =
        Utc::now().with_timezone(&timezone).date_naive() + Duration::days(base_days_from_now);
    match timezone.with_ymd_and_hms(base.year(), base.month(), base.day(), hour, minute, 0) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn resolve_next_weekday(
    timezone: Tz,
    weekday: Weekday,
    hour: u32,
    minute: u32,
) -> Option<chrono::DateTime<Utc>> {
    let now_local = Utc::now().with_timezone(&timezone);
    let today = now_local.date_naive();
    let today_weekday = today.weekday().num_days_from_monday() as i64;
    let target = weekday.num_days_from_monday() as i64;
    let mut offset = (target - today_weekday).rem_euclid(7);
    if offset == 0 {
        offset = 7;
    }
    resolve_datetime_for_day(timezone, offset, hour, minute)
}

fn extract_task_body(text: &str) -> String {
    let mut output = text.trim().to_string();
    let leading_patterns = [
        "每天",
        "每个工作日",
        "工作日",
        "每周",
        "每隔",
        "每",
        "明天",
        "后天",
        "下周",
        "tomorrow",
        "next week",
        "every ",
        "daily ",
        "weekly ",
    ];
    for pattern in leading_patterns {
        if output.to_ascii_lowercase().starts_with(pattern) || output.starts_with(pattern) {
            if let Some(idx) = output.find('，') {
                let should_strip_on_comma = pattern.is_ascii() || idx <= 24;
                if should_strip_on_comma {
                    if let Some((_, tail)) = output.split_once('，') {
                        output = tail.trim().to_string();
                    }
                    break;
                }
            }
            if let Some(idx) = output.find(',') {
                let should_strip_on_comma = pattern.is_ascii() || idx <= 24;
                if should_strip_on_comma {
                    if let Some((_, tail)) = output.split_once(',') {
                        output = tail.trim().to_string();
                    }
                    break;
                }
            }
            if let Some(idx) = output.find(' ') {
                let should_strip_on_space = pattern.is_ascii() || idx <= 24;
                if should_strip_on_space {
                    if let Some((_, tail)) = output.split_once(' ') {
                        output = tail.trim().to_string();
                    }
                    break;
                }
            }
        }
    }
    if output.is_empty() {
        text.trim().to_string()
    } else {
        output
    }
}

fn infer_payload_kind(
    profile: ScheduledTaskProfile,
    contract: &ScheduledTaskExecutionContract,
) -> ScheduledTaskPayloadKind {
    match profile {
        ScheduledTaskProfile::DocumentTask | ScheduledTaskProfile::HybridTask => {
            ScheduledTaskPayloadKind::DocumentPipeline
        }
        ScheduledTaskProfile::RetrievalTask => ScheduledTaskPayloadKind::RetrievalPipeline,
        ScheduledTaskProfile::WorkspaceTask => {
            if matches!(
                contract.output_mode,
                super::models::ScheduledTaskOutputMode::SummaryAndArtifact
            ) {
                ScheduledTaskPayloadKind::ArtifactTask
            } else {
                ScheduledTaskPayloadKind::SystemSummary
            }
        }
    }
}

fn infer_session_binding(text: &str) -> ScheduledTaskSessionBinding {
    if has_any(
        text,
        &[
            "当前对话",
            "当前会话",
            "本次会话",
            "this chat",
            "current session",
            "当前聊天",
        ],
    ) {
        ScheduledTaskSessionBinding::BoundSession
    } else {
        ScheduledTaskSessionBinding::IsolatedTask
    }
}

fn infer_delivery_plan(contract: &ScheduledTaskExecutionContract) -> ScheduledTaskDeliveryPlanKind {
    match contract.publish_behavior {
        super::models::ScheduledTaskPublishBehavior::PublishWorkspaceArtifact
        | super::models::ScheduledTaskPublishBehavior::CreateDocumentFromFile => {
            ScheduledTaskDeliveryPlanKind::ChannelAndPublish
        }
        super::models::ScheduledTaskPublishBehavior::None => {
            if matches!(
                contract.output_mode,
                super::models::ScheduledTaskOutputMode::SummaryAndArtifact
            ) {
                ScheduledTaskDeliveryPlanKind::ChannelAndArtifact
            } else {
                ScheduledTaskDeliveryPlanKind::ChannelOnly
            }
        }
    }
}

fn infer_title(prompt: &str, profile: ScheduledTaskProfile) -> String {
    let clean = prompt
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let truncated = clean.chars().take(36).collect::<String>();
    if !truncated.is_empty() {
        return truncated;
    }
    match profile {
        ScheduledTaskProfile::DocumentTask => "文档定时任务".to_string(),
        ScheduledTaskProfile::WorkspaceTask => "工作区定时任务".to_string(),
        ScheduledTaskProfile::HybridTask => "混合定时任务".to_string(),
        ScheduledTaskProfile::RetrievalTask => "资讯定时任务".to_string(),
    }
}

pub fn parse_scheduled_task_text(
    text: &str,
    timezone: Option<&str>,
    agent_id: Option<String>,
) -> ScheduledTaskParseResult {
    let timezone = trim_to_none(timezone.map(ToString::to_string))
        .unwrap_or_else(|| "Asia/Shanghai".to_string());
    let tz: Tz = timezone.parse().unwrap_or(chrono_tz::Asia::Shanghai);
    let normalized = normalize_task_text(text);
    let lowered = normalized.to_ascii_lowercase();
    let prompt = extract_task_body(&normalized);
    let task_profile = infer_task_profile_from_prompt(&prompt);
    let temp_task_id = "parse-preview";
    let execution_contract = infer_execution_contract(temp_task_id, &prompt, task_profile);
    let payload_kind = infer_payload_kind(task_profile, &execution_contract);
    let session_binding = infer_session_binding(&normalized);
    let delivery_plan = infer_delivery_plan(&execution_contract);
    let mut warnings = Vec::new();
    let mut task_kind = ScheduledTaskKind::Cron;
    let mut one_shot_at = None;
    let mut schedule_config = None;
    let mut cron_expression = None;
    let mut schedule_kind = ScheduledTaskScheduleSpecKind::Calendar;
    let mut ready_to_create = true;

    if let Some(value) = parse_every_minutes(&lowered) {
        schedule_kind = ScheduledTaskScheduleSpecKind::Every;
        schedule_config = Some(ScheduledTaskScheduleConfig {
            mode: ScheduledTaskScheduleMode::EveryMinutes,
            every_minutes: Some(value),
            every_hours: None,
            daily_time: None,
            weekly_days: None,
            cron_expression: Some(format!("*/{value} * * * *")),
        });
        cron_expression = Some(format!("*/{value} * * * *"));
    } else if let Some(value) = parse_every_hours(&lowered) {
        schedule_kind = ScheduledTaskScheduleSpecKind::Every;
        let expr = if value <= 1 {
            "0 * * * *".to_string()
        } else {
            format!("0 */{value} * * *")
        };
        schedule_config = Some(ScheduledTaskScheduleConfig {
            mode: ScheduledTaskScheduleMode::EveryHours,
            every_minutes: None,
            every_hours: Some(value),
            daily_time: None,
            weekly_days: None,
            cron_expression: Some(expr.clone()),
        });
        cron_expression = Some(expr);
    } else if lowered.contains("工作日") || lowered.contains("weekday") {
        let (hour, minute) = parse_hhmm(&normalized).unwrap_or((9, 0));
        let daily_time = format!("{hour:02}:{minute:02}");
        schedule_kind = ScheduledTaskScheduleSpecKind::Calendar;
        schedule_config = Some(ScheduledTaskScheduleConfig {
            mode: ScheduledTaskScheduleMode::WeekdaysAt,
            every_minutes: None,
            every_hours: None,
            daily_time: Some(daily_time),
            weekly_days: Some(vec![
                "1".to_string(),
                "2".to_string(),
                "3".to_string(),
                "4".to_string(),
                "5".to_string(),
            ]),
            cron_expression: Some(format!("{minute} {hour} * * 1-5")),
        });
        cron_expression = Some(format!("{minute} {hour} * * 1-5"));
    } else if lowered.contains("每周")
        || lowered.contains("weekly")
        || weekly_days_from_text(&lowered).is_some()
    {
        let days = weekly_days_from_text(&lowered).unwrap_or_else(|| vec!["1".to_string()]);
        let (hour, minute) = parse_hhmm(&normalized).unwrap_or((9, 0));
        let daily_time = format!("{hour:02}:{minute:02}");
        schedule_kind = ScheduledTaskScheduleSpecKind::Calendar;
        schedule_config = Some(ScheduledTaskScheduleConfig {
            mode: ScheduledTaskScheduleMode::WeeklyOn,
            every_minutes: None,
            every_hours: None,
            daily_time: Some(daily_time),
            weekly_days: Some(days.clone()),
            cron_expression: Some(format!("{minute} {hour} * * {}", days.join(","))),
        });
        cron_expression = Some(format!("{minute} {hour} * * {}", days.join(",")));
    } else if lowered.contains("每天") || lowered.contains("daily") {
        let (hour, minute) = parse_hhmm(&normalized).unwrap_or((9, 0));
        let daily_time = format!("{hour:02}:{minute:02}");
        schedule_kind = ScheduledTaskScheduleSpecKind::Calendar;
        schedule_config = Some(ScheduledTaskScheduleConfig {
            mode: ScheduledTaskScheduleMode::DailyAt,
            every_minutes: None,
            every_hours: None,
            daily_time: Some(daily_time),
            weekly_days: None,
            cron_expression: Some(format!("{minute} {hour} * * *")),
        });
        cron_expression = Some(format!("{minute} {hour} * * *"));
    } else if lowered.contains("明天")
        || lowered.contains("后天")
        || lowered.contains("tomorrow")
        || lowered.contains("下周")
    {
        task_kind = ScheduledTaskKind::OneShot;
        schedule_kind = ScheduledTaskScheduleSpecKind::OneShot;
        let (hour, minute) = parse_hhmm(&normalized).unwrap_or((9, 0));
        let resolved = if lowered.contains("后天") {
            resolve_datetime_for_day(tz, 2, hour, minute)
        } else if lowered.contains("明天") || lowered.contains("tomorrow") {
            resolve_datetime_for_day(tz, 1, hour, minute)
        } else {
            let target = if lowered.contains("周一") || lowered.contains("monday") {
                Weekday::Mon
            } else if lowered.contains("周二") || lowered.contains("tuesday") {
                Weekday::Tue
            } else if lowered.contains("周三") || lowered.contains("wednesday") {
                Weekday::Wed
            } else if lowered.contains("周四") || lowered.contains("thursday") {
                Weekday::Thu
            } else if lowered.contains("周五") || lowered.contains("friday") {
                Weekday::Fri
            } else if lowered.contains("周六") || lowered.contains("saturday") {
                Weekday::Sat
            } else {
                Weekday::Sun
            };
            resolve_next_weekday(tz, target, hour, minute)
        };
        one_shot_at = resolved.map(|value| value.to_rfc3339());
    } else {
        ready_to_create = false;
        warnings.push(
            "没有识别到明确的执行时间，请补充“每天/每周/几点/每隔多久/明天”等时间描述。"
                .to_string(),
        );
    }

    if matches!(task_profile, ScheduledTaskProfile::RetrievalTask) {
        warnings.push("这是高级模式任务：外部检索结果受来源和工具稳定性影响，建议优先用于摘要而不是强依赖自动化产物。".to_string());
    }
    if matches!(session_binding, ScheduledTaskSessionBinding::BoundSession) {
        warnings.push("任务将绑定当前会话；若会话失效，任务可能自动过期。".to_string());
    }

    let human_schedule = match task_kind {
        ScheduledTaskKind::OneShot => format!(
            "一次性执行：{} ({})",
            one_shot_at.clone().unwrap_or_else(|| "未设置".to_string()),
            timezone
        ),
        ScheduledTaskKind::Cron => {
            if let Some(config) = schedule_config.as_ref() {
                match config.mode {
                    ScheduledTaskScheduleMode::EveryMinutes => format!(
                        "每 {} 分钟执行一次 ({})",
                        config.every_minutes.unwrap_or(15),
                        timezone
                    ),
                    ScheduledTaskScheduleMode::EveryHours => format!(
                        "每 {} 小时整点执行一次 ({})",
                        config.every_hours.unwrap_or(1),
                        timezone
                    ),
                    ScheduledTaskScheduleMode::DailyAt => format!(
                        "每天 {} 执行 ({})",
                        config
                            .daily_time
                            .clone()
                            .unwrap_or_else(|| "09:00".to_string()),
                        timezone
                    ),
                    ScheduledTaskScheduleMode::WeekdaysAt => format!(
                        "每个工作日 {} 执行 ({})",
                        config
                            .daily_time
                            .clone()
                            .unwrap_or_else(|| "09:00".to_string()),
                        timezone
                    ),
                    ScheduledTaskScheduleMode::WeeklyOn => format!(
                        "每周 {} {} 执行 ({})",
                        config.weekly_days.clone().unwrap_or_default().join("、"),
                        config
                            .daily_time
                            .clone()
                            .unwrap_or_else(|| "09:00".to_string()),
                        timezone
                    ),
                    ScheduledTaskScheduleMode::Custom => format!(
                        "自定义周期任务：{} ({})",
                        config
                            .cron_expression
                            .clone()
                            .unwrap_or_else(|| cron_expression.clone().unwrap_or_default()),
                        timezone
                    ),
                }
            } else {
                format!("周期任务（{}）", timezone)
            }
        }
    };

    let schedule_spec = ScheduledTaskScheduleSpec {
        kind: schedule_kind,
        one_shot_at,
        schedule_config,
        cron_expression,
        timezone: timezone.clone(),
    };
    let confidence = if ready_to_create {
        if matches!(task_profile, ScheduledTaskProfile::RetrievalTask) {
            0.74
        } else {
            0.91
        }
    } else {
        0.42
    };

    ScheduledTaskParseResult {
        title: infer_title(&prompt, task_profile),
        prompt,
        task_kind,
        task_profile,
        payload_kind,
        session_binding,
        delivery_plan,
        schedule_spec,
        execution_contract,
        human_schedule,
        warnings,
        advanced_mode: matches!(task_profile, ScheduledTaskProfile::RetrievalTask),
        confidence,
        ready_to_create,
        agent_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_daily_document_task() {
        let parsed = parse_scheduled_task_text(
            "每天早上9点读取团队文档并生成一份md总结",
            Some("Asia/Shanghai"),
            Some("agent-1".to_string()),
        );
        assert!(parsed.ready_to_create);
        assert_eq!(parsed.task_kind, ScheduledTaskKind::Cron);
        assert_eq!(parsed.task_profile, ScheduledTaskProfile::HybridTask);
        assert_eq!(
            parsed.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Calendar
        );
        assert_eq!(
            parsed
                .schedule_spec
                .schedule_config
                .as_ref()
                .map(|c| c.mode),
            Some(ScheduledTaskScheduleMode::DailyAt)
        );
    }

    #[test]
    fn parses_every_minutes_workspace_task() {
        let parsed = parse_scheduled_task_text(
            "每15分钟生成一份项目状态报告到工作区",
            Some("Asia/Shanghai"),
            Some("agent-1".to_string()),
        );
        assert!(parsed.ready_to_create);
        assert_eq!(parsed.task_profile, ScheduledTaskProfile::WorkspaceTask);
        assert_eq!(
            parsed.schedule_spec.kind,
            ScheduledTaskScheduleSpecKind::Every
        );
        assert_eq!(
            parsed
                .schedule_spec
                .schedule_config
                .as_ref()
                .and_then(|c| c.every_minutes),
            Some(15)
        );
    }

    #[test]
    fn marks_retrieval_task_as_advanced_mode() {
        let parsed = parse_scheduled_task_text(
            "每天早上8点搜索互联网最新AI资讯并汇总",
            Some("Asia/Shanghai"),
            Some("agent-1".to_string()),
        );
        assert!(parsed.ready_to_create);
        assert_eq!(parsed.task_profile, ScheduledTaskProfile::RetrievalTask);
        assert!(parsed.advanced_mode);
        assert!(!parsed.warnings.is_empty());
    }

    #[test]
    fn returns_warning_when_schedule_missing() {
        let parsed = parse_scheduled_task_text(
            "帮我总结团队文档变化",
            Some("Asia/Shanghai"),
            Some("agent-1".to_string()),
        );
        assert!(!parsed.ready_to_create);
        assert!(!parsed.warnings.is_empty());
    }

    #[test]
    fn keeps_chinese_task_body_when_only_trailing_space_exists() {
        let parsed = parse_scheduled_task_text(
            "每天早上9点读取团队文档变化并生成一份 md 报告到工作区 marker-123",
            Some("Asia/Shanghai"),
            Some("agent-1".to_string()),
        );
        assert!(parsed.prompt.contains("团队文档变化"));
        assert!(matches!(
            parsed.task_profile,
            ScheduledTaskProfile::DocumentTask | ScheduledTaskProfile::HybridTask
        ));
    }
}
