/// Helper functions for filebox quota management and validation.
use crate::common::AppState;

/// Reserve space in the cache for an upload.
/// Returns Ok(()) if space was successfully reserved, Err otherwise.
pub fn try_reserve_space(state: &AppState, bytes: u64) -> Result<(), &'static str> {
    let current = state.cache_usage.get();

    // Check if adding this would exceed the limit
    if current + bytes > state.max_cache_size {
        return Err("Storage space insufficient");
    }

    // Atomically reserve the space
    state.cache_usage.add(bytes);

    // Double-check after reservation (race condition protection)
    if state.cache_usage.get() > state.max_cache_size {
        // Rollback the reservation
        state.cache_usage.sub(bytes);
        return Err("Storage space insufficient");
    }

    Ok(())
}

/// Release previously reserved space.
pub fn release_space(state: &AppState, bytes: u64) {
    state.cache_usage.sub(bytes);
}

/// Check if a file size is within the allowed limit.
pub fn check_file_size_limit(state: &AppState, size: u64) -> Result<(), &'static str> {
    if state.max_file_size > 0 && size > state.max_file_size {
        return Err("File size exceeds maximum allowed size");
    }
    Ok(())
}

/// Validate upload_id to prevent path traversal.
/// Only allows alphanumeric characters, hyphens, and underscores.
pub fn validate_upload_id(upload_id: &str) -> Result<(), &'static str> {
    if upload_id.is_empty() {
        return Err("Missing upload_id");
    }

    if upload_id.len() > 64 {
        return Err("Invalid upload_id: too long");
    }

    // Whitelist: only allow safe characters
    if !upload_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid upload_id: contains forbidden characters");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_upload_id() {
        assert!(validate_upload_id("abc123").is_ok());
        assert!(validate_upload_id("test-upload_123").is_ok());
        assert!(validate_upload_id("").is_err());
        assert!(validate_upload_id("../etc/passwd").is_err());
        assert!(validate_upload_id("test/path").is_err());
        assert!(validate_upload_id("test\\path").is_err());
        assert!(validate_upload_id("test..path").is_err());
    }
}
