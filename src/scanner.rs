//! Condition partitioning and DB selector construction for entity trigger scanning.
//!
//! Conditions are split into two buckets:
//! - **DB conditions** — can be pushed down to the FlowState REST query layer
//!   (`eq`, `neq`, and resolved `tagIds` containment checks).
//! - **Client conditions** — must be evaluated in-process after records are fetched
//!   (range operators, unresolvable tag names, etc.).

use crate::attributes::AttributeMap;
use crate::conditions::evaluate_condition;
use crate::handlers::RunContext;
use crate::models::execution::ExecutionContext;
use crate::models::process::{EntityTriggerCondition, Process};
use crate::models::trigger::Op;
use anyhow::Context;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Partition a slice of conditions into DB-pushable and client-side buckets.
///
/// `tagIds` conditions whose value can be resolved to an attribute ID via
/// `attribute_map` are rewritten with the resolved ID and placed in the DB
/// bucket.  Unresolvable tag names fall through to the client bucket.
///
/// For all other fields only `eq` / `equals` and `neq` / `not-equals`
/// operators are considered DB-pushable.
pub fn partition_conditions(
    conditions: &[EntityTriggerCondition],
    attribute_map: &AttributeMap,
) -> (Vec<EntityTriggerCondition>, Vec<EntityTriggerCondition>) {
    let mut db: Vec<EntityTriggerCondition> = Vec::new();
    let mut client: Vec<EntityTriggerCondition> = Vec::new();
    let mut has_db_tag_condition = false;

    for cond in conditions {
        if cond.property_path == "tagIds" {
            // Only one tagIds condition can be pushed to DB (selector key clobber).
            // Subsequent ones must be evaluated client-side.
            if !has_db_tag_condition {
                if let Some(tag_name) = cond.value.as_str() {
                    if let Some(tag_id) = attribute_map.tag_name_to_id(tag_name) {
                        db.push(EntityTriggerCondition {
                            property_path: cond.property_path.clone(),
                            operator: cond.operator.clone(),
                            value: Value::String(tag_id.to_owned()),
                        });
                        has_db_tag_condition = true;
                        continue;
                    }
                }
            }
            client.push(cond.clone());
            continue;
        }

        // Standard fields: only equality operators are safe to push to the DB layer.
        match cond.operator.as_str() {
            "eq" | "equals" | "neq" | "not-equals" => {
                db.push(cond.clone());
            }
            _ => {
                client.push(cond.clone());
            }
        }
    }

    (db, client)
}

/// Build a JSON selector object suitable for a FlowState REST query.
///
/// The selector always includes `orgId` and `workspaceId` for tenant scoping.
/// Each DB condition is translated into the appropriate MongoDB-style filter:
///
/// | Operator            | Selector form                              |
/// |---------------------|--------------------------------------------|
/// | `eq` / `equals`     | `{ "field": <value> }`                     |
/// | `neq` / `not-equals`| `{ "field": { "$ne": <value> } }`          |
/// | `tagIds` contains   | `{ "tagIds": { "$elemMatch": {"$eq":…} } }` |
///
/// Conditions with unexpected operators are skipped with a warning log.
pub fn build_db_selector(
    db_conditions: &[EntityTriggerCondition],
    org_id: &str,
    workspace_id: &str,
) -> Value {
    let mut selector = serde_json::Map::new();
    selector.insert("orgId".to_string(), json!(org_id));
    selector.insert("workspaceId".to_string(), json!(workspace_id));

    for cond in db_conditions {
        if cond.property_path == "tagIds" {
            selector.insert(
                "tagIds".to_string(),
                json!({"$elemMatch": {"$eq": cond.value}}),
            );
            continue;
        }

        match cond.operator.as_str() {
            "eq" | "equals" => {
                selector.insert(cond.property_path.clone(), cond.value.clone());
            }
            "neq" | "not-equals" => {
                selector.insert(cond.property_path.clone(), json!({"$ne": cond.value}));
            }
            other => {
                tracing::warn!(
                    operator = %other,
                    field = %cond.property_path,
                    "Unexpected operator in DB conditions — skipping"
                );
            }
        }
    }

    Value::Object(selector)
}

