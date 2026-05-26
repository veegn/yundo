/// Structured audit logging for security-relevant operations.
/// Phase 1: writes structured tracing events. Future: persist to DB table.

#[derive(Debug)]
pub struct AuditEvent<'a> {
    pub action: &'a str,
    pub actor: &'a str,
    pub node_id: Option<&'a str>,
    pub result: &'a str,
    pub reason: Option<&'a str>,
}

impl<'a> AuditEvent<'a> {
    pub fn log(&self) {
        tracing::info!(
            audit = true,
            action = self.action,
            actor = self.actor,
            node_id = self.node_id.unwrap_or("-"),
            result = self.result,
            reason = self.reason.unwrap_or("-"),
            "AUDIT"
        );
    }
}

/// Convenience function for logging audit events.
pub fn audit(action: &str, actor: &str, node_id: Option<&str>, result: &str, reason: Option<&str>) {
    AuditEvent {
        action,
        actor,
        node_id,
        result,
        reason,
    }
    .log();
}
