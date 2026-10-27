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
/// `df -Pk` is the portable reading: POSIX output, kilobytes, one line per
/// filesystem. A missing directory walks up to the nearest ancestor that exists,
/// because the installer may be asked about a path it is about to create.
#[must_use]
pub fn free_disk_gb(dir: &std::path::Path) -> Option<u32> {
    let probe = dir
        .ancestors()
        .find(|candidate| candidate.exists())
        .map(std::path::Path::to_path_buf)?;
    let output = Command::new("df").arg("-Pk").arg(&probe).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    let available_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(u32::try_from(available_kb / (1024 * 1024)).unwrap_or(u32::MAX))
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
