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

pub mod mock_provider;

/// Processus enfant tué et attendu à la destruction, y compris quand le test échoue (panic).
pub struct GardeEnfant(Option<std::process::Child>);

impl GardeEnfant {
    pub fn new(enfant: std::process::Child) -> Self {
        Self(Some(enfant))
    }

    /// Attend la fin du processus (déjà tué) et rend ses sorties.
    pub fn sortie(mut self) -> std::io::Result<std::process::Output> {
        self.0.take().unwrap().wait_with_output()
    }
}

impl std::ops::Deref for GardeEnfant {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for GardeEnfant {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap()
    }
}

impl Drop for GardeEnfant {
    fn drop(&mut self) {
        if let Some(mut e) = self.0.take() {
            let _ = e.kill();
            let _ = e.wait();
        }
    }
}
