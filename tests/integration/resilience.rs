use crate::common::dossier_test;
use crate::common::mock_provider::{MockProvider, Panne};
use fastrag::index::pending::synchroniser_avec_reprise;
use fastrag::index::Indexeur;
use fastrag::provider::Provider;
use fastrag::store::Store;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn ecrire(d: &Path, rel: &str, contenu: &[u8]) -> PathBuf {
    let f = d.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(&f, contenu).unwrap();
    f
}

fn indexeur(nom: &str, mock: &MockProvider, dossiers: Vec<PathBuf>) -> Indexeur {
    let donnees = dossier_test(&format!("{nom}_donnees"));
    let store = Store::ouvrir(&donnees, "m").unwrap();
    Indexeur::new(store, Provider::new(&mock.url, None, "m"), dossiers)
}

#[tokio::test]
async fn req005_ac3_avertissement_disparait_quand_le_fichier_est_corrige_ou_supprime() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("res_refus");
    let mauvais = ecrire(&d, "mauvais.md", &[0xff, 0xfe, 0x00, 0xc3, 0x28]);
    let autre = ecrire(&d, "autre.txt", &[0xff, 0xfe, 0xfd]);
    let mut idx = indexeur("res_refus", &mock, vec![d.clone()]);
    let statut = idx.statut();
    idx.synchroniser().await.unwrap();
    let w = statut.avertissements();
    assert!(w.iter().any(|a| a.contains("mauvais.md")), "{w:?}");
    assert!(w.iter().any(|a| a.contains("autre.txt")), "{w:?}");
    // Corrigé : seul l'avertissement de l'autre fichier reste.
    std::fs::write(&mauvais, "# Bon\n\ntexte valide").unwrap();
    idx.synchroniser().await.unwrap();
    let w = statut.avertissements();
    assert!(!w.iter().any(|a| a.contains("mauvais.md")), "{w:?}");
    assert!(w.iter().any(|a| a.contains("autre.txt")), "{w:?}");
    // Supprimé : plus aucun avertissement.
    std::fs::remove_file(&autre).unwrap();
    idx.synchroniser().await.unwrap();
    assert!(statut.avertissements().is_empty());
}

#[tokio::test]
async fn req007_ac1_provider_en_panne_notes_en_attente_ancienne_version_servie() {
    let d = dossier_test("res_panne");
    let f = ecrire(&d, "a.md", b"ancienne version");
    for (nom, cause) in [
        ("cle", "clé invalide"),
        ("quota", "quota"),
        ("reseau", "réseau"),
    ] {
        let mock = MockProvider::demarrer().await;
        let mut idx = indexeur(&format!("res_panne_{nom}"), &mock, vec![d.clone()]);
        std::fs::write(&f, "ancienne version").unwrap();
        idx.synchroniser().await.unwrap();
        assert_eq!(
            idx.store()
                .passages_note(&f.display().to_string())
                .unwrap()
                .len(),
            1
        );
        match nom {
            "cle" => mock.panne(Panne::Cle401),
            "quota" => mock.panne(Panne::Quota429),
            _ => mock.arreter(),
        }
        std::fs::write(&f, "nouvelle version").unwrap();
        let neuve = ecrire(&d, "neuve.md", b"note neuve");
        let r = idx.synchroniser().await.unwrap();
        assert_eq!(r.modifiees + r.nouvelles, 0);
        let w = idx.statut().avertissements();
        assert!(
            w.iter().any(|a| a.contains("a.md") && a.contains(cause)),
            "{cause} : {w:?}"
        );
        assert_eq!(idx.en_attente().len(), 2);
        let p = idx.store().passages_note(&f.display().to_string()).unwrap();
        assert_eq!(p[0].texte, "ancienne version");
        assert!(idx
            .store()
            .passages_note(&neuve.display().to_string())
            .unwrap()
            .is_empty());
        std::fs::remove_file(neuve).unwrap();
    }
}

#[tokio::test]
async fn req007_ac2_retour_du_provider_traite_les_notes_sans_action() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("res_retour");
    let f = ecrire(&d, "a.md", b"contenu a traiter");
    mock.arreter();
    let mut idx = indexeur("res_retour", &mock, vec![d.clone()]);
    let statut = idx.statut();
    let tache = tokio::spawn(async move {
        synchroniser_avec_reprise(&mut idx, Duration::from_millis(100))
            .await
            .unwrap();
        idx
    });
    let limite = std::time::Instant::now() + Duration::from_secs(20);
    while statut.avertissements().is_empty() {
        assert!(std::time::Instant::now() < limite, "pas d'avertissement");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(statut.avertissements()[0].contains("a.md"));
    assert!(!tache.is_finished());
    mock.relancer().await;
    let idx = tokio::time::timeout(Duration::from_secs(20), tache)
        .await
        .expect("reprise trop lente")
        .unwrap();
    assert!(statut.avertissements().is_empty());
    assert!(idx.en_attente().is_empty());
    let p = idx.store().passages_note(&f.display().to_string()).unwrap();
    assert_eq!(p.len(), 1);
}

