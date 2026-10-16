use atlas_core::{ToolKind, ToolProbeResult, ToolStatus};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// Probes all known ATLAS dependencies and returns their status.
pub async fn probe_all() -> Vec<ToolProbeResult> {
    let probes = vec![
        probe_opencode().await,
        probe_blender().await,
        probe_unreal().await,
        probe_ffmpeg().await,
        probe_ffprobe().await,
        probe_vectorcraft().await,
        probe_photocraft().await,
        probe_effectcraft().await,
        probe_kokoro().await,
    ];
    probes
}

/// Returns true if the minimal viable pipeline can run (Blender + FFmpeg + OpenCode).
pub fn has_minimal_pipeline(results: &[ToolProbeResult]) -> bool {
    [ToolKind::OpenCode, ToolKind::Blender, ToolKind::Ffmpeg]
        .iter()
        .all(|kind| {
            results
                .iter()
                .any(|r| &r.kind == kind && r.status.is_available())
        })
}

async fn run_version(args: &[&str], path_hint: Option<&str>) -> Option<(String, String)> {
    let exe: String = path_hint.map(|s| s.to_string()).or_else(|| {
        which::which(args[0])
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    })?;
    let output = Command::new(&exe)
        .args(&args[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{} {}", text, err);
    let first_line = combined
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("unknown")
        .to_string();
    Some((first_line, exe))
}

fn unavailable(kind: ToolKind, reason: impl Into<String>) -> ToolProbeResult {
    ToolProbeResult {
        kind,
        status: ToolStatus::Unavailable {
            reason: reason.into(),
        },
    }
}

fn optional_unavailable(kind: ToolKind, reason: impl Into<String>) -> ToolProbeResult {
    ToolProbeResult {
        kind,
        status: ToolStatus::OptionalUnavailable {
            reason: reason.into(),
        },
    }
}

fn available(kind: ToolKind, version: impl Into<String>, path: impl Into<String>) -> ToolProbeResult {
    ToolProbeResult {
        kind,
        status: ToolStatus::Available {
            version: version.into(),
            path: path.into(),
        },
    }
}

pub async fn probe_opencode() -> ToolProbeResult {
    match run_version(&["opencode", "--version"], None).await {
        Some((version, path)) => available(ToolKind::OpenCode, version, path),
        None => unavailable(ToolKind::OpenCode, "opencode --version failed or not in PATH"),
    }
}

pub async fn probe_blender() -> ToolProbeResult {
    match run_version(&["blender", "--version"], None).await {
        Some((line, path)) => {
            let version = line
                .split_whitespace()
                .nth(1)
                .map(|s| s.to_string())
                .unwrap_or_else(|| line.clone());
            available(ToolKind::Blender, version, path)
        }
        None => unavailable(ToolKind::Blender, "blender --version failed or not in PATH"),
    }
}

pub async fn probe_unreal() -> ToolProbeResult {
    // Common UE 5.x editor-cmd locations on Windows.
    let candidates = [
        r"C:\Program Files\Epic Games\UE_5.8\Engine\Binaries\Win64\UnrealEditor-Cmd.exe",
        r"C:\Program Files\Epic Games\UE_5.5\Engine\Binaries\Win64\UnrealEditor-Cmd.exe",
        r"C:\Program Files\Epic Games\UE_5.4\Engine\Binaries\Win64\UnrealEditor-Cmd.exe",
        r"C:\Program Files\Epic Games\UE_5.3\Engine\Binaries\Win64\UnrealEditor-Cmd.exe",
    ];
    for candidate in &candidates {
        if Path::new(candidate).exists() {
            // UE -version can be slow; just report presence and path.
            return available(ToolKind::UnrealEditor, "5.x detected".to_string(), candidate.to_string());
        }
    }
    optional_unavailable(
        ToolKind::UnrealEditor,
        "UnrealEditor-Cmd.exe not found in common locations; Blender-only pipeline supported",
    )
}

pub async fn probe_ffmpeg() -> ToolProbeResult {
    match run_version(&["ffmpeg", "-version"], None).await {
        Some((line, path)) => {
            let version = line
                .split_whitespace()
                .nth(2)
                .map(|s| s.to_string())
                .unwrap_or_else(|| line.clone());
            available(ToolKind::Ffmpeg, version, path)
        }
        None => unavailable(ToolKind::Ffmpeg, "ffmpeg -version failed or not in PATH"),
    }
}

pub async fn probe_ffprobe() -> ToolProbeResult {
    match run_version(&["ffprobe", "-version"], None).await {
        Some((line, path)) => {
            let version = line
                .split_whitespace()
                .nth(2)
                .map(|s| s.to_string())
                .unwrap_or_else(|| line.clone());
            available(ToolKind::Ffprobe, version, path)
        }
        None => unavailable(ToolKind::Ffprobe, "ffprobe -version failed or not in PATH"),
    }
}

/// Looks for VectorCraft in PATH or in `tools/vectorcraft/` within the workspace.
pub async fn probe_vectorcraft() -> ToolProbeResult {
    let path_candidates = [
        PathBuf::from(r"tools\vectorcraft\vectorcraft-cli.exe"),
        PathBuf::from(r"tools\vectorcraft\bin\vectorcraft-cli.exe"),
        PathBuf::from(r"tools\vectorcraft\vectorcraft-0.11.0-windows-x64-portable\vectorcraft-cli.exe"),
    ];
    for candidate in &path_candidates {
        if candidate.exists() {
            return available(
                ToolKind::VectorCraft,
                "found in tools/vectorcraft",
                candidate.to_string_lossy().to_string(),
            );
        }
    }
    match run_version(&["vectorcraft-cli", "--version"], None).await {
        Some((version, path)) => available(ToolKind::VectorCraft, version, path),
        None => optional_unavailable(
            ToolKind::VectorCraft,
            "vectorcraft-cli not in PATH or tools/vectorcraft; install to enable vector art",
        ),
    }
}

pub async fn probe_photocraft() -> ToolProbeResult {
    let path_candidates = [
        PathBuf::from(r"tools\photocraft\photocraft-cli.exe"),
        PathBuf::from(r"tools\photocraft\bin\photocraft-cli.exe"),
        PathBuf::from(r"tools\photocraft\photocraft-0.9.0-windows-x64-portable\photocraft-cli.exe"),
    ];
    for candidate in &path_candidates {
        if candidate.exists() {
            return available(
                ToolKind::PhotoCraft,
                "found in tools/photocraft",
                candidate.to_string_lossy().to_string(),
            );
        }
    }
    match run_version(&["photocraft-cli", "--version"], None).await {
        Some((version, path)) => available(ToolKind::PhotoCraft, version, path),
        None => optional_unavailable(
            ToolKind::PhotoCraft,
            "photocraft-cli not in PATH or tools/photocraft; install to enable photo editing",
        ),
    }
}

pub async fn probe_effectcraft() -> ToolProbeResult {
    let path_candidates = [
        PathBuf::from(r"tools\effectcraft\effectcraft-cli.exe"),
        PathBuf::from(r"tools\effectcraft\bin\effectcraft-cli.exe"),
        PathBuf::from(r"tools\effectcraft\effectcraft-0.1.1-windows-x64-portable\effectcraft-cli.exe"),
    ];
    for candidate in &path_candidates {
        if candidate.exists() {
            return available(
                ToolKind::EffectCraft,
                "found in tools/effectcraft",
                candidate.to_string_lossy().to_string(),
            );
        }
    }
    match run_version(&["effectcraft-cli", "--version"], None).await {
        Some((version, path)) => available(ToolKind::EffectCraft, version, path),
        None => optional_unavailable(
            ToolKind::EffectCraft,
            "effectcraft-cli not in PATH or tools/effectcraft; install to enable motion graphics",
        ),
    }
}

pub async fn probe_kokoro() -> ToolProbeResult {
    let path_candidates = [
        PathBuf::from(r"tools\kokoro\kokoro.exe"),
        PathBuf::from(r"tools\kokoro\kokoro-cli.exe"),
        PathBuf::from(r"tools\kokoro-fastapi\kokoro.exe"),
    ];
    for candidate in &path_candidates {
        if candidate.exists() {
            return available(
                ToolKind::Kokoro,
                "found in tools/kokoro",
                candidate.to_string_lossy().to_string(),
            );
        }
    }
    match run_version(&["kokoro", "--version"], None).await {
        Some((version, path)) => available(ToolKind::Kokoro, version, path),
        None => optional_unavailable(
            ToolKind::Kokoro,
            "kokoro not in PATH or tools/kokoro; narration synthesis unavailable",
        ),
    }
}

/// Searches the filesystem for an executable by name under a root.
pub async fn find_executable_under(root: &Path, name: &str) -> Option<PathBuf> {
    use tokio::fs;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                if let Some(file_name) = path.file_stem() {
                    if file_name.eq_ignore_ascii_case(name) {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

/// Probe a single tool by kind.
pub async fn probe_tool(kind: ToolKind) -> ToolProbeResult {
    match kind {
        ToolKind::OpenCode => probe_opencode().await,
        ToolKind::Blender => probe_blender().await,
        ToolKind::UnrealEditor => probe_unreal().await,
        ToolKind::VectorCraft => probe_vectorcraft().await,
        ToolKind::PhotoCraft => probe_photocraft().await,
        ToolKind::EffectCraft => probe_effectcraft().await,
        ToolKind::Kokoro => probe_kokoro().await,
        ToolKind::Ffmpeg => probe_ffmpeg().await,
        ToolKind::Ffprobe => probe_ffprobe().await,
    }
}
