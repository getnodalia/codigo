// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where Código keeps its files.

use std::path::PathBuf;

/// The folder for settings and session state: `$CODIGO_CONFIG_DIR` when set (portable installs,
/// tests), else the platform's config folder (`~/.config/codigo` on Linux).
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODIGO_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("dev", "danielmiguelt", "Codigo")
        .map(|d| d.config_dir().to_path_buf())
}

/// The folder for caches: language-server downloads, indexes.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODIGO_CACHE_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("dev", "danielmiguelt", "Codigo")
        .map(|d| d.cache_dir().to_path_buf())
}
