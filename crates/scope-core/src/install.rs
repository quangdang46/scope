use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{json, Value};

use crate::{ScopeError, ScopeResult};

const USER_SCOPE_HOSTS: &[&str] = &[
    "claude-code",
    "cursor",
    "windsurf",
    "claude-desktop",
    "opencode",
    "gemini",
    "codex",
    "amp",
    "droid",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpInstallStatus {
    Installed,
    Updated,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpInstallHostResult {
    pub host: String,
    pub path: String,
    pub status: McpInstallStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpInstallReport {
    pub mode: String,
    pub server_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub configured_count: usize,
    pub failed_count: usize,
    pub results: Vec<McpInstallHostResult>,
}

#[derive(Debug)]
enum ConfigFormat {
    Json { servers_key: &'static str },
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallWriteStatus {
    Installed,
    Updated,
    Unchanged,
}

#[derive(Debug)]
struct HostInfo {
    name: &'static str,
    path: PathBuf,
    format: ConfigFormat,
}

pub fn supported_install_hosts() -> &'static [&'static str] {
    USER_SCOPE_HOSTS
}

pub fn install_mcp_for_hosts(hosts: &[String]) -> ScopeResult<McpInstallReport> {
    if hosts.is_empty() {
        return Err(ScopeError::InvalidInput(format!(
            "install-mcp requires at least one --host; supported hosts: {}",
            USER_SCOPE_HOSTS.join(", ")
        )));
    }

    let command_and_args = scope_mcp_command_and_args();
    let mut results = Vec::new();

    for host in hosts {
        let info = resolve_host(host)?;
        let (status, reason) = install_host(&info)?;
        results.push(McpInstallHostResult {
            host: info.name.to_string(),
            path: info.path.display().to_string(),
            status,
            reason,
        });
    }

    Ok(build_report("manual", command_and_args, results))
}

pub fn auto_install_user_mcp() -> ScopeResult<McpInstallReport> {
    let command_and_args = scope_mcp_command_and_args();
    let mut results = Vec::new();

    for host in USER_SCOPE_HOSTS {
        let info = match resolve_host(host) {
            Ok(info) => info,
            Err(error) => {
                results.push(McpInstallHostResult {
                    host: (*host).to_string(),
                    path: String::new(),
                    status: McpInstallStatus::Skipped,
                    reason: Some(error.to_string()),
                });
                continue;
            }
        };
        let path_display = info.path.display().to_string();
        match detect_host(&info)? {
            Some(_) => match install_host(&info) {
                Ok((status, reason)) => results.push(McpInstallHostResult {
                    host: info.name.to_string(),
                    path: path_display,
                    status,
                    reason,
                }),
                Err(error) => results.push(McpInstallHostResult {
                    host: info.name.to_string(),
                    path: path_display,
                    status: McpInstallStatus::Failed,
                    reason: Some(error.to_string()),
                }),
            },
            None => results.push(McpInstallHostResult {
                host: info.name.to_string(),
                path: path_display,
                status: McpInstallStatus::Skipped,
                reason: Some("host not detected on this machine".to_string()),
            }),
        }
    }

    Ok(build_report("auto_user", command_and_args, results))
}

fn build_report(
    mode: &str,
    (command, args): (String, Vec<String>),
    results: Vec<McpInstallHostResult>,
) -> McpInstallReport {
    let configured_count = results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                McpInstallStatus::Installed | McpInstallStatus::Updated
            ) || matches!(
                (result.status.clone(), result.reason.as_deref()),
                (McpInstallStatus::Skipped, Some("already configured"))
            )
        })
        .count();
    let failed_count = results
        .iter()
        .filter(|result| matches!(result.status, McpInstallStatus::Failed))
        .count();

    McpInstallReport {
        mode: mode.to_string(),
        server_name: "scope".to_string(),
        command,
        args,
        configured_count,
        failed_count,
        results,
    }
}

fn install_host(host: &HostInfo) -> ScopeResult<(McpInstallStatus, Option<String>)> {
    if let Some(parent) = host.path.parent() {
        fs::create_dir_all(parent).map_err(|error| ScopeError::io(parent, error))?;
    }

    let status = match host.format {
        ConfigFormat::Json { servers_key } => write_json_config(host, servers_key)?,
        ConfigFormat::Toml => write_toml_config(host)?,
    };

    Ok(match status {
        InstallWriteStatus::Installed => (McpInstallStatus::Installed, None),
        InstallWriteStatus::Updated => (McpInstallStatus::Updated, None),
        InstallWriteStatus::Unchanged => (
            McpInstallStatus::Skipped,
            Some("already configured".to_string()),
        ),
    })
}

fn write_json_config(host: &HostInfo, servers_key: &str) -> ScopeResult<InstallWriteStatus> {
    let existed = host.path.exists();
    let mut config: Value = if existed {
        let raw =
            fs::read_to_string(&host.path).map_err(|error| ScopeError::io(&host.path, error))?;
        serde_json::from_str(&raw).map_err(|error| {
            ScopeError::InvalidInput(format!("invalid JSON in {}: {error}", host.path.display()))
        })?
    } else {
        json!({})
    };

    let entry = scope_server_entry();
    if json_scope_server_matches(&config, servers_key, &entry) {
        return Ok(InstallWriteStatus::Unchanged);
    }

    upsert_json_server(&mut config, servers_key, entry)?;
    let output = serde_json::to_string_pretty(&config)
        .map_err(|error| ScopeError::Serialization(error.to_string()))?;
    fs::write(&host.path, output).map_err(|error| ScopeError::io(&host.path, error))?;
    Ok(if existed {
        InstallWriteStatus::Updated
    } else {
        InstallWriteStatus::Installed
    })
}

fn write_toml_config(host: &HostInfo) -> ScopeResult<InstallWriteStatus> {
    let existed = host.path.exists();
    let (command, args) = scope_mcp_command_and_args();
    let existing = if existed {
        fs::read_to_string(&host.path).map_err(|error| ScopeError::io(&host.path, error))?
    } else {
        String::new()
    };
    if toml_scope_server_matches(&existing, &command, &args)? {
        return Ok(InstallWriteStatus::Unchanged);
    }

    let section = render_toml_server_section(&command, &args);
    let updated = upsert_toml_server_section(&existing, &section);
    fs::write(&host.path, updated).map_err(|error| ScopeError::io(&host.path, error))?;
    Ok(if existed {
        InstallWriteStatus::Updated
    } else {
        InstallWriteStatus::Installed
    })
}

fn scope_server_entry() -> Value {
    let (command, args) = scope_mcp_command_and_args();
    json!({
        "command": command,
        "args": args,
    })
}

fn scope_mcp_command_and_args() -> (String, Vec<String>) {
    let command = env::current_exe()
        .ok()
        .and_then(|path| scope_cli_command_path(&path))
        .unwrap_or_else(|| scope_binary_name().to_string());
    (command, vec!["mcp".to_string()])
}

fn scope_cli_command_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let scope_name = scope_binary_name().to_ascii_lowercase();
    let scope_mcp_name = scope_mcp_binary_name().to_ascii_lowercase();

    if file_name == scope_name {
        return Some(path.to_string_lossy().to_string());
    }

    let sibling = path.with_file_name(scope_binary_name());
    if (file_name == scope_mcp_name || !file_name.starts_with("scope")) && sibling.exists() {
        return Some(sibling.to_string_lossy().to_string());
    }

    None
}

