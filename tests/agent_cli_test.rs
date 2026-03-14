/// Tests for the pure functions in `agent::claude_cli`.
///
/// The executor's process-spawning path is not covered here — that requires
/// an integration environment with the `claude` binary present.  These tests
/// exercise the parsing, extraction, and command-building logic that is
/// fully deterministic.
use flowstate_runner::agent::claude_cli::{
    build_command_args, extract_facts, extract_metrics, parse_jsonl_line,
};
use flowstate_runner::agent::AgentEvent;
use flowstate_runner::models::agent::AgentConfig;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_config() -> AgentConfig {
    AgentConfig {
        agent_name: None,
        provider: None,
        model: None,
        timeout: None,
        memory_context: None,
        working_dir: None,
        permission_mode: None,
        team_member_id: None,
    }
}

// ---------------------------------------------------------------------------
// parse_jsonl_line
// ---------------------------------------------------------------------------

#[test]
fn parse_empty_line_returns_none() {
    assert!(parse_jsonl_line("").is_none());
}

#[test]
fn parse_whitespace_only_returns_none() {
    assert!(parse_jsonl_line("   \t  ").is_none());
}

#[test]
fn parse_invalid_json_returns_none() {
    assert!(parse_jsonl_line("{not json}").is_none());
    assert!(parse_jsonl_line("plain text").is_none());
}

#[test]
fn parse_start_event() {
    let line = r#"{"type":"system","subtype":"init","model":"claude-sonnet-4-20250514"}"#;
    let event = parse_jsonl_line(line).expect("should parse start");
    match event {
        AgentEvent::Start { model } => {
            assert_eq!(model, "claude-sonnet-4-20250514");
        }
        other => panic!("expected Start, got {:?}", other),
    }
}

#[test]
fn parse_start_event_without_model_uses_empty_string() {
    let line = r#"{"type":"system","subtype":"init"}"#;
    let event = parse_jsonl_line(line).expect("should parse start without model");
    match event {
        AgentEvent::Start { model } => {
            assert_eq!(model, "", "missing model should default to empty string");
        }
        other => panic!("expected Start, got {:?}", other),
    }
}

#[test]
fn parse_tool_use_event() {
    let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/test.rs"}}]}}"#;
    let event = parse_jsonl_line(line).expect("should parse tool_use");
    match event {
        AgentEvent::ToolUse { tool, input } => {
            assert_eq!(tool, "Read");
            assert_eq!(input, serde_json::json!({"file_path": "/tmp/test.rs"}));
        }
        other => panic!("expected ToolUse, got {:?}", other),
    }
}

