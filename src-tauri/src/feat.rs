//！
//! feat mod 里的函数主要用于
//! - hotkey 快捷键
//! - timer 定时器
//! - cmds 页面调用
//!
use crate::config::*;
use crate::core::*;
use crate::log_err;
use crate::utils::resolve;
use anyhow::{bail, Result};
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::collections::{HashMap, HashSet};
use tauri::{AppHandle, ClipboardManager, Manager};
use tokio::time::{sleep, Duration, Instant};

// 打开面板
pub fn open_or_close_dashboard() {
    log::trace!("hotkey/dashboard entry received: open_or_close_dashboard");
    let handle = handle::Handle::global();
    let app_handle = handle.app_handle.lock();
    if let Some(app_handle) = app_handle.as_ref() {
        if let Some(window) = app_handle.get_window("main") {
            if let Ok(true) = window.is_focused() {
                let _ = window.hide();
                return;
            }
        }
        log::trace!("hotkey/dashboard entry -> resolve::show_main_window");
        resolve::show_main_window(app_handle);
    }
}

// 重启clash
pub fn restart_clash_core() {
    tauri::async_runtime::spawn(async {
        match CoreManager::global().run_core().await {
            Ok(_) => {
                handle::Handle::refresh_clash();
                handle::Handle::notice_message("set_config::ok", "ok");
            }
            Err(err) => {
                handle::Handle::notice_message("set_config::error", format!("{err}"));
                log::error!(target:"app", "{err}");
            }
        }
    });
}

// 切换模式 rule/global/direct/script mode
pub fn change_clash_mode(mode: String) {
    let mut mapping = Mapping::new();
    mapping.insert(Value::from("mode"), mode.clone().into());

    tauri::async_runtime::spawn(async move {
        log::debug!(target: "app", "change clash mode to {mode}");

        match clash_api::patch_configs(&mapping).await {
            Ok(_) => {
                // 更新订阅
                Config::clash().data().patch_config(mapping);

                if Config::clash().data().save_config().is_ok() {
                    handle::Handle::refresh_clash();
                    log_err!(handle::Handle::update_systray_part());
                }
            }
            Err(err) => log::error!(target: "app", "{err}"),
        }
    });
}

// 切换系统代理
pub fn toggle_system_proxy() {
    let enable = Config::verge().draft().enable_system_proxy;
    let enable = enable.unwrap_or(false);

    tauri::async_runtime::spawn(async move {
        match patch_verge(IVerge {
            enable_system_proxy: Some(!enable),
            ..IVerge::default()
        })
        .await
        {
            Ok(_) => handle::Handle::refresh_verge(),
            Err(err) => log::error!(target: "app", "{err}"),
        }
    });
}

// 切换tun模式
pub fn toggle_tun_mode() {
    let enable = Config::verge().data().enable_tun_mode;
    let enable = enable.unwrap_or(false);

    tauri::async_runtime::spawn(async move {
        match patch_verge(IVerge {
            enable_tun_mode: Some(!enable),
            ..IVerge::default()
        })
        .await
        {
            Ok(_) => handle::Handle::refresh_verge(),
            Err(err) => log::error!(target: "app", "{err}"),
        }
    });
}

