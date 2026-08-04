use crate::{Error, Result, Sysproxy};
use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};
use windows::core::PWSTR;
use windows::Win32::Networking::WinInet::{
    InternetSetOptionW, INTERNET_OPTION_PER_CONNECTION_OPTION,
    INTERNET_OPTION_PROXY_SETTINGS_CHANGED, INTERNET_OPTION_REFRESH, INTERNET_PER_CONN_FLAGS,
    INTERNET_PER_CONN_OPTIONW, INTERNET_PER_CONN_OPTIONW_0, INTERNET_PER_CONN_OPTION_LISTW,
    INTERNET_PER_CONN_PROXY_BYPASS, INTERNET_PER_CONN_PROXY_SERVER, PROXY_TYPE_DIRECT,
    PROXY_TYPE_PROXY,
};
use winreg::transaction::Transaction;
use winreg::{enums, RegKey, RegValue};

pub use windows::core::Error as Win32Error;

const SUB_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
const CONNECTIONS_SUB_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\\Connections";
const RECOVERY_PARENT_KEY: &str = "SOFTWARE\\Clash-Verge-Buty";
const RECOVERY_KEY: &str = "SOFTWARE\\Clash-Verge-Buty\\ProxyRecovery";
const RECOVERY_ORIGINAL_KEY: &str = "SOFTWARE\\Clash-Verge-Buty\\ProxyRecovery\\Original";
const RECOVERY_OWNED_KEY: &str = "SOFTWARE\\Clash-Verge-Buty\\ProxyRecovery\\Owned";
const RECOVERY_PHASE_PREPARED: u32 = 1;
const RECOVERY_PHASE_OWNED: u32 = 2;
const SNAPSHOT_PROXY_ENABLE: u32 = 1 << 0;
const SNAPSHOT_PROXY_SERVER: u32 = 1 << 1;
const SNAPSHOT_PROXY_OVERRIDE: u32 = 1 << 2;
const SNAPSHOT_AUTO_CONFIG_URL: u32 = 1 << 3;
const SNAPSHOT_AUTO_DETECT: u32 = 1 << 4;
const SNAPSHOT_DEFAULT_CONNECTION: u32 = 1 << 5;
const SNAPSHOT_SAVED_LEGACY: u32 = 1 << 6;

fn parse_proxy_endpoint(endpoint: &str) -> Option<(String, u16)> {
    let endpoint = endpoint.trim();
    let endpoint = endpoint
        .split_once("://")
        .map(|(_, value)| value)
        .unwrap_or(endpoint);

    if let Some(ipv6) = endpoint.strip_prefix('[') {
        let bracket = ipv6.find(']')?;
        let host = ipv6[..bracket].trim();
        let port = ipv6[bracket + 1..].strip_prefix(':')?.parse().ok()?;
        return (!host.is_empty() && port != 0).then(|| (host.to_string(), port));
    }

    let (host, port) = endpoint.rsplit_once(':')?;
    let host = host.trim();
    if host.is_empty() || host.contains(':') {
        return None;
    }

    let port = port.trim().parse().ok()?;
    (port != 0).then(|| (host.to_string(), port))
}

fn parse_proxy_server(server: &str) -> Result<(String, u16)> {
    let entries = server
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();

    for protocol in ["http", "https", "socks"] {
        for entry in &entries {
            let Some((name, endpoint)) = entry.split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case(protocol) {
                if let Some(proxy) = parse_proxy_endpoint(endpoint) {
                    return Ok(proxy);
                }
            }
        }
    }

    for entry in entries.iter().filter(|entry| !entry.contains('=')) {
        if let Some(proxy) = parse_proxy_endpoint(entry) {
            return Ok(proxy);
        }
    }

    Err(Error::ParseStr(server.to_string()))
}

fn resolve_proxy_endpoint(enable: bool, server: &str) -> Result<(String, u16)> {
    if enable {
        parse_proxy_server(server)
    } else {
        Ok((String::new(), 0))
    }
}

