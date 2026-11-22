//! Lecture de la configuration TOML (REQ-001).

use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Embedding {
    pub url: String,
    pub cle_env: String,
    pub modele: String,
}

impl Default for Embedding {
    fn default() -> Self {
        Self {
            url: "https://api.openai.com/v1".into(),
            cle_env: "FASTRAG_CLE_EMBEDDING".into(),
            modele: "bge-m3".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reranker {
    pub actif: bool,
    pub url: String,
    pub cle_env: String,
    pub modele: String,
}

impl Default for Reranker {
    fn default() -> Self {
        Self {
            actif: false,
            url: "https://api.openai.com/v1".into(),
            cle_env: "FASTRAG_CLE_RERANKER".into(),
            modele: "bge-reranker-v2-m3".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub dossiers: Vec<PathBuf>,
    pub embedding: Embedding,
    pub reranker: Reranker,
    pub port: u16,
    pub dossier_donnees: PathBuf,
    /// Délai avant un nouvel essai quand le provider d'embedding échoue.
    pub reessai_secondes: u64,
    /// Période de la relecture des dossiers sous `/mnt/` (REQ-008).
    pub releve_secondes: u64,
    /// Délai de regroupement des événements de surveillance, en millisecondes (REQ-008).
    pub regroupement_ms: u64,
    /// Relire tous les dossiers périodiquement au lieu d'utiliser inotify (REQ-008).
    pub forcer_releve: bool,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dossiers: Vec::new(),
            embedding: Embedding::default(),
            reranker: Reranker::default(),
            port: 7878,
            dossier_donnees: home().join(".local/share/fastrag"),
            reessai_secondes: 30,
            releve_secondes: 5,
            regroupement_ms: 300,
            forcer_releve: false,
        }
    }
}

/// Erreur de configuration : nomme toujours le fichier, et le champ quand il est connu.
#[derive(Debug)]
pub struct ErreurConfig {
    pub fichier: PathBuf,
    pub detail: String,
}

impl fmt::Display for ErreurConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Configuration invalide ({}) : {}",
            self.fichier.display(),
            self.detail
        )
    }
}

impl std::error::Error for ErreurConfig {}

impl Config {
    /// Chemin par défaut : `~/.config/fastrag/config.toml`.
    pub fn chemin_par_defaut() -> PathBuf {
        home().join(".config/fastrag/config.toml")
    }

    /// Charge le fichier donné, ou celui par défaut. Le fichier par défaut peut être absent
    /// (valeurs par défaut) ; un fichier donné explicitement doit exister.
    pub fn charger(chemin: Option<&Path>) -> Result<Config, ErreurConfig> {
        let (fichier, explicite) = match chemin {
            Some(p) => (p.to_path_buf(), true),
            None => (Self::chemin_par_defaut(), false),
        };
        let texte = match std::fs::read_to_string(&fichier) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicite => {
                return Ok(Config::default())
            }
            Err(e) => {
                return Err(ErreurConfig {
                    fichier,
                    detail: format!("fichier illisible ({e})"),
                })
            }
        };
        Self::depuis_texte(&texte, &fichier)
    }

    pub fn depuis_texte(texte: &str, fichier: &Path) -> Result<Config, ErreurConfig> {
        let config: Config = toml::from_str(texte).map_err(|e| ErreurConfig {
            fichier: fichier.to_path_buf(),
            detail: e.to_string(),
        })?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(nom: &str) -> PathBuf {
        let d = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let d = d.join(format!("config_{nom}_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn req001_ac1_champs_lus() {
        let d = tmp("champs");
        let f = d.join("c.toml");
        std::fs::write(
            &f,
            r#"
dossiers = ["/a", "/b"]
port = 9000
dossier_donnees = "/donnees"
[embedding]
url = "http://x/v1"
cle_env = "MA_CLE"
modele = "m1"
[reranker]
actif = true
url = "http://r/v1"
cle_env = "CLE_R"
modele = "m2"
"#,
        )
        .unwrap();
        let c = Config::charger(Some(&f)).unwrap();
        assert_eq!(c.dossiers, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(c.port, 9000);
        assert_eq!(c.dossier_donnees, PathBuf::from("/donnees"));
        assert_eq!(c.embedding.url, "http://x/v1");
        assert_eq!(c.embedding.cle_env, "MA_CLE");
        assert_eq!(c.embedding.modele, "m1");
        assert!(c.reranker.actif);
        assert_eq!(c.reranker.url, "http://r/v1");
        assert_eq!(c.reranker.cle_env, "CLE_R");
        assert_eq!(c.reranker.modele, "m2");
    }

    #[test]
    fn req001_ac1_champs_absents_valeurs_par_defaut() {
        let c = Config::depuis_texte("port = 8000\n", Path::new("c.toml")).unwrap();
        assert_eq!(c.port, 8000);
        assert!(c.dossiers.is_empty());
        assert_eq!(c.embedding.modele, "bge-m3");
        assert_eq!(c.reranker.modele, "bge-reranker-v2-m3");
        assert!(!c.reranker.actif);
        assert!(c.dossier_donnees.ends_with(".local/share/fastrag"));
        let vide = Config::depuis_texte("", Path::new("c.toml")).unwrap();
        assert_eq!(vide.port, 7878);
        assert_eq!(vide, Config::default());
    }

    #[test]
    fn req001_ac2_valeur_invalide_nomme_fichier_et_champ() {
        let d = tmp("invalide");
        let f = d.join("mauvais.toml");
        std::fs::write(&f, "[embedding]\nmodele = 12\n").unwrap();
        let msg = Config::charger(Some(&f)).unwrap_err().to_string();
        assert!(msg.contains("mauvais.toml"), "{msg}");
        assert!(msg.contains("modele"), "{msg}");
    }

    #[test]
    fn req001_ac2_champ_inconnu_et_toml_casse() {
        let d = tmp("inconnu");
        let f = d.join("c.toml");
        std::fs::write(&f, "portt = 1\n").unwrap();
        let msg = Config::charger(Some(&f)).unwrap_err().to_string();
        assert!(msg.contains("c.toml") && msg.contains("portt"), "{msg}");
        std::fs::write(&f, "port = = 1\n").unwrap();
        let msg = Config::charger(Some(&f)).unwrap_err().to_string();
        assert!(msg.contains("c.toml") && msg.contains("port"), "{msg}");
    }

    #[test]
    fn req001_ac2_fichier_illisible() {
        let d = tmp("absent");
        let f = d.join("absent.toml");
        let msg = Config::charger(Some(&f)).unwrap_err().to_string();
        assert!(msg.contains("absent.toml"), "{msg}");
    }

    #[test]
    fn exemple_documente_est_valide() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
        let c = Config::charger(Some(&f)).unwrap();
        assert_eq!(c.port, 7878);
        assert_eq!(c.dossiers.len(), 2);
        assert_eq!(c.embedding.modele, "bge-m3");
    }
}
