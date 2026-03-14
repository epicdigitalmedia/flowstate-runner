// tests/cli_test.rs
use clap::Parser;
use flowstate_runner::cli::{Cli, Command};

#[test]
fn test_cli_scan_subcommand() {
    let cli = Cli::try_parse_from(["flowstate-runner", "scan"]).unwrap();
    assert!(matches!(cli.command, Command::Scan));
}

#[test]
fn test_cli_resume_subcommand() {
    let cli = Cli::try_parse_from(["flowstate-runner", "resume"]).unwrap();
    assert!(matches!(cli.command, Command::Resume));
}

#[test]
fn test_cli_run_subcommand_with_id() {
    let cli = Cli::try_parse_from(["flowstate-runner", "run", "exec_abc123"]).unwrap();
    match cli.command {
        Command::Run { execution_id } => assert_eq!(execution_id, "exec_abc123"),
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_cli_run_subcommand_requires_id() {
    let result = Cli::try_parse_from(["flowstate-runner", "run"]);
    assert!(result.is_err(), "run requires execution_id argument");
}

#[test]
fn test_cli_daemon_subcommand_default_interval() {
    let cli = Cli::try_parse_from(["flowstate-runner", "daemon"]).unwrap();
    match cli.command {
        Command::Daemon { interval } => assert_eq!(interval, 60),
        _ => panic!("Expected Daemon command"),
    }
}

#[test]
fn test_cli_daemon_subcommand_custom_interval() {
    let cli = Cli::try_parse_from(["flowstate-runner", "daemon", "--interval", "30"]).unwrap();
    match cli.command {
        Command::Daemon { interval } => assert_eq!(interval, 30),
        _ => panic!("Expected Daemon command"),
    }
}

#[test]
fn test_cli_project_root_option() {
    let cli = Cli::try_parse_from(["flowstate-runner", "--project-root", "/custom/path", "scan"])
        .unwrap();
    assert_eq!(cli.project_root, std::path::PathBuf::from("/custom/path"));
}

#[test]
fn test_cli_project_root_defaults_to_cwd() {
    let cli = Cli::try_parse_from(["flowstate-runner", "scan"]).unwrap();
    assert_eq!(cli.project_root, std::path::PathBuf::from("."));
}

#[test]
fn test_cli_no_subcommand_fails() {
    let result = Cli::try_parse_from(["flowstate-runner"]);
    assert!(result.is_err(), "Should require a subcommand");
}
