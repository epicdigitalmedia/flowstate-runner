use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Condition operators for trigger evaluation and decision routing.
/// 14 total: 12 logical operators + 2 aliases (ChangesTo = Equals, ChangesFrom = NotEquals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Equals,
    NotEquals,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    In,
    NotIn,
    Exists,
    NotExists,
    Regex,
    ChangesTo,
    ChangesFrom,
}

impl Op {
    pub fn parse(s: &str) -> Result<Self, OpParseError> {
        match s {
            "equals" | "eq" => Ok(Op::Equals),
            "not-equals" | "neq" => Ok(Op::NotEquals),
            "changes-to" => Ok(Op::ChangesTo),
            "changes-from" => Ok(Op::ChangesFrom),
            "gt" => Ok(Op::Gt),
            "gte" => Ok(Op::Gte),
            "lt" => Ok(Op::Lt),
            "lte" => Ok(Op::Lte),
            "contains" => Ok(Op::Contains),
            "in" => Ok(Op::In),
            "not-in" => Ok(Op::NotIn),
            "exists" => Ok(Op::Exists),
            "not-exists" => Ok(Op::NotExists),
            "regex" => Ok(Op::Regex),
            _ => Err(OpParseError(s.to_string())),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Op::Equals => "equals",
            Op::NotEquals => "not-equals",
            Op::Gt => "gt",
            Op::Gte => "gte",
            Op::Lt => "lt",
            Op::Lte => "lte",
            Op::Contains => "contains",
            Op::In => "in",
            Op::NotIn => "not-in",
            Op::Exists => "exists",
            Op::NotExists => "not-exists",
            Op::Regex => "regex",
            Op::ChangesTo => "changes-to",
            Op::ChangesFrom => "changes-from",
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Unknown operator: {0}")]
pub struct OpParseError(pub String);

impl Serialize for Op {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Op {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Op::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Type alias satisfying the spec's TriggerConfig requirement.
pub type TriggerConfig = super::process::ProcessTrigger;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerCondition {
    pub field: String,
    pub operator: Op,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub value_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepCondition {
    #[serde(default)]
    pub id: Option<String>,
    pub field: String,
    pub operator: Op,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub value_from: Option<String>,
    pub target_step_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub order: Option<u32>,
}
