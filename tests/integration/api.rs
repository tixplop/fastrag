use crate::common::dossier_test;
use crate::common::mock_provider::{MockProvider, Panne};
use fastrag::config::Config;
use fastrag::search::EntreeRecherche;
use fastrag::service::{demarrer, ServiceEnCours};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub fn ecrire(nom: &str, notes: &[(&str, &str)]) -> std::path::PathBuf {
    let racine = dossier_test(&format!("{nom}_notes"));
    for (rel, contenu) in notes {
        std::fs::write(racine.join(rel), contenu).unwrap();
    }
    racine
}

fn config(nom: &str, mock: &MockProvider, dossiers: Vec<std::path::PathBuf>) -> Config {
    let mut c = Config {
        dossiers,
        dossier_donnees: dossier_test(&format!("{nom}_donnees")),
        ..Config::default()
    };
    c.embedding.url = mock.url.clone();
    c.embedding.modele = "m".into();
    c.port = 0;
    c.reessai_secondes = 1;
    c
}

async fn lire(s: &ServiceEnCours, chemin: &str) -> Value {
    reqwest::get(format!("http://{}{chemin}", s.adresse))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn poster(s: &ServiceEnCours, chemin: &str, corps: &str) -> (u16, Value) {
    let r = reqwest::Client::new()
        .post(format!("http://{}{chemin}", s.adresse))
        .body(corps.to_string())
        .send()
        .await
        .unwrap();
    let code = r.status().as_u16();
    (code, r.json().await.unwrap())
}

/// Attend la fin de l'indexation en cours (au plus 20 s) et rend l'état.
async fn attendre_repos(s: &ServiceEnCours) -> Value {
    let limite = Instant::now() + Duration::from_secs(20);
    loop {
        let e = lire(s, "/status").await;
        if e["en_cours"] == false && !e["derniere_mise_a_jour"].is_null() {
            return e;
        }
        assert!(Instant::now() < limite, "indexation trop longue : {e}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req016_ac1_search_rend_les_resultats_de_la_recherche_du_service() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire(
        "api_search",
        &[
            ("a.md", "# Tartes\n\nrecette de la tarte aux pommes"),
            ("b.md", "# Voyage\n\nbillet de train pour Lyon"),
        ],
    );
    let s = demarrer(&config("api_search", &mock, vec![notes.clone()]))
        .await
        .unwrap();
    attendre_repos(&s).await;
    let entree = EntreeRecherche {
        question: "tarte aux pommes".into(),
        nombre: Some(2),
        dossier: None,
        tag: None,
    };
    let attendu = s.service.chercher(&entree).await.unwrap();
    assert_eq!(attendu.len(), 2);
    let (code, v) = poster(
        &s,
        "/search",
        r#"{"question":"tarte aux pommes","nombre":2}"#,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(v["resultats"], serde_json::to_value(&attendu).unwrap());
    assert!(v["resultats"][0]["chemin"]
        .as_str()
        .unwrap()
        .ends_with("a.md"));
    // Les filtres passent aussi par l'API.
    let (_, v) = poster(&s, "/search", r#"{"question":"train","dossier":"inconnu"}"#).await;
    assert_eq!(v["resultats"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req009_ac1_ac2_req016_ac2_reindex_lancee_puis_deja_en_cours() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire("api_reindex", &[("a.md", "# A\n\npremiere note")]);
    let s = demarrer(&config("api_reindex", &mock, vec![notes.clone()]))
        .await
        .unwrap();
    let e = attendre_repos(&s).await;
    assert_eq!(e["notes"], 1);
    // Une note nouvelle et un provider en quota : l'indexation reste en cours.
    std::fs::write(notes.join("b.md"), "# B\n\nseconde note").unwrap();
    mock.panne(Panne::Quota429);
    let (code, v) = poster(&s, "/reindex", "").await;
    assert_eq!((code, v["statut"].as_str()), (200, Some("lancée")));
    assert_eq!(lire(&s, "/status").await["en_cours"], true);
    // Juste après un essai du provider : la seconde demande ne déclenche aucun appel de plus.
    let limite = Instant::now() + Duration::from_secs(10);
    let mut appels = mock.autorisations().len();
    while appels < 2 {
        assert!(Instant::now() < limite, "pas de nouvel essai");
        tokio::time::sleep(Duration::from_millis(10)).await;
        appels = mock.autorisations().len();
    }
    let (code, v) = poster(&s, "/reindex", "").await;
    assert_eq!((code, v["statut"].as_str()), (200, Some("déjà en cours")));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(mock.autorisations().len(), appels);
    assert_eq!(lire(&s, "/status").await["en_cours"], true);
    // Le provider revient : la même indexation se termine, sans seconde indexation.
    mock.panne(Panne::Aucune);
    let limite = Instant::now() + Duration::from_secs(20);
    loop {
        let e = lire(&s, "/status").await;
        if e["en_cours"] == false {
            assert_eq!(e["notes"], 2);
            break;
        }
        assert!(Instant::now() < limite, "indexation trop longue : {e}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Au repos, une nouvelle demande est de nouveau acceptée ; les vecteurs connus sont réutilisés.
    let avant = mock.textes_recus().len();
    let (_, v) = poster(&s, "/reindex", "").await;
    assert_eq!(v["statut"], "lancée");
    attendre_repos(&s).await;
    assert_eq!(mock.textes_recus().len(), avant);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req016_ac3_req017_ac1_status_notes_passages_date_avertissements() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire(
        "api_status",
        &[("a.md", "# A\n\n## Un\n\ntexte un\n\n## Deux\n\ntexte deux")],
    );
    let absent = notes.join("absent");
    let s = demarrer(&config(
        "api_status",
        &mock,
        vec![notes.clone(), absent.clone()],
    ))
    .await
    .unwrap();
    let e = attendre_repos(&s).await;
    assert_eq!(e["notes"], 1);
    assert!(e["passages"].as_u64().unwrap() >= 2, "{e}");
    let date = e["derniere_mise_a_jour"].as_str().unwrap();
    let b = date.as_bytes();
    assert!(
        date.len() == 20 && b[4] == b'-' && b[10] == b'T' && b[13] == b':' && b[19] == b'Z',
        "{date}"
    );
    assert!(date.starts_with("20"), "{date}");
    let w = e["avertissements"].as_array().unwrap();
    assert!(
        w.iter().any(|a| a.as_str().unwrap().contains("absent")),
        "{w:?}"
    );
    // Même contenu que la fonction d'état du service (réutilisée par le MCP).
    assert_eq!(e, serde_json::to_value(s.service.etat().unwrap()).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req016_ac4_requete_mal_formee_donne_400_avec_le_champ() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire("api_400", &[("a.md", "# A\n\ntexte")]);
    let s = demarrer(&config("api_400", &mock, vec![notes]))
        .await
        .unwrap();
    let cas = [
        ("{pas du json", "corps"),
        ("[1,2]", "corps"),
        (r#"{"nombre":3}"#, "question"),
        (r#"{"question":42}"#, "question"),
        (r#"{"question":"   "}"#, "question"),
        (r#"{"question":"x","nombre":"trois"}"#, "nombre"),
        (r#"{"question":"x","nombre":2.5}"#, "nombre"),
        (r#"{"question":"x","nombre":0}"#, "nombre"),
        (r#"{"question":"x","dossier":7}"#, "dossier"),
        (r#"{"question":"x","tag":["a"]}"#, "tag"),
    ];
    for (corps, champ) in cas {
        let (code, v) = poster(&s, "/search", corps).await;
        assert_eq!(code, 400, "{corps}");
        assert_eq!(v["field"], champ, "{corps}");
        assert!(!v["error"].as_str().unwrap().is_empty(), "{corps}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req019_ac1_ac2_un_processus_ecoute_sur_127_0_0_1_sans_mot_de_passe() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire("api_local", &[("a.md", "# A\n\nbonjour le monde")]);
    let s = demarrer(&config("api_local", &mock, vec![notes]))
        .await
        .unwrap();
    assert_eq!(s.adresse.ip().to_string(), "127.0.0.1");
    // Le même processus a indexé la note et répond, sans aucun identifiant.
    attendre_repos(&s).await;
    let (code, v) = poster(&s, "/search", r#"{"question":"bonjour"}"#).await;
    assert_eq!(code, 200);
    assert_eq!(v["resultats"].as_array().unwrap().len(), 1);
}
