//! Design system de la page de test : feuille de style embarquée, composants askama
//! (`templates/composants.html`) et fonctions de format partagées.

use std::path::Path;
use std::time::Duration;

/// Feuille de style unique, servie depuis le binaire.
pub const STYLE_CSS: &str = include_str!("style.css");

/// Durée en millisecondes entières : « 168 ms », « < 1 ms ».
pub fn duree(d: Duration) -> String {
    match d.as_millis() {
        0 => "< 1 ms".to_string(),
        ms => format!("{ms} ms"),
    }
}

/// Score à 4 décimales avec un point : « 0.0325 ».
pub fn score(s: f64) -> String {
    format!("{s:.4}")
}

/// Largeur d'un segment en pourcentage, à 0,1 % près : « 83.9% ».
pub fn largeur(partie: Duration, total: Duration) -> String {
    let part = if total.is_zero() {
        0.0
    } else {
        partie.as_secs_f64() / total.as_secs_f64()
    };
    format!("{:.1}%", (part * 100.0).clamp(0.0, 100.0))
}

/// Un passage prêt à être affiché (C-007).
pub struct PassageVue {
    pub rang: usize,
    pub titres: String,
    pub fichier: String,
    pub debut: usize,
    pub fin: usize,
    pub tags: Vec<String>,
    pub fusion: f64,
    pub rang_sens: Option<usize>,
    pub rang_mots: Option<usize>,
    pub reranker: Option<f64>,
    pub texte: String,
}

impl PassageVue {
    /// Chemin de titres, ou nom du fichier suivi de « (sans titre) » s'il est vide.
    pub fn titre_affiche(&self) -> String {
        if self.titres.trim().is_empty() {
            let nom = Path::new(&self.fichier)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.fichier.clone());
            format!("{nom} (sans titre)")
        } else {
            self.titres.clone()
        }
    }
    pub fn fusion_texte(&self) -> String {
        score(self.fusion)
    }
    pub fn reranker_texte(&self) -> String {
        self.reranker.map(score).unwrap_or_default()
    }
    pub fn rang_sens_texte(&self) -> String {
        rang(self.rang_sens)
    }
    pub fn rang_mots_texte(&self) -> String {
        rang(self.rang_mots)
    }
}

fn rang(r: Option<usize>) -> String {
    r.map_or_else(|| "absent".to_string(), |r| r.to_string())
}

pub struct Segment {
    pub classe: &'static str,
    pub largeur: String,
}

pub struct EntreeLegende {
    pub classe: &'static str,
    pub libelle: &'static str,
    pub valeur: String,
    pub absent: bool,
}

/// Répartition d'une durée totale (C-006).
pub struct Repartition {
    pub segments: Vec<Segment>,
    pub legende: Vec<EntreeLegende>,
}

impl Repartition {
    /// `reranker` vaut `None` s'il est désactivé ; la recherche locale est le reste.
    pub fn new(total: Duration, embedding: Duration, reranker: Option<Duration>) -> Self {
        let local = total.saturating_sub(embedding + reranker.unwrap_or_default());
        let mut segments = Vec::new();
        if !total.is_zero() {
            let parts = [
                ("temps-embedding", Some(embedding)),
                ("temps-reranker", reranker),
                ("temps-local", Some(local)),
            ];
            for (classe, d) in parts {
                if let Some(d) = d.filter(|d| !d.is_zero()) {
                    segments.push(Segment {
                        classe,
                        largeur: largeur(d, total),
                    });
                }
            }
        }
        let legende = vec![
            EntreeLegende {
                classe: "temps-embedding",
                libelle: "Embedding de la question",
                valeur: duree(embedding),
                absent: false,
            },
            EntreeLegende {
                classe: "temps-reranker",
                libelle: "Reranker",
                valeur: reranker.map_or_else(|| "désactivé".to_string(), duree),
                absent: reranker.is_none(),
            },
            EntreeLegende {
                classe: "temps-local",
                libelle: "Recherche locale",
                valeur: duree(local),
                absent: false,
            },
        ];
        Repartition { segments, legende }
    }
}

#[cfg(test)]
mod tests;
