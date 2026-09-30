//! Reading and patching Claude Code's `settings.json`, on this PC or over SSH.
//!
//! Only a fixed set of `env` entries and the top-level `model` are ever
//! touched. Everything else — hooks, permissions, plugins — is parsed as
//! opaque JSON and written back unchanged and in its original order.
//!
//! Every call here blocks (file I/O or an `ssh` child process), so callers
//! run it on a blocking pool, never on the UI thread (rule 1 in `taskbar.rs`).

use crate::config::ClaudeTarget;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const ENV_BASE_URL: &str = "ANTHROPIC_BASE_URL";
pub const ENV_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const ENV_AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";

/// Model-alias overrides shown in the editor, in display order.
pub const MODEL_VARS: [&str; 4] = [
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
];

const DEFAULT_REMOTE_PATH: &str = "~/.claude/settings.json";

/// What the editor needs from one settings.json. The secret itself stays in
/// the backend; the UI gets which stored key it matches.
#[derive(Debug, Clone, Default)]
pub struct ClaudeEnv {
    pub exists: bool,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub auth_token: Option<String>,
    pub models: Vec<(String, Option<String>)>,
    pub model: Option<String>,
}

impl ClaudeEnv {
    /// The key Claude Code will actually send. It prefers the auth token when
    /// both are set, and they normally hold the same value.
    pub fn secret(&self) -> Option<&str> {
        self.auth_token.as_deref().or(self.api_key.as_deref())
    }
}

/// Changes to apply. `None` leaves a field alone; `Some("")` removes it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClaudePatch {
    #[serde(default)]
    pub base_url: Option<String>,
    /// Plain secret, resolved from a stored key id by the caller.
    #[serde(skip)]
    pub secret: Option<String>,
    #[serde(default)]
    pub models: Vec<(String, String)>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: String,
    pub sync: bool,
}

pub fn describe(t: &ClaudeTarget) -> TargetInfo {
    TargetInfo {
        id: t.id.clone(),
        kind: t.kind.clone(),
        label: label(t),
        path: display_path(t),
        sync: t.sync,
    }
}

pub fn label(t: &ClaudeTarget) -> String {
    if t.kind == "local" {
        "This PC".into()
    } else {
        t.host.clone().unwrap_or_else(|| "ssh".into())
    }
}

fn display_path(t: &ClaudeTarget) -> String {
    match (&t.path, t.kind.as_str()) {
        (Some(p), _) => p.clone(),
        (None, "local") => local_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.claude/settings.json".into()),
        (None, _) => DEFAULT_REMOTE_PATH.into(),
    }
}

fn local_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".claude").join("settings.json"))
}

/// A host goes straight onto the ssh command line, so it may only be a
/// user@host or an ssh_config alias — never something ssh reads as an option.
pub fn validate_host(host: &str) -> Result<(), String> {
    let ok = !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._-:".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!("'{host}' is not a valid ssh host (use user@host or an ssh_config alias)"))
    }
}

/// The remote path is spliced into a shell command unquoted so `~` expands,
/// which is only safe for a path with no shell metacharacters.
pub fn validate_remote_path(path: &str) -> Result<(), String> {
    let ok = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!("'{path}' may only contain letters, digits and / . _ - ~"))
    }
}

/// Prefer the Windows OpenSSH client. Git for Windows often puts its own
/// `ssh` first on PATH, and that one runs ssh_config `Match exec` lines
/// through sh, where a cmd-style `>nul` creates a file named `nul`.
fn ssh_program() -> std::ffi::OsString {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let native = PathBuf::from(root).join("System32").join("OpenSSH").join("ssh.exe");
        if native.exists() {
            return native.into_os_string();
        }
    }
    "ssh".into()
}

