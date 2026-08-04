use crate::config::*;
use crate::utils::{dirs, help, redact::redact_log_text};
use anyhow::{Context, Result};
use chrono::{Duration, Local};
use log::LevelFilter;
use log4rs::append::console::ConsoleAppender;
use log4rs::append::file::FileAppender;
use log4rs::config::{Appender, Logger, Root};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::encode::{self, writer::simple::SimpleWriter};
use std::backtrace::Backtrace;
use std::fmt;
use std::fs::{self, DirEntry};
use std::panic;
use std::path::PathBuf;
use std::time::SystemTime;
use tauri::api::process::Command;

struct RedactingEncoder {
    inner: PatternEncoder,
}

impl RedactingEncoder {
    fn new(pattern: &str) -> Self {
        Self {
            inner: PatternEncoder::new(pattern),
        }
    }
}

impl fmt::Debug for RedactingEncoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedactingEncoder").finish_non_exhaustive()
    }
}

impl encode::Encode for RedactingEncoder {
    fn encode(&self, w: &mut dyn encode::Write, record: &log::Record) -> anyhow::Result<()> {
        let mut bytes = Vec::new();
        {
            let mut writer = SimpleWriter(&mut bytes);
            self.inner.encode(&mut writer, record)?;
        }

        let text = String::from_utf8_lossy(&bytes);
        let redacted = redact_log_text(&text);
        w.write_all(redacted.as_bytes())?;
        Ok(())
    }
}

/// Initialize panic hook to persist crash diagnostics before the process exits.
pub fn init_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let timestamp = Local::now();
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| value.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "<unknown>".to_string());
        let backtrace = Backtrace::force_capture();
        let content = format!(
            "timestamp: {}\nthread: {}\npayload: {}\nlocation: {}\n\nbacktrace:\n{}\n",
            timestamp.to_rfc3339(),
            thread_name,
            payload,
            location,
            backtrace
        );

        match dirs::app_logs_dir() {
            Ok(log_dir) => {
                let file_name = format!("crash-{}.log", timestamp.format("%Y-%m-%d-%H%M%S"));
                let content = redact_log_text(&content);
                if let Err(err) = fs::write(log_dir.join(file_name), &content) {
                    log::error!(target: "app", "failed to write panic crash log: {err}");
                }
            }
            Err(err) => {
                log::error!(target: "app", "failed to resolve panic crash log dir: {err}");
            }
        }

        log::error!(
            target: "app",
            "panic captured, thread={}, payload={}, location={}",
            thread_name,
            payload,
            location
        );
    }));
}

/// initialize this instance's log file
fn init_log() -> Result<()> {
    let log_dir = dirs::app_logs_dir()?;

    let log_level = Config::verge().data().get_log_level();
    if log_level == LevelFilter::Off {
        return Ok(());
    }

    let local_time = Local::now().format("%Y-%m-%d-%H%M").to_string();
    let log_file = format!("{}.log", local_time);
    let log_file = log_dir.join(log_file);

    let log_pattern = match log_level {
        LevelFilter::Trace => "{d(%Y-%m-%d %H:%M:%S)} {l} [{M}] - {m}{n}",
        _ => "{d(%Y-%m-%d %H:%M:%S)} {l} - {m}{n}",
    };

    let stdout_encode = Box::new(PatternEncoder::new(log_pattern));
    let file_encode = Box::new(RedactingEncoder::new(log_pattern));

    let stdout = ConsoleAppender::builder().encoder(stdout_encode).build();
    let tofile = FileAppender::builder()
        .encoder(file_encode)
        .build(log_file)?;

    let mut logger_builder = Logger::builder();
    let mut root_builder = Root::builder();

    let log_more = log_level == LevelFilter::Trace || log_level == LevelFilter::Debug;
    let dependency_log_level = LevelFilter::Warn;

    #[cfg(feature = "verge-dev")]
    {
        logger_builder = logger_builder.appenders(["file", "stdout"]);
        if log_more {
            root_builder = root_builder.appenders(["file", "stdout"]);
        } else {
            root_builder = root_builder.appenders(["stdout"]);
        }
    }
    #[cfg(not(feature = "verge-dev"))]
    {
        logger_builder = logger_builder.appenders(["file"]);
        if log_more {
            root_builder = root_builder.appenders(["file"]);
        }
    }

    let (config, _) = log4rs::config::Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .appender(Appender::builder().build("file", Box::new(tofile)))
        .logger(logger_builder.additive(false).build("app", log_level))
        .build_lossy(root_builder.build(dependency_log_level));

    log4rs::init_config(config)?;

    Ok(())
}

