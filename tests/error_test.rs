use flowstate_runner::error::RunnerError;

#[test]
fn test_runner_error_display_config() {
    let err = RunnerError::Config("missing org_id".to_string());
    assert!(err.to_string().contains("missing org_id"));
}

#[test]
fn test_runner_error_display_template() {
    let err = RunnerError::Template("unresolved variable: foo".to_string());
    assert!(err.to_string().contains("unresolved variable: foo"));
}

#[test]
fn test_runner_error_display_condition() {
    let err = RunnerError::Condition("invalid operator".to_string());
    assert!(err.to_string().contains("invalid operator"));
}

#[test]
fn test_runner_error_display_output() {
    let err = RunnerError::Output("json_path not found".to_string());
    assert!(err.to_string().contains("json_path not found"));
}

#[test]
fn test_runner_error_display_handler() {
    let err = RunnerError::Handler("unknown step type: foo".to_string());
    assert!(err.to_string().contains("unknown step type: foo"));
}

#[test]
fn test_runner_error_display_executor() {
    let err = RunnerError::Executor("no current step".to_string());
    assert!(err.to_string().contains("no current step"));
}

#[test]
fn test_runner_error_display_state() {
    let err = RunnerError::State("invalid status transition".to_string());
    assert!(err.to_string().contains("invalid status transition"));
}

#[test]
fn test_runner_error_display_rest() {
    let err = RunnerError::Rest("connection refused".to_string());
    assert!(err.to_string().contains("connection refused"));
}

#[test]
fn test_runner_error_display_mcp() {
    let err = RunnerError::Mcp("connection timeout".to_string());
    assert!(err.to_string().contains("connection timeout"));
}

#[test]
fn test_runner_error_display_serialization() {
    let err = RunnerError::Serialization("invalid JSON".to_string());
    assert!(err.to_string().contains("invalid JSON"));
}

#[test]
fn test_runner_error_display_agent() {
    let err = RunnerError::Agent("claude invocation failed".to_string());
    assert!(err.to_string().contains("claude invocation failed"));
}

#[test]
fn test_runner_error_display_subprocess() {
    let err = RunnerError::Subprocess("child process exited 1".to_string());
    assert!(err.to_string().contains("child process exited 1"));
}

#[test]
fn test_runner_error_display_io() {
    let err = RunnerError::Io("file not found".to_string());
    assert!(err.to_string().contains("file not found"));
}
