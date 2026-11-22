//! Recherche hybride : vecteurs + mots exacts, fusion RRF, filtres, reranker facultatif (REQ-012 à REQ-015).

use crate::provider::{ErreurProvider, Provider};
use crate::store::{Candidat, Filtres, Store, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Constante de la fusion RRF (DEC-007).
const K_RRF: f64 = 60.0;
/// Nombre de résultats par défaut et plafond.
pub const NOMBRE_DEFAUT: usize = 5;
pub const NOMBRE_MAX: usize = 50;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EntreeRecherche {
    pub question: String,
    #[serde(default)]
    pub nombre: Option<i64>,
    #[serde(default)]
    pub dossier: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Resultat {
    pub texte: String,
    pub chemin: String,
    pub titres: String,
    pub ligne_debut: u32,
    pub ligne_fin: u32,
    pub tags: Vec<String>,
    pub score: f64,
}

#[derive(Debug)]
pub enum ErreurRecherche {
    /// Entrée refusée ; `champ` nomme le champ fautif.
    Entree {
        message: String,
        champ: String,
    },
    Provider(ErreurProvider),
    Store(StoreError),
}

impl fmt::Display for ErreurRecherche {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entree { message, .. } => write!(f, "{message}"),
            Self::Provider(e) => write!(f, "{e}"),
            Self::Store(e) => write!(f, "erreur de l'index : {e}"),
        }
    }
}

impl std::error::Error for ErreurRecherche {}

impl From<ErreurProvider> for ErreurRecherche {
    fn from(e: ErreurProvider) -> Self {
        Self::Provider(e)
    }
}
impl From<StoreError> for ErreurRecherche {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Vérifie l'entrée et renvoie le nombre de résultats à rendre (plafonné à 50).
pub fn valider(entree: &EntreeRecherche) -> Result<usize, ErreurRecherche> {
    if entree.question.trim().is_empty() {
        return Err(ErreurRecherche::Entree {
            message: "question vide".into(),
            champ: "question".into(),
        });
    }
    match entree.nombre {
        None => Ok(NOMBRE_DEFAUT),
        Some(n) if n <= 0 => Err(ErreurRecherche::Entree {
            message: "le nombre de résultats doit être supérieur à 0".into(),
            champ: "nombre".into(),
        }),
        Some(n) => Ok((n as u64).min(NOMBRE_MAX as u64) as usize),
    }
}

pub struct Recherche {
    /// Connexion de lecture, séparée de celle de l'indexeur.
    store: Mutex<Store>,
    provider: Provider,
    reranker: Option<Provider>,
    dossiers: Vec<PathBuf>,
}

/// Préfixes de chemin absolus (terminés par `/`) correspondant au filtre dossier.
fn prefixes(dossier: &str, configures: &[PathBuf]) -> Vec<String> {
    let avec_barre = |p: &Path| format!("{}/", p.display().to_string().trim_end_matches('/'));
    if dossier.starts_with('/') {
        return vec![avec_barre(Path::new(dossier))];
    }
    let rel = dossier.trim_matches('/');
    let tous: Vec<PathBuf> = configures.iter().map(|d| d.join(rel)).collect();
    let existants: Vec<String> = tous
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| avec_barre(p))
        .collect();
    if existants.is_empty() {
        tous.iter().map(|p| avec_barre(p)).collect()
    } else {
        existants
    }
}

/// Fusion RRF : score = somme de 1 / (60 + rang) ; renvoie les candidats triés.
fn fusionner(voisins: Vec<Candidat>, mots: Vec<Candidat>) -> Vec<(Candidat, f64)> {
    let mut scores: HashMap<i64, (Candidat, f64)> = HashMap::new();
    for liste in [voisins, mots] {
        for (rang, c) in liste.into_iter().enumerate() {
            let s = 1.0 / (K_RRF + rang as f64 + 1.0);
            scores
                .entry(c.passage_id)
                .and_modify(|e| e.1 += s)
                .or_insert((c, s));
        }
    }
    let mut v: Vec<_> = scores.into_values().collect();
    v.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then(a.0.passage_id.cmp(&b.0.passage_id))
    });
    v
}

impl Recherche {
    pub fn new(
        store: Store,
        provider: Provider,
        reranker: Option<Provider>,
        dossiers: Vec<PathBuf>,
    ) -> Self {
        Self {
            store: Mutex::new(store),
            provider,
            reranker,
            dossiers,
        }
    }

