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
