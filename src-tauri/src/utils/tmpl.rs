//! Some config file template

/// template for new a profile item
pub const ITEM_LOCAL: &str = "# Profile Template for Clash-Verge-Buty

proxies:

proxy-groups:

rules:
";

/// enhanced profile
pub const ITEM_MERGE: &str = "# Merge Template for Clash-Verge-Buty
# The `Merge` format used to enhance profile

prepend-rules:

prepend-rule-providers:

prepend-proxies:

prepend-proxy-providers:

prepend-proxy-groups:

append-rules:

append-rule-providers:

append-proxies:

append-proxy-providers:

append-proxy-groups:
";

/// enhanced profile
pub const ITEM_SCRIPT: &str = "// Define the `main` function

function main(params) {
  return params;
}
";

/// fixed global overwrite script template
pub const ITEM_GLOBAL_SCRIPT: &str = r#"// Global Overwrite Script
//
// This script runs for every subscription profile after the subscription
// config is loaded, and before normal profile merge/script items.
// You can modify the generated mihomo/Clash config here.
//
// Parameters:
// - config: the current subscription config object.
// - profileName: the current subscription name. It may be an empty string.
//
// Common fields you may edit:
// - config.rules
// - config.proxies
// - config["proxy-groups"]
// - config.dns
//
// Requirements:
// - Keep the main function.
// - Always return the config object.
// - Invalid JavaScript, missing main(), non-object returns, or broken basic
//   config structure will be rejected when saving.
//
// Tip:
// Start with small changes, such as adding custom rules before subscription
// rules. If you do not need custom rules, keep customRules empty.

function main(config, profileName) {
  const customRules = [
    "DOMAIN-SUFFIX,baidu.com,DIRECT",
  ];

  config.rules = customRules.concat(config.rules || []);

  return config;
}
"#;
