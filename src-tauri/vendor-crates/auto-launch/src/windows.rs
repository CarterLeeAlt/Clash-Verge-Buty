use crate::{AutoLaunch, Error, Result};
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

static ADMIN_AL_REGKEY: &str = "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run";
static AL_REGKEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";
static ADMIN_TASK_MANAGER_OVERRIDE_REGKEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run32";
static TASK_MANAGER_OVERRIDE_REGKEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

const CREATE_NO_WINDOW: u32 = 0x08000000;
const HRESULT_ACCESS_DENIED: i32 = 0x80070005u32 as i32;
const HRESULT_FILE_NOT_FOUND: i32 = 0x80070002u32 as i32;
const HRESULT_PATH_NOT_FOUND: i32 = 0x80070003u32 as i32;
const SCHED_E_TASK_NOT_FOUND: i32 = 0x8004130fu32 as i32;
const TASK_SUFFIX: &str = " Auto Launch";

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

fn remove_legacy_registrations(app_name: &str) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    remove_owned_registration(
        &hkcu,
        AL_REGKEY,
        TASK_MANAGER_OVERRIDE_REGKEY,
        app_name,
    )?;

    // Older versions could register an elevated Run32 entry. Remove only an
    // entry that still belongs to this executable name and only when the
    // current process already has permission to do so.
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let _ = remove_owned_registration(
        &hklm,
        ADMIN_AL_REGKEY,
        ADMIN_TASK_MANAGER_OVERRIDE_REGKEY,
        app_name,
    );
    Ok(())
}

fn decode_command_output(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) {
        let utf16 = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&utf16);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        let utf16 = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&utf16);
    }
    if bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0 {
        let utf16 = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&utf16);
    }
    if bytes.len() >= 4 && bytes[0] == 0 && bytes[2] == 0 {
        let utf16 = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&utf16);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn schtasks(args: &[&str]) -> std::io::Result<Output> {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

fn elevated_schtasks(args: &[&str]) -> std::io::Result<Output> {
    const ELEVATED_SCHTASKS_SCRIPT: &str = r#"
$ErrorActionPreference = "Stop"
try {
    $process = Start-Process `
        -FilePath (Join-Path $env:SystemRoot "System32\schtasks.exe") `
        -ArgumentList $env:CLASH_VERGE_SCHTASKS_ARGUMENTS `
        -Verb RunAs `
        -WindowStyle Hidden `
        -Wait `
        -PassThru
    exit $process.ExitCode
} catch {
    exit 1
}
"#;
    let arguments = args
        .iter()
        .map(|argument| quote_windows_argument(argument))
        .collect::<Vec<_>>()
        .join(" ");

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            ELEVATED_SCHTASKS_SCRIPT,
        ])
        .env("CLASH_VERGE_SCHTASKS_ARGUMENTS", arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

fn schtasks_error(operation: &str, output: &Output) -> std::io::Error {
    let stdout = decode_command_output(&output.stdout);
    let stderr = decode_command_output(&output.stderr);
    let details = [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|text| !text.is_empty() && !text.contains('\u{fffd}'))
        .collect::<Vec<_>>()
        .join(" | ");
    let details = if details.is_empty() {
        String::new()
    } else {
        format!(": {details}")
    };
    std::io::Error::new(
        ErrorKind::Other,
        format!(
            "schtasks.exe {operation} failed with status {}{details}",
            output.status.code().unwrap_or(-1),
        ),
    )
}

fn schtasks_mutation(
    operation: &str,
    args: &[&str],
    allow_not_found: bool,
) -> std::io::Result<()> {
    let output = schtasks(args)?;
    if output.status.success()
        || (allow_not_found && task_not_found(output.status.code()))
    {
        return Ok(());
    }
    if output.status.code() != Some(HRESULT_ACCESS_DENIED) {
        return Err(schtasks_error(operation, &output));
    }

    let elevated = elevated_schtasks(args)?;
    if elevated.status.success()
        || (allow_not_found && task_not_found(elevated.status.code()))
    {
        return Ok(());
    }
    Err(std::io::Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "schtasks.exe {operation} requires administrator approval; elevation was denied or failed with status {}",
            elevated.status.code().unwrap_or(-1)
        ),
    ))
}

