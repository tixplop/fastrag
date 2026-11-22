use crate::common::dossier_test;
use crate::common::mock_provider::{MockProvider, Panne};
use fastrag::index::Indexeur;
use fastrag::provider::Provider;
use fastrag::search::{EntreeRecherche, ErreurRecherche, Recherche};
use fastrag::store::Store;
use std::path::PathBuf;

/// Vecteur de base `i` (dimension 8) avec une petite part de `j`.
fn base(i: usize, j: usize, part: f32) -> Vec<f32> {
    let mut v = vec![0.0; 8];
    v[i] = 1.0;
    v[j] += part;
    v
}

fn entree(q: &str) -> EntreeRecherche {
    EntreeRecherche {
        question: q.into(),
        nombre: None,
        dossier: None,
        tag: None,
    }
}

/// Indexe les notes `(chemin relatif, contenu)` puis ouvre une recherche sur une connexion séparée.
async fn monter(
    nom: &str,
    mock: &MockProvider,
    reranker: Option<&MockProvider>,
    notes: &[(&str, &str)],
) -> (Recherche, PathBuf) {
    let racine = dossier_test(&format!("{nom}_notes"));
    for (rel, contenu) in notes {
        let f = racine.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, contenu).unwrap();
    }
    let donnees = dossier_test(&format!("{nom}_donnees"));
    let mut idx = Indexeur::new(
        Store::ouvrir(&donnees, "m").unwrap(),
        Provider::new(&mock.url, None, "m"),
        vec![racine.clone()],
    );
    idx.synchroniser().await.unwrap();
    let r = Recherche::new(
        Store::ouvrir(&donnees, "m").unwrap(),
        Provider::new(&mock.url, None, "m"),
        reranker.map(|m| Provider::new(&m.url, None, "m")),
        vec![racine.clone()],
    )
    .unwrap();
    (r, racine)
}

#[tokio::test]
async fn req012_ac1_ac2_entree_par_defaut_et_forme_des_resultats() {
    let mock = MockProvider::demarrer().await;
    let mut notes = Vec::new();
    let textes: Vec<String> = (0..7)
        .map(|i| format!("## Bas\n\npassage numéro{i}"))
        .collect();
    for (i, t) in textes.iter().enumerate() {
        mock.imposer_vecteur(t, base(0, 1, i as f32 * 0.2));
    }
    mock.imposer_vecteur("sujet", base(0, 1, 0.0));
    let contenus: Vec<String> = textes
        .iter()
        .map(|t| {
            format!("---\ntitle: Titre\ntags: [recette, four]\n---\n# Haut\n\n## Bas\n\n{t}\n")
        })
        .collect();
    for (i, c) in contenus.iter().enumerate() {
        notes.push((format!("n{i}.md"), c.as_str()));
    }
    let notes_ref: Vec<(&str, &str)> = notes.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    let (r, racine) = monter("rech_forme", &mock, None, &notes_ref).await;

    // Par défaut : 5 résultats, triés par score décroissant.
    let res = r.chercher(&entree("sujet")).await.unwrap();
    assert_eq!(res.len(), 5);
    assert!(res.windows(2).all(|w| w[0].score >= w[1].score), "{res:?}");
    assert_eq!(res[0].texte, "## Bas\n\npassage numéro0");
    let premier = &res[0];
    assert_eq!(premier.chemin, racine.join("n0.md").display().to_string());
    assert_eq!(premier.titres, "Haut > Bas");
    assert_eq!(
        premier.tags,
        vec!["recette".to_string(), "four".to_string()]
    );
    assert!(premier.ligne_debut >= 1 && premier.ligne_fin >= premier.ligne_debut);
    assert!(premier.score > 0.0);

    // Nombre explicite, plafonné.
    let mut e = entree("sujet");
    e.nombre = Some(2);
    assert_eq!(r.chercher(&e).await.unwrap().len(), 2);
    e.nombre = Some(500);
    assert_eq!(r.chercher(&e).await.unwrap().len(), 7);
    e.nombre = Some(0);
    assert!(matches!(
        r.chercher(&e).await,
        Err(ErreurRecherche::Entree { .. })
    ));
}

#[tokio::test]
async fn req012_ac5_index_vide_ou_filtres_sans_resultat() {
    let mock = MockProvider::demarrer().await;
    let (vide, _) = monter("rech_vide", &mock, None, &[]).await;
    assert!(vide.chercher(&entree("quoi")).await.unwrap().is_empty());

    let (r, _) = monter("rech_filtre_vide", &mock, None, &[("a.md", "un texte")]).await;
    let mut e = entree("texte");
    e.tag = Some("inconnu".into());
    assert!(r.chercher(&e).await.unwrap().is_empty());
    let mut e = entree("texte");
    e.dossier = Some("absent/".into());
    assert!(r.chercher(&e).await.unwrap().is_empty());
}