/// Evaluate all client-side conditions against a fetched entity.
///
/// Returns `true` if **all** conditions match (logical AND).  An empty slice
/// is vacuously true.  If a condition contains an operator string that cannot
/// be parsed, the condition is treated as failing and a warning is emitted.
pub fn evaluate_client_conditions(entity: &Value, conditions: &[EntityTriggerCondition]) -> bool {
    conditions.iter().all(|cond| {
        let op = match Op::parse(&cond.operator) {
            Ok(op) => op,
            Err(_) => {
                tracing::warn!(
                    operator = %cond.operator,
                    field = %cond.property_path,
                    "Cannot parse trigger condition operator — condition fails"
                );
                return false;
            }
        };
        evaluate_condition(entity, &cond.property_path, &op, &cond.value, None, None)
    })
}

/// Seed an execution variable map from a fetched entity record.
///
/// Only fields that are present and non-null on the entity are inserted.
/// Tag IDs are resolved to names via `attribute_map`; the `tags` key is
/// omitted entirely when `tagIds` is absent from the entity.
pub fn seed_variables(
    entity: &Value,
    collection_name: &str,
    attribute_map: &AttributeMap,
) -> Map<String, Value> {
    let mut vars = Map::new();

    if let Some(id) = entity.get("id").and_then(Value::as_str) {
        vars.insert("entityId".to_string(), json!(id));

        // Type-specific ID aliases so process steps can reference
        // `taskId`, `milestoneId`, or `projectId` directly.
        match collection_name {
            "tasks" => {
                vars.insert("taskId".to_string(), json!(id));
            }
            "milestones" => {
                vars.insert("milestoneId".to_string(), json!(id));
            }
            "projects" => {
                vars.insert("projectId".to_string(), json!(id));
            }
            _ => {}
        }
    }
    vars.insert("entityType".to_string(), json!(collection_name));

    // Context fields — orgId and workspaceId from the entity record
    for field in &["orgId", "workspaceId"] {
        if let Some(val) = entity.get(field) {
            if !val.is_null() {
                vars.insert(field.to_string(), val.clone());
            }
        }
    }

    // Simple string fields — only insert if present and non-null
    for field in &["title", "description", "projectId", "milestoneId"] {
        if let Some(val) = entity.get(field) {
            if !val.is_null() {
                vars.insert(field.to_string(), val.clone());
            }
        }
    }

    // Resolve tagIds to tag names — only if tagIds exists on the entity
    if let Some(tag_ids) = entity.get("tagIds").and_then(Value::as_array) {
        let id_strings: Vec<&str> = tag_ids.iter().filter_map(Value::as_str).collect();
        let tag_names = attribute_map.resolve_tag_ids(&id_strings);
        if !tag_names.is_empty() {
            vars.insert(
                "tags".to_string(),
                Value::Array(tag_names.into_iter().map(Value::String).collect()),
            );
        }
    }

    // Resolve categoryId to category name
    if let Some(cat_id) = entity.get("categoryId").and_then(Value::as_str) {
        if let Some(cat_name) = attribute_map.category_id_to_name(cat_id) {
            vars.insert("category".to_string(), json!(cat_name));
        }
    }

    // metadata.assignedAgent
    if let Some(agent) = entity
        .get("metadata")
        .and_then(|m| m.get("assignedAgent"))
        .and_then(Value::as_str)
    {
        vars.insert("assignedAgent".to_string(), json!(agent));
    }

    vars
}

/// Convert a singular entity type to its plural form.
///
/// Applies simple English pluralization rules:
/// - Words already ending in 's' are returned unchanged
/// - Words ending in consonant + 'y' become '-ies'
/// - All others get '-s' appended
///
/// # Examples
///
/// ```
/// use flowstate_runner::scanner::pluralize_entity_type;
/// assert_eq!(pluralize_entity_type("task"), "tasks");
/// assert_eq!(pluralize_entity_type("category"), "categories");
/// assert_eq!(pluralize_entity_type("tasks"), "tasks");
/// ```
pub fn pluralize_entity_type(entity_type: &str) -> String {
    if entity_type.ends_with('s') {
        return entity_type.to_owned();
    }
    if let Some(prefix) = entity_type.strip_suffix('y') {
        if let Some(c) = prefix.chars().last() {
            if !"aeiou".contains(c) {
                return format!("{}ies", prefix);
            }
        }
        return format!("{}s", entity_type);
    }
    format!("{}s", entity_type)
}

