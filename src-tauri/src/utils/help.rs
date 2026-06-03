use crate::utils::dirs;
use anyhow::{anyhow, bail, Context, Result};
use nanoid::nanoid;
use serde::{de::DeserializeOwned, Serialize};
use serde_yaml::{Mapping, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{
    api::shell::{open, Program},
    Manager,
};

/// read data from yaml as struct T
pub fn read_yaml<T: DeserializeOwned>(path: &PathBuf) -> Result<T> {
    if !path.exists() {
        bail!("file not found \"{}\"", path.display());
    }

    let yaml_str = fs::read_to_string(path)
        .with_context(|| format!("failed to read the file \"{}\"", path.display()))?;

    serde_yaml::from_str::<T>(&yaml_str).with_context(|| {
        format!(
            "failed to read the file with yaml format \"{}\"",
            path.display()
        )
    })
}

/// read mapping from yaml fix #165
pub fn read_merge_mapping(path: &PathBuf) -> Result<Mapping> {
    let mut val: Value = read_yaml(path)?;
    val.apply_merge()
        .with_context(|| format!("failed to apply merge \"{}\"", path.display()))?;

    Ok(val
        .as_mapping()
        .ok_or(anyhow!(
            "failed to transform to yaml mapping \"{}\"",
            path.display()
        ))?
        .to_owned())
}

/// save the data to the file
/// can set `prefix` string to add some comments
pub fn save_yaml<T: Serialize>(path: &PathBuf, data: &T, prefix: Option<&str>) -> Result<()> {
    let data_str = serde_yaml::to_string(data)?;

    let yaml_str = match prefix {
        Some(prefix) => format!("{prefix}\n\n{data_str}"),
        None => data_str,
    };

    let path_str = path.as_os_str().to_string_lossy().to_string();
    write_file_atomic(path, yaml_str.as_bytes())
        .with_context(|| format!("failed to save file \"{path_str}\""))
}

pub fn write_file_atomic(path: &PathBuf, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(anyhow!("failed to get parent dir for {}", path.display()))?;

    if !parent.exists() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent dir '{}'", parent.display()))?;
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".{}.{}.tmp", nanoid!(6, &ALPHABET), nanos);
    let tmp_path = parent.join(tmp_name);

    let write_result = (|| -> Result<()> {
        let mut file = fs::File::create(&tmp_path)
            .with_context(|| format!("failed to create temp file '{}'", tmp_path.display()))?;
        file.write_all(data)
            .with_context(|| format!("failed to write temp file '{}'", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync temp file '{}'", tmp_path.display()))?;
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    match fs::rename(&tmp_path, path) {
        Ok(_) => Ok(()),
        Err(err) => {
            #[cfg(target_os = "windows")]
            {
                if path.exists() {
                    let backup_name = format!(".{}.{}.bak", nanoid!(6, &ALPHABET), nanos);
                    let backup_path = parent.join(backup_name);

                    fs::rename(path, &backup_path).with_context(|| {
                        format!(
                            "failed to move existing file '{}' to backup '{}'",
                            path.display(),
                            backup_path.display()
                        )
                    })?;

                    match fs::rename(&tmp_path, path) {
                        Ok(_) => {
                            let _ = fs::remove_file(&backup_path);
                            return Ok(());
                        }
                        Err(rename_err) => {
                            let _ = fs::rename(&backup_path, path);
                            let _ = fs::remove_file(&tmp_path);
                            return Err(rename_err).with_context(|| {
                                format!(
                                    "failed to replace file '{}' with temp file '{}'",
                                    path.display(),
                                    tmp_path.display()
                                )
                            });
                        }
                    }
                }
            }

            let _ = fs::remove_file(&tmp_path);
            Err(err).with_context(|| {
                format!(
                    "failed to replace file '{}' with temp file '{}'",
                    path.display(),
                    tmp_path.display()
                )
            })
        }
    }
}
const ALPHABET: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i',
    'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B',
    'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U',
    'V', 'W', 'X', 'Y', 'Z',
];

/// generate the uid
pub fn get_uid(prefix: &str) -> String {
    let id = nanoid!(11, &ALPHABET);
    format!("{prefix}{id}")
}

/// parse the string
/// xxx=123123; => 123123
pub fn parse_str<T: FromStr>(target: &str, key: &str) -> Option<T> {
    target.split(';').map(str::trim).find_map(|s| {
        let mut parts = s.splitn(2, '=');
        match (parts.next(), parts.next()) {
            (Some(k), Some(v)) if k == key => v.parse::<T>().ok(),
            _ => None,
        }
    })
}

/// get the last part of the url, if not found, return empty string
pub fn get_last_part_and_decode(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(""); // Splits URL and takes the path part
    let segments: Vec<&str> = path.split('/').collect();
    let last_segment = segments.last()?;

    Some(
        percent_encoding::percent_decode_str(last_segment)
            .decode_utf8_lossy()
            .to_string(),
    )
}

pub fn resolve_profile_path(file: &str) -> Result<PathBuf> {
    let file = file.trim();
    if file.is_empty() {
        bail!("Invalid profile file name");
    }

    if file == "." || file == ".." {
        bail!("Invalid profile file name");
    }

    let candidate = std::path::Path::new(file);
    if candidate.is_absolute() {
        bail!("Invalid profile file name");
    }

    if candidate.components().count() != 1 {
        bail!("Invalid profile file name");
    }

    if file.contains(['/', '\\']) || file.contains(':') {
        bail!("Invalid profile file name");
    }

    let base = dirs::app_profiles_dir()?;
    Ok(base.join(file))
}
/// open file
/// use vscode by default
pub fn open_file(app: tauri::AppHandle, path: PathBuf) -> Result<()> {
    #[cfg(target_os = "macos")]
    let code = "Visual Studio Code";
    #[cfg(not(target_os = "macos"))]
    let code = "code";

    let _ = match Program::from_str(code) {
        Ok(code) => open(&app.shell_scope(), path.to_string_lossy(), Some(code)),
        Err(err) => {
            log::error!(target: "app", "Can't find VScode `{err}`");
            // default open
            open(&app.shell_scope(), path.to_string_lossy(), None)
        }
    };

    Ok(())
}

#[macro_export]
macro_rules! error {
    ($result: expr) => {
        log::error!(target: "app", "{}", $result);
    };
}

#[macro_export]
macro_rules! log_err {
    ($result: expr) => {
        if let Err(err) = $result {
            log::error!(target: "app", "{err}");
        }
    };

    ($result: expr, $err_str: expr) => {
        if let Err(_) = $result {
            log::error!(target: "app", "{}", $err_str);
        }
    };
}

#[macro_export]
macro_rules! trace_err {
    ($result: expr, $err_str: expr) => {
        if let Err(err) = $result {
            log::trace!(target: "app", "{}, err {}", $err_str, err);
        }
    }
}

/// wrap the anyhow error
/// transform the error to String
#[macro_export]
macro_rules! wrap_err {
    ($stat: expr) => {
        match $stat {
            Ok(a) => Ok(a),
            Err(err) => {
                log::error!(target: "app", "{}", err.to_string());
                Err(format!("{}", err.to_string()))
            }
        }
    };
}

/// return the string literal error
#[macro_export]
macro_rules! ret_err {
    ($str: expr) => {
        return Err($str.into())
    };
}

#[test]
fn test_parse_value() {
    let test_1 = "upload=111; download=2222; total=3333; expire=444";
    let test_2 = "attachment; filename=Clash.yaml";

    assert_eq!(parse_str::<usize>(test_1, "upload").unwrap(), 111);
    assert_eq!(parse_str::<usize>(test_1, "download").unwrap(), 2222);
    assert_eq!(parse_str::<usize>(test_1, "total").unwrap(), 3333);
    assert_eq!(parse_str::<usize>(test_1, "expire").unwrap(), 444);
    assert_eq!(
        parse_str::<String>(test_2, "filename").unwrap(),
        format!("Clash.yaml")
    );

    assert_eq!(parse_str::<usize>(test_1, "aaa"), None);
    assert_eq!(parse_str::<usize>(test_1, "upload1"), None);
    assert_eq!(parse_str::<usize>(test_1, "expire1"), None);
    assert_eq!(parse_str::<usize>(test_2, "attachment"), None);
}
