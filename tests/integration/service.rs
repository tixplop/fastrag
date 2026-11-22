use crate::common::dossier_test;
use crate::common::mock_provider::{MockProvider, Panne};
use fastrag::index::Indexeur;
use fastrag::provider::Provider;
use fastrag::store::Store;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn lancer(conf: &std::path::Path, cle: Option<&str>) -> (Child, Option<String>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fastrag"));
    cmd.arg("--config")
        .arg(conf)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(c) = cle {
        cmd.env("TEMOIN_CLE", c);
    }
    let mut enfant = cmd.spawn().unwrap();
    let mut ligne = String::new();
    BufReader::new(enfant.stdout.as_mut().unwrap())
        .read_line(&mut ligne)
        .unwrap();
    let adresse = ligne
        .trim()
        .strip_prefix("Service prêt sur http://")
        .map(String::from);
    (enfant, adresse)
}

fn config(nom: &str, mock: &MockProvider) -> (std::path::PathBuf, std::path::PathBuf) {
    let notes = dossier_test(&format!("{nom}_notes"));
    std::fs::write(notes.join("a.md"), "# A\n\nbonjour le monde").unwrap();
    let donnees = dossier_test(&format!("{nom}_donnees"));
    let conf = dossier_test(&format!("{nom}_conf")).join("config.toml");
    std::fs::write(
        &conf,
        format!(
            "dossiers = [{:?}]\ndossier_donnees = {:?}\nport = 0\nreessai_secondes = 1\n[embedding]\nurl = {:?}\nmodele = \"m\"\ncle_env = \"TEMOIN_CLE\"\n",
            notes.display().to_string(),
            donnees.display().to_string(),
            mock.url
        ),
    )
    .unwrap();
    (conf, donnees)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req019_ac3_second_lancement_refuse_sans_toucher_a_l_index() {
    let mock = MockProvider::demarrer().await;
    let (conf, donnees) = config("svc_second", &mock);
    let (mut premier, adresse) = lancer(&conf, None);
    let adresse = adresse.expect("le service n'a pas démarré");
    let url = format!("http://{adresse}/status");
    for _ in 0..500 {
        let e: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
        if e["notes"] == 1 && e["en_cours"] == false {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let appels = mock.textes_recus().len();
    let second = Command::new(env!("CARGO_BIN_EXE_fastrag"))
        .arg("--config")
        .arg(&conf)
        .output()
        .unwrap();
    assert!(!second.status.success());
    let err = String::from_utf8_lossy(&second.stderr);
    assert!(err.contains("déjà utilisé"), "{err}");
    assert!(err.contains(&donnees.display().to_string()), "{err}");
    assert_eq!(mock.textes_recus().len(), appels);
    // Le premier service tourne toujours.
    let e: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
    assert_eq!(e["notes"], 1);
    premier.kill().unwrap();
    premier.wait().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req001_ac3_la_cle_n_apparait_ni_dans_l_etat_ni_les_journaux_ni_les_erreurs() {
    const CLE: &str = "cle-temoin-7f3a9c1e";
    let mock = MockProvider::demarrer().await;
    let (conf, _) = config("svc_cle", &mock);
    let (mut enfant, adresse) = lancer(&conf, Some(CLE));
    let adresse = adresse.expect("le service n'a pas démarré");
    let url = |c: &str| format!("http://{adresse}{c}");
    for _ in 0..500 {
        let e: serde_json::Value = reqwest::get(url("/status"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if e["notes"] == 1 && e["en_cours"] == false {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Clé refusée : une note nouvelle ne s'indexe pas, l'avertissement est dans l'état.
    mock.panne(Panne::Cle401);
    let notes = dossier_test("svc_cle_notes");
    std::fs::write(notes.join("b.md"), "# B\n\nautre note").unwrap();
    let client = reqwest::Client::new();
    client.post(url("/reindex")).send().await.unwrap();
    let mut etat = String::new();
    for _ in 0..500 {
        etat = reqwest::get(url("/status"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        if etat.contains("clé") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(etat.contains("clé invalide"), "{etat}");
    assert!(!etat.contains(CLE), "{etat}");
    // Une recherche échoue aussi sur la clé refusée, sans la montrer.
    let r = client
        .post(url("/search"))
        .body(r#"{"question":"bonjour"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 502);
    let corps = r.text().await.unwrap();
    assert!(corps.contains("clé"), "{corps}");
    assert!(!corps.contains(CLE), "{corps}");
    // La clé a bien été envoyée au provider.
    assert!(
        mock.autorisations().iter().any(|a| a.contains(CLE)),
        "{:?}",
        mock.autorisations()
    );
    enfant.kill().unwrap();
    let sortie = enfant.wait_with_output().unwrap();
    let journaux = String::from_utf8_lossy(&sortie.stderr);
    assert!(
        journaux.contains("clé invalide"),
        "journaux vides : {journaux}"
    );
    assert!(!journaux.contains(CLE), "{journaux}");
    assert!(!String::from_utf8_lossy(&sortie.stdout).contains(CLE));
}

#[tokio::test]
async fn req009_reindexation_complete_retraite_aussi_les_notes_inchangees() {
    let mock = MockProvider::demarrer().await;
    let notes = dossier_test("svc_complet_notes");
    std::fs::write(notes.join("a.md"), "# A\n\ntexte a").unwrap();
    std::fs::write(notes.join("b.md"), "# B\n\ntexte b").unwrap();
    let donnees = dossier_test("svc_complet_donnees");
    let mut idx = Indexeur::new(
        Store::ouvrir(&donnees, "m").unwrap(),
        Provider::new(&mock.url, None, "m"),
        vec![notes],
    );
    assert_eq!(idx.synchroniser().await.unwrap().nouvelles, 2);
    assert_eq!(idx.synchroniser().await.unwrap().inchangees, 2);
    idx.demander_reindexation_complete();
    let r = idx.synchroniser().await.unwrap();
    assert_eq!((r.modifiees, r.inchangees), (2, 0));
    // La demande ne vaut que pour un passage.
    assert_eq!(idx.synchroniser().await.unwrap().inchangees, 2);
    assert_eq!(mock.textes_recus().len(), 2);
}
