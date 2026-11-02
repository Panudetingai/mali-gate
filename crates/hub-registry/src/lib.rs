//! คลัง skill: parse SKILL.md (YAML frontmatter: name, description)
//! index ลง SQLite FTS5, โครงการติดตั้ง skill (validate + pin hash)
//!
//! เริ่มจริงในเฟส 1 (parse + index) และเฟส 4 (install + registry ภายนอก)

pub mod config;
pub mod index;
pub mod install;
pub mod pending_install;
pub mod registry;
pub mod scripts;
pub mod skill;
pub mod sync;

pub use config::{InstallSection, RegistryConfigFile, RegistrySection};
pub use install::{
    InstallError, InstallSource, PinRecord, install_skill, parse_source, preview_source,
};
pub use pending_install::{PendingInstall, PendingInstallStore};
pub use registry::{RegistryEntry, load_entries, merge_search_json, search_registry};
pub use sync::{
    CleanOptions, CleanReport, CleanedEntry, Conflict, NotInLibrary, SkippedIdentical, SyncError,
    SyncOptions, SyncReport, clean_cli_skills, default_sync_roots, remove_skill, sync_skills,
};
