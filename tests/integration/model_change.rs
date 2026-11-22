//! Changement de modèle d'embedding (REQ-011) : recalcul en tâche de fond, ancien index servi
//! jusqu'à la bascule, reprise après une coupure.

use crate::common::dossier_test;
use crate::common::mock_provider::MockProvider;
use crate::common::GardeEnfant;
use fastrag::config::Config;
use fastrag::service::demarrer;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn config(nom: &str, mock: &MockProvider, modele: &str) -> Config {
    let notes = dossier_test(&format!("{nom}_notes"));
    std::fs::write(notes.join("a.md"), "# A\n\nles tartes aux pommes").unwrap();
    std::fs::write(notes.join("b.md"), "# B\n\nle voyage en train").unwrap();
    let mut c = Config {
        dossiers: vec![notes],
        dossier_donnees: dossier_test(&format!("{nom}_donnees")),
        ..Config::default()
    };
    c.embedding.url = mock.url.clone();
    c.embedding.modele = modele.into();
    c.port = 0;
    c.reessai_secondes = 1;
    c
}

async fn attendre<F: Fn() -> bool>(quoi: &str, f: F) {
    let limite = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < limite, "délai dépassé : {quoi}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn chercher(adresse: &str) -> Vec<Value> {
    let r = reqwest::Client::new()
        .post(format!("http://{adresse}/search"))
        .body(r#"{"question":"tartes pommes"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    v["resultats"].as_array().unwrap().clone()
}

async fn attendre_notes(adresse: &str, n: u64) {
    let limite = Instant::now() + Duration::from_secs(20);
    loop {
        let e: Value = reqwest::get(format!("http://{adresse}/status"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if e["notes"] == n && e["en_cours"] == false {
            return;
        }
        assert!(Instant::now() < limite, "indexation trop longue : {e}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn existe(donnees: &Path, f: &str) -> bool {
    donnees.join(f).exists()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req011_ac1_ac2_recalcul_automatique_ancien_index_servi_puis_bascule() {
    let mock = MockProvider::demarrer().await;
    let c1 = config("chgmod", &mock, "m1");
    let donnees = c1.dossier_donnees.clone();
    {
        let s = demarrer(&c1).await.unwrap();
        attendre_notes(&s.adresse.to_string(), 2).await;
    }
    assert_eq!(
        std::fs::read_to_string(donnees.join("index-actif")).unwrap(),
        "m1"
    );
    assert!(mock.modeles_recus().iter().all(|m| m == "m1"));

    // Nouveau modèle : le recalcul démarre seul ; il est retenu pour observer l'entre-deux.
    mock.retenir_modele("m2");
    let mut c2 = c1.clone();
    c2.embedding.modele = "m2".into();
    let s = demarrer(&c2).await.unwrap();
    let adresse = s.adresse.to_string();
    attendre("recalcul démarré", || {
        mock.modeles_recus().iter().any(|m| m == "m2")
    })
    .await;
    assert!(existe(&donnees, "index-m2.db"));
    // Pendant le recalcul : l'ancien index répond (le nouveau est encore vide).
    let avant = mock.modeles_recus().len();
    let res = chercher(&adresse).await;
    assert!(!res.is_empty());
    assert!(res[0]["chemin"].as_str().unwrap().ends_with("a.md"));
    assert!(mock.modeles_recus()[avant..].iter().all(|m| m == "m1"));
    assert_eq!(
        std::fs::read_to_string(donnees.join("index-actif")).unwrap(),
        "m1"
    );
    assert!(existe(&donnees, "index-m1.db"));

    // Fin du recalcul : bascule, ancien fichier supprimé, recherches sur le nouveau modèle.
    mock.liberer();
    attendre("ancien index supprimé", || {
        !existe(&donnees, "index-m1.db")
    })
    .await;
    assert!(!existe(&donnees, "index-m1.db-wal"));
    assert_eq!(
        std::fs::read_to_string(donnees.join("index-actif")).unwrap(),
        "m2"
    );
    let avant = mock.modeles_recus().len();
    let res = chercher(&adresse).await;
    assert!(res[0]["chemin"].as_str().unwrap().ends_with("a.md"));
    assert!(mock.modeles_recus()[avant..].iter().all(|m| m == "m2"));
    attendre_notes(&adresse, 2).await;
}

fn lancer(conf: &Path) -> (GardeEnfant, String) {
    let mut enfant = Command::new(env!("CARGO_BIN_EXE_fastrag"))
        .arg("--config")
        .arg(conf)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut ligne = String::new();
    BufReader::new(enfant.stdout.as_mut().unwrap())
        .read_line(&mut ligne)
        .unwrap();
    let adresse = ligne
        .trim()
        .strip_prefix("Service prêt sur http://")
        .expect("le service n'a pas démarré")
        .to_string();
    (GardeEnfant::new(enfant), adresse)
}

fn ecrire_conf(c: &Config, modele: &str) -> PathBuf {
    let conf = dossier_test("chgmod_bin_conf").join("config.toml");
    std::fs::write(
        &conf,
        format!(
            "dossiers = [{:?}]\ndossier_donnees = {:?}\nport = 0\nreessai_secondes = 1\n[embedding]\nurl = {:?}\nmodele = {modele:?}\n",
            c.dossiers[0].display().to_string(),
            c.dossier_donnees.display().to_string(),
            c.embedding.url
        ),
    )
    .unwrap();
    conf
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req011_ac3_binaire_coupe_pendant_le_recalcul_reprend_au_redemarrage() {
    let mock = MockProvider::demarrer().await;
    let c = config("chgmod_bin", &mock, "m1");
    let donnees = c.dossier_donnees.clone();
    let (mut premier, adresse) = lancer(&ecrire_conf(&c, "m1"));
    attendre_notes(&adresse, 2).await;
    premier.kill().unwrap();
    premier.wait().unwrap();

    // Recalcul vers m2, coupé en plein travail.
    mock.retenir_modele("m2");
    let conf2 = ecrire_conf(&c, "m2");
    let (mut second, _) = lancer(&conf2);
    attendre("recalcul démarré", || {
        mock.modeles_recus().iter().any(|m| m == "m2")
    })
    .await;
    second.kill().unwrap();
    second.wait().unwrap();
    assert!(existe(&donnees, "index-m1.db"));

    // Redémarrage : l'ancien index sert pendant que le recalcul reprend.
    let (_troisieme, adresse) = lancer(&conf2);
    let res = chercher(&adresse).await;
    assert!(res[0]["chemin"].as_str().unwrap().ends_with("a.md"));
    assert_eq!(
        std::fs::read_to_string(donnees.join("index-actif")).unwrap(),
        "m1"
    );
    mock.liberer();
    attendre("ancien index supprimé", || {
        !existe(&donnees, "index-m1.db")
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(donnees.join("index-actif")).unwrap(),
        "m2"
    );
    attendre_notes(&adresse, 2).await;
    assert!(!chercher(&adresse).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req011_ac2_ac3_au_demarrage_les_index_orphelins_sont_supprimes() {
    let mock = MockProvider::demarrer().await;
    let c = config("chgmod_orph", &mock, "m1");
    let donnees = c.dossier_donnees.clone();
    {
        let s = demarrer(&c).await.unwrap();
        attendre_notes(&s.adresse.to_string(), 2).await;
    }
    // Restes d'un ancien modèle (coupure entre la bascule et la suppression).
    for f in ["index-m0.db", "index-m0.db-wal", "index-m0.db-shm"] {
        std::fs::write(donnees.join(f), "reste").unwrap();
    }
    let s = demarrer(&c).await.unwrap();
    attendre_notes(&s.adresse.to_string(), 2).await;
    for f in ["index-m0.db", "index-m0.db-wal", "index-m0.db-shm"] {
        assert!(!existe(&donnees, f), "{f}");
    }
    assert!(existe(&donnees, "index-m1.db"));
}