/// Summary of a single scan pass — how many executions were created, skipped,
/// or failed with an error.
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    /// IDs of execution records created during this scan.
    pub created: Vec<String>,
    /// Number of entities skipped because an active execution already existed.
    pub skipped: u32,
    /// Human-readable error messages for any non-fatal failures during the scan.
    pub errors: Vec<String>,
}

/// Scan all active processes with entity triggers and create pending executions
/// for entities that match but have no existing active execution.
///
/// Note: There is a TOCTOU race between the `has_active_execution` check and
/// execution creation via `set()`. The RxDB REST API does not support atomic
/// check-and-create. Duplicate executions are mitigated by the unique
/// `externalId` field — the executor handles duplicates gracefully.
///
/// # Errors
///
/// Returns an error only if the top-level process query fails.  Per-entity
/// failures are captured in `ScanReport::errors` so a single bad entity does
/// not abort the entire scan.
pub async fn scan(ctx: &RunContext) -> anyhow::Result<ScanReport> {
    let mut report = ScanReport::default();

    let processes: Vec<Process> = ctx
        .query(
            "processes",
            json!({
                "status": "active",
                "orgId": ctx.config.org_id,
                "workspaceId": ctx.config.workspace_id
            }),
        )
        .await
        .context("Failed to query active processes")?;

    tracing::info!(count = processes.len(), "Found active processes to scan");

    for process in &processes {
        let trigger = match &process.trigger {
            Some(t) if t.trigger_type == "entity" => t,
            _ => continue,
        };
        let entity_trigger = match &trigger.entity_trigger {
            Some(et) => et,
            None => continue,
        };

        let start_step_id = match &process.start_step_id {
            Some(id) => id.clone(),
            None => {
                let msg = format!("Process '{}' has no start_step_id — skipping", process.name);
                tracing::warn!("{}", msg);
                report.errors.push(msg);
                continue;
            }
        };

        let collection = pluralize_entity_type(&entity_trigger.entity_type);
        let (db_conditions, client_conditions) =
            partition_conditions(&entity_trigger.conditions, &ctx.attribute_map);
        let selector =
            build_db_selector(&db_conditions, &ctx.config.org_id, &ctx.config.workspace_id);

        let entities: Vec<Value> = match ctx.query(&collection, selector).await {
            Ok(e) => e,
            Err(err) => {
                let msg = format!(
                    "Failed to query {} for process '{}': {}",
                    collection, process.name, err
                );
                tracing::warn!("{}", msg);
                report.errors.push(msg);
                continue;
            }
        };

        tracing::debug!(
            process = %process.name,
            collection = %collection,
            entity_count = entities.len(),
            "Queried entities"
        );

        // Collect candidate entity IDs (those passing client conditions)
        let candidate_entities: Vec<&Value> = entities
            .iter()
            .filter(|e| evaluate_client_conditions(e, &client_conditions))
            .collect();

        let candidate_ids: Vec<&str> = candidate_entities
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str))
            .collect();

        // Batch check for existing active executions — one query replaces N serial queries
        let active_ids = match batch_active_check(ctx, &process.id, &candidate_ids).await {
            Ok(ids) => ids,
            Err(err) => {
                let msg = format!(
                    "Failed to batch-check active executions for process '{}': {}",
                    process.name, err
                );
                tracing::warn!("{}", msg);
                report.errors.push(msg);
                continue;
            }
        };

        for entity in &candidate_entities {
            let entity_id = match entity.get("id").and_then(Value::as_str) {
                Some(id) => id,
                None => continue,
            };

            if active_ids.contains(entity_id) {
                tracing::debug!(
                    process = %process.name,
                    entity_id = entity_id,
                    "Skipping — active execution exists"
                );
                report.skipped += 1;
                continue;
            }

            let variables = seed_variables(entity, &collection, &ctx.attribute_map);
            let exec_id = format!("exec_{}", nanoid::nanoid!(10));
            let now = chrono::Utc::now().to_rfc3339();

            let context = match serde_json::to_value(&ExecutionContext {
                entity_type: entity_trigger.entity_type.clone(),
                entity_id: entity_id.to_owned(),
                user_id: entity
                    .get("userId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                tags: vec![],
                category: None,
                depth: 0,
                max_depth: ctx.config.max_subprocess_depth,
                process_name: Some(process.name.clone()),
            }) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to serialize execution context");
                    json!({})
                }
            };

            // Build a descriptive title for the VCA record (required by records schema)
            let exec_title = format!(
                "{} — {}",
                process.title.as_deref().unwrap_or(&process.id),
                entity_id
            );

            let execution_record = json!({
                "id": exec_id,
                "title": exec_title,
                "processId": process.id,
                "processVersion": process.version.clone().unwrap_or_else(|| "1.0.0".to_string()),
                "processName": process.name,
                "orgId": ctx.config.org_id,
                "workspaceId": ctx.config.workspace_id,
                "userId": entity.get("userId"),
                "status": "pending",
                "completed": false,
                "progress": 0,
                "currentStepId": start_step_id,
                "variables": variables,
                "stepHistory": [],
                "inputs": {},
                "context": context,
                "externalId": entity_id,
                "retryCount": 0,
                "maxRetries": 3,
                "archived": false,
                "metadata": {},
                "createdAt": now.clone(),
                "updatedAt": now
            });

            match ctx.set("processexecutions", &execution_record).await {
                Ok(()) => {
                    tracing::info!(
                        execution_id = %exec_id,
                        process = %process.name,
                        entity_id = entity_id,
                        "Created execution"
                    );
                    report.created.push(exec_id);
                }
                Err(err) => {
                    let msg = format!(
                        "Failed to create execution for entity '{}': {}",
                        entity_id, err
                    );
                    tracing::warn!("{}", msg);
                    report.errors.push(msg);
                }
            }
        }
    }

    tracing::info!(
        created = report.created.len(),
        skipped = report.skipped,
        errors = report.errors.len(),
        "Scan complete"
    );
    Ok(report)
}

