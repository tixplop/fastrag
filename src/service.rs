//! Service : un seul processus qui indexe, répond à l'API HTTP et tient l'index (REQ-009, REQ-017, REQ-019).

use crate::api;
use crate::config::Config;
use crate::index::Indexeur;
use crate::provider::Provider;
use crate::search::{EntreeRecherche, ErreurRecherche, Recherche, Resultat};
use crate::status::{rfc3339, EtatIndex, Statut};
use crate::store::{ecrire_actif, lire_actif, supprimer_index, supprimer_orphelins, Store, Verrou};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};
use tokio::task::JoinHandle;

/// Réponse à une demande de réindexation complète.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reindexation {
    Lancee,
    DejaEnCours,
}

/// Recalcul en cours vers un nouveau modèle (REQ-011) : l'ancien index sert jusqu'à la bascule.
struct Migration {
    ancien_modele: String,
    nouveau_modele: String,
    config: Config,
}

pub struct Service {
    /// Index actif : les recherches lisent ce pointeur, remplacé d'un coup à la bascule.
    recherche: RwLock<Arc<Recherche>>,
    /// Connexion de lecture pour l'état, séparée de celle de l'indexeur.
    lecture: Mutex<Store>,
    migration: Mutex<Option<Migration>>,
    statut: Statut,
    en_cours: AtomicBool,
    derniere: Mutex<Option<String>>,
    indexeur: tokio::sync::Mutex<Indexeur>,
    reessai: Duration,
    tache: Mutex<Option<JoinHandle<()>>>,
}

/// Remet « en cours » à faux quand la tâche d'indexation se termine, même interrompue.
struct FinIndexation(Arc<Service>);
impl Drop for FinIndexation {
    fn drop(&mut self) {
        self.0.en_cours.store(false, Ordering::SeqCst);
    }
}

impl Service {
    /// Recherche partagée avec l'API (et le MCP).
    pub async fn chercher(
        &self,
        entree: &EntreeRecherche,
    ) -> Result<Vec<Resultat>, ErreurRecherche> {
        let recherche = self.recherche.read().unwrap().clone();
        recherche.chercher(entree).await
    }

    /// Contenu de l'état de l'index (REQ-017).
    pub fn etat(&self) -> Result<EtatIndex, crate::store::StoreError> {
        let (notes, passages) = self.lecture.lock().unwrap().compter()?;
        Ok(EtatIndex {
            notes,
            passages,
            derniere_mise_a_jour: self.derniere.lock().unwrap().clone(),
            en_cours: self.en_cours.load(Ordering::SeqCst),
            avertissements: self.statut.avertissements(),
        })
    }

    /// Lance une réindexation complète, sauf si une indexation est déjà en cours (REQ-009).
    pub fn reindexer(self: &Arc<Self>) -> Reindexation {
        if self.demarrer_indexation(true) {
            Reindexation::Lancee
        } else {
            Reindexation::DejaEnCours
        }
    }

    /// Fin du recalcul : le nouvel index devient l'actif en une opération, puis l'ancien fichier
    /// est supprimé (REQ-011). Sans effet s'il n'y a pas de recalcul.
    fn basculer(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut m = self.migration.lock().unwrap();
        let Some(mig) = m.as_ref() else {
            return Ok(());
        };
        let recherche = ouvrir_recherche(&mig.config, &mig.nouveau_modele)?;
        let lecture = Store::ouvrir(&mig.config.dossier_donnees, &mig.nouveau_modele)?;
        let donnees = &mig.config.dossier_donnees;
        ecrire_actif(donnees, &mig.nouveau_modele)?;
        *self.recherche.write().unwrap() = Arc::new(recherche);
        *self.lecture.lock().unwrap() = lecture;
        supprimer_index(donnees, &mig.ancien_modele)?;
        tracing::info!("nouvel index actif : modèle {}", mig.nouveau_modele);
        *m = None;
        Ok(())
    }

    /// Une seule tâche d'indexation à la fois. Rend `false` si l'une tourne déjà.
    pub(crate) fn demarrer_indexation(self: &Arc<Self>, complete: bool) -> bool {
        if self.en_cours.swap(true, Ordering::SeqCst) {
            return false;
        }
        let service = self.clone();
        let h = tokio::spawn(async move {
            let _fin = FinIndexation(service.clone());
            let mut idx = service.indexeur.lock().await;
            if complete {
                idx.demander_reindexation_complete();
            }
            loop {
                let resultat = idx.synchroniser().await;
                for a in service.statut.avertissements() {
                    tracing::warn!("{a}");
                }
                match resultat {
                    Ok(_) => {
                        *service.derniere.lock().unwrap() = Some(rfc3339(SystemTime::now()));
                    }
                    Err(e) => {
                        tracing::error!("indexation interrompue : {e}");
                        break;
                    }
                }
                if idx.en_attente().is_empty() {
                    if let Err(e) = service.basculer() {
                        tracing::error!("bascule vers le nouvel index impossible : {e}");
                    }
                    break;
                }
                tokio::time::sleep(service.reessai).await;
            }
        });
        *self.tache.lock().unwrap() = Some(h);
        true
    }
}

