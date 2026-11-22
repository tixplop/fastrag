use crate::common::dossier_test;
use crate::common::mock_provider::MockProvider;
use fastrag::config::Config;
use fastrag::search::EntreeRecherche;
use fastrag::service::{demarrer, ServiceEnCours};
use std::time::{Duration, Instant};

async fn trouve(s: &ServiceEnCours, question: &str, fin: &str) -> bool {
    let e = EntreeRecherche {
        question: question.into(),
        nombre: Some(50),
        dossier: None,
        tag: None,
    };
    s.service
        .chercher(&e)
        .await
        .unwrap()
        .iter()
        .any(|r| r.chemin.ends_with(fin))
}

/// Attend (au plus 5 s) que la présence de `fin` dans les résultats vaille `attendu`.
async fn attendre(s: &ServiceEnCours, question: &str, fin: &str, attendu: bool) -> bool {
    let limite = Instant::now() + Duration::from_secs(5);
    loop {
        if trouve(s, question, fin).await == attendu {
            return true;
        }
        if Instant::now() >= limite {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn parcours(nom: &str, forcer_releve: bool) {
    let mock = MockProvider::demarrer().await;
    let notes = dossier_test(&format!("{nom}_notes"));
    std::fs::write(notes.join("depart.md"), "# Départ\n\nnote de départ").unwrap();
    let mut c = Config {
        dossiers: vec![notes.clone()],
        dossier_donnees: dossier_test(&format!("{nom}_donnees")),
        forcer_releve,
        releve_secondes: 1,
        regroupement_ms: 100,
        ..Config::default()
    };
    c.embedding.url = mock.url.clone();
    c.embedding.modele = "m".into();
    c.port = 0;
    c.reessai_secondes = 1;
    let s = demarrer(&c).await.unwrap();
    assert!(attendre(&s, "note de départ", "depart.md", true).await);

    // Ajout dans un sous-dossier, puis modification.
    std::fs::create_dir_all(notes.join("sous")).unwrap();
    let f = notes.join("sous/neuve.md");
    std::fs::write(&f, "# Neuve\n\nrecette de la tarte aux pommes").unwrap();
    assert!(
        attendre(&s, "recette de la tarte aux pommes", "neuve.md", true).await,
        "ajout non vu en 5 s"
    );
    std::fs::write(
        &f,
        "# Neuve\n\nbillet de train pour Lyon, texte nettement plus long",
    )
    .unwrap();
    let limite = Instant::now() + Duration::from_secs(5);
    loop {
        let r = s.service.etat().unwrap();
        let p = s
            .service
            .chercher(&EntreeRecherche {
                question: "billet de train pour Lyon, texte nettement plus long".into(),
                nombre: Some(50),
                dossier: None,
                tag: None,
            })
            .await
            .unwrap();
        if p.iter().any(|x| x.texte.contains("billet de train")) {
            assert_eq!(r.notes, 2);
            break;
        }
        assert!(Instant::now() < limite, "modification non vue en 5 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Suppression.
    std::fs::remove_file(&f).unwrap();
    let limite = Instant::now() + Duration::from_secs(5);
    while s.service.etat().unwrap().notes != 1 {
        assert!(Instant::now() < limite, "suppression non vue en 5 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!trouve(&s, "billet de train pour Lyon", "neuve.md").await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req008_ac1_ac2_inotify_ajout_modification_suppression() {
    parcours("watch_inotify", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req008_ac1_ac2_releve_forcee_ajout_modification_suppression() {
    parcours("watch_releve", true).await;
}