fn notify_proxy_settings_changed() -> Result<()> {
    unsafe {
        InternetSetOptionW(None, INTERNET_OPTION_PROXY_SETTINGS_CHANGED, None, 0)?;
        InternetSetOptionW(None, INTERNET_OPTION_REFRESH, None, 0)?;
    }
    Ok(())
}

#[derive(Clone, PartialEq)]
struct RawRegistryValue {
    bytes: Vec<u8>,
    vtype: enums::RegType,
}

impl RawRegistryValue {
    fn from_reg_value(value: RegValue) -> Self {
        Self {
            bytes: value.bytes,
            vtype: value.vtype,
        }
    }

    fn to_reg_value(&self) -> RegValue {
        RegValue {
            bytes: self.bytes.clone(),
            vtype: self.vtype.clone(),
        }
    }
}

/// Exact user-scoped Windows proxy registry state used for reversible updates.
#[derive(Clone, PartialEq)]
pub struct WindowsProxySnapshot {
    connections_existed: bool,
    proxy_enable: Option<RawRegistryValue>,
    proxy_server: Option<RawRegistryValue>,
    proxy_override: Option<RawRegistryValue>,
    auto_config_url: Option<RawRegistryValue>,
    auto_detect: Option<RawRegistryValue>,
    default_connection_settings: Option<RawRegistryValue>,
    saved_legacy_settings: Option<RawRegistryValue>,
}

struct WindowsProxyRecovery {
    original: WindowsProxySnapshot,
    owned: Option<WindowsProxySnapshot>,
    target: Sysproxy,
}

