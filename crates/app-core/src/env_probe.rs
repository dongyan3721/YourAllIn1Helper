//! Zero-bundled-runtime environment detector (`EnvProbeService`).
//!
//! Adheres to the <15MB installer size constraint by discovering external runtimes
//! (`ffmpeg`, `ffprobe`, `node`, `python3`, `cscript`, `powershell`, `osascript`, `zsh`)
//! from user-configured custom paths or the system `PATH` environment variable.

use crate::error::{CoreError, CoreResult};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Command,
};

/// Supported external runtimes and multimedia tools probed from the host OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalToolKind {
    Ffmpeg,
    Ffprobe,
    Node,
    Python3,
    /// Windows Script Host (`cscript.exe //NoLogo`) for VBScript/JScript execution.
    Cscript,
    /// Windows PowerShell (`pwsh.exe` or `powershell.exe`).
    PowerShell,
    /// macOS AppleScript / JXA (`osascript`).
    Osascript,
    /// Unix shell (`/bin/zsh` or `/bin/bash`).
    Zsh,
}

impl ExternalToolKind {
    /// Candidate binary names searched in `PATH` in priority order.
    pub fn candidate_binary_names(&self) -> &'static [&'static str] {
        match self {
            Self::Ffmpeg => &["ffmpeg"],
            Self::Ffprobe => &["ffprobe"],
            Self::Node => &["node"],
            Self::Python3 => &["python3", "python"],
            Self::Cscript => &["cscript"],
            Self::PowerShell => &["pwsh", "powershell"],
            Self::Osascript => &["osascript"],
            Self::Zsh => &["zsh", "bash"],
        }
    }

    /// Version flag passed when verifying executable readiness.
    pub fn version_args(&self) -> &'static [&'static str] {
        match self {
            Self::Ffmpeg | Self::Ffprobe => &["-version"],
            Self::Cscript => &["//?"],
            Self::Osascript => &["-e", "version of AppleScript"],
            _ => &["--version"],
        }
    }

    /// Actionable installation guidance displayed in the UI when a tool is missing.
    pub fn install_hint(&self) -> &'static str {
        match self {
            Self::Ffmpeg | Self::Ffprobe => {
                #[cfg(target_os = "macos")]
                {
                    "Run `brew install ffmpeg` or specify a custom binary path in Settings."
                }
                #[cfg(target_os = "windows")]
                {
                    "Run `winget install Gyan.FFmpeg` or specify a custom ffmpeg.exe path in Settings."
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    "Install `ffmpeg` via your system package manager or configure a custom path."
                }
            }
            Self::Node => {
                "Install Node.js (`winget install OpenJS.NodeJS.LTS` or `brew install node`)."
            }
            Self::Python3 => {
                "Install Python 3 (`winget install Python.Python.3.12` or `brew install python`)."
            }
            Self::Cscript => "Built into Windows (`C:\\Windows\\System32\\cscript.exe`).",
            Self::PowerShell => "Built into Windows (`powershell.exe` / `pwsh.exe`).",
            Self::Osascript => "Built into macOS (`/usr/bin/osascript`).",
            Self::Zsh => "Built into macOS/Linux (`/bin/zsh` or `/bin/bash`).",
        }
    }
}

/// Probe result for a single external tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolProbeResult {
    pub tool: ExternalToolKind,
    pub available: bool,
    pub resolved_path: Option<PathBuf>,
    pub version_summary: Option<String>,
    pub from_custom_path: bool,
    pub install_hint: String,
}

/// Service that repairs GUI `$PATH` on macOS and probes external runtimes on demand.
pub struct EnvProbeService {
    custom_overrides: RwLock<HashMap<ExternalToolKind, PathBuf>>,
    cache: RwLock<HashMap<ExternalToolKind, ToolProbeResult>>,
}

