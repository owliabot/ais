use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn load_prompt_strips_frontmatter() {
    let root = temp_dir("prompt-catalog-frontmatter");
    let path = root.join("agent.controller.system.md");
    fs::write(
        &path,
        "---\nid: agent.controller.system\n---\n# Title\nDo X.\n",
    )
    .expect("write");
    let catalog = PromptCatalog::from_prompts_dir(Some(root.to_string_lossy().as_ref()));
    let loaded = catalog
        .load_prompt("agent.controller.system")
        .expect("load");
    assert!(loaded.contains("Do X."));
    assert!(!loaded.contains("id:"));
}

#[test]
fn load_lines_prompt_normalizes_bullets() {
    let root = temp_dir("prompt-catalog-lines");
    let path = root.join("segmented.base_rules.md");
    fs::write(&path, "## rules\n- first\n* second\n1. third\nnot-list\n").expect("write");
    let catalog = PromptCatalog::from_prompts_dir(Some(root.to_string_lossy().as_ref()));
    let lines = catalog
        .load_lines_prompt("segmented.base_rules")
        .expect("lines");
    assert_eq!(lines, vec!["first", "second", "third", "not-list"]);
}

#[test]
fn load_json_prompt_parses_json_body() {
    let root = temp_dir("prompt-catalog-json");
    let path = root.join("segmented.segment.patch.md");
    fs::write(&path, r#"{"segment_contract":{"notes":"override"}}"#).expect("write");
    let catalog = PromptCatalog::from_prompts_dir(Some(root.to_string_lossy().as_ref()));
    let value = catalog
        .load_json_prompt("segmented.segment.patch")
        .expect("json");
    assert_eq!(
        value.pointer("/segment_contract/notes"),
        Some(&serde_json::json!("override"))
    );
}

#[test]
fn operator_template_catalog_loads_markdown_body() {
    let root = temp_dir("operator-template-catalog");
    let path = root.join("operator.output.summary.md");
    fs::write(
        &path,
        "---\nid: operator.output.summary\n---\nstatus={{status}}\n",
    )
    .expect("write");
    let catalog =
        OperatorTemplateCatalog::from_templates_dir(Some(root.to_string_lossy().as_ref()));
    let value = catalog
        .load_template("operator.output.summary")
        .expect("template");
    assert_eq!(value.trim(), "status={{status}}");
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
    fs::create_dir_all(&path).expect("mkdir");
    path
}
