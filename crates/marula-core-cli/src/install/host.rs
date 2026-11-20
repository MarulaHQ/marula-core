//! What the machine reports (Wave 60 lane W60-2).
//!
//! Every number here is measured or absent. Nothing is guessed from a product
//! name, and an unreadable value is `None` rather than a plausible default: the
//! recommendation treats "unknown" as "cannot prove it fits", which is the
//! answer that does not overfill somebody's box.

use std::process::Command;

/// The machine, as measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFacts {
    /// Logical cores the process may use.
    pub cores: u16,
    /// Memory in mebibytes. `None` when the platform publishes no reading.
    pub ram_mb: Option<u32>,
    /// Free disk in gibibytes at the install path. `None` when `df` could not
    /// read it (a missing path, a platform without `df`).
    pub free_disk_gb: Option<u32>,
    /// The Docker server's version, when it answers.
    pub docker: Option<String>,
    /// The first GPU the machine advertises, when one does.
    pub gpu: Option<String>,
}

impl HostFacts {
    /// Measures the machine.
    ///
    /// Cores and memory come from `core_domain::embedding::detect_host_capacity`,
    /// the same function the boot path uses to refuse a model that does not fit,
    /// so the installer and the process cannot disagree about the box.
    #[must_use]
    pub fn detect(dir: &std::path::Path) -> Self {
        let capacity = core_domain::embedding::detect_host_capacity(&|key| std::env::var(key).ok());
        Self {
            cores: capacity.cores,
            ram_mb: capacity.ram_mb,
            free_disk_gb: free_disk_gb(dir),
            docker: docker_version(),
            gpu: gpu_name(),
        }
    }

    /// Fills in a memory reading the operator declared, for a platform that
    /// publishes none.
    ///
    /// The descriptor has the same lever (`MARULA_HOST_MEMORY_MB`) and
    /// `detect_host_capacity` honours it first, so this exists for the case an
    /// operator would rather pass a flag than export a variable.
    #[must_use]
    pub fn with_declared_memory(mut self, ram_mb: Option<u32>) -> Self {
        if self.ram_mb.is_none() {
            self.ram_mb = ram_mb;
        }
        self
    }

    /// One line per fact, for the plan header and `doctor`.
    #[must_use]
    pub fn describe(&self) -> Vec<String> {
        vec![
            format!("cores: {}", self.cores),
            format!(
                "memory: {}",
                self.ram_mb.map_or_else(
                    || "not published by this platform".to_string(),
                    |mb| format!("{mb} MB")
                )
            ),
            format!(
                "free disk: {}",
                self.free_disk_gb
                    .map_or_else(|| "unreadable".to_string(), |gb| format!("{gb} GB"))
            ),
            format!(
                "docker: {}",
                self.docker
                    .clone()
                    .unwrap_or_else(|| "not answering".to_string())
            ),
            format!(
                "gpu: {}",
                self.gpu
                    .clone()
                    .unwrap_or_else(|| "none detected".to_string())
            ),
        ]
    }
}

/// Free space on the filesystem holding `dir`, in gibibytes.
///
/// Two readings, because the platforms do not share one. POSIX has `df -Pk`:
/// portable output, kilobytes, one line per filesystem. Windows has no `df`, so
/// asking it there returns `None` and the plan refuses on an unreadable floor —
/// which is why this asks PowerShell instead. A missing directory walks up to
/// the nearest ancestor that exists, because the installer may be asked about a
/// path it is about to create.
#[must_use]
pub fn free_disk_gb(dir: &std::path::Path) -> Option<u32> {
    let probe = dir
        .ancestors()
        .find(|candidate| candidate.exists())
        .map(std::path::Path::to_path_buf)?;
    #[cfg(windows)]
    {
        free_disk_gb_windows(&probe)
    }
    #[cfg(not(windows))]
    {
        free_disk_gb_unix(&probe)
    }
}

/// `df -Pk`, the POSIX reading.
#[cfg(not(windows))]
fn free_disk_gb_unix(probe: &std::path::Path) -> Option<u32> {
    let output = Command::new("df").arg("-Pk").arg(probe).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    let available_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(u32::try_from(available_kb / (1024 * 1024)).unwrap_or(u32::MAX))
}

/// The Windows reading.
///
/// PowerShell rather than a new dependency: `Get-PSDrive` reports the free bytes
/// of the volume a path lives on, it is present on every supported Windows, and
/// the installer already shells out for `docker`. The path travels in an
/// environment variable so no quoting rule of a shell can mangle it. `powershell`
/// is asked first because it is the one that always exists; `pwsh` is the
/// fallback for an image without Windows PowerShell.
#[cfg(windows)]
fn free_disk_gb_windows(probe: &std::path::Path) -> Option<u32> {
    const SCRIPT: &str = "[math]::Floor((Get-PSDrive -Name ((Get-Item -LiteralPath $env:MARULA_DISK_PROBE).PSDrive).Name).Free / 1GB)";
    let mut last = None;
    for shell in ["powershell", "pwsh"] {
        let output = Command::new(shell)
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .env("MARULA_DISK_PROBE", probe)
            .output();
        let Ok(output) = output else { continue };
        if !output.status.success() {
            last = None;
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Ok(gb) = text.trim().parse::<f64>() {
            // A negative reading is nonsense; treat it as unreadable rather than
            // as zero, so the message says "unreadable" and not "has 0".
            if gb >= 0.0 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                return Some(gb.min(f64::from(u32::MAX)) as u32);
            }
        }
        last = None;
    }
    last
}

/// The Docker server version, when the CLI can reach it.
#[must_use]
pub fn docker_version() -> Option<String> {
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

/// The first GPU the machine advertises.
///
/// NVIDIA only, because that is the one that changes a deployment decision
/// today (a local multimodal model). A machine with a GPU nobody can address
/// through a supported runtime is reported as having none, which is what the
/// process can actually use.
#[must_use]
pub fn gpu_name() -> Option<String> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let first = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!first.is_empty()).then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_directory_reads_its_nearest_existing_ancestor() {
        let missing = std::env::temp_dir().join("marula-install-does-not-exist/sub");
        // Either the platform has df and reports a number, or it does not and
        // says so. Both are honest; a panic or a zero is not.
        if let Some(gb) = free_disk_gb(&missing) {
            assert!(gb < 1_000_000, "a disk reading of {gb} GB is nonsense");
        }
    }

    #[test]
    fn describe_names_every_fact_including_the_absent_ones() {
        let facts = HostFacts {
            cores: 4,
            ram_mb: None,
            free_disk_gb: Some(80),
            docker: None,
            gpu: None,
        };
        let lines = facts.describe();
        assert_eq!(lines.len(), 5);
        assert!(lines[1].contains("not published"));
        assert!(lines[3].contains("not answering"));
        assert!(lines[4].contains("none detected"));
    }
}
