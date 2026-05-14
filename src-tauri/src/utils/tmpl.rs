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
