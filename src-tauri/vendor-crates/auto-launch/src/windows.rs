use crate::{AutoLaunch, Result};
use std::io::ErrorKind;
use std::path::Path;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

static ADMIN_AL_REGKEY: &str = "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run";
static AL_REGKEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";
static ADMIN_TASK_MANAGER_OVERRIDE_REGKEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run32";
static TASK_MANAGER_OVERRIDE_REGKEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

fn delete_value_if_present(key: &RegKey, name: &str) -> std::io::Result<()> {
    match key.delete_value(name) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn command_targets_app(command: &str, app_name: &str) -> bool {
    let command = command.trim();
    let executable = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split_once('"').map(|(path, _)| path)
    } else {
        command.split_whitespace().next()
    };

    executable
        .and_then(|path| Path::new(path).file_stem())
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.eq_ignore_ascii_case(app_name))
        .unwrap_or(false)
}

fn remove_owned_registration(
    root: &RegKey,
    run_key: &str,
    startup_approved_key: &str,
    app_name: &str,
) -> std::io::Result<()> {
    let run = match root.open_subkey_with_flags(run_key, KEY_READ | KEY_SET_VALUE) {
        Ok(run) => Some(run),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    let registered_command = run
        .as_ref()
        .and_then(|run| run.get_value::<String, _>(app_name).ok());

    if registered_command
        .as_deref()
        .is_some_and(|command| !command_targets_app(command, app_name))
    {
        return Ok(());
    }

    if let Some(run) = run.as_ref() {
        delete_value_if_present(run, app_name)?;
    }
    if let Ok(startup_approved) =
        root.open_subkey_with_flags(startup_approved_key, KEY_SET_VALUE)
    {
        delete_value_if_present(&startup_approved, app_name)?;
    }
    Ok(())
}

/// Windows implement
impl AutoLaunch {
    /// Create a new AutoLaunch instance
    /// - `app_name`: application name
    /// - `app_path`: application path
    /// - `args`: startup args passed to the binary
    ///
    /// ## Notes
    ///
    /// The parameters of `AutoLaunch::new` are different on each platform.
    pub fn new(app_name: &str, app_path: &str, args: &[impl AsRef<str>]) -> AutoLaunch {
        AutoLaunch {
            app_name: app_name.into(),
            app_path: app_path.into(),
            args: args.iter().map(|s| s.as_ref().to_string()).collect(),
        }
    }

    /// Enable the AutoLaunch setting
    ///
    /// ## Errors
    ///
    /// - failed to open the registry key
    /// - failed to set value
    pub fn enable(&self) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run = hkcu.open_subkey_with_flags(AL_REGKEY, KEY_READ | KEY_SET_VALUE)?;
        let command = if self.args.is_empty() {
            self.app_path.clone()
        } else {
            format!("{} {}", &self.app_path, &self.args.join(" "))
        };
        let command_changed = run
            .get_value::<String, _>(&self.app_name)
            .map(|current| current != command)
            .unwrap_or(true);

        if command_changed {
            run.set_value(&self.app_name, &command)?;
        }

        // StartupApproved is deliberately not written. When absent, Windows
        // treats this Run entry as enabled; when present, a user's Task Manager
        // choice must remain authoritative.

        // Older versions could register an elevated Run32 entry. Remove only
        // an entry that still belongs to this executable name and only when the
        // current process already has permission to do so.
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let _ = remove_owned_registration(
            &hklm,
            ADMIN_AL_REGKEY,
            ADMIN_TASK_MANAGER_OVERRIDE_REGKEY,
            &self.app_name,
        );

        Ok(())
    }

    /// Disable the AutoLaunch setting
    ///
    /// ## Errors
    ///
    /// - failed to open the registry key
    /// - failed to delete value
    pub fn disable(&self) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        remove_owned_registration(
            &hkcu,
            AL_REGKEY,
            TASK_MANAGER_OVERRIDE_REGKEY,
            &self.app_name,
        )?;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let _ = remove_owned_registration(
            &hklm,
            ADMIN_AL_REGKEY,
            ADMIN_TASK_MANAGER_OVERRIDE_REGKEY,
            &self.app_name,
        );
        Ok(())
    }

    /// Check whether the AutoLaunch setting is enabled
    pub fn is_enabled(&self) -> Result<bool> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let command = hkcu
            .open_subkey_with_flags(AL_REGKEY, KEY_READ)?
            .get_value::<String, _>(&self.app_name)
            .ok();
        let registered = command
            .as_deref()
            .map(|command| command_targets_app(command, &self.app_name))
            .unwrap_or(false);
        let task_manager_enabled =
            self.task_manager_enabled(hkcu, TASK_MANAGER_OVERRIDE_REGKEY);

        Ok(registered && task_manager_enabled.unwrap_or(true))
    }

    fn task_manager_enabled(&self, hk: RegKey, path: &str) -> Option<bool> {
        let task_manager_override_raw_value = hk
            .open_subkey_with_flags(path, KEY_READ)
            .ok()?
            .get_raw_value(&self.app_name)
            .ok()?;
        Some(last_eight_bytes_all_zeros(
            &task_manager_override_raw_value.bytes,
        )?)
    }
}

fn last_eight_bytes_all_zeros(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 {
        return None;
    }
    Some(bytes.iter().rev().take(8).all(|v| *v == 0u8))
}

#[cfg(test)]
mod tests {
    use super::command_targets_app;

    #[test]
    fn detects_quoted_and_unquoted_owned_commands() {
        assert!(command_targets_app(
            r#""D:\Portable Apps\Clash_Verge_Buty.exe""#,
            "Clash_Verge_Buty"
        ));
        assert!(command_targets_app(
            r#"D:\Clash\Clash_Verge_Buty.exe --silent"#,
            "Clash_Verge_Buty"
        ));
    }

    #[test]
    fn rejects_another_app_command() {
        assert!(!command_targets_app(
            r#""D:\Other\other.exe""#,
            "Clash_Verge_Buty"
        ));
    }
}
