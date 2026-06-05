use once_cell::sync::Lazy;
use regex::Regex;

static PROXY_URI_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\b(vless|vmess|trojan|ss|ssr|hysteria|hysteria2|tuic)://[^\s'\"<>]+"#)
        .expect("valid proxy URI redaction regex")
});

static SENSITIVE_QUERY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)(^|[?&\s])(config|url|token|password|passwd|pbk|sid|uuid)=([^&\s#]+)"#)
        .expect("valid sensitive query redaction regex")
});

/// Redact credentials that may appear in app/file logs while preserving ordinary
/// domains, rule names, strategy names, and node display names used for troubleshooting.
pub fn redact_log_text(input: &str) -> String {
    let text = PROXY_URI_RE.replace_all(input, "$1://[REDACTED]");
    let text = SENSITIVE_QUERY_RE.replace_all(&text, "$1$2=[REDACTED]");
    text.into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact_log_text;

    #[test]
    fn redacts_complete_proxy_uris() {
        let input = "vless://uuid@example.com:443?pbk=abc&sid=def#AWJP-TCP-Reality vmess://abcdef trojan://pass@host ss://YWVz hysteria://host hysteria2://host tuic://uuid:pwd@host ssr://encoded";
        let redacted = redact_log_text(input);

        assert!(redacted.contains("vless://[REDACTED]"));
        assert!(redacted.contains("vmess://[REDACTED]"));
        assert!(redacted.contains("trojan://[REDACTED]"));
        assert!(redacted.contains("ss://[REDACTED]"));
        assert!(redacted.contains("ssr://[REDACTED]"));
        assert!(redacted.contains("hysteria://[REDACTED]"));
        assert!(redacted.contains("hysteria2://[REDACTED]"));
        assert!(redacted.contains("tuic://[REDACTED]"));
        assert!(!redacted.contains("uuid@example.com"));
        assert!(!redacted.contains("pass@host"));
    }

    #[test]
    fn redacts_sensitive_query_values() {
        let input = "https://example.com/sub?config=abc&url=https%3A%2F%2Fsecret&token=t0k&password=pwd&passwd=pwd2&pbk=public_key&sid=short&uuid=1234#frag token=standalone";
        let redacted = redact_log_text(input);

        for key in [
            "config", "url", "token", "password", "passwd", "pbk", "sid", "uuid",
        ] {
            assert!(redacted.contains(&format!("{key}=[REDACTED]")));
        }
        assert!(!redacted.contains("abc&"));
        assert!(!redacted.contains("t0k"));
        assert!(!redacted.contains("public_key"));
    }

    #[test]
    fn preserves_troubleshooting_display_text() {
        let input = "chatgpt.com:443 www.youtube.com:443 RuleSet(youtube) RuleSet(geolocation-!cn) AWJP-TCP-Reality VMUS-XHTTP-TLS-CDN DIRECT";
        assert_eq!(redact_log_text(input), input);
    }
}
