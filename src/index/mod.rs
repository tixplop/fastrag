//! Indexation : rapprochement des dossiers avec l'index (REQ-002, REQ-006).

pub mod pending;
pub mod scan;

use crate::chunk::{decouper_fichier, TAILLE_FICHIER_MAX};
use crate::provider::{ErreurProvider, Provider};
use crate::status::Statut;
use crate::store::{Note, Passage, Store, StoreError};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Nombre de notes envoyées au provider puis écrites en une fois.
const TAILLE_GROUPE: usize = 200;

#[derive(Debug)]
pub enum ErreurIndex {
    /// Une indexation est déjà en cours.
    DejaEnCours,
    Store(StoreError),
    Provider(ErreurProvider),
}

impl std::fmt::Display for ErreurIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DejaEnCours => write!(f, "une indexation est déjà en cours"),
            Self::Store(e) => write!(f, "{e}"),
            Self::Provider(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErreurIndex {}

impl From<StoreError> for ErreurIndex {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}
impl From<ErreurProvider> for ErreurIndex {
    fn from(e: ErreurProvider) -> Self {
        Self::Provider(e)
    }
}

/// Bilan d'un passage de l'indexeur.
#[derive(Debug, Default, PartialEq)]
pub struct Rapport {
    pub nouvelles: usize,
    pub modifiees: usize,
    pub supprimees: usize,
    pub renommees: usize,
    pub inchangees: usize,
}

pub struct Indexeur {
    store: Store,
    provider: Provider,
    dossiers: Vec<PathBuf>,
    en_cours: Arc<AtomicBool>,
    avertissements: Vec<String>,
    en_attente: Vec<String>,
    statut: Statut,
    /// Prochain passage : retraiter même les notes inchangées (REQ-009).
    tout_retraiter: bool,
}

/// Remet l'état « en cours » à faux à la sortie, même en cas d'erreur.
struct Garde(Arc<AtomicBool>);
impl Drop for Garde {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Indexeur {
    pub fn new(store: Store, provider: Provider, dossiers: Vec<PathBuf>) -> Self {
        Self {
            store,
            provider,
            dossiers,
            en_cours: Arc::new(AtomicBool::new(false)),
            avertissements: Vec::new(),
            en_attente: Vec::new(),
            statut: Statut::default(),
            tout_retraiter: false,
        }
    }

    /// Indicateur partagé « indexation en cours ».
    pub fn indicateur_en_cours(&self) -> Arc<AtomicBool> {
        self.en_cours.clone()
    }

    /// Avertissements du dernier passage.
    pub fn avertissements(&self) -> &[String] {
        &self.avertissements
    }

    /// Notes dont le traitement attend le retour du provider.
    pub fn en_attente(&self) -> &[String] {
        &self.en_attente
    }

    /// État partagé : avertissements courants, lisibles pendant l'indexation.
    pub fn statut(&self) -> Statut {
        self.statut.clone()
    }

    /// Le prochain passage relit et redécoupe toutes les notes ; les vecteurs connus sont réutilisés.
    pub fn demander_reindexation_complete(&mut self) {
        self.tout_retraiter = true;
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Rapproche l'index des dossiers : notes nouvelles, modifiées, supprimées, renommées.
    pub async fn synchroniser(&mut self) -> Result<Rapport, ErreurIndex> {
        let r = self.passer().await;
        self.statut.publier(&self.avertissements);
        r
    }

    async fn passer(&mut self) -> Result<Rapport, ErreurIndex> {
        if self.en_cours.swap(true, Ordering::SeqCst) {
            return Err(ErreurIndex::DejaEnCours);
        }
        let _garde = Garde(self.en_cours.clone());
        let tout = std::mem::take(&mut self.tout_retraiter);
        let parcours = scan::parcourir(&self.dossiers);
        self.avertissements = parcours.avertissements;
        self.en_attente.clear();
        let mut rapport = Rapport::default();

        let presents: HashSet<String> = parcours
            .fichiers
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        // Notes indexées dont le fichier a disparu, par empreinte de contenu.
        let mut disparues: HashMap<String, Vec<String>> = HashMap::new();
        let mut connues = HashSet::new();
        for chemin in self.store.chemins_notes()? {
            if presents.contains(&chemin) {
                connues.insert(chemin);
            } else if let Some(e) = self.store.empreinte_note(&chemin)? {
                disparues.entry(e).or_default().push(chemin);
            }
        }

        // Fichiers à retraiter : (chemin, empreinte du contenu).
        let mut a_traiter = Vec::new();
        for fichier in &parcours.fichiers {
            let chemin = fichier.display().to_string();
            let taille = std::fs::metadata(fichier).map(|m| m.len()).unwrap_or(0);
            let empreinte = if taille > TAILLE_FICHIER_MAX {
                String::new()
            } else {
                match std::fs::read(fichier) {
                    Ok(o) => blake3::hash(&o).to_hex().to_string(),
                    Err(e) => {
                        self.avertissements.push(format!(
                            "{chemin} : fichier ignoré, lecture impossible ({e})"
                        ));
                        continue;
                    }
                }
            };
            if connues.contains(&chemin) {
                if !tout
                    && self.store.empreinte_note(&chemin)?.as_deref() == Some(empreinte.as_str())
                {
                    rapport.inchangees += 1;
                } else {
                    a_traiter.push((chemin, empreinte, false));
                }
            } else if let Some(ancien) = disparues.get_mut(&empreinte).and_then(|v| v.pop()) {
                self.store.renommer_note(&ancien, &chemin)?;
                rapport.renommees += 1;
            } else {
                a_traiter.push((chemin, empreinte, true));
            }
        }
        for chemin in disparues.into_values().flatten() {
            self.store.supprimer_note(&chemin)?;
            rapport.supprimees += 1;
        }

        // Découpage, puis un seul envoi des textes dont le vecteur est inconnu.
        let mut notes = Vec::new();
        for (chemin, empreinte, nouvelle) in a_traiter {
            match decouper_fichier(std::path::Path::new(&chemin)) {
                Ok(d) => {
                    for a in &d.avertissements {
                        self.avertissements.push(a.clone());
                    }
                    let note = Note {
                        chemin,
                        empreinte,
                        titre: d.titre,
                        tags: d.tags,
                    };
                    notes.push((note, d.passages, nouvelle));
                }
                Err(refus) => self.avertissements.push(refus.to_string()),
            }
        }
        // Par groupes de notes : la mémoire reste bornée et l'index se remplit au fil de l'eau.
        let mut restantes = notes;
        while !restantes.is_empty() {
            let reste = restantes.split_off(restantes.len().min(TAILLE_GROUPE));
            let lot = std::mem::replace(&mut restantes, reste);
            let empreintes: Vec<String> = lot
                .iter()
                .flat_map(|(_, ps, _)| ps.iter().map(|p| p.empreinte.clone()))
                .collect();
            let connus: HashSet<String> = self
                .store
                .vecteurs_connus(&empreintes)?
                .into_iter()
                .collect();
            let mut inconnus: Vec<&Passage> = Vec::new();
            let mut vus = HashSet::new();
            for (_, ps, _) in &lot {
                for p in ps {
                    if !connus.contains(&p.empreinte) && vus.insert(p.empreinte.clone()) {
                        inconnus.push(p);
                    }
                }
            }
            let textes: Vec<String> = inconnus.iter().map(|p| p.texte.clone()).collect();
            let mut vecteurs: HashMap<String, Vec<f32>> = HashMap::new();
            if !textes.is_empty() {
                let v = match self.provider.embeddings(&textes).await {
                    Ok(v) => v,
                    Err(e) => {
                        // Les notes restent en attente ; leur ancienne version, s'il y en a une,
                        // reste dans l'index.
                        for (note, _, _) in lot.iter().chain(restantes.iter()) {
                            self.avertissements.push(format!(
                                "{} : en attente, nouvel essai plus tard ({e})",
                                note.chemin
                            ));
                            self.en_attente.push(note.chemin.clone());
                        }
                        return Ok(rapport);
                    }
                };
                if v.len() != textes.len() {
                    return Err(ErreurProvider::ReponseInvalide(
                        "nombre de vecteurs différent du nombre de textes".into(),
                    )
                    .into());
                }
                for (p, v) in inconnus.iter().zip(v) {
                    vecteurs.insert(p.empreinte.clone(), v);
                }
            }
            for (note, passages, nouvelle) in lot {
                let avec: Vec<(Passage, Option<Vec<f32>>)> = passages
                    .into_iter()
                    .map(|p| {
                        let v = vecteurs.get(&p.empreinte).cloned();
                        (p, v)
                    })
                    .collect();
                self.store.remplacer_note(&note, &avec)?;
                if nouvelle {
                    rapport.nouvelles += 1;
                } else {
                    rapport.modifiees += 1;
                }
            }
        }
        Ok(rapport)
    }
}
