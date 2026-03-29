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
        let status = install_host(&info)?;
        results.push(McpInstallHostResult {
            host: info.name.to_string(),
            path: info.path.display().to_string(),
            status,
            reason: None,
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
                Ok(status) => results.push(McpInstallHostResult {
                    host: info.name.to_string(),
                    path: path_display,
                    status,
                    reason: None,
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

fn install_host(host: &HostInfo) -> ScopeResult<McpInstallStatus> {
    if let Some(parent) = host.path.parent() {
        fs::create_dir_all(parent).map_err(|error| ScopeError::io(parent, error))?;
    }

    let existed = host.path.exists();
    match host.format {
        ConfigFormat::Json { servers_key } => write_json_config(host, servers_key)?,
        ConfigFormat::Toml => write_toml_config(host)?,
    }

    Ok(if existed {
        McpInstallStatus::Updated
    } else {
        McpInstallStatus::Installed
    })
}

fn write_json_config(host: &HostInfo, servers_key: &str) -> ScopeResult<()> {
    let mut config: Value = if host.path.exists() {
        let raw =
            fs::read_to_string(&host.path).map_err(|error| ScopeError::io(&host.path, error))?;
        serde_json::from_str(&raw).map_err(|error| {
            ScopeError::InvalidInput(format!("invalid JSON in {}: {error}", host.path.display()))
        })?
    } else {
        json!({})
    };

    upsert_json_server(&mut config, servers_key, scope_server_entry())?;
    let output = serde_json::to_string_pretty(&config)
        .map_err(|error| ScopeError::Serialization(error.to_string()))?;
    fs::write(&host.path, output).map_err(|error| ScopeError::io(&host.path, error))?;
    Ok(())
}

fn write_toml_config(host: &HostInfo) -> ScopeResult<()> {
    let (command, args) = scope_mcp_command_and_args();
    let section = render_toml_server_section(&command, &args);
    let existing = if host.path.exists() {
        fs::read_to_string(&host.path).map_err(|error| ScopeError::io(&host.path, error))?
    } else {
        String::new()
    };
    let updated = upsert_toml_server_section(&existing, &section);
    fs::write(&host.path, updated).map_err(|error| ScopeError::io(&host.path, error))?;
    Ok(())
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
        .map(|path| {
            let sibling = path.with_file_name(scope_mcp_binary_name());
            if sibling.exists() {
                sibling.to_string_lossy().to_string()
            } else {
                scope_mcp_binary_name().to_string()
            }
        })
        .unwrap_or_else(|| scope_mcp_binary_name().to_string());
    (command, Vec::new())
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
        let entry = json!({"command": "scope-mcp", "args": []});
        upsert_json_server(&mut config, "amp.mcpServers", entry).unwrap();

        assert!(config.get("amp.mcpServers").is_some());
        assert!(config.get("amp").is_none());
        assert_eq!(
            config["amp.mcpServers"]["scope"]["command"],
            json!("scope-mcp")
        );
    }

    #[test]
    fn json_upsert_preserves_existing_servers() {
        let mut config = json!({
            "mcpServers": {
                "playwright": {"command": "npx", "args": ["-y", "@playwright/mcp@latest"]}
            }
        });
        let entry = json!({"command": "scope-mcp", "args": []});
        upsert_json_server(&mut config, "mcpServers", entry).unwrap();

        assert_eq!(config["mcpServers"]["playwright"]["command"], json!("npx"));
        assert_eq!(config["mcpServers"]["scope"]["command"], json!("scope-mcp"));
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
        let replacement = render_toml_server_section("/tmp/scope-mcp", &[]);
        let updated = upsert_toml_server_section(existing, &replacement);

        assert!(updated.contains("command = \"/tmp/scope-mcp\""));
        assert!(updated.contains("[other]\nvalue = 1"));
        assert!(!updated.contains("--old"));
    }

    #[test]
    fn unknown_host_lists_supported_values() {
        let error = resolve_host("nope").unwrap_err();
        assert!(error.to_string().contains("claude-code"));
        assert!(error.to_string().contains("codex"));
        assert!(error.to_string().contains("droid"));
    }
}
