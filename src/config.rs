use clap::Parser;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeMode {
    Api,
    Storage,
    All,
}

impl std::str::FromStr for NodeMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "api" => Ok(NodeMode::Api),
            "storage" => Ok(NodeMode::Storage),
            "all" => Ok(NodeMode::All),
            _ => Err(format!("invalid node mode `{s}`; expected api, storage, or all")),
        }
    }
}

impl std::fmt::Display for NodeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeMode::Api => write!(f, "api"),
            NodeMode::Storage => write!(f, "storage"),
            NodeMode::All => write!(f, "all"),
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Args {
    #[arg(short, long, default_value = "./cache")]
    pub cache_dir: PathBuf,

    #[arg(short = 's', long, value_parser = parse_cache_size)]
    pub cache_size: u64,

    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,

    #[arg(short, long, default_value_t = 8080)]
    pub port: u16,

    #[arg(long, default_value = "5GB", value_parser = parse_cache_size)]
    pub filebox_size: u64,

    #[arg(long, default_value = "./frontend/dist")]
    pub frontend_dist: PathBuf,

    #[arg(long, default_value = "/", value_parser = normalize_base_path)]
    pub base_path: String,

    /// Node operating mode: api, storage, or all (default: all)
    #[arg(long, default_value = "all")]
    pub node_mode: NodeMode,

    /// Unique node identifier (required for storage/all modes)
    #[arg(long, default_value = "local")]
    pub node_id: String,

    /// Public endpoint of this storage node (for storage mode registration)
    #[arg(long)]
    pub node_endpoint: Option<String>,

    /// Availability zone for cross-zone replica placement
    #[arg(long)]
    pub node_zone: Option<String>,

    /// API control plane endpoint (required for storage mode)
    #[arg(long)]
    pub api_endpoint: Option<String>,

    /// Shared internal authentication token for node-to-node communication
    #[arg(long)]
    pub internal_token: Option<String>,

    /// Heartbeat interval in seconds
    #[arg(long, default_value_t = 30)]
    pub node_heartbeat_interval: u64,

    /// Heartbeat TTL in seconds (node considered offline after this)
    #[arg(long, default_value_t = 90)]
    pub node_heartbeat_ttl: u64,

    /// Default chunk size for uploads
    #[arg(long, default_value = "16MiB", value_parser = parse_cache_size)]
    pub default_chunk_size: u64,

    /// Default replication factor for new files
    #[arg(long, default_value_t = 2)]
    pub default_replication_factor: i64,
}

pub fn parse_cache_size(input: &str) -> Result<u64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("cache size cannot be empty".to_string());
    }

    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number_part, unit_part) = trimmed.split_at(split_at);

    if number_part.is_empty() {
        return Err(format!(
            "invalid cache size `{input}`; expected formats like 1073741824, 512MB, 2GB, 1GiB"
        ));
    }

    let value = number_part
        .parse::<u64>()
        .map_err(|_| format!("invalid numeric cache size `{input}`"))?;

    let multiplier = match unit_part.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1_u64,
        "k" | "kb" => 1_000_u64,
        "m" | "mb" => 1_000_000_u64,
        "g" | "gb" => 1_000_000_000_u64,
        "t" | "tb" => 1_000_000_000_000_u64,
        "kib" => 1_024_u64,
        "mib" => 1_024_u64.pow(2),
        "gib" => 1_024_u64.pow(3),
        "tib" => 1_024_u64.pow(4),
        _ => {
            return Err(format!(
                "unsupported cache size unit in `{input}`; supported units: B, KB, MB, GB, TB, KiB, MiB, GiB, TiB"
            ))
        }
    };

    value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("cache size `{input}` is too large"))
}

pub fn parse_socket_addr(host: &str, port: u16) -> SocketAddr {
    let ip: IpAddr = host.parse().unwrap_or_else(|_| {
        panic!("invalid host value: {host}");
    });
    SocketAddr::new(ip, port)
}

pub fn normalize_base_path(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok("/".to_string());
    }

    let mut path = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };

    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    if path.contains('?') || path.contains('#') {
        return Err("base path cannot contain query string or fragment".to_string());
    }

    Ok(path)
}
