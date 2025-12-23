use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgimeMode {
    Auto,
    Approve,
    SmartApprove,
    Chat,
}

impl FromStr for AgimeMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(AgimeMode::Auto),
            "approve" => Ok(AgimeMode::Approve),
            "smart_approve" => Ok(AgimeMode::SmartApprove),
            "chat" => Ok(AgimeMode::Chat),
            _ => Err(format!("invalid mode: {}", s)),
        }
    }
}

// Backward compatibility alias
pub type GooseMode = AgimeMode;