#[test]
fn parse_text_event() {
    let line =
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello world"}]}}"#;
    let event = parse_jsonl_line(line).expect("should parse text");
    match event {
        AgentEvent::Text { content } => {
            assert_eq!(content, "Hello world");
        }
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn parse_complete_event_with_cost() {
    let line = r#"{"type":"result","subtype":"success","cost_usd":0.042}"#;
    let event = parse_jsonl_line(line).expect("should parse complete");
    match event {
        AgentEvent::Complete { cost } => {
            assert!((cost.unwrap() - 0.042).abs() < 1e-9, "cost should be 0.042");
        }
        other => panic!("expected Complete, got {:?}", other),
    }
}

#[test]
fn parse_complete_event_without_cost() {
    let line = r#"{"type":"result","subtype":"error_max_turns"}"#;
    let event = parse_jsonl_line(line).expect("should parse complete without cost");
    match event {
        AgentEvent::Complete { cost } => {
            assert!(cost.is_none(), "cost should be None when absent");
        }
        other => panic!("expected Complete, got {:?}", other),
    }
}

#[test]
fn parse_error_event() {
    let line = r#"{"type":"error","error":{"message":"Rate limit exceeded"}}"#;
    let event = parse_jsonl_line(line).expect("should parse error");
    match event {
        AgentEvent::Error { message } => {
            assert_eq!(message, "Rate limit exceeded");
        }
        other => panic!("expected Error, got {:?}", other),
    }
}

#[test]
fn parse_unknown_type_returns_unknown_event() {
    let line = r#"{"type":"debug","payload":"something unexpected"}"#;
    let event = parse_jsonl_line(line).expect("should parse unknown");
    match event {
        AgentEvent::Unknown { raw } => {
            assert_eq!(raw.get("type").and_then(|v| v.as_str()), Some("debug"));
        }
        other => panic!("expected Unknown, got {:?}", other),
    }
}

#[test]
fn parse_json_without_type_field_returns_unknown() {
    let line = r#"{"payload":"no type here","value":42}"#;
    let event = parse_jsonl_line(line).expect("should parse as Unknown");
    assert!(
        matches!(event, AgentEvent::Unknown { .. }),
        "JSON without 'type' should be Unknown"
    );
}

// ---------------------------------------------------------------------------
// extract_metrics
// ---------------------------------------------------------------------------

#[test]
fn extract_metrics_empty_events() {
    let metrics = extract_metrics(&[]);
    assert_eq!(metrics.input_tokens, 0);
    assert_eq!(metrics.output_tokens, 0);
    assert_eq!(metrics.cache_read_tokens, 0);
    assert!(metrics.model.is_none());
    assert!(metrics.cost.is_none());
}

#[test]
fn extract_metrics_from_start_and_complete() {
    let events = vec![
        AgentEvent::Start {
            model: "claude-opus-4-20250514".to_string(),
        },
        AgentEvent::Text {
            content: "some text".to_string(),
        },
        AgentEvent::Complete { cost: Some(0.12) },
    ];
    let metrics = extract_metrics(&events);
    assert_eq!(metrics.model, Some("claude-opus-4-20250514".to_string()));
    assert!((metrics.cost.unwrap() - 0.12).abs() < 1e-9);
}

#[test]
fn extract_metrics_token_counts_from_unknown_events() {
    let events = vec![
        AgentEvent::Start {
            model: "claude-sonnet-4-20250514".to_string(),
        },
        AgentEvent::Unknown {
            raw: serde_json::json!({
                "type": "usage",
                "usage": {
                    "input_tokens": 1500,
                    "output_tokens": 300,
                    "cache_read_input_tokens": 200
                }
            }),
        },
        AgentEvent::Complete { cost: Some(0.05) },
    ];
    let metrics = extract_metrics(&events);
    assert_eq!(metrics.input_tokens, 1500);
    assert_eq!(metrics.output_tokens, 300);
    assert_eq!(metrics.cache_read_tokens, 200);
}

#[test]
fn extract_metrics_accumulates_multiple_usage_events() {
    let events = vec![
        AgentEvent::Unknown {
            raw: serde_json::json!({
                "type": "usage",
                "usage": {"input_tokens": 100, "output_tokens": 50, "cache_read_input_tokens": 0}
            }),
        },
        AgentEvent::Unknown {
            raw: serde_json::json!({
                "type": "usage",
                "usage": {"input_tokens": 200, "output_tokens": 75, "cache_read_input_tokens": 10}
            }),
        },
    ];
    let metrics = extract_metrics(&events);
    assert_eq!(metrics.input_tokens, 300);
    assert_eq!(metrics.output_tokens, 125);
    assert_eq!(metrics.cache_read_tokens, 10);
}

#[test]
fn extract_metrics_uses_first_model_only() {
    let events = vec![
        AgentEvent::Start {
            model: "first-model".to_string(),
        },
        AgentEvent::Start {
            model: "second-model".to_string(),
        },
    ];
    let metrics = extract_metrics(&events);
    assert_eq!(metrics.model, Some("first-model".to_string()));
}

#[test]
fn extract_metrics_empty_model_string_not_stored() {
    // A Start event with empty model string should leave model as None
    let events = vec![AgentEvent::Start {
        model: "".to_string(),
    }];
    let metrics = extract_metrics(&events);
    assert!(
        metrics.model.is_none(),
        "empty model string should not be stored"
    );
}

// ---------------------------------------------------------------------------
// extract_facts
// ---------------------------------------------------------------------------

#[test]
fn extract_facts_empty_events() {
    let (files, tools) = extract_facts(&[]);
    assert!(files.is_empty());
    assert!(tools.is_empty());
}

#[test]
fn extract_facts_write_and_edit_count_as_file_modifications() {
    let events = vec![
        AgentEvent::ToolUse {
            tool: "Write".to_string(),
            input: serde_json::json!({"file_path": "/src/main.rs"}),
        },
        AgentEvent::ToolUse {
            tool: "Edit".to_string(),
            input: serde_json::json!({"file_path": "/src/lib.rs"}),
        },
        AgentEvent::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "/src/helper.rs"}),
        },
    ];
    let (files, tools) = extract_facts(&events);

    assert_eq!(files, vec!["/src/lib.rs", "/src/main.rs"]);
    assert_eq!(tools, vec!["Edit", "Read", "Write"]);
}

