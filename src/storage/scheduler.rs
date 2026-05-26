use super::discovery::NodeView;

/// Select nodes for writing new chunks, scored and ranked.
/// Returns up to `count` nodes, preferring different zones for replica diversity.
pub fn select_upload_nodes(
    nodes: &[NodeView],
    count: usize,
    min_free_bytes: i64,
) -> Vec<NodeView> {
    let mut candidates: Vec<(f64, &NodeView)> = nodes
        .iter()
        .filter(|n| {
            n.status == "active"
                && (n.capacity_bytes - n.used_bytes) > min_free_bytes
                && n.has_feature("chunk-write")
        })
        .map(|n| (upload_score(n), n))
        .collect();

    // Sort by score descending
    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Select up to `count` nodes, preferring different zones
    let mut selected = Vec::new();
    let mut used_zones = Vec::new();

    // First pass: pick best from each unique zone
    for (score, node) in &candidates {
        if selected.len() >= count {
            break;
        }
        let zone = node.zone.as_deref().unwrap_or("default");
        if !used_zones.contains(&zone) {
            selected.push((*score, (*node).clone()));
            used_zones.push(zone);
        }
    }

    // Second pass: fill remaining from any zone
    for (score, node) in &candidates {
        if selected.len() >= count {
            break;
        }
        if !selected.iter().any(|(_, n)| n.id == node.id) {
            selected.push((*score, (*node).clone()));
        }
    }

    selected.into_iter().map(|(_, n)| n).collect()
}

/// Score a node for upload suitability.
/// Higher score = better candidate.
fn upload_score(node: &NodeView) -> f64 {
    let free_ratio = node.free_ratio();
    let avg_rtt = node.avg_rtt_ms.unwrap_or(0) as f64;
    let p95_rtt = node.p95_rtt_ms.unwrap_or(0) as f64;
    let ploss = node.packet_loss.unwrap_or(0.0);
    let torate = node.timeout_rate.unwrap_or(0.0);

    100.0 * free_ratio
        - 0.05 * avg_rtt
        - 0.02 * p95_rtt
        - 200.0 * ploss
        - 100.0 * torate
        - 2.0 * (node.active_uploads as f64)
        - 1.0 * (node.active_downloads as f64)
        - 2.0 * (node.active_replications as f64)
}

/// Select the best replica for downloading a chunk.
/// Returns the node_id of the best replica from the provided list.
pub fn select_download_replica(
    replicas: &[(String, String)], // (node_id, object_key)
    nodes: &[NodeView],
) -> Option<(String, String)> {
    let mut scored: Vec<(f64, &(String, String))> = replicas
        .iter()
        .filter_map(|r| {
            let node = nodes.iter().find(|n| n.id == r.0)?;
            Some((download_score(node), r))
        })
        .collect();

    // Also include replicas on unknown nodes (local/embedded) with default score
    for replica in replicas {
        if !scored.iter().any(|(_, r)| r.0 == replica.0) {
            scored.push((50.0, replica)); // default score for local/unregistered nodes
        }
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.first().map(|(_, r)| (*r).clone())
}

/// Score a node for download suitability.
/// Higher score = better for reading.
fn download_score(node: &NodeView) -> f64 {
    let avg_rtt = node.avg_rtt_ms.unwrap_or(0) as f64;
    let p95_rtt = node.p95_rtt_ms.unwrap_or(0) as f64;
    let ploss = node.packet_loss.unwrap_or(0.0);
    let torate = node.timeout_rate.unwrap_or(0.0);

    100.0
        - 0.1 * avg_rtt
        - 0.03 * p95_rtt
        - 300.0 * ploss
        - 100.0 * torate
        - 2.0 * (node.active_downloads as f64)
}
