//! Surveillance des dossiers (REQ-008, DEC-004) : inotify (`notify`) pour les dossiers Linux,
//! relecture périodique (taille et date) pour ceux sous `/mnt/`. Un changement relance le
//! rapprochement, qui compare les empreintes.

use crate::config::Config;
use crate::index::scan;
use crate::service::Service;
use notify::{RecursiveMode, Watcher};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::task::JoinHandle;

type Instantane = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

fn sous_mnt(d: &Path) -> bool {
    std::path::absolute(d).is_ok_and(|a| a.starts_with("/mnt"))
}

/// Taille et date de chaque note : premier filtre avant de comparer les empreintes.
fn instantane(dossiers: &[PathBuf]) -> Instantane {
    scan::parcourir(dossiers)
        .fichiers
        .into_iter()
        .map(|f| {
            let m = std::fs::metadata(&f).ok();
            let v = (
                m.as_ref().map_or(0, |m| m.len()),
                m.and_then(|m| m.modified().ok()),
            );
            (f, v)
        })
        .collect()
}

/// Lance la surveillance. La tâche rendue doit être arrêtée avec le service.
pub fn lancer(service: Arc<Service>, config: &Config) -> JoinHandle<()> {
    let (releves, evenements): (Vec<PathBuf>, Vec<PathBuf>) = config
        .dossiers
        .iter()
        .cloned()
        .partition(|d| config.forcer_releve || sous_mnt(d));
    let regroupement = Duration::from_millis(config.regroupement_ms);
    let periode = Duration::from_secs(config.releve_secondes.max(1));
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        // Le surveillant vit tant que la tâche vit.
        let mut _surveillant = None;
        if !evenements.is_empty() {
            let tx2 = tx.clone();
            let w = notify::recommended_watcher(move |e: notify::Result<notify::Event>| {
                if e.is_ok_and(|e| !e.kind.is_access()) {
                    let _ = tx2.send(());
                }
            });
            match w {
                Ok(mut w) => {
                    for d in &evenements {
                        if let Err(e) = w.watch(d, RecursiveMode::Recursive) {
                            tracing::warn!("{} : surveillance impossible ({e})", d.display());
                        }
                    }
                    _surveillant = Some(w);
                }
                Err(e) => tracing::warn!("surveillance des dossiers impossible ({e})"),
            }
        }
        let mut avant = if releves.is_empty() {
            Instantane::new()
        } else {
            let d = releves.clone();
            tokio::task::spawn_blocking(move || instantane(&d))
                .await
                .unwrap_or_default()
        };
        let mut releve = tokio::time::interval(periode);
        releve.tick().await;
        loop {
            tokio::select! {
                r = rx.recv() => {
                    if r.is_none() {
                        return;
                    }
                }
                _ = releve.tick(), if !releves.is_empty() => {
                    let d = releves.clone();
                    let Ok(apres) = tokio::task::spawn_blocking(move || instantane(&d)).await
                    else {
                        return;
                    };
                    if apres == avant {
                        continue;
                    }
                    avant = apres;
                }
            }
            // Regroupe les événements rapprochés en un seul passage.
            tokio::time::sleep(regroupement).await;
            while rx.try_recv().is_ok() {}
            // Une indexation en cours ne voit peut-être pas ce changement : on attend son tour.
            while !service.demarrer_indexation(false) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    })
}