/// 修改clash的订阅
pub async fn patch_clash(patch: Mapping) -> Result<()> {
    let has_tun_patch = patch.get("tun").is_some();
    let has_runtime_patch = patch.get("allow-lan").is_some()
        || patch.get("ipv6").is_some()
        || patch.get("log-level").is_some();
    if has_tun_patch {
        log::info!(target: "app", "patch clash tun config requested");
        handle::Handle::emit_log("info", "[tun] patch clash tun config requested");
    }
    Config::clash().draft().patch_config(patch.clone());

    match {
        let mixed_port = patch.get("mixed-port");
        let socks_port = patch.get("socks-port");
        let port = patch.get("port");
        let enable_random_port = Config::verge().latest().enable_random_port.unwrap_or(false);
        if mixed_port.is_some() && !enable_random_port {
            let changed = mixed_port.unwrap()
                != Config::verge()
                    .latest()
                    .verge_mixed_port
                    .unwrap_or(Config::clash().data().get_mixed_port());
            // 检查端口占用
            if changed {
                if let Some(port) = mixed_port.unwrap().as_u64() {
                    if !port_scanner::local_port_available(port as u16) {
                        Config::clash().discard();
                        bail!("port already in use");
                    }
                }
            }
        };

        // 激活订阅
        if mixed_port.is_some()
            || socks_port.is_some()
            || port.is_some()
            || patch.get("secret").is_some()
            || patch.get("external-controller").is_some()
        {
            Config::generate()?;
            CoreManager::global().run_core().await?;
            handle::Handle::refresh_clash();
        }

        // 更新系统代理
        if mixed_port.is_some() {
            log_err!(sysopt::Sysopt::global().init_sysproxy());
        }

        if patch.get("mode").is_some() {
            log_err!(handle::Handle::update_systray_part());
        }

        if has_tun_patch {
            log::info!(target: "app", "tun config changed, reload core config");
            handle::Handle::emit_log("info", "[tun] tun config changed, reload core config");
            update_core_config().await?;
        }

        Config::runtime().latest().patch_config(patch);

        if has_runtime_patch {
            Config::generate_file(ConfigType::Run)?;
            handle::Handle::refresh_clash();
        }

        <Result<()>>::Ok(())
    } {
        Ok(()) => {
            Config::clash().apply();
            Config::clash().data().save_config()?;
            Ok(())
        }
        Err(err) => {
            Config::clash().discard();
            Err(err)
        }
    }
}

/// 修改verge的订阅
/// 一般都是一个个的修改
pub async fn patch_verge(patch: IVerge) -> Result<()> {
    let tun_mode = patch.enable_tun_mode;
    let auto_launch = patch.enable_auto_launch;
    let system_proxy = patch.enable_system_proxy;
    let proxy_bypass = patch.system_proxy_bypass.clone();
    let language = patch.language.clone();
    let port = patch.verge_mixed_port;
    let common_tray_icon = patch.common_tray_icon;
    let sysproxy_tray_icon = patch.sysproxy_tray_icon;
    let tun_tray_icon = patch.tun_tray_icon;

    // Validate invariants first, then apply draft so follow-up operations
    // (e.g. run_core) can observe the target mode consistently.
    #[cfg(target_os = "windows")]
    {
        let service_mode = patch.enable_service_mode;
        let (current_tun_enabled, current_service_enabled) = {
            let verge_config = Config::verge();
            let current_verge = verge_config.latest();
            (
                current_verge.enable_tun_mode.unwrap_or(false),
                current_verge.enable_service_mode.unwrap_or(false),
            )
        };

        if current_tun_enabled && service_mode.is_some() {
            bail!("Tun Mode is enabled. Please disable Tun Mode before changing Service Mode.");
        }

        if let Some(true) = service_mode {
            let status = super::core::win_service::check_service().await?;
            if !status.installed {
                bail!("Service Mode requires clash-verge-service to be installed. Please install service first.");
            }
        }

        if let Some(true) = tun_mode {
            let status = super::core::win_service::check_service().await?;
            if !status.installed {
                bail!("Tun mode on Windows requires clash-verge-service to be installed. Please install and enable Service Mode first.");
            }
            let service_enabled = service_mode.unwrap_or(current_service_enabled);
            if !service_enabled && service_mode != Some(true) {
                bail!("Tun mode on Windows requires Service Mode. Please install and enable Service Mode first.");
            }

            super::core::win_service::ensure_service_ready()
                .await
                .map_err(|err| anyhow::anyhow!("Tun mode on Windows requires clash-verge-service. Please install and enable Service Mode first. {err}"))?;
        }
    }

    Config::verge().draft().patch_config(patch.clone());

    match {
        #[cfg(target_os = "windows")]
        {
            let service_mode = patch.enable_service_mode;
            if service_mode.is_some() {
                log::debug!(target: "app", "change service mode to {}", service_mode.unwrap());

                Config::generate()?;
                CoreManager::global().run_core().await?;
            } else if tun_mode.is_some() {
                update_core_config().await?;
            }
        }

        #[cfg(not(target_os = "windows"))]
        if tun_mode.is_some() {
            update_core_config().await?;
        }

        if auto_launch.is_some() {
            sysopt::Sysopt::global().update_launch()?;
        }
        if system_proxy.is_some() || proxy_bypass.is_some() || port.is_some() {
            sysopt::Sysopt::global()
                .update_sysproxy()
                .map_err(|err| anyhow::anyhow!("failed to update system proxy: {err}"))?;
            sysopt::Sysopt::global().guard_proxy();
        }

        if let Some(true) = patch.enable_proxy_guard {
            sysopt::Sysopt::global().guard_proxy();
        }

        if let Some(hotkeys) = patch.hotkeys {
            hotkey::Hotkey::global().update(hotkeys)?;
        }

        if language.is_some() {
            handle::Handle::update_systray()?;
        } else if system_proxy.is_some()
            || tun_mode.is_some()
            || common_tray_icon.is_some()
            || sysproxy_tray_icon.is_some()
            || tun_tray_icon.is_some()
        {
            handle::Handle::update_systray_part()?;
        }

        <Result<()>>::Ok(())
    } {
        Ok(()) => {
            Config::verge().apply();
            Config::verge().data().save_file()?;
            Ok(())
        }
        Err(err) => {
            Config::verge().discard();
            Err(err)
        }
    }
}

