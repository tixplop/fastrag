//! Notes en attente quand le provider échoue : nouvel essai au délai configuré (REQ-007).

use super::{ErreurIndex, Indexeur, Rapport};
use std::time::Duration;

/// Synchronise, puis recommence tous les `delai` tant que des notes restent en attente.
/// Rend le rapport du dernier passage.
pub async fn synchroniser_avec_reprise(
    indexeur: &mut Indexeur,
    delai: Duration,
) -> Result<Rapport, ErreurIndex> {
    loop {
        let rapport = indexeur.synchroniser().await?;
        if indexeur.en_attente().is_empty() {
            return Ok(rapport);
        }
        tokio::time::sleep(delai).await;
    }
}
