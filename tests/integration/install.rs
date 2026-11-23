use crate::common::dossier_test;
use crate::common::mock_releases::script;
use axum::{http::StatusCode, routing::get, Router};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh");

/// Serveur de téléchargement simulé : sert `/fastrag-linux-x86_64`, ou 404 si `contenu` est `None`.
async fn serveur(contenu: Option<String>) -> (String, tokio::task::JoinHandle<()>) {
    let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", ecoute.local_addr().unwrap());
    let app = Router::new().route(
        "/fastrag-linux-x86_64",
        get(move || {
            let c = contenu.clone();
            async move {
                match c {
                    Some(c) => (StatusCode::OK, c),
                    None => (StatusCode::NOT_FOUND, String::new()),
                }
            }
        }),
    );
    let t = tokio::spawn(async move {
        axum::serve(ecoute, app).await.unwrap();
    });
    (base, t)
}

async fn installer(url: &str, home: &Path, dest: &Path, path: Option<String>) -> Output {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(SCRIPT)
        .env("HOME", home)
        .env("FASTRAG_INSTALL_DIR", dest)
        .env("FASTRAG_DOWNLOAD_URL", url);
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

fn texte(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn noms(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn req023_ac1_installe_et_annonce() {
    let d = dossier_test("install_ac1");
    let dest = d.join("bin");
    let (url, _t) = serveur(Some(script("99.0.0"))).await;
    let s = installer(&url, &d, &dest, None).await;
    let out = texte(&s.stdout);
    assert_eq!(s.status.code(), Some(0), "{out} {}", texte(&s.stderr));
    assert!(
        out.contains(&format!(
            "fastrag 99.0.0 est installé dans {}.",
            dest.join("fastrag").display()
        )),
        "{out}"
    );
    assert!(out.contains("README"), "{out}");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dest.join("fastrag"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(noms(&dest), vec!["fastrag"]);
    let v = Command::new(dest.join("fastrag"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(texte(&v.stdout).trim(), "fastrag 99.0.0");
}

#[tokio::test]
async fn req023_ac1_dossier_par_defaut() {
    let d = dossier_test("install_defaut");
    let (url, _t) = serveur(Some(script("99.0.0"))).await;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(SCRIPT)
        .env("HOME", &d)
        .env_remove("FASTRAG_INSTALL_DIR")
        .env("FASTRAG_DOWNLOAD_URL", &url);
    let s = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert_eq!(s.status.code(), Some(0), "{}", texte(&s.stderr));
    assert!(d.join(".local/bin/fastrag").is_file());
}

#[tokio::test]
async fn req023_ac2_dossier_hors_path() {
    let d = dossier_test("install_ac2");
    let (url, _t) = serveur(Some(script("99.0.0"))).await;
    let dest = d.join("bin");
    let s = installer(&url, &d, &dest, None).await;
    assert!(
        texte(&s.stdout).contains(&format!("Ajoutez {} à votre PATH.", dest.display())),
        "{}",
        texte(&s.stdout)
    );
    let d2 = dossier_test("install_ac2_dans_path");
    let dest2 = d2.join("bin");
    let path = format!("{}:/usr/bin:/bin", dest2.display());
    let s = installer(&url, &d2, &dest2, Some(path)).await;
    assert_eq!(s.status.code(), Some(0), "{}", texte(&s.stderr));
    assert!(
        !texte(&s.stdout).contains("Ajoutez"),
        "{}",
        texte(&s.stdout)
    );
}

#[tokio::test]
async fn req023_ac3_autre_systeme() {
    for (sys, archi) in [("Darwin", "arm64"), ("Linux", "aarch64")] {
        let d = dossier_test(&format!("install_ac3_{sys}"));
        let faux = d.join("faux");
        std::fs::create_dir_all(&faux).unwrap();
        let uname = faux.join("uname");
        std::fs::write(
            &uname,
            format!("#!/bin/sh\nif [ \"$1\" = \"-s\" ]; then echo {sys}; else echo {archi}; fi\n"),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dest = d.join("bin");
        let (url, _t) = serveur(Some(script("99.0.0"))).await;
        let path = format!("{}:/usr/bin:/bin", faux.display());
        let s = installer(&url, &d, &dest, Some(path)).await;
        assert_ne!(s.status.code(), Some(0));
        assert!(
            texte(&s.stderr).contains(&format!(
                "Aucun binaire publié pour {sys} {archi} : compilez fastrag depuis les sources (voir le README)."
            )),
            "{}",
            texte(&s.stderr)
        );
        assert!(!dest.exists(), "rien ne doit être installé");
    }
}

#[tokio::test]
async fn req023_ac4_telechargement_impossible() {
    let d = dossier_test("install_ac4");
    let dest = d.join("bin");
    std::fs::create_dir_all(&dest).unwrap();
    let ancien: PathBuf = dest.join("fastrag");
    std::fs::write(&ancien, "ancien").unwrap();

    // Release absente : 404.
    let (url, mut t) = serveur(None).await;
    let s = installer(&url, &d, &dest, None).await;
    assert_ne!(s.status.code(), Some(0));
    assert!(
        texte(&s.stderr).contains(&format!(
            "Téléchargement impossible : {url}/fastrag-linux-x86_64"
        )),
        "{}",
        texte(&s.stderr)
    );
    assert_eq!(std::fs::read_to_string(&ancien).unwrap(), "ancien");
    assert_eq!(noms(&dest), vec!["fastrag"]);

    // Serveur arrêté.
    t.abort();
    let _ = (&mut t).await;
    let s = installer(&url, &d, &dest, None).await;
    assert_ne!(s.status.code(), Some(0));
    assert!(
        texte(&s.stderr).contains("Téléchargement impossible : "),
        "{}",
        texte(&s.stderr)
    );
    assert_eq!(std::fs::read_to_string(&ancien).unwrap(), "ancien");
    assert_eq!(noms(&dest), vec!["fastrag"]);
}

#[test]
fn req023_ac5_meme_nom_partout() {
    let racine = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lire = |p: &str| std::fs::read_to_string(racine.join(p)).unwrap();
    let wf = lire(".github/workflows/release.yml");
    let nom = "fastrag-linux-x86_64";
    assert!(
        wf.contains("tags:") && wf.contains("'v*'"),
        "déclencheur par tag v*"
    );
    assert!(wf.contains("cargo build --release"));
    assert!(wf.contains("gh release create \"$GITHUB_REF_NAME\""));
    assert!(wf.contains(nom));
    assert!(lire("install.sh").contains(nom));
    assert!(lire("src/update.rs").contains(nom));
}

#[test]
fn req023_ac6_readme() {
    let readme =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    let debut = readme
        .find("## Installation")
        .expect("section Installation");
    let fin = readme[debut + 5..]
        .find("\n## ")
        .map_or(readme.len(), |i| debut + 5 + i);
    let s = &readme[debut..fin];
    assert!(s.contains(
        "curl -fsSL https://raw.githubusercontent.com/tixplop/fastrag/master/install.sh | sh"
    ));
    assert!(s.contains("Linux x86_64") && s.contains("WSL"));
    assert!(s.contains("cargo build --release"));
    assert!(s.contains("fastrag update"));
}