fn task_name(app_name: &str) -> String {
    format!("{app_name}{TASK_SUFFIX}")
}

fn task_not_found(exit_code: Option<i32>) -> bool {
    matches!(
        exit_code,
        Some(HRESULT_FILE_NOT_FOUND)
            | Some(HRESULT_PATH_NOT_FOUND)
            | Some(SCHED_E_TASK_NOT_FOUND)
    )
}

fn query_task_xml(task_name: &str) -> std::io::Result<Option<String>> {
    let output = schtasks(&[
        "/Query",
        "/TN",
        task_name,
        "/XML",
        "ONE",
        "/HRESULT",
    ])?;
    if output.status.success() {
        return Ok(Some(decode_command_output(&output.stdout)));
    }
    if task_not_found(output.status.code()) {
        return Ok(None);
    }
    Err(schtasks_error("query", &output))
}

fn delete_task(task_name: &str) -> std::io::Result<()> {
    schtasks_mutation(
        "delete",
        &["/Delete", "/TN", task_name, "/F", "/HRESULT"],
        true,
    )
}

fn current_user_sid() -> std::io::Result<String> {
    let output = Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::new(
            ErrorKind::Other,
            "whoami.exe failed while querying the current user SID",
        ));
    }
    let line = decode_command_output(&output.stdout);
    line.trim()
        .trim_matches('"')
        .split("\",\"")
        .last()
        .map(str::trim)
        .filter(|sid| sid.starts_with("S-1-"))
        .map(str::to_string)
        .ok_or_else(|| {
            std::io::Error::new(
                ErrorKind::InvalidData,
                "failed to parse the current Windows user SID",
            )
        })
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn xml_element<'a>(xml: &'a str, element: &str) -> Option<&'a str> {
    let start_tag = format!("<{element}>");
    let end_tag = format!("</{element}>");
    let start = xml.find(&start_tag)? + start_tag.len();
    let end = xml[start..].find(&end_tag)? + start;
    Some(xml[start..end].trim())
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte == b'"')
    {
        return argument.to_string();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn build_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn task_is_owned(xml: &str, app_name: &str) -> bool {
    xml_element(xml, "Command")
        .map(xml_unescape)
        .and_then(|command| PathBuf::from(command).file_stem().map(|stem| stem.to_owned()))
        .and_then(|stem| stem.to_str().map(str::to_string))
        .map(|stem| stem.eq_ignore_ascii_case(app_name))
        .unwrap_or(false)
}

fn task_matches(xml: &str, app_name: &str, app_path: &str, args: &[String], sid: &str) -> bool {
    if !task_is_owned(xml, app_name) {
        return false;
    }

    let command_matches = xml_element(xml, "Command")
        .map(xml_unescape)
        .map(|command| command.eq_ignore_ascii_case(app_path))
        .unwrap_or(false);
    let expected_args = build_arguments(args);
    let actual_args = xml_element(xml, "Arguments")
        .map(xml_unescape)
        .unwrap_or_default();
    let user_matches = xml_element(xml, "UserId")
        .map(xml_unescape)
        .map(|user| user.eq_ignore_ascii_case(sid))
        .unwrap_or(false);

    command_matches
        && actual_args == expected_args
        && user_matches
        && xml.contains("<LogonType>InteractiveToken</LogonType>")
        && xml.contains("<RunLevel>HighestAvailable</RunLevel>")
        && xml.contains("<Delay>PT10S</Delay>")
        && xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>")
        && !xml.contains("<RestartOnFailure>")
        && !xml.contains("<Enabled>false</Enabled>")
}

fn task_xml(app_name: &str, app_path: &str, args: &[String], sid: &str) -> String {
    let description = xml_escape(&format!("Start {app_name} when the user logs on"));
    let working_directory = Path::new(app_path)
        .parent()
        .and_then(Path::to_str)
        .map(xml_escape)
        .map(|path| format!("      <WorkingDirectory>{path}</WorkingDirectory>\n"))
        .unwrap_or_default();
    let app_path = xml_escape(app_path);
    let sid = xml_escape(sid);
    let arguments = build_arguments(args);
    let arguments = if arguments.is_empty() {
        String::new()
    } else {
        format!("      <Arguments>{}</Arguments>\n", xml_escape(&arguments))
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{description}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <Delay>PT10S</Delay>
      <UserId>{sid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>false</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{app_path}</Command>
{arguments}{working_directory}    </Exec>
  </Actions>
</Task>
"#
    )
}

fn encode_task_xml(contents: &str) -> Vec<u8> {
    let mut utf16 = Vec::with_capacity(contents.len() * 2 + 2);
    utf16.extend_from_slice(&[0xff, 0xfe]);
    for code_unit in contents.encode_utf16() {
        utf16.extend_from_slice(&code_unit.to_le_bytes());
    }
    utf16
}

struct TemporaryTaskXml {
    path: PathBuf,
}

impl TemporaryTaskXml {
    fn create(contents: &str) -> std::io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "clash-verge-auto-launch-{}-{timestamp}.xml",
            std::process::id()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let utf16 = encode_task_xml(contents);
        if let Err(err) = file.write_all(&utf16) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(err);
        }
        Ok(Self { path })
    }
}

