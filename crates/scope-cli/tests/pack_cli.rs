use std::process::Command;

#[test]
fn pack_invalid_target_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["pack", "does::not::exist", "--change-type", "rename", "--budget", "120"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}