fn scope_binary_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "scope.exe"
    }

    #[cfg(not(target_os = "windows"))]
    {
        "scope"
    }
}

fn scope_mcp_binary_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "scope-mcp.exe"
    }

    #[cfg(not(target_os = "windows"))]
    {
        "scope-mcp"
    }
}

fn resolve_host(host: &str) -> ScopeResult<HostInfo> {
    let home = home_dir()?;

    match host {
        "claude-code" => Ok(HostInfo {
            name: "claude-code",
            path: home.join(".claude.json"),
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        "cursor" => Ok(HostInfo {
            name: "cursor",
            path: home.join(".cursor/mcp.json"),
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        "windsurf" => Ok(HostInfo {
            name: "windsurf",
            path: home.join(".codeium/windsurf/mcp_config.json"),
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        "claude-desktop" => Ok(HostInfo {
            name: "claude-desktop",
            path: claude_desktop_path(&home)?,
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        "opencode" => Ok(HostInfo {
            name: "opencode",
            path: home.join(".opencode.json"),
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        "gemini" => Ok(HostInfo {
            name: "gemini",
            path: home.join(".gemini/settings.json"),
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        "codex" => Ok(HostInfo {
            name: "codex",
            path: home.join(".codex/config.toml"),
            format: ConfigFormat::Toml,
        }),
        "amp" => Ok(HostInfo {
            name: "amp",
            path: home.join(".config/amp/settings.json"),
            format: ConfigFormat::Json {
                servers_key: "amp.mcpServers",
            },
        }),
        "droid" => Ok(HostInfo {
            name: "droid",
            path: home.join(".factory/mcp.json"),
            format: ConfigFormat::Json {
                servers_key: "mcpServers",
            },
        }),
        _ => Err(ScopeError::InvalidInput(format!(
            "unknown install-mcp host `{host}`; supported hosts: {}",
            USER_SCOPE_HOSTS.join(", ")
        ))),
    }
}

fn detect_host(host: &HostInfo) -> ScopeResult<Option<String>> {
    if host.path.exists() {
        return Ok(Some(format!(
            "existing config found at {}",
            host.path.display()
        )));
    }

    let home = home_dir()?;
    let detected = match host.name {
        "claude-code" => has_any_path(&[home.join(".claude")]) || command_exists("claude"),
        "cursor" => has_any_path(&[home.join(".cursor")]) || command_exists("cursor"),
        "windsurf" => has_any_path(&[home.join(".codeium/windsurf")]) || command_exists("windsurf"),
        "claude-desktop" => host.path.parent().is_some_and(Path::exists),
        "opencode" => has_any_path(&[home.join(".config/opencode")]) || command_exists("opencode"),
        "gemini" => has_any_path(&[home.join(".gemini")]) || command_exists("gemini"),
        "codex" => has_any_path(&[home.join(".codex")]) || command_exists("codex"),
        "amp" => has_any_path(&[home.join(".config/amp")]) || command_exists("amp"),
        "droid" => {
            has_any_path(&[home.join(".factory")])
                || command_exists("factory")
                || command_exists("droid")
        }
        _ => false,
    };

    Ok(detected.then(|| "installation markers found".to_string()))
}

fn has_any_path(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| path.exists())
}

fn command_exists(command: &str) -> bool {
    let Some(path_env) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path_env).any(|directory| {
        let candidate = directory.join(command);
        if candidate.exists() {
            return true;
        }
        #[cfg(target_os = "windows")]
        {
            let exe_candidate = directory.join(format!("{command}.exe"));
            if exe_candidate.exists() {
                return true;
            }
        }
        false
    })
}

fn home_dir() -> ScopeResult<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var("USERPROFILE")
            .map(PathBuf::from)
            .map_err(|_| ScopeError::Internal("USERPROFILE not set".to_string()))
    }

    #[cfg(not(target_os = "windows"))]
    {
        env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| ScopeError::Internal("HOME not set".to_string()))
    }
}

fn claude_desktop_path(_home: &Path) -> ScopeResult<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Ok(_home.join("Library/Application Support/Claude/claude_desktop_config.json"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            env::var("APPDATA").map_err(|_| ScopeError::Internal("APPDATA not set".to_string()))?;
        Ok(PathBuf::from(appdata).join("Claude/claude_desktop_config.json"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(ScopeError::InvalidInput(
            "claude-desktop auto-install is not supported on this OS".to_string(),
        ))
    }
}

fn upsert_json_server(config: &mut Value, servers_key: &str, entry: Value) -> ScopeResult<()> {
    config
        .as_object_mut()
        .ok_or_else(|| ScopeError::InvalidInput("config root is not a JSON object".to_string()))?
        .entry(servers_key)
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or_else(|| ScopeError::InvalidInput(format!("{servers_key} is not a JSON object")))?
        .insert("scope".into(), entry);
    Ok(())
}

fn json_scope_server_matches(config: &Value, servers_key: &str, entry: &Value) -> bool {
    config
        .as_object()
        .and_then(|root| root.get(servers_key))
        .and_then(Value::as_object)
        .and_then(|servers| servers.get("scope"))
        == Some(entry)
}

fn render_toml_server_section(command: &str, args: &[String]) -> String {
    let command_escaped = command.replace('\\', "\\\\");
    let args_toml: Vec<String> = args
        .iter()
        .map(|value| format!("\"{}\"", value.replace('\\', "\\\\")))
        .collect();
    format!(
        "[mcp_servers.scope]\ncommand = \"{command_escaped}\"\nargs = [{}]\n",
        args_toml.join(", ")
    )
}

fn toml_scope_server_matches(existing: &str, command: &str, args: &[String]) -> ScopeResult<bool> {
    if existing.trim().is_empty() {
        return Ok(false);
    }

    let parsed: toml::Value = toml::from_str(existing).map_err(|error| {
        ScopeError::InvalidInput(format!("invalid TOML in existing MCP config: {error}"))
    })?;
    let Some(scope) = parsed
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .and_then(|servers| servers.get("scope"))
        .and_then(toml::Value::as_table)
    else {
        return Ok(false);
    };

    let command_matches = scope.get("command").and_then(toml::Value::as_str) == Some(command);
    let args_matches = scope
        .get("args")
        .and_then(toml::Value::as_array)
        .and_then(|values| {
            values
                .iter()
                .map(|value| value.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .is_some_and(|existing_args| existing_args == args);

    Ok(command_matches && args_matches)
}

fn upsert_toml_server_section(existing: &str, section: &str) -> String {
    if let Some(start) = existing.find("[mcp_servers.scope]") {
        let rest = &existing[start..];
        let end = rest[1..]
            .find("\n[")
            .map_or(existing.len(), |index| start + 1 + index + 1);
        format!("{}{}{}", &existing[..start], section, &existing[end..])
    } else {
        let separator = if existing.is_empty() || existing.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        format!("{existing}{separator}\n{section}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amp_dotted_key_is_literal_not_nested() {
        let mut config = json!({});
        let entry = json!({"command": "scope", "args": ["mcp"]});
        upsert_json_server(&mut config, "amp.mcpServers", entry).unwrap();

        assert!(config.get("amp.mcpServers").is_some());
        assert!(config.get("amp").is_none());
        assert_eq!(config["amp.mcpServers"]["scope"]["command"], json!("scope"));
        assert_eq!(config["amp.mcpServers"]["scope"]["args"], json!(["mcp"]));
    }

    #[test]
    fn json_upsert_preserves_existing_servers() {
        let mut config = json!({
            "mcpServers": {
                "playwright": {"command": "npx", "args": ["-y", "@playwright/mcp@latest"]}
            }
        });
        let entry = json!({"command": "scope", "args": ["mcp"]});
        upsert_json_server(&mut config, "mcpServers", entry).unwrap();

        assert_eq!(config["mcpServers"]["playwright"]["command"], json!("npx"));
        assert_eq!(config["mcpServers"]["scope"]["command"], json!("scope"));
        assert_eq!(config["mcpServers"]["scope"]["args"], json!(["mcp"]));
    }

    #[test]
    fn json_scope_server_matches_only_when_entry_is_identical() {
        let entry = json!({"command": "scope", "args": ["mcp"]});
        let config = json!({
            "mcpServers": {
                "scope": {"command": "scope", "args": ["mcp"]}
            }
        });
        assert!(json_scope_server_matches(&config, "mcpServers", &entry));

        let different = json!({
            "mcpServers": {
                "scope": {"command": "other", "args": []}
            }
        });
        assert!(!json_scope_server_matches(&different, "mcpServers", &entry));
    }

    #[test]
    fn toml_scope_server_matches_only_when_command_and_args_match() {
        let existing = concat!(
            "[mcp_servers.scope]\n",
            "command = \"/tmp/scope\"\n",
            "args = [\"mcp\"]\n",
        );
        assert!(toml_scope_server_matches(existing, "/tmp/scope", &[String::from("mcp")]).unwrap());
        assert!(
            !toml_scope_server_matches(existing, "/tmp/other", &[String::from("mcp")]).unwrap()
        );
        assert!(!toml_scope_server_matches(existing, "/tmp/scope", &[]).unwrap());
    }

    #[test]
    fn toml_upsert_replaces_existing_scope_section() {
        let existing = concat!(
            "[mcp_servers.scope]\n",
            "command = \"old\"\n",
            "args = [\"--old\"]\n",
            "\n",
            "[other]\n",
            "value = 1\n"
        );
        let replacement = render_toml_server_section("/tmp/scope", &[String::from("mcp")]);
        let updated = upsert_toml_server_section(existing, &replacement);

        assert!(updated.contains("command = \"/tmp/scope\""));
        assert!(updated.contains("args = [\"mcp\"]"));
        assert!(updated.contains("[other]\nvalue = 1"));
        assert!(!updated.contains("--old"));
    }

    #[test]
    fn build_report_counts_already_configured_hosts_as_configured() {
        let report = build_report(
            "auto_user",
            ("scope".to_string(), vec!["mcp".to_string()]),
            vec![
                McpInstallHostResult {
                    host: "codex".to_string(),
                    path: "/tmp/codex.toml".to_string(),
                    status: McpInstallStatus::Skipped,
                    reason: Some("already configured".to_string()),
                },
                McpInstallHostResult {
                    host: "cursor".to_string(),
                    path: "/tmp/cursor.json".to_string(),
                    status: McpInstallStatus::Skipped,
                    reason: Some("host not detected on this machine".to_string()),
                },
                McpInstallHostResult {
                    host: "claude-code".to_string(),
                    path: "/tmp/claude.json".to_string(),
                    status: McpInstallStatus::Installed,
                    reason: None,
                },
            ],
        );

        assert_eq!(report.configured_count, 2);
        assert_eq!(report.failed_count, 0);
    }

    #[test]
    fn unknown_host_lists_supported_values() {
        let error = resolve_host("nope").unwrap_err();
        assert!(error.to_string().contains("claude-code"));
        assert!(error.to_string().contains("codex"));
        assert!(error.to_string().contains("droid"));
    }
}
