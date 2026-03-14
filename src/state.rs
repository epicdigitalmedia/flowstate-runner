use std::path::Path;

/// Derive the plan directory from a base path and an optional external ID.
/// Returns None if external_id is absent or empty.
pub fn compute_plan_dir(plan_base_dir: &str, external_id: Option<&str>) -> Option<String> {
    match external_id {
        Some(id) if !id.is_empty() => {
            let base = plan_base_dir.trim_end_matches('/');
            Some(Path::new(base).join(id).to_string_lossy().to_string())
        }
        _ => None,
    }
}
