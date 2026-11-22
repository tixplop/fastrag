//! Parcours des dossiers configurés (REQ-002).

use std::path::{Path, PathBuf};

const EXTENSIONS: [&str; 3] = ["md", "markdown", "txt"];

/// Fichiers de notes trouvés (chemins absolus, triés) et avertissements.
pub struct Parcours {
    pub fichiers: Vec<PathBuf>,
    pub avertissements: Vec<String>,
}

pub fn parcourir(dossiers: &[PathBuf]) -> Parcours {
    let mut p = Parcours {
        fichiers: Vec::new(),
        avertissements: Vec::new(),
    };
    for d in dossiers {
        let d = std::path::absolute(d).unwrap_or_else(|_| d.clone());
        if !d.is_dir() {
            p.avertissements.push(format!(
                "{} : dossier introuvable, il est ignoré",
                d.display()
            ));
            continue;
        }
        visiter(&d, &mut p);
    }
    p.fichiers.sort();
    p.fichiers.dedup();
    p
}

fn visiter(dossier: &Path, p: &mut Parcours) {
    let entrees = match std::fs::read_dir(dossier) {
        Ok(e) => e,
        Err(e) => {
            p.avertissements.push(format!(
                "{} : dossier illisible ({e}), il est ignoré",
                dossier.display()
            ));
            return;
        }
    };
    for entree in entrees.flatten() {
        let chemin = entree.path();
        let cache = entree.file_name().to_string_lossy().starts_with('.');
        if entree.file_type().is_ok_and(|t| t.is_dir()) {
            if !cache {
                visiter(&chemin, p);
            }
        } else if chemin.extension().is_some_and(|e| {
            EXTENSIONS
                .iter()
                .any(|x| e.to_string_lossy().eq_ignore_ascii_case(x))
        }) {
            p.fichiers.push(chemin);
        }
    }
}
