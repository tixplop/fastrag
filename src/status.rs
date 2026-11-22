//! État partagé de l'index, lisible pendant que l'indexeur travaille (REQ-005, REQ-007).

use std::sync::{Arc, Mutex};

/// Liste d'avertissements courante. L'indexeur la remplace à chaque passage : un avertissement
/// disparaît donc dès que le fichier est corrigé ou supprimé, ou que le provider répond.
#[derive(Clone, Default)]
pub struct Statut {
    avertissements: Arc<Mutex<Vec<String>>>,
}

impl Statut {
    pub fn avertissements(&self) -> Vec<String> {
        self.avertissements.lock().unwrap().clone()
    }

    pub(crate) fn publier(&self, avertissements: &[String]) {
        *self.avertissements.lock().unwrap() = avertissements.to_vec();
    }
}

/// Contenu de la réponse d'état, partagé par l'API et le MCP (REQ-017).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EtatIndex {
    pub notes: usize,
    pub passages: usize,
    /// Date RFC 3339 (UTC) de la fin du dernier passage d'indexation ; absente avant le premier.
    pub derniere_mise_a_jour: Option<String>,
    pub en_cours: bool,
    pub avertissements: Vec<String>,
}

/// Date RFC 3339 en UTC, à la seconde.
pub fn rfc3339(t: std::time::SystemTime) -> String {
    let s = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (jours, reste) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // Jour civil depuis le nombre de jours écoulés depuis 1970 (algorithme de H. Hinnant).
    let z = jours + 719_468;
    let ere = z.div_euclid(146_097);
    let jde = z.rem_euclid(146_097);
    let ae = (jde - jde / 1460 + jde / 36_524 - jde / 146_096) / 365;
    let doy = jde - (365 * ae + ae / 4 - ae / 100);
    let mp = (5 * doy + 2) / 153;
    let jour = doy - (153 * mp + 2) / 5 + 1;
    let mois = if mp < 10 { mp + 3 } else { mp - 9 };
    let annee = ae + ere * 400 + i64::from(mois <= 2);
    format!(
        "{annee:04}-{mois:02}-{jour:02}T{:02}:{:02}:{:02}Z",
        reste / 3600,
        reste % 3600 / 60,
        reste % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn rfc3339_dates_connues() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_secs(1_709_164_800 + 86_399)),
            "2024-02-29T23:59:59Z"
        );
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20Z"
        );
    }
}
