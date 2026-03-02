use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct PromptCatalog {
    prompts_dir: Option<PathBuf>,
}

impl PromptCatalog {
    pub fn from_prompts_dir(prompts_dir: Option<&str>) -> Self {
        let prompts_dir = prompts_dir
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        Self { prompts_dir }
    }

    pub fn load_prompt(&self, id: &str) -> Option<String> {
        let dir = self.prompts_dir.as_ref()?;
        let path = dir.join(format!("{id}.md"));
        let raw = fs::read_to_string(path).ok()?;
        let body = extract_markdown_body(raw.as_str());
        let trimmed = body.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    pub fn load_lines_prompt(&self, id: &str) -> Option<Vec<String>> {
        let text = self.load_prompt(id)?;
        let lines = text
            .lines()
            .map(normalize_prompt_rule_line)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        (!lines.is_empty()).then_some(lines)
    }

    pub fn load_json_prompt(&self, id: &str) -> Option<serde_json::Value> {
        let text = self.load_prompt(id)?;
        serde_json::from_str::<serde_json::Value>(text.as_str()).ok()
    }
}

fn extract_markdown_body(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---\n") {
        return raw.to_string();
    }
    let mut lines = trimmed.lines();
    let _ = lines.next();
    let mut in_frontmatter = true;
    let mut body = Vec::<&str>::new();
    for line in lines {
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
            continue;
        }
        if !in_frontmatter {
            body.push(line);
        }
    }
    if in_frontmatter {
        raw.to_string()
    } else {
        body.join("\n")
    }
}

fn normalize_prompt_rule_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return String::new();
    }
    let without_bullet = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .unwrap_or(trimmed);
    let without_numbered = strip_numbered_prefix(without_bullet).unwrap_or(without_bullet);
    without_numbered.trim().to_string()
}

fn strip_numbered_prefix(input: &str) -> Option<&str> {
    let mut parts = input.splitn(2, '.');
    let head = parts.next()?;
    let rest = parts.next()?;
    if head.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(rest.trim_start());
    }
    None
}

#[cfg(test)]
#[path = "tests/prompts.rs"]
mod tests;
