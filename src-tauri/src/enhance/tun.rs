use serde_yaml::{Mapping, Value};
use crate::core::handle::Handle;

macro_rules! revise {
    ($map: expr, $key: expr, $val: expr) => {
        let ret_key = Value::String($key.into());
        $map.insert(ret_key, Value::from($val));
    };
}

// if key not exists then append value
macro_rules! append {
    ($map: expr, $key: expr, $val: expr) => {
        let ret_key = Value::String($key.into());
        if !$map.contains_key(&ret_key) {
            $map.insert(ret_key, Value::from($val));
        }
    };
}

pub fn use_tun(
    mut config: Mapping,
    enable: bool,
    source_has_tun: bool,
    default_tun: Mapping,
) -> Mapping {
    let tun_key = Value::from("tun");
    let tun_val = config.get(&tun_key);
    let tun_existed = tun_val.is_some();
    log::info!(target: "app", "tun existed before sync: {tun_existed}, tun existed in source: {source_has_tun}, tun enabled by ui: {enable}");
    Handle::emit_log("info", format!("[tun] sync start: existed_before={tun_existed}, existed_in_source={source_has_tun}, enabled_by_ui={enable}"));

    if enable {
        let mut tun_val = tun_val
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_else(|| default_tun.clone());
        revise!(tun_val, "enable", true);
        revise!(config, "tun", tun_val);
        Handle::emit_log("info", "[tun] action: inject/update tun and set enable=true");
    } else if tun_existed || source_has_tun {
        let mut tun_val = tun_val
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_else(|| default_tun.clone());
        revise!(tun_val, "enable", false);
        revise!(config, "tun", tun_val);
        Handle::emit_log("info", "[tun] action: keep tun and set enable=false");
    } else {
        config.remove(&tun_key);
        Handle::emit_log("info", "[tun] action: leave tun absent");
        return config;
    }

    let stack = config
        .get("tun")
        .and_then(Value::as_mapping)
        .and_then(|m| m.get("stack"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    Handle::emit_log("info", format!("[tun] stack={stack}"));

    if enable {
        use_dns_for_tun(config)
    } else {
        config
    }
}

fn use_dns_for_tun(mut config: Mapping) -> Mapping {
    let dns_key = Value::from("dns");
    let dns_val = config.get(&dns_key);

    let mut dns_val = dns_val.map_or(Mapping::new(), |val| {
        val.as_mapping().cloned().unwrap_or(Mapping::new())
    });

    // 开启tun将同时开启dns
    revise!(dns_val, "enable", true);

    append!(dns_val, "enhanced-mode", "fake-ip");
    append!(dns_val, "fake-ip-range", "198.18.0.1/16");
    append!(
        dns_val,
        "nameserver",
        vec!["114.114.114.114", "223.5.5.5", "8.8.8.8"]
    );
    append!(dns_val, "fallback", vec![] as Vec<&str>);

    #[cfg(target_os = "windows")]
    append!(
        dns_val,
        "fake-ip-filter",
        vec![
            "dns.msftncsi.com",
            "www.msftncsi.com",
            "www.msftconnecttest.com"
        ]
    );
    revise!(config, "dns", dns_val);
    config
}
