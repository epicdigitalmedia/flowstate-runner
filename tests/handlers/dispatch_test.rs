use flowstate_runner::handlers::dispatch_handler;

#[test]
fn test_all_step_types_dispatch() {
    let step_types = [
        "start",
        "end",
        "action",
        "script",
        "api-call",
        "decision",
        "delay",
        "notification",
        "agent-task",
        "approval",
        "human-task",
        "subprocess",
        "parallel-gateway",
        "join-gateway",
    ];

    for step_type in &step_types {
        let result = dispatch_handler(step_type);
        assert!(
            result.is_ok(),
            "dispatch_handler('{}') should succeed",
            step_type
        );
    }
}

#[test]
fn test_unknown_step_type_fails() {
    let result = dispatch_handler("nonexistent");
    assert!(result.is_err());
}