fn ssh(host: &str, script: &str) -> Command {
    let mut cmd = Command::new(ssh_program());
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        "--",
        host,
        script,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: without it every read flashes a console.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn remote_path(t: &ClaudeTarget) -> Result<String, String> {
    let path = t.path.clone().unwrap_or_else(|| DEFAULT_REMOTE_PATH.into());
    validate_remote_path(&path)?;
    Ok(path)
}

fn ssh_host(t: &ClaudeTarget) -> Result<String, String> {
    let host = t.host.clone().unwrap_or_default();
    validate_host(&host)?;
    Ok(host)
}

fn stderr_line(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.trim().chars().take(200).collect()
}

/// Raw file contents, or None when the file does not exist yet.
fn read_raw(t: &ClaudeTarget) -> Result<Option<String>, String> {
    if t.kind == "local" {
        let path = match &t.path {
            Some(p) => PathBuf::from(p),
            None => local_path().ok_or("could not resolve the home directory")?,
        };
        return match std::fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        };
    }
    let host = ssh_host(t)?;
    let path = remote_path(t)?;
    // Exit 3 marks a missing file, so it is not confused with an ssh failure.
    let script = format!("f={path}; if [ -f \"$f\" ]; then cat \"$f\"; else exit 3; fi");
    let out = ssh(&host, &script)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run ssh: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned())),
        Some(3) => Ok(None),
        _ => Err(format!("ssh {host}: {}", stderr_line(&out.stderr))),
    }
}

fn parse(raw: Option<String>) -> Result<Value, String> {
    match raw {
        None => Ok(Value::Object(Map::new())),
        Some(s) if s.trim().is_empty() => Ok(Value::Object(Map::new())),
        Some(s) => {
            let v: Value =
                serde_json::from_str(&s).map_err(|e| format!("settings.json is not valid JSON: {e}"))?;
            if v.is_object() {
                Ok(v)
            } else {
                Err("settings.json is not a JSON object".into())
            }
        }
    }
}

fn str_at(obj: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    obj?.get(key)?.as_str().map(|s| s.to_string())
}

fn extract(root: &Value, exists: bool) -> ClaudeEnv {
    let env = root.get("env").and_then(|v| v.as_object());
    ClaudeEnv {
        exists,
        base_url: str_at(env, ENV_BASE_URL),
        api_key: str_at(env, ENV_API_KEY),
        auth_token: str_at(env, ENV_AUTH_TOKEN),
        models: MODEL_VARS
            .iter()
            .map(|k| (k.to_string(), str_at(env, k)))
            .collect(),
        model: str_at(root.as_object(), "model"),
    }
}

pub fn read(t: &ClaudeTarget) -> Result<ClaudeEnv, String> {
    let raw = read_raw(t)?;
    let exists = raw.is_some();
    Ok(extract(&parse(raw)?, exists))
}

fn set_or_remove(obj: &mut Map<String, Value>, key: &str, value: &str) {
    if value.trim().is_empty() {
        obj.remove(key);
    } else {
        obj.insert(key.into(), Value::String(value.trim().into()));
    }
}

fn apply_patch(root: &mut Value, patch: &ClaudePatch) {
    let obj = root.as_object_mut().expect("parse() only returns objects");
    if let Some(model) = &patch.model {
        set_or_remove(obj, "model", model);
    }
    let env = obj
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    if !env.is_object() {
        *env = Value::Object(Map::new());
    }
    let env = env.as_object_mut().expect("just made it an object");
    if let Some(url) = &patch.base_url {
        set_or_remove(env, ENV_BASE_URL, url);
    }
    if let Some(secret) = &patch.secret {
        // Keep whichever of the two variables the file already uses. A file
        // with neither gets both, which is what a proxy setup usually wants.
        let has_key = env.contains_key(ENV_API_KEY);
        let has_token = env.contains_key(ENV_AUTH_TOKEN);
        if has_key || !has_token {
            set_or_remove(env, ENV_API_KEY, secret);
        }
        if has_token || !has_key {
            set_or_remove(env, ENV_AUTH_TOKEN, secret);
        }
    }
    for (name, value) in &patch.models {
        if MODEL_VARS.contains(&name.as_str()) {
            set_or_remove(env, name, value);
        }
    }
}

