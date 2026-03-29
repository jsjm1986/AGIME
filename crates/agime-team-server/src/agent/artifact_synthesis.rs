use super::mission_mongo::MissionDoc;

pub fn artifact_synthesis_supported_target(target: &str) -> bool {
    let lower = target.trim().replace('\\', "/").to_ascii_lowercase();
    [
        ".html", ".md", ".txt", ".json", ".csv", ".py", ".sh", ".js", ".ts", ".yaml", ".yml",
        ".toml",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}

pub fn extract_synthesized_artifact_content(response: &str) -> Option<String> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            let mut lines = inner.lines();
            let first = lines.next().unwrap_or_default().trim();
            let body = if !first.is_empty()
                && !first.contains(' ')
                && first.len() <= 16
                && !first.contains('<')
                && !first.contains('#')
            {
                lines.collect::<Vec<_>>().join("\n")
            } else {
                inner.to_string()
            };
            let body = body.trim().to_string();
            if !body.is_empty() {
                return Some(body);
            }
        }
    }
    Some(trimmed.to_string())
}

pub fn compact_artifact_input_text(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut collected = String::new();
    for ch in content.chars().take(max_chars) {
        collected.push(ch);
    }
    collected.push_str("\n...[truncated]...");
    collected
}

pub fn build_artifact_synthesis_prompt(
    mission: &MissionDoc,
    target: &str,
    inputs: &[(String, String)],
    task_label: &str,
    require_tools: bool,
) -> String {
    let mut prompt = format!(
        "You are performing a bounded artifact synthesis step for a mission.\n\
Goal: {}\n\
Task label: {}\n\
Target file: {}\n\
Rules:\n\
- Do not explain your reasoning.\n\
- Do not ask for clarification.\n\
- Use the provided input artifacts to generate the target artifact.\n\
- Return only the final file content, preferably inside one fenced code block.\n\
- Do not return placeholders, TODOs, draft markers, or commentary.\n",
        mission.goal, task_label, target
    );
    if require_tools {
        prompt.push_str("- If you can use workspace-aware tools before returning content, do so.\n");
    }
    if !inputs.is_empty() {
        prompt.push_str("\nInput artifacts:\n");
        for (path, content) in inputs {
            prompt.push_str(&format!(
                "\n### {}\n```text\n{}\n```\n",
                path,
                compact_artifact_input_text(content, 6000)
            ));
        }
    }
    prompt
}
