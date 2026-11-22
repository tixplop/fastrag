use crate::common::mock_provider::{vecteur_defaut, MockProvider, Panne};
use fastrag::provider::{ErreurProvider, Provider};

fn textes(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("passage numéro {i}")).collect()
}

#[tokio::test]
async fn req006_ac1_lots_de_64_au_plus() {
    let m = MockProvider::demarrer().await;
    let p = Provider::new(&m.url, Some("cle".into()), "bge-m3");
    let t = textes(150);
    let v = p.embeddings(&t).await.unwrap();
    assert_eq!(m.tailles_lots(), vec![64, 64, 22]);
    assert_eq!(v.len(), 150);
    assert_eq!(v[0], vecteur_defaut(&t[0]));
    assert_eq!(v[149], vecteur_defaut(&t[149]));
    assert_eq!(m.textes_recus(), t);
    assert_eq!(m.autorisations()[0], "Bearer cle");
    // Lot exact de 64 : un seul appel ; liste vide : aucun appel.
    let m2 = MockProvider::demarrer().await;
    let p2 = Provider::new(&m2.url, None, "bge-m3");
    p2.embeddings(&textes(64)).await.unwrap();
    assert_eq!(m2.tailles_lots(), vec![64]);
    assert!(p2.embeddings(&[]).await.unwrap().is_empty());
    assert_eq!(m2.tailles_lots(), vec![64]);
}

#[tokio::test]
async fn req006_vecteurs_imposes() {
    let m = MockProvider::demarrer().await;
    m.imposer_vecteur("budget", vec![1.0, 0.0]);
    let p = Provider::new(&m.url, None, "m");
    let v = p
        .embeddings(&["budget".into(), "autre".into()])
        .await
        .unwrap();
    assert_eq!(v[0], vec![1.0, 0.0]);
    assert_eq!(v[1], vecteur_defaut("autre"));
}

#[tokio::test]
async fn req015_rerank_format_cohere() {
    let m = MockProvider::demarrer().await;
    m.imposer_score("b", 0.9);
    m.imposer_score("a", 0.1);
    let p = Provider::new(&m.url, None, "r");
    let r = p
        .rerank("q", &["a".into(), "b".into(), "c".into()])
        .await
        .unwrap();
    assert_eq!(r[0], (1, 0.9));
    assert_eq!(r.len(), 3);
    assert_eq!(m.appels_rerank(), 1);
}

#[tokio::test]
async fn req015_ac5_chaque_cause_est_nommee() {
    let secret = "sk-tres-secret";
    for (panne, attendu) in [
        (Panne::Cle401, "clé invalide"),
        (Panne::Quota429, "quota"),
        (Panne::Invalide, "réponse invalide"),
    ] {
        let m = MockProvider::demarrer().await;
        m.panne(panne);
        let p = Provider::new(&m.url, Some(secret.into()), "m");
        let e1 = p.embeddings(&textes(2)).await.unwrap_err();
        let e2 = p.rerank("q", &textes(2)).await.unwrap_err();
        for e in [e1, e2] {
            let msg = e.to_string();
            assert!(msg.contains(attendu), "{msg}");
            assert!(!msg.contains(secret), "{msg}");
            assert!(!format!("{e:?}").contains(secret));
        }
    }
    let m = MockProvider::demarrer().await;
    let p = Provider::new(&m.url, Some(secret.into()), "m");
    m.arreter();
    tokio::task::yield_now().await;
    let e = p.embeddings(&textes(1)).await.unwrap_err();
    assert!(matches!(e, ErreurProvider::Reseau(_)), "{e:?}");
    assert!(e.to_string().contains("réseau"));
    assert!(!e.to_string().contains(secret));
    let e = p.rerank("q", &textes(1)).await.unwrap_err();
    assert!(matches!(e, ErreurProvider::Reseau(_)));
}

#[tokio::test]
async fn req006_reponse_incomplete_refusee() {
    // Le mock ne renvoie jamais un nombre de vecteurs différent : on vérifie via une URL qui n'est pas un provider.
    let m = MockProvider::demarrer().await;
    let p = Provider::new(&format!("{}/inexistant", m.url), None, "m");
    let e = p.embeddings(&textes(1)).await.unwrap_err();
    assert!(matches!(e, ErreurProvider::ReponseInvalide(_)), "{e:?}");
}
