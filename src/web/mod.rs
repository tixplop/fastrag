//! Design system de la page de test : feuille de style embarquée, composants askama
//! (`templates/composants.html`) et fonctions de format partagées.

use crate::status::EtatIndex;
use askama::Template;
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

/// Erreur de recherche à afficher.
pub struct ErreurVue {
    pub message: String,
    /// Libellé du champ fautif (« Question », « Nombre de résultats »), s'il y en a un.
    pub libelle_champ: Option<&'static str>,
    pub champ_question: bool,
    pub champ_nombre: bool,
}

/// Résultat d'une recherche réussie.
pub struct ResultatsVue {
    pub resume: String,
    pub total: String,
    pub attente: String,
    pub repartition: Repartition,
    pub passages: Vec<PassageVue>,
}

/// État de l'index tel que lu au chargement de la page.
pub struct EtatVue {
    pub notes: usize,
    pub passages: usize,
    pub date_iso: Option<String>,
    pub date_affichee: String,
    pub en_cours: bool,
    pub avertissements: Vec<String>,
}

impl EtatVue {
    pub fn new(e: EtatIndex) -> Self {
        let date_affichee = e
            .derniere_mise_a_jour
            .as_deref()
            .map_or_else(|| "jamais".to_string(), date_affichee);
        EtatVue {
            notes: e.notes,
            passages: e.passages,
            date_iso: e.derniere_mise_a_jour,
            date_affichee,
            en_cours: e.en_cours,
            avertissements: e.avertissements,
        }
    }
}

/// « 2026-11-22T14:03:12Z » devient « 2026-11-22 14:03:12 UTC ».
pub fn date_affichee(rfc3339: &str) -> String {
    format!("{} UTC", rfc3339.trim_end_matches('Z').replace('T', " "))
}

/// Résumé : « 5 passages · ordre : score de fusion ».
pub fn resume(n: usize, reranker: bool) -> String {
    let mot = if n > 1 { "passages" } else { "passage" };
    let ordre = if reranker {
        "score du reranker"
    } else {
        "score de fusion"
    };
    format!("{n} {mot} · ordre : {ordre}")
}

/// La page `GET /` (V-001) : valeurs du formulaire, zone de résultats, état de l'index.
#[derive(Template)]
#[template(path = "page.html")]
pub struct Page {
    pub question: String,
    pub nombre: String,
    pub dossier: String,
    pub tag: String,
    pub erreur: Option<ErreurVue>,
    pub resultats: Option<ResultatsVue>,
    pub etat: EtatVue,
}

impl Page {
    fn champ_en_erreur(&self, f: impl Fn(&ErreurVue) -> bool) -> bool {
        self.erreur.as_ref().is_some_and(f)
    }
    pub fn id_erreur_question(&self) -> &'static str {
        if self.champ_en_erreur(|e| e.champ_question) {
            "erreur-recherche"
        } else {
            ""
        }
    }
    pub fn id_erreur_nombre(&self) -> &'static str {
        if self.champ_en_erreur(|e| e.champ_nombre) {
            "erreur-recherche"
        } else {
            ""
        }
    }
    /// « Question » prend le focus, sauf après une recherche réussie ou si « Nombre » est fautif.
    pub fn focus_question(&self) -> bool {
        self.resultats.is_none() && !self.champ_en_erreur(|e| e.champ_nombre)
    }
    pub fn focus_nombre(&self) -> bool {
        self.champ_en_erreur(|e| e.champ_nombre)
    }
}

impl ErreurVue {
    pub fn new(message: String, champ: Option<&str>) -> Self {
        ErreurVue {
            message,
            libelle_champ: match champ {
                Some("question") => Some("Question"),
                Some("nombre") => Some("Nombre de résultats"),
                _ => None,
            },
            champ_question: champ == Some("question"),
            champ_nombre: champ == Some("nombre"),
        }
    }
}

#[cfg(test)]
mod tests;