impl Drop for TemporaryTaskXml {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Windows implementation using an interactive, highest-privilege logon task.
impl AutoLaunch {
    /// Create a new AutoLaunch instance.
    pub fn new(app_name: &str, app_path: &str, args: &[impl AsRef<str>]) -> AutoLaunch {
        AutoLaunch {
            app_name: app_name.into(),
            app_path: app_path.into(),
            args: args.iter().map(|s| s.as_ref().to_string()).collect(),
        }
    }

    fn executable_path(&self) -> &str {
        self.app_path
            .trim()
            .strip_prefix('"')
            .and_then(|path| path.strip_suffix('"'))
            .unwrap_or_else(|| self.app_path.trim())
    }

    /// Enable auto launch by creating an elevated task for the current user.
    pub fn enable(&self) -> Result<()> {
        let executable_path = self.executable_path();
        let executable = Path::new(executable_path);
        if !executable.exists() {
            return Err(Error::AppPathDoesntExist(executable.to_path_buf()));
        }

        let task_name = task_name(&self.app_name);
        let existing = query_task_xml(&task_name)?;
        if let Some(existing) = existing.as_ref() {
            if !task_is_owned(existing, &self.app_name) {
                return Err(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("scheduled task '{task_name}' is owned by another application"),
                )
                .into());
            }
        }

        let sid = current_user_sid()?;
        if existing.as_deref().is_some_and(|xml| {
            task_matches(
                xml,
                &self.app_name,
                executable_path,
                &self.args,
                &sid,
            )
        }) {
            remove_legacy_registrations(&self.app_name)?;
            return Ok(());
        }

        let xml = task_xml(&self.app_name, executable_path, &self.args, &sid);
        let temporary_xml = TemporaryTaskXml::create(&xml)?;
        let temporary_xml_path = temporary_xml.path.to_string_lossy();
        schtasks_mutation(
            "create",
            &[
                "/Create",
                "/TN",
                &task_name,
                "/XML",
                &temporary_xml_path,
                "/F",
                "/HRESULT",
            ],
            false,
        )?;

