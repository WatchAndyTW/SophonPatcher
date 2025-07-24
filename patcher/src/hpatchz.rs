use std::sync::OnceLock;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io::Write;
use anyhow::{Result, Context};

// Global static for the extracted executable path
static HPATCHZ_EXE_PATH: OnceLock<PathBuf> = OnceLock::new();

pub struct HPatchZ;

impl HPatchZ {
    /// Get the path to the extracted hpatchz executable (extract once, reuse many times)
    pub fn get_exe_path() -> Result<&'static PathBuf> {
        HPATCHZ_EXE_PATH.get_or_try_init(|| {
            Self::extract_exe_once()
        })
    }

    /// Extract the executable once to a temporary location
    fn extract_exe_once() -> Result<PathBuf> {
        // Embed the executable based on target platform
        #[cfg(target_os = "windows")]
        const HPATCHZ_BYTES: &[u8] = include_bytes!("../bin/hpatchz.exe");

        #[cfg(target_os = "linux")]
        const HPATCHZ_BYTES: &[u8] = include_bytes!("../bin/hpatchz");

        #[cfg(target_os = "macos")]
        const HPATCHZ_BYTES: &[u8] = include_bytes!("../bin/hpatchz_macos");

        // Create a persistent temp directory for this process
        let temp_dir = std::env::temp_dir()
            .join(format!("rust_hpatchz_global_{}", std::process::id()));

        fs::create_dir_all(&temp_dir).context("Failed to create temp directory")?;

        let exe_name = if cfg!(target_os = "windows") {
            "hpatchz.exe"
        } else {
            "hpatchz"
        };

        let exe_path = temp_dir.join(exe_name);

        // Only extract if it doesn't exist
        if !exe_path.exists() {
            let mut temp_file = fs::File::create(&exe_path)
                .context("Failed to create temporary executable file")?;

            temp_file.write_all(HPATCHZ_BYTES)
                .context("Failed to write executable data")?;

            temp_file.flush()
                .context("Failed to flush executable file")?;

            // Make executable on Unix systems
            #[cfg(not(target_os = "windows"))]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&exe_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&exe_path, perms)
                    .context("Failed to set executable permissions")?;
            }
        }

        Ok(exe_path)
    }

    /// Apply a patch using the globally extracted executable
    pub fn apply_patch<P: AsRef<Path>>(
        old_file: P,
        diff_file: P,
        new_file: P,
    ) -> Result<()> {
        let exe_path = Self::get_exe_path()?;

        let output = Command::new(exe_path)
            .arg("-f")
            .arg(old_file.as_ref())
            .arg(diff_file.as_ref())
            .arg(new_file.as_ref())
            .output()
            .context("Failed to execute hpatchz")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("hpatchz failed: {}", stderr)
        }
    }

    /// Apply a patch using the globally extracted executable
    pub fn apply_patch_empty<P: AsRef<Path>>(
        diff_file: P,
        new_file: P,
    ) -> Result<()> {
        let exe_path = Self::get_exe_path()?;

        let output = Command::new(exe_path)
            .arg("-f")
            .arg("")
            .arg(diff_file.as_ref())
            .arg(new_file.as_ref())
            .output()
            .context("Failed to execute hpatchz")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("hpatchz failed: {}", stderr)
        }
    }

    /// Clean up the extracted executable (call this when your program exits)
    pub fn cleanup() -> Result<()> {
        if let Some(exe_path) = HPATCHZ_EXE_PATH.get() {
            if let Some(parent) = exe_path.parent() {
                fs::remove_dir_all(parent)
                    .context("Failed to cleanup temporary directory")?;
            }
        }
        Ok(())
    }
}