/// Batch-check which entities already have active executions for a given process.
/// Returns a set of entity IDs that have active (running, paused, or pending) executions.
///
/// Uses a single `$in` query when possible. Falls back to serial queries if the
/// batch query returns no results for a non-empty input set (which can happen
/// when the VCA layer doesn't support `$in` on data-bag fields like `externalId`).
async fn batch_active_check(
    ctx: &RunContext,
    process_id: &str,
    entity_ids: &[&str],
) -> anyhow::Result<HashSet<String>> {
    if entity_ids.is_empty() {
        return Ok(HashSet::new());
    }

    // Attempt batch query first
    let results: Vec<Value> = ctx
        .query(
            "processexecutions",
            json!({
                "processId": process_id,
                "externalId": { "$in": entity_ids },
                "status": { "$in": ["running", "paused", "pending"] }
            }),
        )
        .await?;

    if !results.is_empty() {
        return Ok(results
            .iter()
            .filter_map(|r| {
                r.get("externalId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect());
    }

    // Fallback: $in on VCA data-bag fields may be unsupported. Query all active
    // executions for this process and filter client-side.
    let all_active: Vec<Value> = ctx
        .query(
            "processexecutions",
            json!({
                "processId": process_id,
                "status": { "$in": ["running", "paused", "pending"] }
            }),
        )
        .await?;

    let candidate_set: HashSet<&str> = entity_ids.iter().copied().collect();
    Ok(all_active
        .iter()
        .filter_map(|r| r.get("externalId").and_then(Value::as_str))
        .filter(|eid| candidate_set.contains(eid))
        .map(str::to_owned)
        .collect())
}
