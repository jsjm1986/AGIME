//! Conflict resolution for sync operations

use crate::error::TeamResult;

/// Conflict resolution strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// Keep local version
    KeepLocal,
    /// Keep remote version
    KeepRemote,
    /// Keep both by creating a new version
    KeepBoth,
}

impl Default for ConflictStrategy {
    fn default() -> Self {
        ConflictStrategy::KeepRemote
    }
}

/// Conflict resolver
pub struct ConflictResolver {
    strategy: ConflictStrategy,
}

impl ConflictResolver {
    /// Create a new conflict resolver with default strategy
    pub fn new() -> Self {
        Self {
            strategy: ConflictStrategy::default(),
        }
    }

    /// Create with specific strategy
    pub fn with_strategy(strategy: ConflictStrategy) -> Self {
        Self { strategy }
    }

    /// Resolve a conflict between local and remote versions
    pub fn resolve<T: Clone>(&self, local: &T, remote: &T) -> TeamResult<ResolvedConflict<T>> {
        match self.strategy {
            ConflictStrategy::KeepLocal => Ok(ResolvedConflict {
                result: local.clone(),
                action: "kept_local".to_string(),
            }),
            ConflictStrategy::KeepRemote => Ok(ResolvedConflict {
                result: remote.clone(),
                action: "kept_remote".to_string(),
            }),
            ConflictStrategy::KeepBoth => {
                // In this case, we return remote but indicate both should be kept
                Ok(ResolvedConflict {
                    result: remote.clone(),
                    action: "kept_both".to_string(),
                })
            }
        }
    }
}

impl Default for ConflictResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of conflict resolution
#[derive(Debug, Clone)]
pub struct ResolvedConflict<T> {
    /// The resolved value
    pub result: T,
    /// Description of the action taken
    pub action: String,
}
