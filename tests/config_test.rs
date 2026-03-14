use flowstate_runner::config::Config;
use std::io::Write;
use tempfile::TempDir;

fn write_config(dir: &std::path::Path, content: &str) {
    let flowstate_dir = dir.join(".flowstate");
    std::fs::create_dir_all(&flowstate_dir).unwrap();
    let mut f = std::fs::File::create(flowstate_dir.join("config.json")).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

#[test]
fn test_config_loads_from_file() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"{
            "orgId": "org_9f3omFEY2H",
            "workspaceId": "work_ojk4TWK5D2",
            "version": "1.0.0",
            "projectName": "test"
        }"#,
    );

    let config = Config::load(dir.path()).unwrap();
    assert_eq!(config.org_id, "org_9f3omFEY2H");
    assert_eq!(config.workspace_id, "work_ojk4TWK5D2");
    assert_eq!(config.rest_base_url, "http://localhost:7080");
    assert!(!config.worker_mode);
    assert_eq!(config.health_port, 9090);
    assert_eq!(config.max_subprocess_depth, 5);
}

#[test]
fn test_config_missing_org_id_fails() {
    let dir = TempDir::new().unwrap();
    write_config(dir.path(), r#"{ "workspaceId": "work_ojk4TWK5D2" }"#);

    let result = Config::load(dir.path());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("orgId"), "Error should mention orgId: {err}");
}

#[test]
fn test_config_missing_file_fails() {
    let dir = TempDir::new().unwrap();
    let result = Config::load(dir.path());
    assert!(result.is_err());
}

#[test]
fn test_config_env_overrides() {
    let dir = TempDir::new().unwrap();
    write_config(dir.path(), r#"{ "orgId": "org_9f3omFEY2H" }"#);

    std::env::set_var("FLOWSTATE_REST_URL", "http://custom:8080");
    std::env::set_var("WORKER_MODE", "true");
    std::env::set_var("HEALTH_PORT", "9999");

    let config = Config::load(dir.path()).unwrap();
    assert_eq!(config.rest_base_url, "http://custom:8080");
    assert!(config.worker_mode);
    assert_eq!(config.health_port, 9999);

    std::env::remove_var("FLOWSTATE_REST_URL");
    std::env::remove_var("WORKER_MODE");
    std::env::remove_var("HEALTH_PORT");
}

#[test]
fn test_config_workspace_defaults_empty() {
    let dir = TempDir::new().unwrap();
    write_config(dir.path(), r#"{ "orgId": "org_9f3omFEY2H" }"#);

    let config = Config::load(dir.path()).unwrap();
    assert_eq!(config.workspace_id, "");
}