#[test]
fn extract_facts_read_does_not_count_as_file_modification() {
    let events = vec![AgentEvent::ToolUse {
        tool: "Read".to_string(),
        input: serde_json::json!({"file_path": "/src/something.rs"}),
    }];
    let (files, _tools) = extract_facts(&events);
    assert!(
        files.is_empty(),
        "Read tool should not appear in files_modified"
    );
}

#[test]
fn extract_facts_deduplicates_files_and_tools() {
    let events = vec![
        AgentEvent::ToolUse {
            tool: "Write".to_string(),
            input: serde_json::json!({"file_path": "/src/foo.rs"}),
        },
        AgentEvent::ToolUse {
            tool: "Write".to_string(),
            input: serde_json::json!({"file_path": "/src/foo.rs"}),
        },
        AgentEvent::ToolUse {
            tool: "Edit".to_string(),
            input: serde_json::json!({"file_path": "/src/foo.rs"}),
        },
    ];
    let (files, tools) = extract_facts(&events);
    assert_eq!(files, vec!["/src/foo.rs"]);
    assert_eq!(tools, vec!["Edit", "Write"]);
}

#[test]
fn extract_facts_results_are_sorted() {
    let events = vec![
        AgentEvent::ToolUse {
            tool: "Write".to_string(),
            input: serde_json::json!({"file_path": "/z_last.rs"}),
        },
        AgentEvent::ToolUse {
            tool: "Edit".to_string(),
            input: serde_json::json!({"file_path": "/a_first.rs"}),
        },
        AgentEvent::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "/m_middle.rs"}),
        },
    ];
    let (files, tools) = extract_facts(&events);
    assert_eq!(files, vec!["/a_first.rs", "/z_last.rs"]);
    assert_eq!(tools, vec!["Edit", "Read", "Write"]);
}

// ---------------------------------------------------------------------------
// build_command_args
// ---------------------------------------------------------------------------

#[test]
fn build_command_args_default_config() {
    let config = default_config();
    let args = build_command_args("do something", &config);

    // Must include --output-format stream-json
    let fmt_pos = args
        .iter()
        .position(|a| a == "--output-format")
        .expect("--output-format flag must be present");
    assert_eq!(args[fmt_pos + 1], "stream-json");

    // Must include --verbose
    assert!(args.contains(&"--verbose".to_string()));

    // Default permission mode: bypassPermissions
    let pm_pos = args
        .iter()
        .position(|a| a == "--permission-mode")
        .expect("--permission-mode must be present");
    assert_eq!(args[pm_pos + 1], "bypassPermissions");

    // Prompt passed via --print
    let print_pos = args
        .iter()
        .position(|a| a == "--print")
        .expect("--print must be present");
    assert_eq!(args[print_pos + 1], "do something");

    // No --model or --agent without config
    assert!(!args.contains(&"--model".to_string()));
    assert!(!args.contains(&"--agent".to_string()));
}

#[test]
fn build_command_args_with_model() {
    let config = AgentConfig {
        model: Some("claude-opus-4-20250514".to_string()),
        ..default_config()
    };
    let args = build_command_args("prompt", &config);

    let model_pos = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model must be present when config.model is set");
    assert_eq!(args[model_pos + 1], "claude-opus-4-20250514");
}

#[test]
fn build_command_args_with_agent_name() {
    let config = AgentConfig {
        agent_name: Some("viktor".to_string()),
        ..default_config()
    };
    let args = build_command_args("prompt", &config);

    let agent_pos = args
        .iter()
        .position(|a| a == "--agent")
        .expect("--agent must be present when config.agent_name is set");
    assert_eq!(args[agent_pos + 1], "viktor");
}

#[test]
fn build_command_args_with_custom_permission_mode() {
    let config = AgentConfig {
        permission_mode: Some("default".to_string()),
        ..default_config()
    };
    let args = build_command_args("prompt", &config);

    let pm_pos = args
        .iter()
        .position(|a| a == "--permission-mode")
        .expect("--permission-mode must be present");
    assert_eq!(args[pm_pos + 1], "default");
}

#[test]
fn build_command_args_prompt_is_last_content_after_print() {
    let prompt = "multi word prompt with spaces";
    let config = default_config();
    let args = build_command_args(prompt, &config);

    let print_pos = args
        .iter()
        .position(|a| a == "--print")
        .expect("--print must be present");
    assert_eq!(
        args[print_pos + 1],
        prompt,
        "prompt must follow --print as a single argument"
    );
}
