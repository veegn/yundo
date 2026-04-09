use clap::Parser;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

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

    #[arg(long, default_value = "./frontend/dist")]
    pub frontend_dist: PathBuf,
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