/// 更新某个profile
/// 如果更新当前订阅就激活订阅
pub async fn update_profile(uid: String, option: Option<PrfOption>) -> Result<()> {
    let url_opt = {
        let profiles = Config::profiles();
        let profiles = profiles.latest();
        let item = profiles.get_item(&uid)?;
        let is_remote = item.itype.as_ref().map_or(false, |s| s == "remote");

        if !is_remote {
            None // 直接更新
        } else if item.url.is_none() {
            bail!("failed to get the profile item url");
        } else {
            Some((item.url.clone().unwrap(), item.option.clone()))
        }
    };

    let should_update = match url_opt {
        Some((url, opt)) => {
            let merged_opt = PrfOption::merge(opt, option);
            let item = PrfItem::from_url(&url, None, None, merged_opt).await?;

            let profiles = Config::profiles();
            let mut profiles = profiles.latest();
            profiles.update_item(uid.clone(), item)?;

            Some(uid) == profiles.get_current()
        }
        None => true,
    };

    if should_update {
        update_core_config().await?;
    }

    Ok(())
}

/// 更新订阅
async fn update_core_config() -> Result<()> {
    log::info!(target: "app", "updating core config (generate/check/reload)");
    handle::Handle::emit_log("info", "[app] updating core config (generate/check/reload)");
    match CoreManager::global().update_config().await {
        Ok(_) => {
            handle::Handle::refresh_clash();
            handle::Handle::notice_message("set_config::ok", "ok");
            log::info!(target: "app", "core config updated successfully");
            handle::Handle::emit_log("info", "[app] core config updated successfully");
            Ok(())
        }
        Err(err) => {
            handle::Handle::notice_message("set_config::error", format!("{err}"));
            log::error!(target: "app", "core config update failed: {err}");
            handle::Handle::emit_log("error", format!("[app] core config update failed: {err}"));
            Err(err)
        }
    }
}

