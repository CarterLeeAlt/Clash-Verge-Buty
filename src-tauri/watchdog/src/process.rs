use std::{ffi::OsString, io, path::Path, process::Command};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
};

pub struct Instance {
    mutex: HANDLE,
}

impl Instance {
    pub fn acquire(exe: &Path) -> io::Result<Option<Self>> {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in exe.to_string_lossy().to_lowercase().as_bytes() {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
        let name: Vec<u16> = format!("Local\\clash-verge-buty-watchdog-{hash:016x}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if mutex == 0 {
            return Err(io::Error::last_os_error());
        }
        let instance = Self { mutex };
        if already_exists {
            return Ok(None);
        }
        Ok(Some(instance))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.mutex) };
    }
}

pub fn run_child(exe: &Path, args: &[OsString]) -> io::Result<i32> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let status = Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?
        .wait()?;
    Ok(status.code().unwrap_or(1))
}
