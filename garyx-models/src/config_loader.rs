#[path = "config_loader/backups.rs"]
mod backups;
#[path = "config_loader/diagnostics.rs"]
mod diagnostics;
#[path = "config_loader/hot_reload.rs"]
mod hot_reload;
#[path = "config_loader/includes.rs"]
mod includes;
#[path = "config_loader/load.rs"]
mod load;
#[path = "config_loader/paths.rs"]
mod paths;
#[path = "config_loader/pipeline.rs"]
mod pipeline;
#[path = "config_loader/write.rs"]
mod write;

pub use backups::{backup_config, list_backups, restore_config};
pub use diagnostics::{ConfigDiagnostic, ConfigDiagnostics};
pub use hot_reload::{ConfigHotReloadOptions, ConfigHotReloader, ConfigReloadMetricsSnapshot};
pub use includes::process_includes;
pub use load::{
    ConfigLoadFailure, ConfigLoadOptions, ConfigRuntimeOverrides, LoadedConfig, load_config,
};
pub use paths::{PreparedConfigPath, default_config_path, prepare_config_path_for_io};
pub use pipeline::{strip_legacy_config_fields, strip_redundant_config_fields};
pub use write::{ConfigWriteOptions, write_config_atomic, write_config_value_atomic};

#[cfg(test)]
#[path = "config_loader/tests.rs"]
mod tests;