/// copy env variable
pub fn copy_clash_env(app_handle: &AppHandle) {
    let port = { Config::verge().latest().verge_mixed_port.unwrap_or(7897) };
    let http_proxy = format!("http://127.0.0.1:{}", port);
    let socks5_proxy = format!("socks5://127.0.0.1:{}", port);

    let sh =
        format!("export https_proxy={http_proxy} http_proxy={http_proxy} all_proxy={socks5_proxy}");
    let cmd: String = format!("set http_proxy={http_proxy} \n set https_proxy={http_proxy}");
    let ps: String = format!("$env:HTTP_PROXY=\"{http_proxy}\"; $env:HTTPS_PROXY=\"{http_proxy}\"");

    let mut cliboard = app_handle.clipboard_manager();

    let env_type = { Config::verge().latest().env_type.clone() };
    let env_type = match env_type {
        Some(env_type) => env_type,
        None => {
            #[cfg(not(target_os = "windows"))]
            let default = "bash";
            #[cfg(target_os = "windows")]
            let default = "powershell";

            default.to_string()
        }
    };
    match env_type.as_str() {
        "bash" => cliboard.write_text(sh).unwrap_or_default(),
        "cmd" => cliboard.write_text(cmd).unwrap_or_default(),
        "powershell" => cliboard.write_text(ps).unwrap_or_default(),
        _ => log::error!(target: "app", "copy_clash_env: Invalid env type! {env_type}"),
    };
}

#[derive(Default, Debug, Clone, Serialize)]
pub struct TestDelayResult {
    pub delay: u32,
    pub proxy: Option<String>,
}

struct TestDelayMatchContext {
    group_names: HashSet<String>,
    proxy_names: HashSet<String>,
    proxies: HashMap<String, clash_api::ProxyItemRes>,
    before_connection_ids: HashSet<String>,
}

pub async fn test_delay(url: String) -> Result<TestDelayResult> {
    let test_url = reqwest::Url::parse(&url).ok();
    let test_host = test_url
        .as_ref()
        .and_then(|url| url.host_str())
        .map(|host| host.to_string());
    let test_port = test_url
        .as_ref()
        .and_then(|url| url.port_or_known_default());

    let match_context = prepare_test_delay_match_context().await;

    let mut builder = reqwest::ClientBuilder::new().use_rustls_tls().no_proxy();

    let port = Config::verge()
        .latest()
        .verge_mixed_port
        .unwrap_or(Config::clash().data().get_mixed_port());
    let tun_mode = Config::verge().latest().enable_tun_mode.unwrap_or(false);

    let proxy_scheme = format!("http://127.0.0.1:{port}");

    if !tun_mode {
        if let Ok(proxy) = reqwest::Proxy::http(&proxy_scheme) {
            builder = builder.proxy(proxy);
        }
        if let Ok(proxy) = reqwest::Proxy::https(&proxy_scheme) {
            builder = builder.proxy(proxy);
        }
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_scheme) {
            builder = builder.proxy(proxy);
        }
    }

    let request = builder
        .timeout(Duration::from_millis(10000))
        .build()?
        .get(url).header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0");

    let start = Instant::now();
    let mut request = Box::pin(request.send());
    let mut matched_proxy = None;

    loop {
        tokio::select! {
            response = &mut request => {
                let delay = match response {
                    Ok(response) if response.status().is_success() => start.elapsed().as_millis() as u32,
                    Ok(_) | Err(_) => 10000u32,
                };

                if matched_proxy.is_none() {
                    matched_proxy = match_test_delay_proxy_until(
                        &match_context,
                        test_host.as_deref(),
                        test_port,
                        Duration::from_millis(600),
                    ).await;
                }

                return Ok(TestDelayResult {
                    delay,
                    proxy: matched_proxy,
                });
            }
            _ = sleep(Duration::from_millis(100)), if matched_proxy.is_none() => {
                matched_proxy = match_test_delay_proxy(
                    &match_context,
                    test_host.as_deref(),
                    test_port,
                ).await;
            }
        }
    }
}

