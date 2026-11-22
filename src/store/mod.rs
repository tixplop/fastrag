//! Stockage SQLite d'un index (un fichier par modèle d'embedding, DEC-008) : notes, passages,
//! vecteurs, FTS5, mode WAL, verrou exclusif sur le dossier de données (REQ-010, REQ-014).

mod query;

pub use query::{Candidat, Filtres, NB_CANDIDATS};

use rusqlite::{params, Connection, OptionalExtension};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Once;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// Un autre processus utilise déjà ce dossier de données.
    Verrouille(PathBuf),
    /// Vecteur dont la dimension diffère de celle déjà enregistrée.
    Dimension {
        attendue: usize,
        recue: usize,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "erreur de l'index : {e}"),
            Self::Io(e) => write!(f, "erreur de fichier : {e}"),
            Self::Verrouille(d) => write!(
                f,
                "le dossier de données {} est déjà utilisé par un autre fastrag",
                d.display()
            ),
            Self::Dimension { attendue, recue } => write!(
                f,
                "dimension de vecteur invalide : {recue} reçu, {attendue} attendu"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Verrou exclusif sur le dossier de données ; libéré à la destruction.
#[derive(Debug)]
pub struct Verrou {
    _fichier: File,
}

impl Verrou {
    pub fn prendre(dossier: &Path) -> Result<Self> {
        use fs2::FileExt;
        std::fs::create_dir_all(dossier)?;
        let fichier = File::create(dossier.join("fastrag.lock"))?;
        fichier
            .try_lock_exclusive()
            .map_err(|_| StoreError::Verrouille(dossier.to_path_buf()))?;
        Ok(Self { _fichier: fichier })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub chemin: String,
    pub empreinte: String,
    pub titre: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    pub texte: String,
    /// Chemin de titres, par exemple « Recettes > Tartes ».
    pub titres: String,
    pub ligne_debut: u32,
    pub ligne_fin: u32,
    pub empreinte: String,
}

pub struct Store {
    conn: Connection,
}

fn enregistrer_sqlite_vec() {
    static UNE_FOIS: Once = Once::new();
    UNE_FOIS.call_once(|| unsafe {
        // Schéma documenté de sqlite-vec : enregistrement de l'extension pour toute connexion.
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    });
}

pub(crate) fn vers_octets(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Nom de fichier d'index pour un modèle (caractères sûrs seulement).
pub fn nom_fichier_index(modele: &str) -> String {
    let sur: String = modele
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("index-{sur}.db")
}

impl Store {
    /// Ouvre (ou crée) l'index du modèle dans `dossier`.
    pub fn ouvrir(dossier: &Path, modele: &str) -> Result<Self> {
        std::fs::create_dir_all(dossier)?;
        Self::ouvrir_fichier(&dossier.join(nom_fichier_index(modele)), modele)
    }

    pub fn ouvrir_fichier(fichier: &Path, modele: &str) -> Result<Self> {
        enregistrer_sqlite_vec();
        let conn = Connection::open(fichier)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        // Pages assez grandes pour garder un vecteur dans une seule page (sans effet sur un index existant).
        conn.pragma_update(None, "page_size", 16384)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(include_str!("schema.sql"))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(cle, valeur) VALUES ('modele', ?1)",
            [modele],
        )?;
        Ok(Self { conn })
    }

    pub fn modele(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT valeur FROM meta WHERE cle='modele'", [], |r| {
                r.get(0)
            })?)
    }

    /// Dimension des vecteurs, enregistrée à la première réponse du provider.
    pub fn dimension(&self) -> Result<Option<usize>> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT valeur FROM meta WHERE cle='dimension'", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()))
    }

    /// Remplace une note et tous ses passages en une seule transaction. Chaque passage peut
    /// porter son vecteur ; sans vecteur, celui déjà connu pour la même empreinte reste valable.
    pub fn remplacer_note(
        &mut self,
        note: &Note,
        passages: &[(Passage, Option<Vec<f32>>)],
    ) -> Result<()> {
        let mut dim = self.dimension()?;
        for (_, v) in passages {
            if let Some(v) = v {
                match dim {
                    Some(d) if d != v.len() => {
                        return Err(StoreError::Dimension {
                            attendue: d,
                            recue: v.len(),
                        })
                    }
                    _ => dim = Some(v.len()),
                }
            }
        }
        let tx = self.conn.transaction()?;
        supprimer_dans(&tx, &note.chemin)?;
        if let Some(d) = dim {
            tx.execute(
                "INSERT OR IGNORE INTO meta(cle, valeur) VALUES ('dimension', ?1)",
                [d.to_string()],
            )?;
        }
        tx.execute(
            "INSERT INTO notes(chemin, empreinte, titre) VALUES (?1, ?2, ?3)",
            params![note.chemin, note.empreinte, note.titre],
        )?;
        let note_id = tx.last_insert_rowid();
        for tag in &note.tags {
            tx.execute(
                "INSERT INTO note_tags(note_id, tag) VALUES (?1, ?2)",
                params![note_id, tag],
            )?;
        }
        for (ordre, (p, v)) in passages.iter().enumerate() {
            tx.execute(
                "INSERT INTO passages(note_id, ordre, texte, titres, ligne_debut, ligne_fin, empreinte)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![note_id, ordre as i64, p.texte, p.titres, p.ligne_debut, p.ligne_fin, p.empreinte],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO passages_fts(rowid, texte) VALUES (?1, ?2)",
                params![id, p.texte],
            )?;
            if let Some(v) = v {
                tx.execute(
                    "INSERT OR REPLACE INTO vecteurs(empreinte, vecteur) VALUES (?1, ?2)",
                    params![p.empreinte, vers_octets(v)],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Supprime la note et ses passages ; renvoie `false` si elle n'existait pas.
    pub fn supprimer_note(&mut self, chemin: &str) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let existait = supprimer_dans(&tx, chemin)?;
        tx.commit()?;
        Ok(existait)
    }

    /// Change le chemin d'une note sans toucher à ses passages ni à ses vecteurs.
    pub fn renommer_note(&mut self, ancien: &str, nouveau: &str) -> Result<bool> {
        let tx = self.conn.transaction()?;
        if nouveau != ancien {
            supprimer_dans(&tx, nouveau)?;
        }
        let n = tx.execute(
            "UPDATE notes SET chemin = ?2 WHERE chemin = ?1",
            params![ancien, nouveau],
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// Empreinte du contenu de la note indexée, si elle l'est.
    pub fn empreinte_note(&self, chemin: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT empreinte FROM notes WHERE chemin = ?1",
                [chemin],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Nombre de notes et de passages indexés.
    pub fn compter(&self) -> Result<(usize, usize)> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?;
        let p: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM passages", [], |r| r.get(0))?;
        Ok((n as usize, p as usize))
    }

    pub fn chemins_notes(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT chemin FROM notes ORDER BY chemin")?;
        let v = st
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }

    /// Parmi `empreintes`, celles dont le vecteur est déjà connu.
    pub fn vecteurs_connus(&self, empreintes: &[String]) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT 1 FROM vecteurs WHERE empreinte = ?1")?;
        let mut connus = Vec::new();
        for e in empreintes {
            if st.exists([e])? {
                connus.push(e.clone());
            }
        }
        Ok(connus)
    }

    /// Passages d'une note, dans l'ordre du fichier.
    pub fn passages_note(&self, chemin: &str) -> Result<Vec<Passage>> {
        let mut st = self.conn.prepare(
            "SELECT p.texte, p.titres, p.ligne_debut, p.ligne_fin, p.empreinte
             FROM passages p JOIN notes n ON n.id = p.note_id
             WHERE n.chemin = ?1 ORDER BY p.ordre",
        )?;
        let v = st
            .query_map([chemin], |r| {
                Ok(Passage {
                    texte: r.get(0)?,
                    titres: r.get(1)?,
                    ligne_debut: r.get(2)?,
                    ligne_fin: r.get(3)?,
                    empreinte: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }
}

fn supprimer_dans(tx: &rusqlite::Transaction, chemin: &str) -> Result<bool> {
    let id: Option<i64> = tx
        .query_row("SELECT id FROM notes WHERE chemin = ?1", [chemin], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(id) = id else { return Ok(false) };
    tx.execute(
        "DELETE FROM passages_fts WHERE rowid IN (SELECT id FROM passages WHERE note_id = ?1)",
        [id],
    )?;
    tx.execute("DELETE FROM notes WHERE id = ?1", [id])?;
    Ok(true)
}