impl Default for EnvProbeService {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvProbeService {
    pub fn new() -> Self {
        Self {
            custom_overrides: RwLock::new(HashMap::new()),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Repairs the process `PATH` when launched as a macOS `.app` bundle from Finder/Launchpad,
    /// ensuring user-installed binaries in `/opt/homebrew/bin` or `~/.zshrc` are discoverable.
    pub fn hydrate_gui_path_env() {
        #[cfg(target_os = "macos")]
        {
            let mut merged_paths: Vec<PathBuf> = env::var_os("PATH")
                .map(|p| env::split_paths(&p).collect())
                .unwrap_or_default();

            // Attempt to query login shell PATH first
            let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            if let Ok(output) = Command::new(&shell)
                .args(["-ilc", "echo -n \"$PATH\""])
                .output()
            {
                if output.status.success() {
                    let shell_path = String::from_utf8_lossy(&output.stdout);
                    for p in env::split_paths(shell_path.trim()) {
                        if !p.as_os_str().is_empty() && !merged_paths.contains(&p) {
                            merged_paths.push(p);
                        }
                    }
                }
            }

            // Guarantee standard Homebrew / local bin locations are included
            for fallback in [
                "/opt/homebrew/bin",
                "/opt/homebrew/sbin",
                "/usr/local/bin",
                "/usr/bin",
                "/bin",
            ] {
                let pb = PathBuf::from(fallback);
                if !merged_paths.contains(&pb) {
                    merged_paths.push(pb);
                }
            }

            if let Ok(joined) = env::join_paths(merged_paths) {
                env::set_var("PATH", joined);
            }
        }
    }

    /// Sets or clears a user-specified custom binary path for a given tool and invalidates its cache entry.
    pub fn set_custom_override(&self, tool: ExternalToolKind, path: Option<PathBuf>) {
        let mut overrides = self.custom_overrides.write();
        match path {
            Some(p) if !p.as_os_str().is_empty() => {
                overrides.insert(tool, p);
            }
            _ => {
                overrides.remove(&tool);
            }
        }
        self.cache.write().remove(&tool);
    }

    /// Probes a single tool (using cached result unless `force_refresh` is true).
    pub fn probe(&self, tool: ExternalToolKind, force_refresh: bool) -> ToolProbeResult {
        if !force_refresh {
            if let Some(cached) = self.cache.read().get(&tool).cloned() {
                return cached;
            }
        }

        let custom_path = self.custom_overrides.read().get(&tool).cloned();
        let result = self.probe_uncached(tool, custom_path);
        self.cache.write().insert(tool, result.clone());
        result
    }

    /// Resolves the executable path for `tool`, returning `CoreError::ExternalToolUnavailable` if missing.
    pub fn require_executable(&self, tool: ExternalToolKind) -> CoreResult<PathBuf> {
        let res = self.probe(tool, false);
        res.resolved_path
            .ok_or_else(|| CoreError::ExternalToolUnavailable {
                tool: format!("{:?}", tool),
                hint: res.install_hint,
            })
    }

    /// Probes all platform-relevant tools and returns their status report.
    pub fn probe_all(&self, force_refresh: bool) -> Vec<ToolProbeResult> {
        let mut tools = vec![
            ExternalToolKind::Ffmpeg,
            ExternalToolKind::Ffprobe,
            ExternalToolKind::Node,
            ExternalToolKind::Python3,
        ];
        #[cfg(target_os = "windows")]
        {
            tools.push(ExternalToolKind::Cscript);
            tools.push(ExternalToolKind::PowerShell);
        }
        #[cfg(target_os = "macos")]
        {
            tools.push(ExternalToolKind::Osascript);
            tools.push(ExternalToolKind::Zsh);
        }

        tools
            .into_iter()
            .map(|t| self.probe(t, force_refresh))
            .collect()
    }

    fn probe_uncached(
        &self,
        tool: ExternalToolKind,
        custom_override: Option<PathBuf>,
    ) -> ToolProbeResult {
        // Priority 1: User custom path
        if let Some(custom_bin) = custom_override {
            if custom_bin.is_file() {
                if let Some(version) = Self::verify_binary_version(&custom_bin, tool.version_args())
                {
                    return ToolProbeResult {
                        tool,
                        available: true,
                        resolved_path: Some(custom_bin),
                        version_summary: Some(version),
                        from_custom_path: true,
                        install_hint: tool.install_hint().to_string(),
                    };
                }
            }
        }

        // Priority 2: System PATH scan
        for bin_name in tool.candidate_binary_names() {
            if let Some(found_path) = Self::find_in_system_path(bin_name) {
                if let Some(version) = Self::verify_binary_version(&found_path, tool.version_args())
                {
                    return ToolProbeResult {
                        tool,
                        available: true,
                        resolved_path: Some(found_path),
                        version_summary: Some(version),
                        from_custom_path: false,
                        install_hint: tool.install_hint().to_string(),
                    };
                }
            }
        }

        ToolProbeResult {
            tool,
            available: false,
            resolved_path: None,
            version_summary: None,
            from_custom_path: false,
            install_hint: tool.install_hint().to_string(),
        }
    }

    /// Searches `PATH` (and Windows `PATHEXT` extensions) for `bin_name`.
    pub fn find_in_system_path(bin_name: &str) -> Option<PathBuf> {
        let path_os = env::var_os("PATH")?;
        #[cfg(target_os = "windows")]
        let extensions: Vec<String> = env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
            .split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .collect();

        #[cfg(not(target_os = "windows"))]
        let extensions: Vec<String> = vec![String::new()];

        for dir in env::split_paths(&path_os) {
            for ext in &extensions {
                let file_name = if ext.is_empty() || bin_name.to_ascii_lowercase().ends_with(ext) {
                    bin_name.to_string()
                } else {
                    format!("{}{}", bin_name, ext)
                };
                let candidate = dir.join(file_name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn verify_binary_version(bin_path: &Path, args: &[&str]) -> Option<String> {
        let mut cmd = Command::new(bin_path);
        cmd.args(args);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let output = cmd.output().ok()?;
        let combined = if !output.stdout.is_empty() {
            String::from_utf8_lossy(&output.stdout).into_owned()
        } else {
            String::from_utf8_lossy(&output.stderr).into_owned()
        };

        let first_line = combined
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("verified")
            .to_string();
        Some(first_line)
    }
}
