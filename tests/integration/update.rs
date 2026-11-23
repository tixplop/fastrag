use crate::common::dossier_test;
use crate::common::mock_releases::{release, script, MockReleases};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASSET: &str = "fastrag-linux-x86_64";

/// Copie du binaire dans un dossier à soi (pas de lien : `current_exe` le suivrait).
fn copie(nom: &str) -> PathBuf {
    let d = dossier_test(nom);
    let exe = d.join("fastrag");
    std::fs::copy(env!("CARGO_BIN_EXE_fastrag"), &exe).unwrap();
    exe
}

async fn lancer(exe: &Path, args: &[&str], url: Option<&str>, home: Option<&Path>) -> Output {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    if let Some(u) = url {
        cmd.env("FASTRAG_RELEASES_URL", u);
    }
    if let Some(h) = home {
        cmd.env("HOME", h);
    }
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

fn texte(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn seul_le_binaire(exe: &Path) {
    let noms: Vec<_> = std::fs::read_dir(exe.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(noms, vec![std::ffi::OsString::from("fastrag")]);
}

#[test]
fn req024_ac1_version() {
    let sortie = Command::new(env!("CARGO_BIN_EXE_fastrag"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(sortie.status.code(), Some(0));
    assert_eq!(texte(&sortie.stdout), format!("fastrag {VERSION}\n"));
}

#[test]
fn req024_ac6_argument_inconnu() {
    let sortie = Command::new(env!("CARGO_BIN_EXE_fastrag"))
        .arg("bidule")
        .output()
        .unwrap();
    assert_eq!(sortie.status.code(), Some(2));
    assert!(texte(&sortie.stderr)
        .contains("Usage : fastrag [--config <fichier>] | fastrag update | fastrag --version"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req024_ac2_ac5_mise_a_jour_sans_config() {
    let mock = MockReleases::demarrer(&release("v99.0.0", ASSET), &script("99.0.0")).await;
    let exe = copie("upd_ok");
    // Configuration invalide à l'emplacement par défaut : elle ne doit pas être lue.
    let home = dossier_test("upd_ok_home");
    let d = home.join(".config/fastrag");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("config.toml"), "ceci = n'est pas [du toml").unwrap();
    let sortie = lancer(&exe, &["update"], Some(&mock.url), Some(&home)).await;
    assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
    assert_eq!(
        texte(&sortie.stdout).trim(),
        format!("fastrag mis à jour : {VERSION} → 99.0.0. Redémarrez le service pour utiliser la nouvelle version.")
    );
    let v = lancer(&exe, &["--version"], None, None).await;
    assert_eq!(texte(&v.stdout), "fastrag 99.0.0\n");
    assert_eq!(
        std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
        0o755
    );
    seul_le_binaire(&exe);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req024_ac3_deja_a_jour() {
    let avant = std::fs::read(env!("CARGO_BIN_EXE_fastrag")).unwrap();
    for tag in [format!("v{VERSION}"), "v0.0.1".to_string()] {
        let mock = MockReleases::demarrer(&release(&tag, ASSET), &script("99.0.0")).await;
        let exe = copie("upd_a_jour");
        let sortie = lancer(&exe, &["update"], Some(&mock.url), None).await;
        assert_eq!(sortie.status.code(), Some(0));
        assert_eq!(
            texte(&sortie.stdout).trim(),
            format!("fastrag {VERSION} est déjà à jour.")
        );
        assert_eq!(std::fs::read(&exe).unwrap(), avant);
        seul_le_binaire(&exe);
    }
}

/// Lance `update` et vérifie le message d'échec, le binaire intact et l'absence de temporaire.
async fn echec(nom: &str, mock: &MockReleases, attendu: &str) {
    let exe = copie(nom);
    let avant = std::fs::read(&exe).unwrap();
    let sortie = lancer(&exe, &["update"], Some(&mock.url), None).await;
    assert_ne!(sortie.status.code(), Some(0));
    let err = texte(&sortie.stderr);
    assert!(err.contains(attendu), "{err}");
    assert!(
        err.trim().ends_with("fastrag n'a pas été modifié."),
        "{err}"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), avant);
    seul_le_binaire(&exe);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req024_ac4_echecs() {
    // Serveur arrêté.
    let mut mock = MockReleases::demarrer("{}", "").await;
    mock.arreter().await;
    echec("upd_arret", &mock, "Impossible de joindre").await;
    // JSON illisible.
    let m = MockReleases::demarrer("pas du json", "").await;
    echec("upd_json", &m, "illisible").await;
    // Fichier absent de la release.
    let m = MockReleases::demarrer(&release("v99.0.0", "autre"), &script("99.0.0")).await;
    echec("upd_absent", &m, ASSET).await;
    // Binaire qui répond une autre version.
    let m = MockReleases::demarrer(&release("v99.0.0", ASSET), &script("98.0.0")).await;
    echec("upd_version", &m, "fastrag 99.0.0").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req024_ac4_dossier_en_lecture_seule() {
    let m = MockReleases::demarrer(&release("v99.0.0", ASSET), &script("99.0.0")).await;
    let exe = copie("upd_ro");
    let avant = std::fs::read(&exe).unwrap();
    let dossier = exe.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&dossier, std::fs::Permissions::from_mode(0o555)).unwrap();
    let sortie = lancer(&exe, &["update"], Some(&m.url), None).await;
    let vide = std::fs::write(dossier.join("sonde"), "").is_ok(); // vrai si root : le test ne prouve rien
    std::fs::set_permissions(&dossier, std::fs::Permissions::from_mode(0o755)).unwrap();
    if vide {
        return;
    }
    assert_ne!(sortie.status.code(), Some(0));
    let err = texte(&sortie.stderr);
    assert!(err.contains("impossible à remplacer"), "{err}");
    assert_eq!(std::fs::read(&exe).unwrap(), avant);
    assert!(
        err.trim().ends_with("fastrag n'a pas été modifié."),
        "{err}"
    );
    seul_le_binaire(&exe);
}
