use crate::utils::{dirs, redact::redact_log_text};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

const REDACTED_EXPORT_DIR: &str = "redacted-export";

fn copy_redacted_logs_from_dir(source_dir: &Path, target_dir: &Path) -> Result<()> {
    if !source_dir.exists() {
        return Ok(());
    }

    fs::create_dir_all(target_dir)?;
    for entry in fs::read_dir(source_dir)?.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };

        if !file_type.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("log") {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                log::warn!(
                    target: "app",
                    "skip redacted log export for unreadable file {:?}: {err}",
                    path.file_name().unwrap_or_default()
                );
                continue;
            }
        };
        let redacted = redact_log_text(&content);
        fs::write(target_dir.join(entry.file_name()), redacted)?;
    }

    Ok(())
}

/// Build a sanitized copy of local log files for issue reports or manual upload.
/// The live log buffers used by the in-app Logs/Connections pages are not touched.
pub fn prepare_redacted_logs_export() -> Result<PathBuf> {
    let logs_dir = dirs::logs_dir()?;
    let export_dir = logs_dir.join(REDACTED_EXPORT_DIR);

    if export_dir.exists() {
        fs::remove_dir_all(&export_dir)?;
    }
    fs::create_dir_all(&export_dir)?;

    copy_redacted_logs_from_dir(&dirs::app_logs_dir()?, &export_dir.join("app"))?;
    copy_redacted_logs_from_dir(&dirs::service_logs_dir()?, &export_dir.join("service"))?;

    Ok(export_dir)
}
