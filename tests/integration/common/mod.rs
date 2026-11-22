//! Aide commune aux tests d'intégration (provider simulé, dossiers de notes : tâches suivantes).

use std::path::PathBuf;

/// Dossier propre à un test, sous `TMPDIR`.
pub fn dossier_test(nom: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("{nom}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