        let registered = query_task_xml(&task_name)?.ok_or_else(|| {
            std::io::Error::new(
                ErrorKind::NotFound,
                format!("scheduled task '{task_name}' was not found after creation"),
            )
        })?;
        if !task_matches(
            &registered,
            &self.app_name,
            executable_path,
            &self.args,
            &sid,
        ) {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!("scheduled task '{task_name}' does not match the requested configuration"),
            )
            .into());
        }

        remove_legacy_registrations(&self.app_name)?;

        Ok(())
    }

    /// Disable auto launch and remove legacy registry registrations.
    pub fn disable(&self) -> Result<()> {
        let task_name = task_name(&self.app_name);
        if let Some(existing) = query_task_xml(&task_name)? {
            if task_is_owned(&existing, &self.app_name) {
                delete_task(&task_name)?;
            }
        }

        remove_legacy_registrations(&self.app_name)?;
        Ok(())
    }

    /// Check whether the elevated logon task matches this application.
    pub fn is_enabled(&self) -> Result<bool> {
        let task_name = task_name(&self.app_name);
        let Some(xml) = query_task_xml(&task_name)? else {
            return Ok(false);
        };
        let sid = current_user_sid()?;
        Ok(task_matches(
            &xml,
            &self.app_name,
            self.executable_path(),
            &self.args,
            &sid,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_arguments, command_targets_app, decode_command_output, encode_task_xml,
        task_is_owned, task_matches, task_name, task_not_found, task_xml, xml_escape,
        HRESULT_ACCESS_DENIED, HRESULT_FILE_NOT_FOUND, HRESULT_PATH_NOT_FOUND,
        SCHED_E_TASK_NOT_FOUND,
    };

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

    #[test]
    fn builds_stable_task_name() {
        assert_eq!(
            task_name("Clash-Verge-Buty"),
            "Clash-Verge-Buty Auto Launch"
        );
    }

    #[test]
    fn recognizes_missing_task_hresult_values() {
        assert!(task_not_found(Some(HRESULT_FILE_NOT_FOUND)));
        assert!(task_not_found(Some(HRESULT_PATH_NOT_FOUND)));
        assert!(task_not_found(Some(SCHED_E_TASK_NOT_FOUND)));
        assert!(!task_not_found(Some(5)));
        assert!(!task_not_found(None));
    }

    #[test]
    fn recognizes_access_denied_hresult() {
        assert_eq!(HRESULT_ACCESS_DENIED, -2147024891);
    }

    #[test]
    fn encodes_task_xml_as_utf16_le_with_bom() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?><Task>测试</Task>"#;
        let encoded = encode_task_xml(xml);

        assert!(encoded.starts_with(&[0xff, 0xfe]));
        assert_eq!(decode_command_output(&encoded), xml);
    }

    #[test]
    fn quotes_task_arguments() {
        assert_eq!(
            build_arguments(&[
                "--silent".to_string(),
                "value with spaces".to_string(),
                r#"quote"inside"#.to_string(),
            ]),
            r#"--silent "value with spaces" "quote\"inside""#
        );
    }

    #[test]
    fn builds_and_recognizes_elevated_interactive_task() {
        let app_name = "Clash-Verge-Buty";
        let app_path = r#"C:\Portable & Tools\Clash-Verge-Buty.exe"#;
        let args = vec!["--silent".to_string()];
        let sid = "S-1-5-21-1001";
        let xml = task_xml(app_name, app_path, &args, sid);

        assert!(xml.contains(&format!("<Command>{}</Command>", xml_escape(app_path))));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<Delay>PT10S</Delay>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(!xml.contains("<RestartOnFailure>"));
        assert!(task_is_owned(&xml, app_name));
        assert!(task_matches(&xml, app_name, app_path, &args, sid));
        let normalized_xml = xml.replace("<Enabled>true</Enabled>", "");
        assert!(task_matches(
            &normalized_xml,
            app_name,
            app_path,
            &args,
            sid
        ));
        let disabled_xml = xml.replace(
            "<Enabled>true</Enabled>",
            "<Enabled>false</Enabled>",
        );
        assert!(!task_matches(
            &disabled_xml,
            app_name,
            app_path,
            &args,
            sid
        ));
        assert!(!task_matches(
            &xml,
            app_name,
            r#"D:\Moved\Clash-Verge-Buty.exe"#,
            &args,
            sid
        ));
    }
}
