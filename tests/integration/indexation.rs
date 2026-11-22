use crate::common::dossier_test;
use crate::common::mock_provider::MockProvider;
use fastrag::index::{ErreurIndex, Indexeur};
use fastrag::provider::Provider;
use fastrag::store::Store;
use std::path::{Path, PathBuf};

fn ecrire(d: &Path, rel: &str, contenu: &str) -> PathBuf {
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

const CUISINE: &str = "# Cuisine\n\n## Tarte\n\nPommes et pâte.\n\n## Soupe\n\nLégumes et eau.\n";

#[tokio::test]
async fn req002_ac1_extensions_et_sous_dossiers_dans_un_seul_index() {
    let mock = MockProvider::demarrer().await;
    let a = dossier_test("idx_ext_a");
    let b = dossier_test("idx_ext_b");
    ecrire(&a, "x.md", "un");
    ecrire(&a, "sous/y.markdown", "deux");
    ecrire(&a, "sous/profond/z.txt", "trois");
    ecrire(&a, "image.png", "pas une note");
    ecrire(&a, "notes.pdf", "pas une note");
    ecrire(&b, "w.md", "quatre");
    let mut idx = indexeur("idx_ext", &mock, vec![a.clone(), b.clone()]);
    idx.synchroniser().await.unwrap();
    let chemins = idx.store().chemins_notes().unwrap();
    let attendus: Vec<String> = [
        a.join("sous/profond/z.txt"),
        a.join("sous/y.markdown"),
        a.join("x.md"),
        b.join("w.md"),
    ]
    .iter()
    .map(|p| p.display().to_string())
    .collect();
    let mut tries = attendus.clone();
    tries.sort();
    assert_eq!(chemins, tries);
}

#[tokio::test]
async fn req002_ac2_dossiers_caches_ignores() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_caches");
    ecrire(&d, "visible.md", "visible");
    ecrire(&d, ".git/objet.md", "caché");
    ecrire(&d, ".obsidian/conf.md", "caché");
    ecrire(&d, "a/.cache/profond/x.txt", "caché");
    ecrire(&d, "a/b.md", "visible aussi");
    let mut idx = indexeur("idx_caches", &mock, vec![d.clone()]);
    idx.synchroniser().await.unwrap();
    let chemins = idx.store().chemins_notes().unwrap();
    assert_eq!(chemins.len(), 2, "{chemins:?}");
    let racine = d.display().to_string();
    assert!(
        chemins.iter().all(|c| !c[racine.len()..].contains("/.")),
        "{chemins:?}"
    );
}

#[tokio::test]
async fn req002_ac3_dossier_inexistant_avertit_et_les_autres_sont_indexes() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_inexistant");
    ecrire(&d, "ok.md", "bonjour");
    let absent = d.join("n-existe-pas");
    let mut idx = indexeur("idx_inexistant", &mock, vec![absent.clone(), d.clone()]);
    idx.synchroniser().await.unwrap();
    assert_eq!(idx.store().chemins_notes().unwrap().len(), 1);
    let av = idx.avertissements();
    assert_eq!(av.len(), 1, "{av:?}");
    assert!(av[0].contains("n-existe-pas"), "{av:?}");
}

#[tokio::test]
async fn req006_ac2_toucher_sans_changer_le_contenu_ne_fait_aucun_appel() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_toucher");
    let f = ecrire(&d, "cuisine.md", CUISINE);
    let mut idx = indexeur("idx_toucher", &mock, vec![d.clone()]);
    let r = idx.synchroniser().await.unwrap();
    assert_eq!(r.nouvelles, 1);
    let avant = mock.textes_recus().len();
    assert!(avant > 0);
    std::fs::write(&f, CUISINE).unwrap(); // même contenu, date nouvelle
    let r = idx.synchroniser().await.unwrap();
    assert_eq!((r.nouvelles, r.modifiees, r.inchangees), (0, 0, 1));
    assert_eq!(mock.textes_recus().len(), avant);
}

#[tokio::test]
async fn req006_ac3_modifier_une_section_n_envoie_que_les_textes_nouveaux() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_modif");
    let f = ecrire(&d, "cuisine.md", CUISINE);
    ecrire(&d, "autre.md", "# Autre\n\nRien ne change ici.\n");
    let mut idx = indexeur("idx_modif", &mock, vec![d.clone()]);
    idx.synchroniser().await.unwrap();
    let avant = mock.textes_recus().len();
    std::fs::write(
        &f,
        CUISINE.replace("Légumes et eau.", "Légumes, eau et sel."),
    )
    .unwrap();
    let r = idx.synchroniser().await.unwrap();
    assert_eq!((r.modifiees, r.inchangees), (1, 1));
    let recus = mock.textes_recus();
    let nouveaux = &recus[avant..];
    assert_eq!(nouveaux.len(), 1, "{nouveaux:?}");
    assert!(nouveaux[0].contains("sel"), "{nouveaux:?}");
    let ps = idx.store().passages_note(&f.display().to_string()).unwrap();
    assert!(ps.iter().any(|p| p.texte.contains("sel")));
    assert!(ps.iter().any(|p| p.texte.contains("Pommes")));
}

#[tokio::test]
async fn req006_ac4_renommer_n_envoie_rien_et_le_chemin_change() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_renom");
    let f = ecrire(&d, "cuisine.md", CUISINE);
    let mut idx = indexeur("idx_renom", &mock, vec![d.clone()]);
    idx.synchroniser().await.unwrap();
    let avant = mock.textes_recus().len();
    let g = d.join("recettes.md");
    std::fs::rename(&f, &g).unwrap();
    let r = idx.synchroniser().await.unwrap();
    assert_eq!((r.renommees, r.nouvelles, r.supprimees), (1, 0, 0));
    assert_eq!(mock.textes_recus().len(), avant);
    assert_eq!(
        idx.store().chemins_notes().unwrap(),
        vec![g.display().to_string()]
    );
    assert!(!idx
        .store()
        .passages_note(&g.display().to_string())
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn suppression_et_indexeur_unique() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_suppr");
    let f = ecrire(&d, "a.md", "contenu");
    let mut idx = indexeur("idx_suppr", &mock, vec![d.clone()]);
    idx.synchroniser().await.unwrap();
    std::fs::remove_file(&f).unwrap();
    let r = idx.synchroniser().await.unwrap();
    assert_eq!(r.supprimees, 1);
    assert!(idx.store().chemins_notes().unwrap().is_empty());
    // Indicateur déjà pris : refus explicite, puis libération.
    let ind = idx.indicateur_en_cours();
    ind.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        idx.synchroniser().await,
        Err(ErreurIndex::DejaEnCours)
    ));
    ind.store(false, std::sync::atomic::Ordering::SeqCst);
    idx.synchroniser().await.unwrap();
    assert!(!ind.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn req002_ac1_lien_symbolique_en_boucle_ne_bloque_pas() {
    let mock = MockProvider::demarrer().await;
    let d = dossier_test("idx_boucle");
    ecrire(&d, "notes/a.md", "contenu");
    std::os::unix::fs::symlink(&d, d.join("notes/boucle")).unwrap();
    let mut idx = indexeur("idx_boucle", &mock, vec![d.clone()]);
    idx.synchroniser().await.unwrap();
    assert_eq!(idx.store().chemins_notes().unwrap().len(), 1);
}