fn read_raw_value(key: Option<&RegKey>, name: &str) -> Result<Option<RawRegistryValue>> {
    let Some(key) = key else {
        return Ok(None);
    };

    match key.get_raw_value(name) {
        Ok(value) => Ok(Some(RawRegistryValue::from_reg_value(value))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn restore_raw_value(key: &RegKey, name: &str, value: &Option<RawRegistryValue>) -> Result<()> {
    match value {
        Some(value) => key.set_raw_value(name, &value.to_reg_value())?,
        None => match key.delete_value(name) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        },
    }
    Ok(())
}

fn restore_raw_value_if_unchanged(
    key: &RegKey,
    name: &str,
    original: &Option<RawRegistryValue>,
    owned: &Option<RawRegistryValue>,
    current: &Option<RawRegistryValue>,
) -> Result<bool> {
    if current != owned {
        log::warn!("skip restoring Windows proxy value {name} because it changed after app enable");
        return Ok(false);
    }

    restore_raw_value(key, name, original)?;
    Ok(true)
}

fn write_snapshot_value(
    key: &RegKey,
    name: &str,
    bit: u32,
    value: &Option<RawRegistryValue>,
    present_mask: &mut u32,
) -> Result<()> {
    if let Some(value) = value {
        key.set_raw_value(name, &value.to_reg_value())?;
        *present_mask |= bit;
    }
    Ok(())
}

fn write_snapshot(key: &RegKey, snapshot: &WindowsProxySnapshot) -> Result<()> {
    let mut present_mask = 0u32;
    write_snapshot_value(
        key,
        "ProxyEnable",
        SNAPSHOT_PROXY_ENABLE,
        &snapshot.proxy_enable,
        &mut present_mask,
    )?;
    write_snapshot_value(
        key,
        "ProxyServer",
        SNAPSHOT_PROXY_SERVER,
        &snapshot.proxy_server,
        &mut present_mask,
    )?;
    write_snapshot_value(
        key,
        "ProxyOverride",
        SNAPSHOT_PROXY_OVERRIDE,
        &snapshot.proxy_override,
        &mut present_mask,
    )?;
    write_snapshot_value(
        key,
        "AutoConfigURL",
        SNAPSHOT_AUTO_CONFIG_URL,
        &snapshot.auto_config_url,
        &mut present_mask,
    )?;
    write_snapshot_value(
        key,
        "AutoDetect",
        SNAPSHOT_AUTO_DETECT,
        &snapshot.auto_detect,
        &mut present_mask,
    )?;
    write_snapshot_value(
        key,
        "DefaultConnectionSettings",
        SNAPSHOT_DEFAULT_CONNECTION,
        &snapshot.default_connection_settings,
        &mut present_mask,
    )?;
    write_snapshot_value(
        key,
        "SavedLegacySettings",
        SNAPSHOT_SAVED_LEGACY,
        &snapshot.saved_legacy_settings,
        &mut present_mask,
    )?;
    key.set_value("PresentMask", &present_mask)?;
    key.set_value(
        "ConnectionsExisted",
        &(if snapshot.connections_existed {
            1u32
        } else {
            0u32
        }),
    )?;
    Ok(())
}

fn read_snapshot_value(
    key: &RegKey,
    name: &str,
    bit: u32,
    present_mask: u32,
) -> Result<Option<RawRegistryValue>> {
    if present_mask & bit == 0 {
        return Ok(None);
    }
    Ok(Some(RawRegistryValue::from_reg_value(
        key.get_raw_value(name)?,
    )))
}

fn read_snapshot(key: &RegKey) -> Result<WindowsProxySnapshot> {
    let present_mask = key.get_value::<u32, _>("PresentMask")?;
    Ok(WindowsProxySnapshot {
        connections_existed: key.get_value::<u32, _>("ConnectionsExisted")? == 1,
        proxy_enable: read_snapshot_value(
            key,
            "ProxyEnable",
            SNAPSHOT_PROXY_ENABLE,
            present_mask,
        )?,
        proxy_server: read_snapshot_value(
            key,
            "ProxyServer",
            SNAPSHOT_PROXY_SERVER,
            present_mask,
        )?,
        proxy_override: read_snapshot_value(
            key,
            "ProxyOverride",
            SNAPSHOT_PROXY_OVERRIDE,
            present_mask,
        )?,
        auto_config_url: read_snapshot_value(
            key,
            "AutoConfigURL",
            SNAPSHOT_AUTO_CONFIG_URL,
            present_mask,
        )?,
        auto_detect: read_snapshot_value(
            key,
            "AutoDetect",
            SNAPSHOT_AUTO_DETECT,
            present_mask,
        )?,
        default_connection_settings: read_snapshot_value(
            key,
            "DefaultConnectionSettings",
            SNAPSHOT_DEFAULT_CONNECTION,
            present_mask,
        )?,
        saved_legacy_settings: read_snapshot_value(
            key,
            "SavedLegacySettings",
            SNAPSHOT_SAVED_LEGACY,
            present_mask,
        )?,
    })
}

fn clear_recovery_key() -> Result<()> {
    let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
    match hkcu.delete_subkey_all(RECOVERY_KEY) {
        Ok(()) => {
            let _ = hkcu.delete_subkey(RECOVERY_PARENT_KEY);
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let _ = hkcu.delete_subkey(RECOVERY_PARENT_KEY);
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

fn load_recovery() -> Result<Option<WindowsProxyRecovery>> {
    let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
    let recovery = match hkcu.open_subkey_with_flags(RECOVERY_KEY, enums::KEY_READ) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let original_key = hkcu.open_subkey_with_flags(RECOVERY_ORIGINAL_KEY, enums::KEY_READ)?;
    let phase = recovery.get_value::<u32, _>("Phase")?;
    let owned = match phase {
        RECOVERY_PHASE_PREPARED => None,
        RECOVERY_PHASE_OWNED => {
            let owned_key =
                hkcu.open_subkey_with_flags(RECOVERY_OWNED_KEY, enums::KEY_READ)?;
            Some(read_snapshot(&owned_key)?)
        }
        value => {
            return Err(Error::RecoveryJournal(format!(
                "unsupported recovery phase {value}"
            )))
        }
    };
    let port = recovery.get_value::<u32, _>("TargetPort")?;
    let port = u16::try_from(port)
        .map_err(|_| Error::RecoveryJournal(format!("invalid target port {port}")))?;
    if port == 0 {
        return Err(Error::RecoveryJournal("invalid target port 0".into()));
    }
    let target_enable = recovery.get_value::<u32, _>("TargetEnable")?;
    if target_enable != 1 {
        return Err(Error::RecoveryJournal(format!(
            "invalid target enable value {target_enable}"
        )));
    }

    Ok(Some(WindowsProxyRecovery {
        original: read_snapshot(&original_key)?,
        owned,
        target: Sysproxy {
            enable: true,
            host: recovery.get_value("TargetHost")?,
            port,
            bypass: recovery.get_value("TargetBypass")?,
        },
    }))
}

fn proxy_matches(actual: &Sysproxy, expected: &Sysproxy) -> bool {
    actual.enable == expected.enable
        && actual.host.eq_ignore_ascii_case(&expected.host)
        && actual.port == expected.port
        && actual.bypass == expected.bypass
}

impl WindowsProxySnapshot {
    /// Capture values touched by WinINet or the registry fallback, including
    /// whether each value existed before the update.
    pub fn capture() -> Result<Self> {
        let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
        let settings = hkcu.open_subkey_with_flags(SUB_KEY, enums::KEY_READ)?;
        let connections = match hkcu.open_subkey_with_flags(CONNECTIONS_SUB_KEY, enums::KEY_READ) {
            Ok(key) => Some(key),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };

        Ok(Self {
            connections_existed: connections.is_some(),
            proxy_enable: read_raw_value(Some(&settings), "ProxyEnable")?,
            proxy_server: read_raw_value(Some(&settings), "ProxyServer")?,
            proxy_override: read_raw_value(Some(&settings), "ProxyOverride")?,
            auto_config_url: read_raw_value(Some(&settings), "AutoConfigURL")?,
            auto_detect: read_raw_value(Some(&settings), "AutoDetect")?,
            default_connection_settings: read_raw_value(
                connections.as_ref(),
                "DefaultConnectionSettings",
            )?,
            saved_legacy_settings: read_raw_value(
                connections.as_ref(),
                "SavedLegacySettings",
            )?,
        })
    }

    /// Persist the original state before any proxy write. The prepared journal
    /// is committed before WinINet or registry settings are changed.
    pub fn prepare_recovery(&self, target: &Sysproxy) -> Result<()> {
        let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
        let transaction = Transaction::new()?;
        let (recovery, _) = hkcu.create_subkey_transacted(RECOVERY_KEY, &transaction)?;
        let (original, _) =
            hkcu.create_subkey_transacted(RECOVERY_ORIGINAL_KEY, &transaction)?;
        write_snapshot(&original, self)?;
        recovery.set_value("Phase", &RECOVERY_PHASE_PREPARED)?;
        recovery.set_value("TargetEnable", &(if target.enable { 1u32 } else { 0u32 }))?;
        recovery.set_value("TargetHost", &target.host)?;
        recovery.set_value("TargetPort", &(target.port as u32))?;
        recovery.set_value("TargetBypass", &target.bypass)?;
        transaction.commit()?;
        Ok(())
    }

    /// Record the exact state observed after the app successfully owns the
    /// proxy so recovery can preserve later changes made by other software.
    pub fn record_owned_recovery(&self, target: &Sysproxy) -> Result<()> {
        let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
        let transaction = Transaction::new()?;
        let recovery = hkcu.open_subkey_transacted_with_flags(
            RECOVERY_KEY,
            &transaction,
            enums::KEY_ALL_ACCESS,
        )?;
        let (owned, _) = hkcu.create_subkey_transacted(RECOVERY_OWNED_KEY, &transaction)?;
        write_snapshot(&owned, self)?;
        recovery.set_value("TargetEnable", &(if target.enable { 1u32 } else { 0u32 }))?;
        recovery.set_value("TargetHost", &target.host)?;
        recovery.set_value("TargetPort", &(target.port as u32))?;
        recovery.set_value("TargetBypass", &target.bypass)?;
        recovery.set_value("Phase", &RECOVERY_PHASE_OWNED)?;
        transaction.commit()?;
        Ok(())
    }

    /// Remove a completed or deliberately abandoned recovery journal.
    pub fn clear_recovery() -> Result<()> {
        clear_recovery_key()
    }

    /// Recover a proxy session that ended without running the normal exit
    /// handler. Returns true when a journal was found and handled.
    pub fn recover_pending() -> Result<bool> {
        let Some(recovery) = load_recovery()? else {
            return Ok(false);
        };
        let current = Self::capture()?;
        if current == recovery.original {
            clear_recovery_key()?;
            return Ok(true);
        }

        let target_is_current = Sysproxy::get_system_proxy()
            .map(|actual| proxy_matches(&actual, &recovery.target))
            .unwrap_or(false);

        match recovery.owned {
            Some(owned) => {
                if target_is_current {
                    recovery.original.restore_if_unchanged(&owned)?;
                } else {
                    log::warn!("pending Windows proxy recovery was abandoned because another process changed the proxy");
                }
                clear_recovery_key()?;
                Ok(true)
            }
            None if target_is_current => {
                recovery.original.restore_if_unchanged(&current)?;
                clear_recovery_key()?;
                Ok(true)
            }
            None => Err(Error::RecoveryJournal(
                "prepared journal does not match either the original or target proxy state".into(),
            )),
        }
    }

    /// Restore the captured values transactionally and notify Windows.
    pub fn restore(&self) -> Result<()> {
        let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
        let transaction = Transaction::new()?;
        let (settings, _) = hkcu.create_subkey_transacted(SUB_KEY, &transaction)?;
        let (connections, _) =
            hkcu.create_subkey_transacted(CONNECTIONS_SUB_KEY, &transaction)?;

        restore_raw_value(&settings, "ProxyEnable", &self.proxy_enable)?;
        restore_raw_value(&settings, "ProxyServer", &self.proxy_server)?;
        restore_raw_value(&settings, "ProxyOverride", &self.proxy_override)?;
        restore_raw_value(&settings, "AutoConfigURL", &self.auto_config_url)?;
        restore_raw_value(&settings, "AutoDetect", &self.auto_detect)?;
        restore_raw_value(
            &connections,
            "DefaultConnectionSettings",
            &self.default_connection_settings,
        )?;
        restore_raw_value(
            &connections,
            "SavedLegacySettings",
            &self.saved_legacy_settings,
        )?;

        transaction.commit()?;

        if !self.connections_existed {
            drop(connections);
            if let Err(err) = hkcu.delete_subkey(CONNECTIONS_SUB_KEY) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("could not remove newly created empty proxy Connections key: {err}");
                }
            }
        }

        notify_proxy_settings_changed()?;
        let restored = Self::capture()?;
        if &restored != self {
            log::warn!("restored Windows proxy registry differs from the saved snapshot after system refresh");
        }
        Ok(())
    }

    /// Restore only values that still match the state written by this app.
    /// Values changed by another process are left untouched.
    pub fn restore_if_unchanged(&self, owned: &Self) -> Result<()> {
        let current = Self::capture()?;
        let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
        let transaction = Transaction::new()?;
        let (settings, _) = hkcu.create_subkey_transacted(SUB_KEY, &transaction)?;
        let (connections, _) =
            hkcu.create_subkey_transacted(CONNECTIONS_SUB_KEY, &transaction)?;

        restore_raw_value_if_unchanged(
            &settings,
            "ProxyEnable",
            &self.proxy_enable,
            &owned.proxy_enable,
            &current.proxy_enable,
        )?;
        restore_raw_value_if_unchanged(
            &settings,
            "ProxyServer",
            &self.proxy_server,
            &owned.proxy_server,
            &current.proxy_server,
        )?;
        restore_raw_value_if_unchanged(
            &settings,
            "ProxyOverride",
            &self.proxy_override,
            &owned.proxy_override,
            &current.proxy_override,
        )?;
        restore_raw_value_if_unchanged(
            &settings,
            "AutoConfigURL",
            &self.auto_config_url,
            &owned.auto_config_url,
            &current.auto_config_url,
        )?;
        restore_raw_value_if_unchanged(
            &settings,
            "AutoDetect",
            &self.auto_detect,
            &owned.auto_detect,
            &current.auto_detect,
        )?;
        let default_restored = restore_raw_value_if_unchanged(
            &connections,
            "DefaultConnectionSettings",
            &self.default_connection_settings,
            &owned.default_connection_settings,
            &current.default_connection_settings,
        )?;
        let legacy_restored = restore_raw_value_if_unchanged(
            &connections,
            "SavedLegacySettings",
            &self.saved_legacy_settings,
            &owned.saved_legacy_settings,
            &current.saved_legacy_settings,
        )?;

        transaction.commit()?;

        if !self.connections_existed && default_restored && legacy_restored {
            drop(connections);
            if let Err(err) = hkcu.delete_subkey(CONNECTIONS_SUB_KEY) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("could not remove newly created empty proxy Connections key: {err}");
                }
            }
        }

        notify_proxy_settings_changed()
    }

    /// Refresh only values that WinINet writes while guarding the app proxy.
    /// PAC and auto-detect values remain unchanged so external edits are still
    /// detected during restoration.
    pub fn refresh_app_owned_values(&mut self) -> Result<()> {
        let current = Self::capture()?;
        self.connections_existed = current.connections_existed;
        self.proxy_enable = current.proxy_enable;
        self.proxy_server = current.proxy_server;
        self.proxy_override = current.proxy_override;
        self.default_connection_settings = current.default_connection_settings;
        self.saved_legacy_settings = current.saved_legacy_settings;
        Ok(())
    }
}

fn set_registry_proxy(proxy: &Sysproxy) -> Result<()> {
    let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
    let transaction = Transaction::new()?;
    let (settings, _) = hkcu.create_subkey_transacted(SUB_KEY, &transaction)?;

    settings.set_value("ProxyEnable", &(if proxy.enable { 1u32 } else { 0u32 }))?;
    if proxy.enable {
        settings.set_value("ProxyServer", &format!("{}:{}", proxy.host, proxy.port))?;
        settings.set_value("ProxyOverride", &proxy.bypass)?;
        match settings.delete_value("AutoConfigURL") {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }

    transaction.commit()?;
    notify_proxy_settings_changed()
}

/// unset proxy for the default LAN connection
fn unset_proxy() -> Result<()> {
    let mut p_opts = ManuallyDrop::new(Vec::<INTERNET_PER_CONN_OPTIONW>::with_capacity(1));
    p_opts.push(INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_FLAGS,
        Value: {
            let mut v = INTERNET_PER_CONN_OPTIONW_0::default();
            v.dwValue = PROXY_TYPE_DIRECT;
            v
        },
    });
    let opts = INTERNET_PER_CONN_OPTION_LISTW {
        dwSize: size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
        dwOptionCount: 1,
        dwOptionError: 0,
        pOptions: p_opts.as_mut_ptr() as *mut INTERNET_PER_CONN_OPTIONW,
        pszConnection: PWSTR::null(),
    };
    let res = apply(&opts);
    unsafe {
        ManuallyDrop::drop(&mut p_opts);
    }
    res
}

/// set proxy for the default LAN connection
fn set_global_proxy(server: &str, bypass: &str) -> Result<()> {
    let mut p_opts = ManuallyDrop::new(Vec::<INTERNET_PER_CONN_OPTIONW>::with_capacity(3));
    p_opts.push(INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_FLAGS,
        Value: INTERNET_PER_CONN_OPTIONW_0 {
            dwValue: PROXY_TYPE_PROXY | PROXY_TYPE_DIRECT,
        },
    });

    let mut s = ManuallyDrop::new(server.encode_utf16().chain([0u16]).collect::<Vec<u16>>());
    p_opts.push(INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_PROXY_SERVER,
        Value: INTERNET_PER_CONN_OPTIONW_0 {
            pszValue: PWSTR::from_raw(s.as_ptr() as *mut u16),
        },
    });

    let mut b = ManuallyDrop::new(
        bypass.encode_utf16().chain([0u16]).collect::<Vec<u16>>(),
    );
    p_opts.push(INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_PROXY_BYPASS,
        Value: INTERNET_PER_CONN_OPTIONW_0 {
            pszValue: PWSTR::from_raw(b.as_ptr() as *mut u16),
        },
    });

    let opts = INTERNET_PER_CONN_OPTION_LISTW {
        dwSize: size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
        dwOptionCount: 3,
        dwOptionError: 0,
        pOptions: p_opts.as_mut_ptr() as *mut INTERNET_PER_CONN_OPTIONW,
        pszConnection: PWSTR::null(),
    };

    let res = apply(&opts);
    unsafe {
        ManuallyDrop::drop(&mut s);
        ManuallyDrop::drop(&mut b);
        ManuallyDrop::drop(&mut p_opts);
    }
    res
}

