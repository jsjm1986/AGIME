pub mod agime_mode;
pub mod base;
pub mod declarative_providers;
pub mod env_compat;
mod experiments;
pub mod extensions;
pub mod paths;
pub mod permission;
pub mod search_path;
pub mod signup_openrouter;
pub mod signup_tetrate;

pub use crate::agents::ExtensionConfig;
pub use agime_mode::{AgimeMode, GooseMode};
pub use base::{Config, ConfigError};
pub use declarative_providers::DeclarativeProviderConfig;
pub use experiments::ExperimentManager;
pub use extensions::{
    get_all_extension_names, get_all_extensions, get_enabled_extensions, get_extension_by_name,
    is_extension_enabled, remove_extension, set_extension, set_extension_enabled, ExtensionEntry,
};
pub use permission::PermissionManager;
pub use signup_openrouter::configure_openrouter;
pub use signup_tetrate::configure_tetrate;

pub use extensions::DEFAULT_DISPLAY_NAME;
pub use extensions::DEFAULT_EXTENSION;
pub use extensions::DEFAULT_EXTENSION_DESCRIPTION;
pub use extensions::DEFAULT_EXTENSION_TIMEOUT;

// Environment variable compatibility layer for AGIME/Goose migration
pub use env_compat::{
    env_compat_exists, get_env_compat, get_env_compat_or, get_env_compat_parsed,
    get_env_compat_parsed_or, get_env_compat_with_source, is_agime_key, is_legacy_key,
    migrate_key_to_agime, AGIME_PREFIX, GOOSE_PREFIX,
};