/// Service démarré : adresse d'écoute et verrou du dossier de données. L'arrêt (ou la
/// destruction) coupe le serveur et l'indexation et libère le verrou.
pub struct ServiceEnCours {
    pub adresse: SocketAddr,
    pub service: Arc<Service>,
    serveur: JoinHandle<()>,
    surveillance: JoinHandle<()>,
    _verrou: Verrou,
}

impl ServiceEnCours {
    /// Attend la fin du serveur (jamais, sauf erreur).
    pub async fn attendre(&mut self) {
        let _ = (&mut self.serveur).await;
    }
}

impl Drop for ServiceEnCours {
    fn drop(&mut self) {
        self.serveur.abort();
        self.surveillance.abort();
        if let Some(h) = self.service.tache.lock().unwrap().take() {
            h.abort();
        }
    }
}

/// Recherche sur l'index d'un modèle : connexion et provider propres.
fn ouvrir_recherche(
    config: &Config,
    modele: &str,
) -> Result<Recherche, Box<dyn std::error::Error>> {
    let cle = std::env::var(&config.embedding.cle_env).ok();
    let reranker = config.reranker.actif.then(|| {
        let cle = std::env::var(&config.reranker.cle_env).ok();
        Provider::new(&config.reranker.url, cle, &config.reranker.modele)
    });
    Ok(Recherche::new(
        Store::ouvrir(&config.dossier_donnees, modele)?,
        Provider::new(&config.embedding.url, cle, modele),
        reranker,
        config.dossiers.clone(),
    ))
}

/// Démarre le service sur `127.0.0.1` seulement : verrou, index, indexation de rattrapage, API.
pub async fn demarrer(config: &Config) -> Result<ServiceEnCours, Box<dyn std::error::Error>> {
    let verrou = Verrou::prendre(&config.dossier_donnees)?;
    let modele = &config.embedding.modele;
    let cle = std::env::var(&config.embedding.cle_env).ok();
    let provider = || Provider::new(&config.embedding.url, cle.clone(), modele);
    let donnees = config.dossier_donnees.clone();
    // Index actif : celui noté dans le dossier de données, sinon celui du modèle configuré.
    let actif = lire_actif(&donnees)?.unwrap_or_else(|| modele.clone());
    if actif == *modele {
        ecrire_actif(&donnees, modele)?;
    }
    // Restes d'une coupure ou d'un recalcul abandonné : ni l'index actif ni la cible.
    supprimer_orphelins(&donnees, &[&actif, modele])?;
    let indexeur = Indexeur::new(
        Store::ouvrir(&donnees, modele)?,
        provider(),
        config.dossiers.clone(),
    );
    let migration = (actif != *modele).then(|| Migration {
        ancien_modele: actif.clone(),
        nouveau_modele: modele.clone(),
        config: config.clone(),
    });
    // Pendant un recalcul, les recherches et l'état lisent l'ancien index.
    let service = Arc::new(Service {
        recherche: RwLock::new(Arc::new(ouvrir_recherche(config, &actif)?)),
        lecture: Mutex::new(Store::ouvrir(&donnees, &actif)?),
        migration: Mutex::new(migration),
        statut: indexeur.statut(),
        en_cours: AtomicBool::new(false),
        derniere: Mutex::new(None),
        indexeur: tokio::sync::Mutex::new(indexeur),
        reessai: Duration::from_secs(config.reessai_secondes),
        tache: Mutex::new(None),
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
        .await
        .map_err(|e| format!("impossible d'écouter sur 127.0.0.1:{} : {e}", config.port))?;
    let adresse = listener.local_addr()?;
    let app = api::routeur(service.clone())
        .nest_service("/mcp", crate::mcp::service_http(service.clone()));
    let serveur = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("serveur HTTP arrêté : {e}");
        }
    });
    let surveillance = crate::watch::lancer(service.clone(), config);
    service.demarrer_indexation(false);
    Ok(ServiceEnCours {
        adresse,
        service,
        serveur,
        surveillance,
        _verrou: verrou,
    })
}
