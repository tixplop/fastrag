//! Requêtes candidates : plus proches voisins et FTS5 (bm25), filtres dossier et tag (REQ-014).

use super::{vers_octets, Result, Store};
use rusqlite::{params_from_iter, types::Value};

/// Nombre de candidats renvoyés par chaque requête.
pub const NB_CANDIDATS: usize = 50;

#[derive(Debug, Default, Clone)]
pub struct Filtres {
    /// Préfixe de chemin absolu.
    pub dossier: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidat {
    pub passage_id: i64,
    pub chemin: String,
    pub titre: String,
    pub titres: String,
    pub texte: String,
    pub ligne_debut: u32,
    pub ligne_fin: u32,
    /// Distance cosinus (voisins) ou bm25 (plus petit = meilleur) pour FTS5.
    pub score: f64,
}

impl Filtres {
    /// Clauses SQL (sur les alias `n` = notes) et leurs paramètres.
    fn clauses(&self, params: &mut Vec<Value>) -> String {
        let mut s = String::new();
        if let Some(d) = &self.dossier {
            s.push_str(" AND substr(n.chemin, 1, length(?)) = ?");
            params.push(Value::Text(d.clone()));
            params.push(Value::Text(d.clone()));
        }
        if let Some(t) = &self.tag {
            s.push_str(
                " AND EXISTS (SELECT 1 FROM note_tags t WHERE t.note_id = n.id AND t.tag = ?)",
            );
            params.push(Value::Text(t.clone()));
        }
        s
    }
}

/// Transforme une question en requête FTS5 sûre : chaque mot entre guillemets, reliés par OR.
fn requete_fts(question: &str) -> Option<String> {
    let mots: Vec<String> = question
        .split_whitespace()
        .map(|m| format!("\"{}\"", m.replace('"', "\"\"")))
        .collect();
    if mots.is_empty() {
        None
    } else {
        Some(mots.join(" OR "))
    }
}

fn candidat(r: &rusqlite::Row) -> rusqlite::Result<Candidat> {
    Ok(Candidat {
        passage_id: r.get(0)?,
        chemin: r.get(1)?,
        titre: r.get(2)?,
        titres: r.get(3)?,
        texte: r.get(4)?,
        ligne_debut: r.get(5)?,
        ligne_fin: r.get(6)?,
        score: r.get(7)?,
    })
}

impl Store {
    /// Tags de la note (vide si elle est inconnue).
    pub fn tags_note(&self, chemin: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare(
            "SELECT t.tag FROM note_tags t JOIN notes n ON n.id = t.note_id
             WHERE n.chemin = ? ORDER BY t.rowid",
        )?;
        let v = st
            .query_map([chemin], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }

    /// Les 50 passages les plus proches du vecteur (distance cosinus croissante).
    pub fn voisins(&self, vecteur: &[f32], filtres: &Filtres) -> Result<Vec<Candidat>> {
        let mut p = vec![Value::Blob(vers_octets(vecteur))];
        let clauses = filtres.clauses(&mut p);
        // Les mêmes filtres servent dans la sous-requête puis en sortie.
        let filtre_params = p[1..].to_vec();
        p.extend(filtre_params);
        // On parcourt d'abord les vecteurs (lecture séquentielle), puis on retrouve les passages.
        let existe = if clauses.is_empty() {
            String::new()
        } else {
            format!(
                "WHERE EXISTS (SELECT 1 FROM passages p JOIN notes n ON n.id = p.note_id
                               WHERE p.empreinte = v.empreinte {clauses})"
            )
        };
        let sql = format!(
            "SELECT p.id, n.chemin, n.titre, p.titres, p.texte, p.ligne_debut, p.ligne_fin, c.d
             FROM (SELECT v.empreinte AS empreinte, vec_distance_cosine(v.vecteur, ?) AS d
                   FROM vecteurs v {existe}
                   ORDER BY d LIMIT {NB_CANDIDATS}) c
             JOIN passages p ON p.empreinte = c.empreinte
             JOIN notes n ON n.id = p.note_id
             WHERE 1=1 {clauses}
             ORDER BY c.d LIMIT {NB_CANDIDATS}"
        );
        let mut st = self.conn.prepare(&sql)?;
        let v = st
            .query_map(params_from_iter(p), candidat)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }

    /// Les 50 meilleurs passages par mots (bm25), sans tenir compte de la casse ni des accents.
    pub fn mots(&self, question: &str, filtres: &Filtres) -> Result<Vec<Candidat>> {
        let Some(q) = requete_fts(question) else {
            return Ok(Vec::new());
        };
        let mut p = vec![Value::Text(q)];
        let clauses = filtres.clauses(&mut p);
        let sql = format!(
            "SELECT p.id, n.chemin, n.titre, p.titres, p.texte, p.ligne_debut, p.ligne_fin,
                    bm25(passages_fts) AS s
             FROM passages_fts
             JOIN passages p ON p.id = passages_fts.rowid
             JOIN notes n ON n.id = p.note_id
             WHERE passages_fts MATCH ? {clauses}
             ORDER BY s LIMIT {NB_CANDIDATS}"
        );
        let mut st = self.conn.prepare(&sql)?;
        let v = st
            .query_map(params_from_iter(p), candidat)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Note, Passage};

    fn passage(texte: &str, emp: &str) -> (Passage, Option<Vec<f32>>) {
        let p = Passage {
            texte: texte.into(),
            titres: "T".into(),
            ligne_debut: 1,
            ligne_fin: 2,
            empreinte: emp.into(),
        };
        (p, Some(vec![1.0, 0.0]))
    }

    #[test]
    fn req014_ac2_mots_sans_casse_ni_accents() {
        let d = std::path::PathBuf::from(std::env::var_os("TMPDIR").unwrap_or("/tmp".into()))
            .join(format!("fts_unit_{}", std::process::id()));
        let mut s = Store::ouvrir(&d, "m").unwrap();
        let note = Note {
            chemin: "/r.md".into(),
            empreinte: "e".into(),
            titre: "T".into(),
            tags: vec![],
        };
        s.remplacer_note(
            &note,
            &[
                passage("Une Pâte brisée maison", "1"),
                passage("Autre sujet ERR_4021", "2"),
            ],
        )
        .unwrap();
        let c = s.mots("pate brisee", &Filtres::default()).unwrap();
        assert_eq!(c.len(), 1);
        assert!(c[0].texte.contains("Pâte brisée"));
        assert_eq!(s.mots("err_4021", &Filtres::default()).unwrap().len(), 1);
        assert!(s.mots("\"", &Filtres::default()).is_ok());
    }
}