    pub async fn chercher(
        &self,
        entree: &EntreeRecherche,
    ) -> Result<Vec<Resultat>, ErreurRecherche> {
        let nombre = valider(entree)?;
        let question = entree.question.trim();
        let vide = |s: &Store| -> Result<bool, StoreError> { Ok(s.dimension()?.is_none()) };
        if vide(&self.store.lock().unwrap())? {
            return Ok(Vec::new());
        }
        let vecteur = self
            .provider
            .embeddings(&[question.to_string()])
            .await?
            .pop()
            .ok_or_else(|| ErreurProvider::ReponseInvalide("aucun vecteur reçu".into()))?;

        let tag = entree.tag.clone().filter(|t| !t.is_empty());
        let dossiers: Vec<Option<String>> = match entree.dossier.as_deref() {
            Some(d) if !d.trim().is_empty() => prefixes(d.trim(), &self.dossiers)
                .into_iter()
                .map(Some)
                .collect(),
            _ => vec![None],
        };
        let (mut voisins, mut mots) = (Vec::new(), Vec::new());
        let tags_par_chemin;
        {
            let store = self.store.lock().unwrap();
            for dossier in dossiers {
                let filtres = Filtres {
                    dossier,
                    tag: tag.clone(),
                };
                voisins.extend(store.voisins(&vecteur, &filtres)?);
                mots.extend(store.mots(question, &filtres)?);
            }
            voisins.sort_by(|a, b| a.score.total_cmp(&b.score));
            mots.sort_by(|a, b| a.score.total_cmp(&b.score));
            voisins.truncate(crate::store::NB_CANDIDATS);
            mots.truncate(crate::store::NB_CANDIDATS);
        }
        let mut fusion = fusionner(voisins, mots);
        if fusion.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(r) = &self.reranker {
            let docs: Vec<String> = fusion.iter().map(|(c, _)| c.texte.clone()).collect();
            let mut notes = r.rerank(question, &docs).await?;
            notes.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut pris: Vec<Option<(Candidat, f64)>> = fusion.into_iter().map(Some).collect();
            fusion = notes
                .into_iter()
                .filter_map(|(i, s)| pris[i].take().map(|(c, _)| (c, s as f64)))
                .collect();
        }
        fusion.truncate(nombre);
        {
            let store = self.store.lock().unwrap();
            let mut m = HashMap::new();
            for (c, _) in &fusion {
                if !m.contains_key(&c.chemin) {
                    m.insert(c.chemin.clone(), store.tags_note(&c.chemin)?);
                }
            }
            tags_par_chemin = m;
        }
        Ok(fusion
            .into_iter()
            .map(|(c, score)| Resultat {
                tags: tags_par_chemin.get(&c.chemin).cloned().unwrap_or_default(),
                texte: c.texte,
                chemin: c.chemin,
                titres: c.titres,
                ligne_debut: c.ligne_debut,
                ligne_fin: c.ligne_fin,
                score,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entree(q: &str, n: Option<i64>) -> EntreeRecherche {
        EntreeRecherche {
            question: q.into(),
            nombre: n,
            dossier: None,
            tag: None,
        }
    }

    #[test]
    fn req012_ac3_nombre_plafonne_a_50_et_refus_des_valeurs_non_positives() {
        assert_eq!(valider(&entree("q", None)).unwrap(), 5);
        assert_eq!(valider(&entree("q", Some(51))).unwrap(), 50);
        assert_eq!(valider(&entree("q", Some(1000))).unwrap(), 50);
        assert_eq!(valider(&entree("q", Some(50))).unwrap(), 50);
        assert_eq!(valider(&entree("q", Some(3))).unwrap(), 3);
        for n in [0, -1] {
            let e = valider(&entree("q", Some(n))).unwrap_err();
            assert!(matches!(&e, ErreurRecherche::Entree { champ, .. } if champ == "nombre"));
            assert!(e.to_string().contains("supérieur à 0"));
        }
    }

    #[test]
    fn req012_ac4_question_vide_ou_blanche() {
        for q in ["", "   ", "\t\n "] {
            let e = valider(&entree(q, None)).unwrap_err();
            assert_eq!(e.to_string(), "question vide");
        }
    }

    #[test]
    fn dossier_relatif_ou_absolu() {
        let c = vec![PathBuf::from("/notes")];
        assert_eq!(prefixes("/a/b", &c), vec!["/a/b/"]);
        assert_eq!(prefixes("travail/", &c), vec!["/notes/travail/"]);
    }
}
