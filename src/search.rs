//! Recherche hybride : vecteurs + mots exacts, fusion RRF, filtres, reranker facultatif (REQ-012 à REQ-015).

use crate::provider::{ErreurProvider, Provider};
use crate::store::{Candidat, Filtres, Store, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Constante de la fusion RRF (DEC-007).
const K_RRF: f64 = 60.0;
/// Nombre de résultats par défaut et plafond.
pub const NOMBRE_DEFAUT: usize = 5;
pub const NOMBRE_MAX: usize = 50;

#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct EntreeRecherche {
    /// Question en langage naturel.
    pub question: String,
    /// Nombre de résultats voulus (1 à 50, 5 par défaut).
    #[serde(default)]
    pub nombre: Option<i64>,
    /// Limite la recherche à ce dossier (chemin absolu, ou relatif aux dossiers configurés).
    #[serde(default)]
    pub dossier: Option<String>,
    /// Limite la recherche aux notes portant ce tag.
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

/// Un passage trouvé, avec le détail de son classement (REQ-021).
#[derive(Debug, Clone, PartialEq)]
pub struct PassageDetail {
    pub resultat: Resultat,
    /// Score de fusion RRF.
    pub fusion: f64,
    /// Rang (1 à 50) dans la liste des voisins ; `None` = absent.
    pub rang_sens: Option<usize>,
    /// Rang (1 à 50) dans la liste des mots ; `None` = absent.
    pub rang_mots: Option<usize>,
    /// Score du reranker, s'il est activé.
    pub reranker: Option<f64>,
}

/// Durées mesurées par le service.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Durees {
    pub total: Duration,
    pub embedding: Duration,
    /// `None` si le reranker est désactivé.
    pub reranker: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct Detail {
    pub passages: Vec<PassageDetail>,
    pub durees: Durees,
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
    /// Seconde connexion, pour lancer les mots en même temps que les voisins.
    store_mots: Mutex<Store>,
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
    ) -> Result<Self, StoreError> {
        let store_mots = Mutex::new(store.autre_connexion()?);
        Ok(Self {
            store: Mutex::new(store),
            store_mots,
            provider,
            reranker,
            dossiers,
        })
    }

    pub async fn chercher(
        &self,
        entree: &EntreeRecherche,
    ) -> Result<Vec<Resultat>, ErreurRecherche> {
        let detail = self.chercher_detail(entree).await?;
        Ok(detail.passages.into_iter().map(|p| p.resultat).collect())
    }

    /// Comme `chercher`, avec rangs, scores et durées (REQ-021).
    pub async fn chercher_detail(
        &self,
        entree: &EntreeRecherche,
    ) -> Result<Detail, ErreurRecherche> {
        let debut = Instant::now();
        let mut durees = Durees {
            total: Duration::ZERO,
            embedding: Duration::ZERO,
            reranker: None,
        };
        let nombre = valider(entree)?;
        let question = entree.question.trim();
        let vide = |s: &Store| -> Result<bool, StoreError> { Ok(s.dimension()?.is_none()) };
        if vide(&self.store.lock().unwrap())? {
            durees.total = debut.elapsed();
            return Ok(Detail {
                passages: Vec::new(),
                durees,
            });
        }
        let t = Instant::now();
        let vecteur = self
            .provider
            .embeddings(&[question.to_string()])
            .await?
            .pop()
            .ok_or_else(|| ErreurProvider::ReponseInvalide("aucun vecteur reçu".into()))?;
        durees.embedding = t.elapsed();

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
                let (v, m) = std::thread::scope(|sc| {
                    let h = sc.spawn(|| self.store_mots.lock().unwrap().mots(question, &filtres));
                    let v = store.voisins(&vecteur, &filtres);
                    (v, h.join().expect("recherche des mots interrompue"))
                });
                voisins.extend(v?);
                mots.extend(m?);
            }
            // Des dossiers imbriqués peuvent ramener deux fois le même passage.
            for liste in [&mut voisins, &mut mots] {
                liste.sort_by(|a, b| a.score.total_cmp(&b.score));
                let mut vus = std::collections::HashSet::new();
                liste.retain(|c| vus.insert(c.passage_id));
            }
            voisins.truncate(crate::store::NB_CANDIDATS);
            mots.truncate(crate::store::NB_CANDIDATS);
        }
        let rangs = |l: &[Candidat]| -> HashMap<i64, usize> {
            l.iter()
                .enumerate()
                .map(|(i, c)| (c.passage_id, i + 1))
                .collect()
        };
        let (rangs_sens, rangs_mots) = (rangs(&voisins), rangs(&mots));
        // (candidat, score de fusion, score du reranker)
        let mut fusion: Vec<(Candidat, f64, Option<f64>)> = fusionner(voisins, mots)
            .into_iter()
            .map(|(c, s)| (c, s, None))
            .collect();
        if fusion.is_empty() {
            durees.total = debut.elapsed();
            return Ok(Detail {
                passages: Vec::new(),
                durees,
            });
        }
        if let Some(r) = &self.reranker {
            let docs: Vec<String> = fusion.iter().map(|(c, _, _)| c.texte.clone()).collect();
            let t = Instant::now();
            let mut notes = r.rerank(question, &docs).await?;
            durees.reranker = Some(t.elapsed());
            notes.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut pris: Vec<Option<(Candidat, f64, Option<f64>)>> =
                fusion.into_iter().map(Some).collect();
            fusion = notes
                .into_iter()
                .filter_map(|(i, s)| pris[i].take().map(|(c, f, _)| (c, f, Some(s as f64))))
                .collect();
        }
        fusion.truncate(nombre);
        {
            let store = self.store.lock().unwrap();
            let mut m = HashMap::new();
            for (c, _, _) in &fusion {
                if !m.contains_key(&c.chemin) {
                    m.insert(c.chemin.clone(), store.tags_note(&c.chemin)?);
                }
            }
            tags_par_chemin = m;
        }
        let passages = fusion
            .into_iter()
            .map(|(c, f, rr)| PassageDetail {
                fusion: f,
                rang_sens: rangs_sens.get(&c.passage_id).copied(),
                rang_mots: rangs_mots.get(&c.passage_id).copied(),
                reranker: rr,
                resultat: Resultat {
                    tags: tags_par_chemin.get(&c.chemin).cloned().unwrap_or_default(),
                    texte: c.texte,
                    chemin: c.chemin,
                    titres: c.titres,
                    ligne_debut: c.ligne_debut,
                    ligne_fin: c.ligne_fin,
                    score: rr.unwrap_or(f),
                },
            })
            .collect();
        durees.total = debut.elapsed();
        Ok(Detail { passages, durees })
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
