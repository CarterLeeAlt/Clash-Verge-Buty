#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod process;
#[path = "../../src/watchdog_protocol.rs"]
mod protocol;

use protocol::{CHILD_ARG, RESTART_EXIT_CODE, WATCHDOG_EXE};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_RETRIES: usize = 3;
const RETRY_DELAY_SECS: u64 = 10;

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("Watchdog failed: {err}");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<i32> {
    let mut args = std::env::args_os().skip(1);
    let exe = args
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing GUI executable"))?;
    let exe = fs::canonicalize(exe)?;
    let helper = fs::canonicalize(std::env::current_exe()?)?;
    if exe == helper || exe.parent() != helper.parent() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "GUI executable must be beside Watchdog",
        ));
    }
    let Some(_instance) = process::Instance::acquire(&exe)? else {
        return Ok(0);
    };
    let gui_args: Vec<OsString> = args
        .filter(|arg| arg != CHILD_ARG)
        .chain(std::iter::once(OsString::from(CHILD_ARG)))
        .collect();
    let mut logger = Logger::new(exe.parent().unwrap())?;
    logger.write(&format!("{WATCHDOG_EXE} supervisor started"));
    let mut retries = 0;
    loop {
        let status = process::run_child(&exe, &gui_args);
        match status {
            Ok(0) => {
                logger.write("GUI exited normally; supervisor stopping");
                return Ok(0);
            }
            Ok(RESTART_EXIT_CODE) => {
                logger.write("GUI requested restart");
                retries = 0;
            }
            result => {
                let code = match &result {
                    Ok(code) => *code,
                    Err(_) => 1,
                };
                logger.write(&format!("GUI failed: {result:?}"));
                if retries == MAX_RETRIES {
                    logger.write("retry limit reached; supervisor stopping");
                    return Ok(code);
                }
                retries += 1;
                logger.write(&format!(
                    "restarting in {RETRY_DELAY_SECS}s ({retries}/{MAX_RETRIES})"
                ));
                std::thread::sleep(Duration::from_secs(RETRY_DELAY_SECS));
            }
        }
    }
}

struct Logger {
    file: File,
}

impl Logger {
    fn new(exe_dir: &Path) -> io::Result<Self> {
        let path: PathBuf = exe_dir
            .join(".config")
            .join("io.github.clash-verge-buty.data")
            .join("logs")
            .join("watchdog");
        fs::create_dir_all(&path)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(Self {
            file: OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path.join(format!("{timestamp}-{}.log", std::process::id())))?,
        })
    }

    fn write(&mut self, message: &str) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(self.file, "{timestamp} {message}");
        let _ = self.file.flush();
    }
}