#[tokio::test]
async fn req013_ac1_passage_proche_en_sens_sans_mot_commun() {
    let mock = MockProvider::demarrer().await;
    mock.imposer_vecteur("### Brisée\n\npâte à tarte", base(0, 1, 0.1));
    mock.imposer_vecteur("Les voitures roulent vite", base(3, 4, 0.1));
    mock.imposer_vecteur("Un chat dort", base(5, 6, 0.1));
    mock.imposer_vecteur("dessert au four", base(0, 1, 0.0));
    let (r, _) = monter(
        "rech_sens",
        &mock,
        None,
        &[
            (
                "cuisine.md",
                "# Cuisine\n\n## Pâtes\n\n### Brisée\n\npâte à tarte\n",
            ),
            ("auto.md", "Les voitures roulent vite"),
            ("chat.md", "Un chat dort"),
        ],
    )
    .await;
    let res = r.chercher(&entree("dessert au four")).await.unwrap();
    assert_eq!(res[0].texte, "### Brisée\n\npâte à tarte");
    assert!(res[0].chemin.ends_with("cuisine.md"));
    assert_eq!(res[0].titres, "Cuisine > Pâtes > Brisée");
}

#[tokio::test]
async fn req013_ac2_question_francaise_trouve_la_note_anglaise() {
    let mock = MockProvider::demarrer().await;
    mock.imposer_vecteur("budget meeting", base(0, 1, 0.1));
    mock.imposer_vecteur("grocery list", base(4, 5, 0.1));
    mock.imposer_vecteur("réunion budget", base(0, 1, 0.0));
    let (r, _) = monter(
        "rech_anglais",
        &mock,
        None,
        &[("b.md", "budget meeting"), ("c.md", "grocery list")],
    )
    .await;
    let res = r.chercher(&entree("réunion budget")).await.unwrap();
    assert_eq!(res[0].texte, "budget meeting");
}

#[tokio::test]
async fn req014_ac1_mot_exact_en_tete_meme_loin_en_vecteur() {
    let mock = MockProvider::demarrer().await;
    mock.imposer_vecteur("Le serveur a renvoyé ERR_4021 hier", base(5, 6, 0.0));
    mock.imposer_vecteur("Autre sujet proche", base(0, 1, 0.1));
    mock.imposer_vecteur("Encore un autre", base(0, 1, 0.3));
    mock.imposer_vecteur("ERR_4021", base(0, 1, 0.0));
    let (r, _) = monter(
        "rech_mots",
        &mock,
        None,
        &[
            ("a.md", "Le serveur a renvoyé ERR_4021 hier"),
            ("b.md", "Autre sujet proche"),
            ("c.md", "Encore un autre"),
        ],
    )
    .await;
    let res = r.chercher(&entree("ERR_4021")).await.unwrap();
    assert_eq!(res[0].texte, "Le serveur a renvoyé ERR_4021 hier");
    // Caractères spéciaux FTS : pas d'erreur.
    assert!(r.chercher(&entree("\"ERR* OR (")).await.is_ok());
}

#[tokio::test]
async fn req015_ac1_ac2_filtres_dossier_et_tag() {
    let mock = MockProvider::demarrer().await;
    let (r, racine) = monter(
        "rech_filtres",
        &mock,
        None,
        &[
            (
                "travail/a.md",
                "---\ntags: [recette]\n---\ngâteau au chocolat",
            ),
            ("travail/b.md", "gâteau aux noix"),
            ("travail2/c.md", "gâteau de riz"),
            ("perso/d.md", "---\ntags: [recette]\n---\ngâteau aux pommes"),
        ],
    )
    .await;
    let mut e = entree("gâteau");
    e.nombre = Some(50);

    e.dossier = Some("travail/".into());
    let res = r.chercher(&e).await.unwrap();
    assert_eq!(res.len(), 2);
    let prefixe = format!("{}/", racine.join("travail").display());
    assert!(
        res.iter().all(|x| x.chemin.starts_with(&prefixe)),
        "{res:?}"
    );

    e.dossier = Some(racine.join("perso").display().to_string());
    let res = r.chercher(&e).await.unwrap();
    assert_eq!(res.len(), 1);
    assert!(res[0].chemin.ends_with("perso/d.md"));

    e.dossier = None;
    e.tag = Some("recette".into());
    let res = r.chercher(&e).await.unwrap();
    assert_eq!(res.len(), 2);
    assert!(res.iter().all(|x| x.tags.contains(&"recette".to_string())));

    e.dossier = Some("travail/".into());
    let res = r.chercher(&e).await.unwrap();
    assert_eq!(res.len(), 1);
    assert!(res[0].chemin.ends_with("travail/a.md"));
}