/// 删除log文件
pub fn delete_log() -> Result<()> {
    let auto_log_clean = Config::verge().data().auto_log_clean.unwrap_or(1);

    let day = match auto_log_clean {
        0 => return Ok(()),
        1 => 7,
        2 => 30,
        3 => 90,
        _ => 7,
    };

    log::debug!(target: "app", "try to delete log files, day: {day}");
    let cutoff = SystemTime::from(Local::now() - Duration::days(day));
    fn clean_log_files_in_dir(dir: &PathBuf, cutoff: SystemTime) -> Result<usize> {
        if !dir.exists() {
            return Ok(0);
        }

        let mut deleted_count = 0usize;

        let mut process_file = |file: DirEntry| -> Result<()> {
            let file_type = file.file_type()?;
            if !file_type.is_file() {
                return Ok(());
            }

            let file_name = file.file_name();
            let file_name = file_name.to_str().unwrap_or_default();

            if file_name.ends_with(".log") {
                let metadata = fs::metadata(file.path())?;
                let file_time = metadata.modified().or_else(|_| metadata.created());
                let file_time = match file_time {
                    Ok(time) => time,
                    Err(err) => {
                        log::warn!(
                            target: "app",
                            "skip log file due to unreadable timestamp: {file_name}, {err}"
                        );
                        return Ok(());
                    }
                };

                if file_time < cutoff {
                    match fs::remove_file(file.path()) {
                        Ok(_) => {
                            deleted_count += 1;
                            log::info!(target: "app", "delete log file: {file_name}");
                        }
                        Err(err) => {
                            log::warn!(target: "app", "failed to delete log file: {file_name}, {err}");
                        }
                    }
                }
            }
            Ok(())
        };

        for file in fs::read_dir(dir)?.flatten() {
            let _ = process_file(file);
        }

        Ok(deleted_count)
    }

    let mut deleted_count = 0usize;
    let app_log_dir = dirs::app_logs_dir()?;
    deleted_count += clean_log_files_in_dir(&app_log_dir, cutoff)?;

    let service_log_dir = dirs::service_logs_dir()?;
    deleted_count += clean_log_files_in_dir(&service_log_dir, cutoff)?;

    log::info!(target: "app", "log clean finished, deleted {deleted_count} files");
    Ok(())
}

/// Initialize all the config files
/// before tauri setup
pub fn init_config() -> Result<()> {
    let app_dir = dirs::app_home_dir()?;
    fs::create_dir_all(&app_dir).with_context(|| {
        format!(
            "failed to create portable application directory {}",
            app_dir.display()
        )
    })?;
    let profiles_dir = dirs::app_profiles_dir()?;
    fs::create_dir_all(&profiles_dir).with_context(|| {
        format!(
            "failed to create portable profiles directory {}",
            profiles_dir.display()
        )
    })?;

    init_log()?;
    init_panic_hook();
    let _ = delete_log();

    let path = dirs::clash_path()?;
    if !path.exists() {
        help::save_yaml(&path, &IClashTemp::template().0, Some("# Clash-Verge-Buty"))?;
    }

    let path = dirs::verge_path()?;
    if !path.exists() {
        help::save_yaml(&path, &IVerge::template(), Some("# Clash-Verge-Buty"))?;
    }

    let path = dirs::profiles_path()?;
    if !path.exists() {
        help::save_yaml(&path, &IProfiles::template(), Some("# Clash-Verge-Buty"))?;
    }

    Ok(())
}

