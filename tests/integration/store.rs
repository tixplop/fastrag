use crate::common::dossier_test;
use fastrag::store::{Filtres, Note, Passage, Store};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn passage(texte: &str, emp: &str) -> (Passage, Option<Vec<f32>>) {
    (
        Passage {
            texte: texte.into(),
            titres: "T".into(),
            ligne_debut: 1,
            ligne_fin: 2,
            empreinte: emp.into(),
        },
        Some(vec![1.0, 0.0]),
    )
}

fn note(chemin: &str, tags: &[&str]) -> Note {
    Note {
        chemin: chemin.into(),
        empreinte: "e".into(),
        titre: "Titre".into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

#[test]
fn req010_ac1_lecteur_pendant_remplacement() {
    let d = dossier_test("store_wal");
    let mut ecrivain = Store::ouvrir(&d, "m").unwrap();
    let lecteur = Store::ouvrir(&d, "m").unwrap();
    let n = note("/a.md", &[]);
    let version = |v: &str, k: usize| -> Vec<_> {
        (0..k)
            .map(|i| passage(&format!("{v} {i}"), &format!("{v}{i}")))
            .collect()
    };
    ecrivain.remplacer_note(&n, &version("A", 3)).unwrap();

    let fin = Arc::new(AtomicBool::new(false));
    let fin2 = fin.clone();
    let th = std::thread::spawn(move || {
        let mut lectures = 0;
        while !fin2.load(Ordering::Relaxed) {
            let ps = lecteur.passages_note("/a.md").unwrap();
            let a = ps.iter().filter(|p| p.texte.starts_with('A')).count();
            let b = ps.iter().filter(|p| p.texte.starts_with('B')).count();
            assert!(
                (a == 3 && b == 0) || (a == 0 && b == 5),
                "mélange vu : {a} anciens, {b} nouveaux"
            );
            let c = lecteur.mots("A OR B", &Filtres::default()).unwrap();
            let ca = c.iter().filter(|x| x.texte.starts_with('A')).count();
            let cb = c.iter().filter(|x| x.texte.starts_with('B')).count();
            assert!((ca == 3 && cb == 0) || (ca == 0 && cb == 5), "mélange FTS");
            lectures += 1;
        }
        lectures
    });
    for i in 0..200 {
        let (v, k) = if i % 2 == 0 { ("B", 5) } else { ("A", 3) };
        ecrivain.remplacer_note(&n, &version(v, k)).unwrap();
    }
    fin.store(true, Ordering::Relaxed);
    assert!(th.join().unwrap() > 0);
}

#[test]
fn store_voisins_filtres_suppression_renommage() {
    let d = dossier_test("store_vec");
    let mut s = Store::ouvrir(&d, "m").unwrap();
    s.remplacer_note(&note("/w/a.md", &["recette"]), &[passage("alpha", "ha")])
        .unwrap();
    let mut proche = passage("beta", "hb");
    proche.1 = Some(vec![0.0, 1.0]);
    s.remplacer_note(&note("/x/b.md", &[]), &[proche]).unwrap();
    assert_eq!(s.dimension().unwrap(), Some(2));

    let v = s.voisins(&[1.0, 0.1], &Filtres::default()).unwrap();
    assert_eq!(v[0].chemin, "/w/a.md");
    let f = Filtres {
        dossier: Some("/x/".into()),
        tag: None,
    };
    assert_eq!(s.voisins(&[1.0, 0.1], &f).unwrap().len(), 1);
    let f = Filtres {
        dossier: None,
        tag: Some("recette".into()),
    };
    assert_eq!(s.mots("alpha beta", &f).unwrap().len(), 1);
    assert_eq!(
        s.vecteurs_connus(&["ha".into(), "zz".into()]).unwrap(),
        vec!["ha".to_string()]
    );

    assert!(s.renommer_note("/w/a.md", "/w/c.md").unwrap());
    assert_eq!(s.passages_note("/w/c.md").unwrap().len(), 1);
    // Renommage vers un chemin déjà indexé : l'ancienne note disparaît entièrement.
    assert!(s.renommer_note("/w/c.md", "/x/b.md").unwrap());
    assert_eq!(s.chemins_notes().unwrap(), vec!["/x/b.md".to_string()]);
    assert_eq!(s.mots("beta", &Filtres::default()).unwrap().len(), 0);
    assert_eq!(s.mots("alpha", &Filtres::default()).unwrap().len(), 1);
    s.remplacer_note(&note("/y/n.md", &[]), &[passage("gamma", "hg")])
        .unwrap();
    assert_eq!(s.mots("gamma", &Filtres::default()).unwrap().len(), 1);
    assert!(s.supprimer_note("/x/b.md").unwrap());
    assert!(s.mots("alpha", &Filtres::default()).unwrap().is_empty());
    assert!(s
        .remplacer_note(
            &note("/x/c.md", &[]),
            &[(passage("g", "g").0, Some(vec![1.0]))]
        )
        .is_err());
}

#[test]
fn store_verrou_exclusif() {
    let d = dossier_test("store_verrou");
    let _v = fastrag::store::Verrou::prendre(&d).unwrap();
    assert!(fastrag::store::Verrou::prendre(&d).is_err());
}