#[tokio::test]
async fn req008_ac3_rattrapage_au_demarrage() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("res_rattrape");
    let modif = ecrire(&d, "modif.md", b"avant");
    let suppr = ecrire(&d, "suppr.md", b"a supprimer");
    ecrire(&d, "stable.md", b"stable");
    let donnees = dossier_test("res_rattrape_donnees");
    {
        let store = Store::ouvrir(&donnees, "m").unwrap();
        let mut idx = Indexeur::new(store, Provider::new(&mock.url, None, "m"), vec![d.clone()]);
        idx.synchroniser().await.unwrap();
    }
    // Service arrêté : ajout, modification, suppression.
    let ajout = ecrire(&d, "ajout.md", b"ajoute pendant l'arret");
    std::fs::write(&modif, "apres").unwrap();
    std::fs::remove_file(&suppr).unwrap();
    let store = Store::ouvrir(&donnees, "m").unwrap();
    let mut idx = Indexeur::new(store, Provider::new(&mock.url, None, "m"), vec![d.clone()]);
    let r = idx.synchroniser().await.unwrap();
    assert_eq!(
        (r.nouvelles, r.modifiees, r.supprimees, r.inchangees),
        (1, 1, 1, 1)
    );
    let chemins = idx.store().chemins_notes().unwrap();
    assert!(chemins.contains(&ajout.display().to_string()));
    assert!(!chemins.contains(&suppr.display().to_string()));
    let p = idx
        .store()
        .passages_note(&modif.display().to_string())
        .unwrap();
    assert_eq!(p[0].texte, "apres");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req010_ac2_binaire_tue_pendant_l_indexation_puis_rattrape() {
    const N: usize = 40;
    let mock = MockProvider::demarrer().await;
    let notes = dossier_test("res_kill_notes");
    for i in 0..N {
        ecrire(
            &notes,
            &format!("n{i}.md"),
            format!("# Note {i}\n\navant {i}").as_bytes(),
        );
    }
    let donnees = dossier_test("res_kill_donnees");
    let conf = dossier_test("res_kill_conf").join("config.toml");
    let q = |p: &Path| format!("{:?}", p.display().to_string());
    std::fs::write(
        &conf,
        format!(
            "dossiers = [{}]\ndossier_donnees = {}\nport = 0\n[embedding]\nurl = {:?}\nmodele = \"m\"\n",
            q(&notes),
            q(&donnees),
            mock.url
        ),
    )
    .unwrap();
    let lancer = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_fastrag"))
            .arg("--config")
            .arg(&conf)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    // Première indexation complète (contenu « avant ») : le service ne s'arrête plus seul.
    // Connexion de suivi ouverte une fois : elle ne fait que lire, sans gêner l'écriture du binaire.
    let suivi = Store::ouvrir(&donnees, "m").unwrap();
    let mut premier = lancer();
    let limite = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let fini = suivi.compter().map(|(n, _)| n == N).unwrap_or(false);
        if fini {
            break;
        }
        assert!(std::time::Instant::now() < limite, "indexation trop longue");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    premier.kill().unwrap();
    premier.wait().unwrap();
    let avant = mock.textes_recus().len();
    assert!(avant >= N);
    for i in 0..N {
        ecrire(
            &notes,
            &format!("n{i}.md"),
            format!("# Note {i}\n\napres {i}").as_bytes(),
        );
    }
    let mut enfant = lancer();
    let limite = std::time::Instant::now() + Duration::from_secs(30);
    while mock.textes_recus().len() <= avant {
        assert!(
            std::time::Instant::now() < limite,
            "le binaire n'indexe pas"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    enfant.kill().unwrap();
    enfant.wait().unwrap();
    // Dernier état valide : N notes, chacune entièrement ancienne ou entièrement nouvelle.
    let store = Store::ouvrir(&donnees, "m").unwrap();
    let presentes = store.chemins_notes().unwrap();
    assert_eq!(presentes.len(), N);
    for c in &presentes {
        let p = store.passages_note(c).unwrap();
        assert_eq!(p.len(), 1, "{c}");
        assert!(
            p[0].texte.contains("avant") || p[0].texte.contains("apres"),
            "{c}"
        );
    }
    drop(store);
    // Relance : rattrapage complet, tout à la nouvelle version.
    let mut relance = lancer();
    let limite = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let store = Store::ouvrir(&donnees, "m").unwrap();
        let fini = store
            .chemins_notes()
            .unwrap()
            .iter()
            .all(|c| store.passages_note(c).unwrap()[0].texte.contains("apres"));
        if fini {
            break;
        }
        assert!(std::time::Instant::now() < limite, "rattrapage trop long");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    relance.kill().unwrap();
    relance.wait().unwrap();
    let store = Store::ouvrir(&donnees, "m").unwrap();
    let presentes = store.chemins_notes().unwrap();
    assert_eq!(presentes.len(), N);
    for c in &presentes {
        let p = store.passages_note(c).unwrap();
        assert_eq!(p.len(), 1, "{c}");
        assert!(p[0].texte.contains("apres"), "{c} : {}", p[0].texte);
    }
}