/// initialize app resources
/// after tauri setup
pub fn init_resources() -> Result<()> {
    let app_dir = dirs::app_home_dir()?;
    let res_dir = dirs::app_resources_dir()?;

    if !app_dir.exists() {
        let _ = fs::create_dir_all(&app_dir);
    }
    if !res_dir.is_dir() {
        anyhow::bail!(
            "application resources directory not found: {}",
            res_dir.display()
        );
    }

    #[cfg(target_os = "windows")]
    let file_list = ["Country.mmdb", "geoip.dat", "geosite.dat"];
    #[cfg(not(target_os = "windows"))]
    let file_list = ["Country.mmdb", "geoip.dat", "geosite.dat"];

    // copy the resource file
    // if the source file is newer than the destination file, copy it over
    for file in file_list.iter() {
        let src_path = res_dir.join(file);
        let dest_path = app_dir.join(file);

        let handle_copy = || {
            match fs::copy(&src_path, &dest_path) {
                Ok(_) => log::debug!(target: "app", "resources copied '{file}'"),
                Err(err) => {
                    log::error!(target: "app", "failed to copy resources '{file}', {err}")
                }
            };
        };

        if src_path.exists() && !dest_path.exists() {
            handle_copy();
            continue;
        }

        let src_modified = fs::metadata(&src_path).and_then(|m| m.modified());
        let dest_modified = fs::metadata(&dest_path).and_then(|m| m.modified());

        match (src_modified, dest_modified) {
            (Ok(src_modified), Ok(dest_modified)) => {
                if src_modified > dest_modified {
                    handle_copy();
                } else {
                    log::debug!(target: "app", "skipping resource copy '{file}'");
                }
            }
            _ => {
                log::debug!(target: "app", "failed to get modified '{file}'");
                handle_copy();
            }
        };
    }

    Ok(())
}

/// initialize url scheme
#[cfg(target_os = "windows")]
pub fn init_scheme() -> Result<()> {
    use tauri::utils::platform::current_exe;
    use winreg::enums::*;
    use winreg::RegKey;

    let app_exe = current_exe()?;
    let app_exe = dunce::canonicalize(app_exe)?;
    let app_exe = app_exe.to_string_lossy().into_owned();

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (clash, _) = hkcu.create_subkey("Software\\Classes\\Clash")?;
    clash.set_value("", &"Clash-Verge-Buty")?;
    clash.set_value("URL Protocol", &"Clash-Verge-Buty URL Scheme Protocol")?;
    let (default_icon, _) = hkcu.create_subkey("Software\\Classes\\Clash\\DefaultIcon")?;
    default_icon.set_value("", &app_exe)?;
    let (command, _) = hkcu.create_subkey("Software\\Classes\\Clash\\Shell\\Open\\Command")?;
    command.set_value("", &format!("{app_exe} \"%1\""))?;

    Ok(())
}
#[cfg(target_os = "linux")]
pub fn init_scheme() -> Result<()> {
    let output = std::process::Command::new("xdg-mime")
        .arg("default")
        .arg("clash-verge.desktop")
        .arg("x-scheme-handler/clash")
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "failed to set clash scheme, {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
pub fn init_scheme() -> Result<()> {
    Ok(())
}

pub fn startup_script() -> Result<()> {
    let path = {
        let verge = Config::verge();
        let verge = verge.latest();
        verge.startup_script.clone().unwrap_or("".to_string())
    };

    if !path.is_empty() {
        let mut shell = "";
        if path.ends_with(".sh") {
            shell = "bash";
        }
        if path.ends_with(".ps1") {
            shell = "powershell";
        }
        if path.ends_with(".bat") {
            shell = "cmd";
        }
        if shell.is_empty() {
            return Err(anyhow::anyhow!("unsupported script: {path}"));
        }
        let current_dir = PathBuf::from(path.clone());
        if !current_dir.exists() {
            return Err(anyhow::anyhow!("script not found: {path}"));
        }
        let current_dir = current_dir.parent();
        match current_dir {
            Some(dir) => {
                let _ = Command::new(shell)
                    .current_dir(dir.to_path_buf())
                    .args(&[path])
                    .output()?;
            }
            None => {
                let _ = Command::new(shell).args(&[path]).output()?;
            }
        }
    }
    Ok(())
}