fn apply(options: &INTERNET_PER_CONN_OPTION_LISTW) -> Result<()> {
    unsafe {
        // setting options
        let opts = options as *const INTERNET_PER_CONN_OPTION_LISTW as *const c_void;
        InternetSetOptionW(
            None,
            INTERNET_OPTION_PER_CONNECTION_OPTION,
            Some(opts),
            size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
        )?;
    }
    Ok(())
}

fn set_wininet_proxy(proxy: &Sysproxy) -> Result<()> {
    let server = format!("{}:{}", proxy.host, proxy.port);
    match proxy.enable {
        true => set_global_proxy(&server, &proxy.bypass)?,
        false => unset_proxy()?,
    }

    notify_proxy_settings_changed()
}

impl Sysproxy {
    pub fn get_system_proxy() -> Result<Sysproxy> {
        let hkcu = RegKey::predef(enums::HKEY_CURRENT_USER);
        let cur_var = hkcu.open_subkey_with_flags(SUB_KEY, enums::KEY_READ)?;

        let enable = cur_var.get_value::<u32, _>("ProxyEnable").unwrap_or(0u32) == 1u32;
        let server = cur_var
            .get_value::<String, _>("ProxyServer")
            .unwrap_or("".into());
        let bypass = cur_var.get_value("ProxyOverride").unwrap_or("".into());

        // A missing or empty ProxyServer is a valid and common state while the
        // system proxy is disabled. Do not make enabling the proxy depend on a
        // stale value that Windows is not currently using.
        let (host, port) = resolve_proxy_endpoint(enable, &server)?;

        Ok(Sysproxy {
            enable,
            host,
            port,
            bypass,
        })
    }

