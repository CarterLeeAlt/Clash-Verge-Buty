use crate::watchdog_protocol::{CHILD_ARG, WATCHDOG_EXE};
use std::{ffi::OsStr, io, process::Command};

pub fn is_supervised() -> bool {
    if !std::env::args_os().any(|arg| arg == OsStr::new(CHILD_ARG)) {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Ok(expected) = dunce::canonicalize(exe.with_file_name(WATCHDOG_EXE)) else {
        return false;
    };
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    system.refresh_process(pid);
    let Some(parent) = system.process(pid).and_then(|process| process.parent()) else {
        return false;
    };
    system.refresh_process(parent);
    system
        .process(parent)
        .and_then(|process| process.exe())
        .and_then(|path| dunce::canonicalize(path).ok())
        .map(|path| {
            path.to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy())
        })
        .unwrap_or(false)
}

pub fn launch_supervisor() -> io::Result<bool> {
    if is_supervised() {
        return Ok(false);
    }

    let exe = std::env::current_exe()?;
    let helper = exe.with_file_name(WATCHDOG_EXE);
    if !helper.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("missing supervisor: {}", helper.display()),
        ));
    }

    Command::new(helper)
        .arg(exe)
        .args(std::env::args_os().skip(1))
        .spawn()?;
    Ok(true)
}
