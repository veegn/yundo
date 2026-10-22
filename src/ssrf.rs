//! Validate the addresses used by the HTTP connector, including redirect targets.
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    ClientBuilder,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use url::Url;

const FORBIDDEN: &str = "access to local or private networks is forbidden";

pub fn safe_client_builder() -> ClientBuilder {
    reqwest::Client::builder()
        // Environment proxies resolve/connect outside our DNS filter.
        .no_proxy()
        .timeout(Duration::from_secs(
            crate::constants::PROXY_CLIENT_TIMEOUT_SECS,
        ))
        .dns_resolver(Arc::new(SafeResolver(SystemResolver)))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= crate::constants::MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if validate_url_host(attempt.url()).is_err() {
                attempt.error(FORBIDDEN)
            } else {
                attempt.follow()
            }
        }))
}

pub fn validate_url_host(url: &Url) -> Result<(), &'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only HTTP and HTTPS URLs are supported");
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or("invalid URL: missing host")?;
    if is_forbidden_hostname(host) {
        return Err(FORBIDDEN);
    }
    Ok(())
}

pub async fn validate_url_safe(url: &Url) -> Result<(), &'static str> {
    validate_url_host(url)?;
    let host = url.host_str().unwrap().trim_matches(['[', ']']);
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let addresses = tokio::net::lookup_host((host, 0))
        .await
        .map_err(|_| "DNS resolution failed")?;
    validate_addresses(addresses.collect())
        .map(|_| ())
        .map_err(|_| FORBIDDEN)
}

struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((name.as_str(), 0)).await?;
            Ok(Box::new(addresses.collect::<Vec<_>>().into_iter()) as Addrs)
        })
    }
}

struct SafeResolver<R>(R);

impl<R: Resolve> Resolve for SafeResolver<R> {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = self.0.resolve(name);
        Box::pin(async move {
            let addresses = validate_addresses(lookup.await?.collect())?;
            // The connector uses these exact addresses, without a second lookup.
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn validate_addresses(addresses: Vec<SocketAddr>) -> std::io::Result<Vec<SocketAddr>> {
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| is_forbidden_ip(&address.ip()))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            FORBIDDEN,
        ));
    }
    Ok(addresses)
}

pub fn is_forbidden_hostname(host: &str) -> bool {
    let lower = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return is_forbidden_ip(&ip);
    }
    lower == "localhost"
        || lower == "localhost.localdomain"
        || [".localhost", ".local", ".internal", ".corp"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

pub fn is_forbidden_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || octets[0] == 0
                || octets[0] >= 240
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 198 && (18..=19).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        }
        IpAddr::V6(ip) => {
            if let Some(ipv4) = ip.to_ipv4() {
                return is_forbidden_ip(&IpAddr::V4(ipv4));
            }
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || segments[0] & 0xffc0 == 0xfe80
                || segments[0] & 0xfe00 == 0xfc00
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_local_names_and_encoded_private_addresses() {
        for host in [
            "localhost",
            "localhost.",
            "test.localhost.",
            "test.local",
            "metadata.google.internal",
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.1.1",
            "::1",
            "[::ffff:127.0.0.1]",
            "[::ffff:192.168.1.1]",
        ] {
            assert!(is_forbidden_hostname(host), "{host}");
        }
        assert!(!is_forbidden_hostname("example.com"));
        assert!(!is_forbidden_hostname("8.8.8.8"));
    }

    #[test]
    fn rejects_mixed_or_empty_dns_answers() {
        let public = "8.8.8.8:443".parse().unwrap();
        assert!(validate_addresses(vec![public]).is_ok());
        assert!(validate_addresses(vec![]).is_err());
        for private in [
            "127.0.0.1:80",
            "10.0.0.1:80",
            "[::1]:80",
            "[::ffff:127.0.0.1]:80",
            "100.64.0.1:80",
        ] {
            assert!(validate_addresses(vec![public, private.parse().unwrap()]).is_err());
        }
    }

    #[tokio::test]
    async fn connector_rejects_private_dns_answers() {
        struct LocalResolver;
        impl Resolve for LocalResolver {
            fn resolve(&self, _: Name) -> Resolving {
                Box::pin(async {
                    Ok(
                        Box::new(vec!["127.0.0.1:80".parse::<SocketAddr>().unwrap()].into_iter())
                            as Addrs,
                    )
                })
            }
        }
        let client = safe_client_builder()
            .dns_resolver(Arc::new(SafeResolver(LocalResolver)))
            .build()
            .unwrap();
        let error = client
            .get("http://upstream.test/file")
            .send()
            .await
            .unwrap_err();
        let mut source: &dyn std::error::Error = &error;
        while let Some(next) = source.source() {
            source = next;
        }
        assert!(source.to_string().contains(FORBIDDEN));
    }
}
