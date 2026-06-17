use crate::config::Config;
use anyhow::{bail, Result};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::collections::HashMap;

/// PUT /configs
/// path 是绝对路径
pub async fn put_configs(path: &str) -> Result<()> {
    let (url, headers) = clash_client_info()?;
    let url = format!("{url}/configs");

    let mut data = HashMap::new();
    data.insert("path", path);

    let client = reqwest::ClientBuilder::new().no_proxy().build()?;
    let builder = client.put(&url).headers(headers).json(&data);
    let response = builder.send().await?;

    match response.status().as_u16() {
        204 => Ok(()),
        status => {
            bail!("failed to put configs with status \"{status}\"")
        }
    }
}

/// PATCH /configs
pub async fn patch_configs(config: &Mapping) -> Result<()> {
    let (url, headers) = clash_client_info()?;
    let url = format!("{url}/configs");

    let client = reqwest::ClientBuilder::new().no_proxy().build()?;
    let builder = client.patch(&url).headers(headers.clone()).json(config);
    builder.send().await?;
    Ok(())
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ProxyItemRes {
    pub name: String,
    #[serde(default)]
    pub all: Option<Vec<String>>,
    #[serde(default)]
    pub now: Option<String>,
    #[serde(default)]
    pub selected: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ProxiesRes {
    #[serde(default)]
    pub proxies: HashMap<String, ProxyItemRes>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionMetadataRes {
    #[serde(default)]
    pub network: String,
    #[serde(default, rename = "type")]
    pub conn_type: Option<String>,
    #[serde(default)]
    pub host: String,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: Option<String>,
    #[serde(default, rename = "destinationPort")]
    pub destination_port: String,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionItemRes {
    pub id: String,
    #[serde(default)]
    pub metadata: ConnectionMetadataRes,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: Option<String>,
    #[serde(default, rename = "rulePayload")]
    pub rule_payload: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionsRes {
    #[serde(default)]
    pub connections: Vec<ConnectionItemRes>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct DelayRes {
    delay: u64,
}

/// GET /proxies/{name}/delay
/// 获取代理延迟
pub async fn get_proxy_delay(
    name: String,
    test_url: Option<String>,
    timeout: i32,
) -> Result<DelayRes> {
    let (base_url, headers) = clash_client_info()?;
    let encoded_name = utf8_percent_encode(&name, NON_ALPHANUMERIC).to_string();
    let url = format!("{base_url}/proxies/{encoded_name}/delay");

    let default_url = "http://captive.apple.com/hotspot-detect.html";
    let test_url = test_url
        .map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                default_url.into()
            } else {
                trimmed.into()
            }
        })
        .unwrap_or(default_url.into());

    let timeout = if timeout <= 0 {
        10000
    } else if timeout > 60000 {
        60000
    } else {
        timeout
    };

    log::debug!(
        "requesting Clash proxy delay: proxy={name}, path=/proxies/{encoded_name}/delay, timeout={timeout}, url={test_url}"
    );

    let client = reqwest::ClientBuilder::new().no_proxy().build()?;
    let response = client
        .get(&url)
        .headers(headers)
        .query(&[("timeout", &format!("{timeout}")), ("url", &test_url)])
        .send()
        .await
        .map_err(|err| {
            anyhow::anyhow!("failed to request Clash proxy delay for '{name}' via {url}: {err}")
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|err| {
        anyhow::anyhow!("failed to read Clash proxy delay response for '{name}' from {url}: {err}")
    })?;

    if !status.is_success() {
        bail!("Clash proxy delay request failed for '{name}' with status {status}: {body}");
    }

    serde_json::from_str::<DelayRes>(&body).map_err(|err| {
        anyhow::anyhow!(
            "failed to parse Clash proxy delay response for '{name}' as JSON: {err}; body: {body}"
        )
    })
}

/// GET /proxies
pub async fn get_proxies() -> Result<ProxiesRes> {
    let (url, headers) = clash_client_info()?;
    let url = format!("{url}/proxies");

    let client = reqwest::ClientBuilder::new().no_proxy().build()?;
    let response = client.get(&url).headers(headers).send().await?;

    Ok(response.json::<ProxiesRes>().await?)
}

/// GET /connections
pub async fn get_connections() -> Result<ConnectionsRes> {
    let (url, headers) = clash_client_info()?;
    let url = format!("{url}/connections");

    let client = reqwest::ClientBuilder::new().no_proxy().build()?;
    let response = client.get(&url).headers(headers).send().await?;

    Ok(response.json::<ConnectionsRes>().await?)
}

/// 根据clash info获取clash服务地址和请求头
fn clash_client_info() -> Result<(String, HeaderMap)> {
    let client = { Config::clash().data().get_client_info() };

    let server = format!("http://{}", client.server);

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse()?);

    if let Some(secret) = client.secret {
        let secret = format!("Bearer {}", secret).parse()?;
        headers.insert("Authorization", secret);
    }

    Ok((server, headers))
}

/// 缩短clash的日志
pub fn parse_log(log: String) -> String {
    if log.starts_with("time=") && log.len() > 33 {
        return (log[33..]).to_owned();
    }
    if log.len() > 9 {
        return (log[9..]).to_owned();
    }
    log
}

/// 缩短clash -t的错误输出
/// 仅适配 clash p核 8-26、clash meta 1.13.1
pub fn parse_check_output(log: String) -> String {
    let t = log.find("time=");
    let m = log.find("msg=");
    let mr = log.rfind('"');

    if let (Some(_), Some(m), Some(mr)) = (t, m, mr) {
        let e = match log.find("level=error msg=") {
            Some(e) => e + 17,
            None => m + 5,
        };

        if mr > m {
            return (log[e..mr]).to_owned();
        }
    }

    let l = log.find("error=");
    let r = log.find("path=").or(Some(log.len()));

    if let (Some(l), Some(r)) = (l, r) {
        return (log[(l + 6)..(r - 1)]).to_owned();
    }

    log
}

#[test]
fn test_parse_check_output() {
    let str1 = r#"xxxx\n time="2022-11-18T20:42:58+08:00" level=error msg="proxy 0: 'alpn' expected type 'string', got unconvertible type '[]interface {}'""#;
    let str2 = r#"20:43:49 ERR [Config] configuration file test failed error=proxy 0: unsupport proxy type: hysteria path=xxx"#;
    let str3 = r#"
    "time="2022-11-18T21:38:01+08:00" level=info msg="Start initial configuration in progress"
    time="2022-11-18T21:38:01+08:00" level=error msg="proxy 0: 'alpn' expected type 'string', got unconvertible type '[]interface {}'"
    configuration file xxx\n
    "#;

    let res1 = parse_check_output(str1.into());
    let res2 = parse_check_output(str2.into());
    let res3 = parse_check_output(str3.into());

    println!("res1: {res1}");
    println!("res2: {res2}");
    println!("res3: {res3}");

    assert_eq!(res1, res3);
}
