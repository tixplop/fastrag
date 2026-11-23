//! `fastrag update` : remplace le binaire lancé par la dernière release publiée (REQ-024).

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Fichier de la release pour Linux x86_64.
pub const NOM_BINAIRE: &str = "fastrag-linux-x86_64";
const URL_PAR_DEFAUT: &str = "https://api.github.com/repos/tixplop/fastrag/releases/latest";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Versions « a.b.c » en nombres ; `None` si ce n'est pas le cas.
fn analyser(version: &str) -> Option<Vec<u64>> {
    version.split('.').map(|p| p.parse().ok()).collect()
}

/// Vrai si `candidate` est strictement plus récente que `actuelle`, nombre par nombre.
pub fn plus_recente(candidate: &str, actuelle: &str) -> Option<bool> {
    let (mut c, mut a) = (analyser(candidate)?, analyser(actuelle)?);
    let n = c.len().max(a.len());
    c.resize(n, 0);
    a.resize(n, 0);
    Some(c > a)
}

/// Fichier temporaire supprimé à la destruction tant qu'il n'a pas été renommé.
struct Temporaire(PathBuf);

impl Drop for Temporaire {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn echec(cause: impl std::fmt::Display) -> u8 {
    eprintln!("{cause}. fastrag n'a pas été modifié.");
    1
}

/// Exécute la commande ; rend le code de sortie.
pub async fn lancer() -> u8 {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        eprintln!(
            "Aucun binaire publié pour ce système : compilez fastrag depuis les sources (voir le README)."
        );
        return 1;
    }
    match mettre_a_jour().await {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(cause) => echec(cause),
    }
}

async fn mettre_a_jour() -> Result<String, String> {
    let url = std::env::var("FASTRAG_RELEASES_URL").unwrap_or_else(|_| URL_PAR_DEFAUT.into());
    let client = reqwest::Client::builder()
        .user_agent(format!("fastrag/{VERSION}"))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Client HTTP impossible à créer ({e})"))?;
    let reponse = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Impossible de joindre le serveur des releases ({e})"))?;
    if !reponse.status().is_success() {
        return Err(format!(
            "Le serveur des releases a répondu {}",
            reponse.status()
        ));
    }
    let json: serde_json::Value = reponse
        .json()
        .await
        .map_err(|_| "Réponse du serveur des releases illisible".to_string())?;
    let tag = json["tag_name"]
        .as_str()
        .ok_or("Réponse du serveur des releases illisible")?;
    let nouvelle = tag.strip_prefix('v').unwrap_or(tag).to_string();
    let recente = plus_recente(&nouvelle, VERSION)
        .ok_or_else(|| format!("Version de la release illisible ({tag})"))?;
    if !recente {
        return Ok(format!("fastrag {VERSION} est déjà à jour."));
    }
    let lien = json["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"] == NOM_BINAIRE)
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| format!("Le fichier {NOM_BINAIRE} est absent de la release {tag}"))?;
    let octets = client
        .get(lien)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Téléchargement impossible ({e})"))?
        .bytes()
        .await
        .map_err(|e| format!("Téléchargement interrompu ({e})"))?;

    let exe =
        std::env::current_exe().map_err(|e| format!("Chemin du binaire introuvable ({e})"))?;
    remplacer(&exe, &octets, &nouvelle)?;
    Ok(format!(
        "fastrag mis à jour : {VERSION} → {nouvelle}. Redémarrez le service pour utiliser la nouvelle version."
    ))
}

#[cfg(unix)]
fn remplacer(exe: &Path, octets: &[u8], nouvelle: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let dossier = exe.parent().ok_or("Dossier du binaire introuvable")?;
    let impossible = |e: std::io::Error| format!("Fichier impossible à remplacer ({e})");
    let temp = Temporaire(dossier.join(format!(".fastrag-update-{}", std::process::id())));
    std::fs::write(&temp.0, octets).map_err(impossible)?;
    std::fs::set_permissions(&temp.0, std::fs::Permissions::from_mode(0o755))
        .map_err(impossible)?;
    let sortie = std::process::Command::new(&temp.0)
        .arg("--version")
        .output()
        .map_err(|e| format!("Le binaire téléchargé ne se lance pas ({e})"))?;
    let attendu = format!("fastrag {nouvelle}");
    if String::from_utf8_lossy(&sortie.stdout).trim() != attendu {
        return Err(format!(
            "Le binaire téléchargé ne répond pas « {attendu} » à --version"
        ));
    }
    std::fs::rename(&temp.0, exe).map_err(impossible)?;
    std::mem::forget(temp);
    Ok(())
}

#[cfg(not(unix))]
fn remplacer(_: &Path, _: &[u8], _: &str) -> Result<(), String> {
    Err("Système non pris en charge".into())
}

#[cfg(test)]
mod tests {
    use super::plus_recente;

    #[test]
    fn req024_ac3_versions_comparees_nombre_par_nombre() {
        assert_eq!(plus_recente("0.10.0", "0.9.0"), Some(true));
        assert_eq!(plus_recente("0.9.0", "0.10.0"), Some(false));
        assert_eq!(plus_recente("1.2.3", "1.2.3"), Some(false));
        assert_eq!(plus_recente("1.2", "1.2.0"), Some(false));
        assert_eq!(plus_recente("1.2.1", "1.2"), Some(true));
        assert_eq!(plus_recente("abc", "1.0.0"), None);
    }
}