#[tokio::test]
async fn req015_ac3_ac4_reranker_reordonne_et_desactive_sans_appel() {
    let mock = MockProvider::demarrer().await;
    let notes = [
        ("a.md", "alpha sujet"),
        ("b.md", "beta sujet"),
        ("c.md", "gamma sujet"),
    ];
    mock.imposer_vecteur("sujet", base(0, 1, 0.0));
    mock.imposer_vecteur("alpha sujet", base(0, 1, 0.1));
    mock.imposer_vecteur("beta sujet", base(0, 1, 0.5));
    mock.imposer_vecteur("gamma sujet", base(0, 1, 0.9));

    // Désactivé : aucun appel à /rerank, ordre de la fusion.
    let (sans, _) = monter("rech_sans_rerank", &mock, None, &notes).await;
    let res = sans.chercher(&entree("sujet")).await.unwrap();
    assert_eq!(res[0].texte, "alpha sujet");
    assert_eq!(mock.appels_rerank(), 0);

    // Activé : mêmes passages, ordre et scores du reranker.
    mock.imposer_score("gamma sujet", 0.9);
    mock.imposer_score("beta sujet", 0.5);
    mock.imposer_score("alpha sujet", 0.1);
    let (avec, _) = monter("rech_avec_rerank", &mock, Some(&mock), &notes).await;
    let res2 = avec.chercher(&entree("sujet")).await.unwrap();
    assert_eq!(mock.appels_rerank(), 1);
    let textes: Vec<&str> = res2.iter().map(|x| x.texte.as_str()).collect();
    assert_eq!(textes, ["gamma sujet", "beta sujet", "alpha sujet"]);
    assert!((res2[0].score - 0.9).abs() < 1e-6);
    let mut a: Vec<_> = res.iter().map(|x| x.texte.clone()).collect();
    let mut b: Vec<_> = res2.iter().map(|x| x.texte.clone()).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
}

#[tokio::test]
async fn req015_ac5_provider_injoignable_cle_invalide_quota() {
    let mock = MockProvider::demarrer().await;
    let (r, _) = monter("rech_pannes", &mock, None, &[("a.md", "du texte")]).await;
    mock.panne(Panne::Cle401);
    let e = r.chercher(&entree("texte")).await.unwrap_err();
    assert!(matches!(e, ErreurRecherche::Provider(_)));
    assert!(e.to_string().contains("clé invalide"), "{e}");
    mock.panne(Panne::Quota429);
    let e = r.chercher(&entree("texte")).await.unwrap_err();
    assert!(e.to_string().contains("quota"), "{e}");
    mock.arreter();
    let e = r.chercher(&entree("texte")).await.unwrap_err();
    assert!(e.to_string().contains("injoignable"), "{e}");

    // Reranker en panne, embeddings disponibles.
    let emb = MockProvider::demarrer().await;
    let rr = MockProvider::demarrer().await;
    let (r2, _) = monter("rech_pannes_rr", &emb, Some(&rr), &[("a.md", "du texte")]).await;
    rr.panne(Panne::Cle401);
    let e = r2.chercher(&entree("texte")).await.unwrap_err();
    assert!(e.to_string().contains("clé invalide"), "{e}");
}

#[tokio::test]
async fn req014_dossiers_configures_imbriques_ne_dupliquent_pas_les_passages() {
    let mock = MockProvider::demarrer().await;
    let racine = dossier_test("search_imbr_notes");
    let interne = racine.join("x");
    std::fs::create_dir_all(interne.join("x")).unwrap();
    std::fs::write(interne.join("x/n.md"), "# N\n\ntarte aux pommes").unwrap();
    std::fs::write(interne.join("m.md"), "# M\n\ntarte aux poires").unwrap();
    let donnees = dossier_test("search_imbr_donnees");
    let dossiers = vec![racine.clone(), interne.clone()];
    let mut idx = Indexeur::new(
        Store::ouvrir(&donnees, "m").unwrap(),
        Provider::new(&mock.url, None, "m"),
        dossiers.clone(),
    );
    idx.synchroniser().await.unwrap();
    let r = Recherche::new(
        Store::ouvrir(&donnees, "m").unwrap(),
        Provider::new(&mock.url, None, "m"),
        None,
        dossiers,
    )
    .unwrap();
    // « x » désigne <racine>/x et <racine>/x/x : le passage de x/x/n.md correspond aux deux.
    // Un doublon dans les candidats gonflerait son score : il doit être celui d'un seul dossier.
    let seul = Recherche::new(
        Store::ouvrir(&donnees, "m").unwrap(),
        Provider::new(&mock.url, None, "m"),
        None,
        vec![racine.clone()],
    )
    .unwrap();
    let mut e = entree("tarte");
    e.dossier = Some("x".into());
    e.nombre = Some(10);
    let res = r.chercher(&e).await.unwrap();
    let attendu = seul.chercher(&e).await.unwrap();
    assert_eq!(attendu.len(), 2);
    assert_eq!(res, attendu);
}

