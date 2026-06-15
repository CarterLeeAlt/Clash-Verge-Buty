use super::use_lowercase;
use anyhow::{bail, Result};
use serde_yaml::{Mapping, Value};

// Global Script behavior and profileName support are adapted from
// clash-verge-rev/clash-verge-rev.
// Source:
// https://github.com/clash-verge-rev/clash-verge-rev/blob/658312fe7be95d69b4dad379c6cd00fc93f29d5f/src-tauri/src/enhance/mod.rs
// https://github.com/clash-verge-rev/clash-verge-rev/blob/658312fe7be95d69b4dad379c6cd00fc93f29d5f/src-tauri/src/enhance/script.rs
// License: GPL-3.0-only

pub fn use_script(
    script: String,
    config: Mapping,
    profile_name: &str,
) -> Result<(Mapping, Vec<(String, String)>)> {
    run_script_inner(script, config, profile_name, false)
}

pub fn validate_script_strict(
    script: String,
    config: Mapping,
    profile_name: &str,
) -> Result<Mapping> {
    let (config, _) = run_script_inner(script, config, profile_name, true)?;
    validate_minimal_clash_config(&config)?;
    Ok(config)
}

fn run_script_inner(
    script: String,
    config: Mapping,
    profile_name: &str,
    strict: bool,
) -> Result<(Mapping, Vec<(String, String)>)> {
    use rquickjs::{function::Func, Context, Runtime};
    use std::sync::{Arc, Mutex};

    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let outputs = Arc::new(Mutex::new(vec![]));

    let copy_outputs = outputs.clone();
    let result = context.with(|ctx| -> Result<Mapping> {
        ctx.globals().set(
            "__verge_log__",
            Func::from(move |level: String, data: String| {
                let mut out = copy_outputs.lock().unwrap();
                out.push((level, data));
            }),
        )?;

        ctx.eval::<(), _>(
            r#"var console = Object.freeze({
        log(data){__verge_log__("log",JSON.stringify(data))}, 
        info(data){__verge_log__("info",JSON.stringify(data))}, 
        error(data){__verge_log__("error",JSON.stringify(data))},
        debug(data){__verge_log__("debug",JSON.stringify(data))},
      });"#,
        )?;

        let config = use_lowercase(config.clone());
        let config_str = serde_json::to_string(&config)?;
        let profile_name = serde_json::to_string(profile_name)?;

        let code = format!(
            r#"try{{
        {script};
        if (typeof main !== "function") {{ throw new Error("main function is not defined"); }}
        const result = main({config_str}, {profile_name});
        if (result === null || typeof result !== "object" || Array.isArray(result)) {{
          throw new Error("main function should return object");
        }}
        JSON.stringify(result)
      }} catch(err) {{
        `__error_flag__ ${{err && err.stack ? err.stack : err.toString()}}`
      }}"#
        );
        let result: String = ctx.eval(code.as_str())?;
        if result.starts_with("__error_flag__") {
            bail!(result[15..].to_owned());
        }
        Ok(serde_json::from_str::<Mapping>(result.as_str())?)
    });

    let mut out = outputs.lock().unwrap();
    match result {
        Ok(config) => Ok((use_lowercase(config), out.to_vec())),
        Err(err) if strict => Err(err),
        Err(err) => {
            out.push(("exception".into(), err.to_string()));
            Ok((config, out.to_vec()))
        }
    }
}

pub fn validate_minimal_clash_config(config: &Mapping) -> Result<()> {
    if !config.contains_key(&Value::String("proxies".into()))
        && !config.contains_key(&Value::String("proxy-providers".into()))
    {
        bail!("profile does not contain `proxies` or `proxy-providers`");
    }

    for key in ["rules", "proxies", "proxy-groups"] {
        if let Some(value) = config.get(&Value::String(key.into())) {
            if !value.is_sequence() {
                bail!("`{key}` must be a sequence");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> Mapping {
        serde_yaml::from_str(
            r#"
proxy-providers:
  provider-a:
    type: http
    url: https://example.com/sub.yaml
    path: ./provider-a.yaml
rules:
  - MATCH,DIRECT
"#,
        )
        .unwrap()
    }

    #[test]
    fn strict_validation_rejects_invalid_scripts() {
        let cases = [
            "function test(config) { return config; }",
            "function main(config) { return config",
            "function main(config) { return 'bad'; }",
            "function main(config) { return [config]; }",
            "function main(config) { return null; }",
            "function main(config) { return {}; }",
        ];

        for script in cases {
            assert!(
                validate_script_strict(script.into(), base_config(), "Profile").is_err(),
                "script should be rejected: {script}"
            );
        }
    }

    #[test]
    fn strict_validation_accepts_provider_only_config() {
        let script = r#"
function main(config, profileName) {
  config.rules = ["DOMAIN-SUFFIX,nodeseek.com,DIRECT"].concat(config.rules || []);
  return config;
}
"#;

        assert!(validate_script_strict(script.into(), base_config(), "Provider Only").is_ok());
    }
}
