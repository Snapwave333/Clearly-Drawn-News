use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

/// Discovers OpenCode's configured providers and active NVIDIA account.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenCodeProviders {
    pub opencode_version: String,
    pub credentials: Vec<OpenCodeCredential>,
    pub environment_providers: Vec<EnvProviderHint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenCodeCredential {
    pub provider: String,
    pub account: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvProviderHint {
    pub provider: String,
    pub env_var: String,
    pub present: bool,
}

/// Discovers OpenCode installation and provider state.
pub struct OpenCodeClient {
    exe: String,
}

impl OpenCodeClient {
    pub fn new() -> Self {
        Self {
            exe: "opencode".to_string(),
        }
    }

    pub fn with_exe(exe: impl Into<String>) -> Self {
        Self { exe: exe.into() }
    }

    pub async fn version(&self) -> AtlasResult<String> {
        let out = self.run(&["--version"]).await?;
        Ok(out.trim().to_string())
    }

    pub async fn discover_providers(&self) -> AtlasResult<OpenCodeProviders> {
        let version = self.version().await.unwrap_or_default();
        let mut providers = OpenCodeProviders {
            opencode_version: version,
            credentials: Vec::new(),
            environment_providers: vec![
                EnvProviderHint {
                    provider: "nvidia".to_string(),
                    env_var: "NVIDIA_API_KEY".to_string(),
                    present: std::env::var("NVIDIA_API_KEY").is_ok(),
                },
                EnvProviderHint {
                    provider: "opencode".to_string(),
                    env_var: "OPENCODE_API_KEY".to_string(),
                    present: std::env::var("OPENCODE_API_KEY").is_ok(),
                },
            ],
        };

        let text = self.run(&["providers", "list"]).await?;
        for line in text.lines() {
            let line = strip_ansi(line).trim().to_string();
            // Parse tree lines like: "•  Nvidia  api" or "•  OpenCode Go  api"
            if line.starts_with('•') {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let provider = parts[1..parts.len() - 1].join(" ");
                    let account = parts.last().unwrap_or(&"").to_string();
                    providers.credentials.push(OpenCodeCredential {
                        provider,
                        account,
                        source: "opencode providers list".to_string(),
                    });
                }
            }
        }

        Ok(providers)
    }

    pub async fn list_nvidia_models(&self) -> AtlasResult<Vec<String>> {
        let text = self.run(&["models", "nvidia"]).await?;
        let models: Vec<String> = text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && l.starts_with("nvidia/"))
            .collect();
        Ok(models)
    }

    /// Tests whether a configured NVIDIA provider is reachable through OpenCode.
    /// Returns the active provider hint and whether an env fallback is available.
    pub async fn nvidia_status(&self) -> AtlasResult<NvidiaStatus> {
        let providers = self.discover_providers().await?;
        let credential = providers
            .credentials
            .iter()
            .find(|c| c.provider.to_lowercase().contains("nvidia"))
            .cloned();
        let env_available = providers
            .environment_providers
            .iter()
            .any(|e| e.provider == "nvidia" && e.present);

        let models = self.list_nvidia_models().await.ok().unwrap_or_default();

        Ok(NvidiaStatus {
            opencode_account: credential,
            env_fallback_available: env_available,
            model_count: models.len(),
            sample_models: models.into_iter().take(5).collect(),
        })
    }

    async fn run(&self, args: &[&str]) -> AtlasResult<String> {
        debug!("Running: {} {}", self.exe, args.join(" "));

        // On Windows, opencode is installed as an npm .cmd shim; Command::new
        // cannot execute .cmd files directly, so run through cmd /c.
        let use_cmd = cfg!(windows)
            && (self.exe.eq_ignore_ascii_case("opencode") || self.exe.ends_with(".cmd"));
        let (program, program_args): (&str, Vec<String>) = if use_cmd {
            (
                "cmd",
                std::iter::once("/c".to_string())
                    .chain(std::iter::once(self.exe.clone()))
                    .chain(args.iter().map(|s| s.to_string()))
                    .collect(),
            )
        } else {
            (self.exe.as_str(), args.iter().map(|s| s.to_string()).collect())
        };

        let output = Command::new(program)
            .args(&program_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("failed to run opencode: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(AtlasError::OpenCode(format!(
                "opencode {} exited with {}: {}",
                args.join(" "),
                output.status,
                err
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NvidiaStatus {
    pub opencode_account: Option<OpenCodeCredential>,
    pub env_fallback_available: bool,
    pub model_count: usize,
    pub sample_models: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_providers_list_with_ansi() {
        let raw = "\u{1b}[0m\n\u{1b}[90mT\u{1b}[39m  Credentials \u{1b}[90m~\\.local\\share\\opencode\\auth.json\u{1b}[39m\n\u{1b}[90m|\u{1b}[39m\n\u{1b}[34m•\u{1b}[39m  Nvidia  \u{1b}[90mapi\u{1b}[39m\n";
        let mut providers = OpenCodeProviders::default();
        for line in raw.lines() {
            let line = strip_ansi(line).trim().to_string();
            if line.starts_with('•') {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let provider = parts[1..parts.len() - 1].join(" ");
                    let account = parts.last().unwrap_or(&"").to_string();
                    providers.credentials.push(OpenCodeCredential {
                        provider,
                        account,
                        source: "opencode providers list".to_string(),
                    });
                }
            }
        }
        assert_eq!(providers.credentials.len(), 1);
        assert_eq!(providers.credentials[0].provider, "Nvidia");
        assert_eq!(providers.credentials[0].account, "api");
    }
}

fn strip_ansi(s: &str) -> String {
    // Basic ANSI escape stripper.
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            // skip until letter or end
            while let Some(&next) = chars.peek() {
                chars.next();
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}