async fn prepare_test_delay_match_context() -> TestDelayMatchContext {
    let proxies = clash_api::get_proxies()
        .await
        .map(|res| res.proxies)
        .unwrap_or_default();

    let (group_names, proxy_names) = proxies.iter().fold(
        (HashSet::new(), HashSet::new()),
        |(mut group_names, mut proxy_names), (name, proxy)| {
            if proxy.all.is_some() {
                group_names.insert(name.clone());
            } else {
                proxy_names.insert(name.clone());
            }

            (group_names, proxy_names)
        },
    );

    let before_connection_ids = clash_api::get_connections()
        .await
        .map(|res| {
            res.connections
                .into_iter()
                .map(|connection| connection.id)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    TestDelayMatchContext {
        group_names,
        proxy_names,
        proxies,
        before_connection_ids,
    }
}

async fn match_test_delay_proxy_until(
    context: &TestDelayMatchContext,
    test_host: Option<&str>,
    test_port: Option<u16>,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(proxy) = match_test_delay_proxy(context, test_host, test_port).await {
            return Some(proxy);
        }

        if Instant::now() >= deadline {
            return None;
        }

        sleep(Duration::from_millis(80)).await;
    }
}

async fn match_test_delay_proxy(
    context: &TestDelayMatchContext,
    test_host: Option<&str>,
    test_port: Option<u16>,
) -> Option<String> {
    let connections = clash_api::get_connections().await.ok()?.connections;

    connections
        .into_iter()
        .filter(|connection| !context.before_connection_ids.contains(&connection.id))
        .filter(|connection| connection_matches_test_url(connection, test_host, test_port))
        .find_map(|connection| select_outbound_proxy(&connection, context))
}

fn select_outbound_proxy(
    connection: &clash_api::ConnectionItemRes,
    context: &TestDelayMatchContext,
) -> Option<String> {
    if let Some(proxy) = select_outbound_from_chains(&connection.chains, context) {
        return Some(proxy);
    }

    connection
        .rule
        .as_deref()
        .and_then(normalize_builtin_outbound)
        .or_else(|| {
            connection
                .rule_payload
                .as_deref()
                .and_then(normalize_builtin_outbound)
        })
        .or_else(|| {
            connection
                .chains
                .iter()
                .rev()
                .find_map(|chain| select_builtin_from_policy_hint(Some(chain), context))
        })
}

fn select_outbound_from_chains(
    chains: &[String],
    context: &TestDelayMatchContext,
) -> Option<String> {
    for chain in chains.iter().rev().map(|chain| chain.trim()) {
        if chain.is_empty() {
            continue;
        }

        if let Some(builtin) = normalize_builtin_outbound(chain) {
            return Some(builtin);
        }

        if context.proxy_names.contains(chain) && !context.group_names.contains(chain) {
            return Some(chain.to_string());
        }
    }

    chains
        .iter()
        .rev()
        .map(|chain| chain.trim())
        .find(|chain| !chain.is_empty() && !context.group_names.contains(*chain))
        .map(ToString::to_string)
}

fn select_builtin_from_policy_hint(
    policy_name: Option<&str>,
    context: &TestDelayMatchContext,
) -> Option<String> {
    let policy_name = policy_name?.trim();

    let proxy = context.proxies.get(policy_name)?;
    let selected = proxy
        .now
        .as_deref()
        .or(proxy.selected.as_deref())
        .map(str::trim)?;

    normalize_builtin_outbound(selected)
}

fn normalize_builtin_outbound(name: &str) -> Option<String> {
    if name.eq_ignore_ascii_case("DIRECT") {
        Some("DIRECT".to_string())
    } else if name.eq_ignore_ascii_case("REJECT") {
        Some("REJECT".to_string())
    } else {
        None
    }
}

fn connection_matches_test_url(
    connection: &clash_api::ConnectionItemRes,
    test_host: Option<&str>,
    test_port: Option<u16>,
) -> bool {
    let metadata = &connection.metadata;

    let host_matches = match test_host {
        Some(host) => {
            metadata.host.eq_ignore_ascii_case(host)
                || metadata
                    .host
                    .to_ascii_lowercase()
                    .contains(&host.to_ascii_lowercase())
        }
        None => true,
    };

    let port_matches = match test_port {
        Some(port) => metadata
            .destination_port
            .parse::<u16>()
            .map(|destination_port| destination_port == port)
            .unwrap_or(true),
        None => true,
    };

    host_matches && port_matches
}