#[tokio::test]
async fn req021_ac3_rangs_scores_fusion_et_reranker() {
    let mock = MockProvider::demarrer().await;
    mock.imposer_vecteur("sujet zorglub", base(0, 1, 0.0));
    mock.imposer_vecteur("alpha sujet", base(0, 1, 0.1));
    mock.imposer_vecteur("beta sujet", base(0, 1, 0.5));
    // Loin en vecteur (hors des 50 voisins), mais seul à contenir le mot « zorglub ».
    mock.imposer_vecteur("zorglub", base(5, 6, 0.0));
    let remplissage: Vec<(String, String)> = (0..51)
        .map(|i| {
            let t = format!("remplissage{i}");
            mock.imposer_vecteur(&t, base(0, 1, 1.0 + i as f32));
            (format!("r{i}.md"), t)
        })
        .collect();
    let mut notes: Vec<(&str, &str)> = vec![
        ("a.md", "alpha sujet"),
        ("b.md", "beta sujet"),
        ("c.md", "zorglub"),
    ];
    notes.extend(remplissage.iter().map(|(a, b)| (a.as_str(), b.as_str())));
    let (r, _) = monter("det_rangs", &mock, None, &notes).await;
    let mut e = entree("sujet zorglub");
    e.nombre = Some(50);
    let d = r.chercher_detail(&e).await.unwrap();
    let simple = r.chercher(&e).await.unwrap();
    let rs: Vec<_> = d.passages.iter().map(|p| p.resultat.clone()).collect();
    assert_eq!(rs, simple);
    let zorglub = d
        .passages
        .iter()
        .find(|p| p.resultat.texte == "zorglub")
        .unwrap();
    assert_eq!(zorglub.rang_sens, None);
    assert_eq!(zorglub.rang_mots, Some(1));
    let alpha = d
        .passages
        .iter()
        .find(|p| p.resultat.texte == "alpha sujet")
        .unwrap();
    assert_eq!(alpha.rang_sens, Some(1));
    assert!(alpha.rang_mots.is_some_and(|n| n >= 1));
    assert!(alpha.reranker.is_none());
    assert!((alpha.fusion - alpha.resultat.score).abs() < 1e-12);
    assert!(d.durees.reranker.is_none());

    // Reranker activé.
    mock.imposer_score("beta sujet", 0.9);
    mock.imposer_score("alpha sujet", 0.5);
    mock.imposer_score("zorglub", 0.1);
    let (avec, _) = monter("det_rangs_rr", &mock, Some(&mock), &notes).await;
    let d2 = avec.chercher_detail(&e).await.unwrap();
    let textes: Vec<&str> = d2
        .passages
        .iter()
        .map(|p| p.resultat.texte.as_str())
        .collect();
    assert_eq!(textes[..3], ["beta sujet", "alpha sujet", "zorglub"]);
    assert!((d2.passages[0].reranker.unwrap() - 0.9).abs() < 1e-6);
    assert!((d2.passages[0].resultat.score - 0.9).abs() < 1e-6);
    let beta_sans = d
        .passages
        .iter()
        .find(|p| p.resultat.texte == "beta sujet")
        .unwrap();
    assert!((d2.passages[0].fusion - beta_sans.fusion).abs() < 1e-12);
    assert!(d2.durees.reranker.is_some());
}

#[tokio::test]
async fn req021_ac4_durees_mesurees_avec_delai_du_provider() {
    let mock = MockProvider::demarrer().await;
    let notes = [("a.md", "alpha sujet"), ("b.md", "beta sujet")];
    let (r, _) = monter("det_durees", &mock, Some(&mock), &notes).await;
    mock.retarder(50);
    let d = r.chercher_detail(&entree("sujet")).await.unwrap();
    let ms50 = std::time::Duration::from_millis(50);
    assert!(d.durees.embedding >= ms50, "{:?}", d.durees);
    assert!(d.durees.reranker.unwrap() >= ms50, "{:?}", d.durees);
    assert!(d.durees.total >= d.durees.embedding + d.durees.reranker.unwrap());
}
