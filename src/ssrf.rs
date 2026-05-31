/// Enhanced SSRF protection with DNS resolution checking.
use hickory_resolver::{
    config::{ResolverConfig, ResolverOpts},
    TokioAsyncResolver,
};
use std::net::IpAddr;
use url::Url;

/// Validates a URL and checks for SSRF vulnerabilities.
/// Returns Ok(()) if the URL is safe, Err with a message otherwise.
pub async fn validate_url_safe(url: &Url) -> Result<(), &'static str> {
    // Check scheme
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only HTTP and HTTPS URLs are supported");
    }

    let host = url.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err("invalid URL: missing host");
    }

    // First check: hostname-based blocking
    if is_forbidden_hostname(host) {
        return Err("access to local or private networks is forbidden");
    }

    // Second check: resolve DNS and check IP addresses
    if let Err(e) = check_resolved_ips(host).await {
        return Err(e);
    }

    Ok(())
}

/// Checks if a hostname is forbidden (localhost, private ranges by name).
pub fn is_forbidden_hostname(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();

    // Localhost variants
    if lower == "localhost" || lower == "localhost.localdomain" || lower.ends_with(".localhost") {
        return true;
    }

    // IPv6 loopback
    if lower == "::1" || lower == "[::1]" {
        return true;
    }

    // IPv4 loopback and private ranges
    if lower == "0.0.0.0" || lower == "127.0.0.1" {
        return true;
    }

    // Check if it's an IP address string
    if let Ok(ip) = lower
        .trim_matches(|c| c == '[' || c == ']')
        .parse::<IpAddr>()
    {
        return is_forbidden_ip(&ip);
    }

    // Private IP ranges by string prefix (for non-parsed IPs)
    if lower.starts_with("10.") || lower.starts_with("192.168.") || is_172_private_range(&lower) {
        return true;
    }

    // Link-local addresses
    if lower.starts_with("169.254.") {
        return true;
    }

    // Internal/special domains
    if lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower.ends_with(".corp")
        || lower == "metadata.google.internal"
    {
        return true;
    }

    false
}

/// Checks if an IP address is forbidden (loopback, private, link-local, etc.).
pub fn is_forbidden_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();

            // Loopback: 127.0.0.0/8
            if octets[0] == 127 {
                return true;
            }

            // Private: 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }

            // Private: 172.16.0.0/12
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }

            // Private: 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }

            // Link-local: 169.254.0.0/16
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }

            // Broadcast: 255.255.255.255
            if octets == [255, 255, 255, 255] {
                return true;
            }

            // This network: 0.0.0.0/8
            if octets[0] == 0 {
                return true;
            }

            false
        }
        IpAddr::V6(ipv6) => {
            // Loopback: ::1
            if ipv6.is_loopback() {
                return true;
            }

            // Unspecified: ::
            if ipv6.is_unspecified() {
                return true;
            }

            let segments = ipv6.segments();

            // Link-local: fe80::/10
            if segments[0] & 0xffc0 == 0xfe80 {
                return true;
            }

            // Unique local: fc00::/7
            if segments[0] & 0xfe00 == 0xfc00 {
                return true;
            }

            false
        }
    }
}

/// Resolves a hostname and checks if any resolved IP is forbidden.
async fn check_resolved_ips(host: &str) -> Result<(), &'static str> {
    // Skip resolution for IP addresses (already checked)
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }

    // Create resolver
    let resolver = TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

    // Resolve hostname
    let lookup = match resolver.lookup_ip(host).await {
        Ok(lookup) => lookup,
        Err(e) => {
            tracing::warn!("DNS resolution failed for {}: {}", host, e);
            // Allow the request to proceed - the upstream connection will fail naturally
            return Ok(());
        }
    };

    // Check all resolved IPs
    for ip in lookup.iter() {
        if is_forbidden_ip(&ip) {
            tracing::warn!(
                "Blocked SSRF attempt: {} resolves to forbidden IP {}",
                host,
                ip
            );
            return Err("access to local or private networks is forbidden");
        }
    }

    Ok(())
}

fn is_172_private_range(host: &str) -> bool {
    let Some(rest) = host.strip_prefix("172.") else {
        return false;
    };

    let Some(second_octet) = rest.split('.').next() else {
        return false;
    };

    second_octet
        .parse::<u8>()
        .map(|octet| (16..=31).contains(&octet))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forbidden_hostnames() {
        assert!(is_forbidden_hostname("localhost"));
        assert!(is_forbidden_hostname("127.0.0.1"));
        assert!(is_forbidden_hostname("::1"));
        assert!(is_forbidden_hostname("10.0.0.1"));
        assert!(is_forbidden_hostname("192.168.1.1"));
        assert!(is_forbidden_hostname("172.16.0.1"));
        assert!(is_forbidden_hostname("169.254.1.1"));
        assert!(is_forbidden_hostname("test.local"));

        assert!(!is_forbidden_hostname("example.com"));
        assert!(!is_forbidden_hostname("8.8.8.8"));
    }

    #[test]
    fn test_forbidden_ips() {
        assert!(is_forbidden_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_forbidden_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_forbidden_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_forbidden_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_forbidden_ip(&"169.254.1.1".parse().unwrap()));
        assert!(is_forbidden_ip(&"::1".parse().unwrap()));
        assert!(is_forbidden_ip(&"fe80::1".parse().unwrap()));

        assert!(!is_forbidden_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_forbidden_ip(&"1.1.1.1".parse().unwrap()));
    }
}