fn write_raw(t: &ClaudeTarget, body: &str) -> Result<(), String> {
    if t.kind == "local" {
        let path = match &t.path {
            Some(p) => PathBuf::from(p),
            None => local_path().ok_or("could not resolve the home directory")?,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        if path.exists() {
            // One pristine copy from before this app ever wrote, plus the
            // copy from right before this write.
            let orig = path.with_extension("json.quotabar-orig");
            if !orig.exists() {
                std::fs::copy(&path, &orig).map_err(|e| format!("backup: {e}"))?;
            }
            std::fs::copy(&path, path.with_extension("json.quotabar-bak"))
                .map_err(|e| format!("backup: {e}"))?;
        }
        let tmp = path.with_extension("json.quotabar-tmp");
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let host = ssh_host(t)?;
    let path = remote_path(t)?;
    // Same backups as locally; the new body arrives on stdin and replaces the
    // file with a rename so Claude Code never reads half a file.
    let script = format!(
        "set -e; f={path}; mkdir -p \"$(dirname \"$f\")\"; \
         if [ -f \"$f\" ]; then [ -f \"$f.quotabar-orig\" ] || cp -p \"$f\" \"$f.quotabar-orig\"; \
         cp -p \"$f\" \"$f.quotabar-bak\"; fi; \
         cat > \"$f.quotabar-tmp\"; \
         if [ -f \"$f\" ]; then chmod --reference=\"$f\" \"$f.quotabar-tmp\" 2>/dev/null || true; fi; \
         mv \"$f.quotabar-tmp\" \"$f\""
    );
    let mut child = ssh(&host, &script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run ssh: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("ssh stdin unavailable")?
        .write_all(body.as_bytes())
        .map_err(|e| format!("ssh {host}: {e}"))?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("ssh {host}: {}", stderr_line(&out.stderr)))
    }
}

/// Read, patch, write back, and return what the file now says.
pub fn patch(t: &ClaudeTarget, patch: &ClaudePatch) -> Result<ClaudeEnv, String> {
    let mut root = parse(read_raw(t)?)?;
    apply_patch(&mut root, patch);
    let mut body = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    body.push('\n');
    write_raw(t, &body)?;
    Ok(extract(&root, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_keeps_unrelated_fields_and_their_order() {
        let mut root: Value = serde_json::from_str(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://a","ANTHROPIC_API_KEY":"old","X":"1"},"hooks":{"Stop":[]},"model":"opus"}"#,
        )
        .unwrap();
        apply_patch(
            &mut root,
            &ClaudePatch {
                base_url: Some("https://b".into()),
                secret: Some("new".into()),
                models: vec![],
                model: None,
            },
        );
        let out = serde_json::to_string(&root).unwrap();
        assert_eq!(
            out,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://b","ANTHROPIC_API_KEY":"new","X":"1"},"hooks":{"Stop":[]},"model":"opus"}"#
        );
    }

    #[test]
    fn secret_goes_to_both_vars_only_when_neither_exists() {
        let mut root = Value::Object(Map::new());
        let p = ClaudePatch {
            secret: Some("k".into()),
            ..Default::default()
        };
        apply_patch(&mut root, &p);
        let env = extract(&root, true);
        assert_eq!(env.api_key.as_deref(), Some("k"));
        assert_eq!(env.auth_token.as_deref(), Some("k"));

        let mut only_token: Value =
            serde_json::from_str(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"old"}}"#).unwrap();
        apply_patch(&mut only_token, &p);
        let env = extract(&only_token, true);
        assert_eq!(env.api_key, None);
        assert_eq!(env.auth_token.as_deref(), Some("k"));
    }

    #[test]
    fn empty_model_value_removes_the_override() {
        let mut root: Value =
            serde_json::from_str(r#"{"env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"x"},"model":"opus"}"#)
                .unwrap();
        apply_patch(
            &mut root,
            &ClaudePatch {
                models: vec![("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), "".into())],
                model: Some("".into()),
                ..Default::default()
            },
        );
        assert_eq!(serde_json::to_string(&root).unwrap(), r#"{"env":{}}"#);
    }

    #[test]
    fn hosts_and_paths_that_could_inject_are_rejected() {
        assert!(validate_host("main@mdc-server").is_ok());
        assert!(validate_host("-oProxyCommand=x").is_err());
        assert!(validate_host("a;rm").is_err());
        assert!(validate_remote_path("~/.claude/settings.json").is_ok());
        assert!(validate_remote_path("~/x; rm -rf ~").is_err());
        assert!(validate_remote_path("$(id)").is_err());
    }
}