    pub fn set_system_proxy(&self) -> Result<()> {
        set_wininet_proxy(self)
    }

    pub fn set_system_proxy_registry(&self) -> Result<()> {
        set_registry_proxy(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_proxy_server, resolve_proxy_endpoint};

    #[test]
    fn accepts_empty_proxy_while_disabled() {
        assert_eq!(
            resolve_proxy_endpoint(false, "").unwrap(),
            (String::new(), 0)
        );
    }

    #[test]
    fn parses_plain_ipv4_proxy() {
        assert_eq!(
            parse_proxy_server("127.0.0.1:7897").unwrap(),
            ("127.0.0.1".to_string(), 7897)
        );
    }

    #[test]
    fn parses_hostname_proxy() {
        assert_eq!(
            parse_proxy_server("localhost:7897").unwrap(),
            ("localhost".to_string(), 7897)
        );
    }

    #[test]
    fn parses_protocol_specific_proxy() {
        assert_eq!(
            parse_proxy_server("https=127.0.0.1:7898;http=127.0.0.1:7897;socks=127.0.0.1:7899")
                .unwrap(),
            ("127.0.0.1".to_string(), 7897)
        );
    }

    #[test]
    fn skips_malformed_protocol_entry() {
        assert_eq!(
            parse_proxy_server("http=;https=localhost:7898").unwrap(),
            ("localhost".to_string(), 7898)
        );
    }

    #[test]
    fn parses_bracketed_ipv6_proxy() {
        assert_eq!(
            parse_proxy_server("[::1]:7897").unwrap(),
            ("::1".to_string(), 7897)
        );
    }

    #[test]
    fn rejects_empty_proxy() {
        assert!(parse_proxy_server("").is_err());
    }
}
