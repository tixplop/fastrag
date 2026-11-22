use crate::api::ecrire;
use crate::common::dossier_test;
use crate::common::mock_provider::{MockProvider, Panne};
use fastrag::config::Config;
use fastrag::service::{demarrer, ServiceEnCours};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceError, ServiceExt};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

type Client = rmcp::service::RunningService<RoleClient, ClientConfig>;

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

async fn client(s: &ServiceEnCours) -> Client {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", s.adresse)),
    );
    ClientConfig::default().serve(transport).await.unwrap()
}

async fn appeler(c: &Client, outil: &str, args: Value) -> Result<CallToolResult, ServiceError> {
    let args = args.as_object().cloned().unwrap_or_default();
    c.call_tool(CallToolRequestParams::new(outil.to_string()).with_arguments(args))
        .await
}

/// Contenu texte d'un résultat d'outil, relu comme JSON.
fn json_de(r: &CallToolResult) -> Value {
    let t = r.content[0].as_text().expect("contenu texte");
    serde_json::from_str(&t.text).unwrap()
}

async fn attendre_repos(s: &ServiceEnCours) {
    let limite = Instant::now() + Duration::from_secs(20);
    loop {
        let e = s.service.etat().unwrap();
        if !e.en_cours && e.derniere_mise_a_jour.is_some() {
            return;
        }
        assert!(Instant::now() < limite, "indexation trop longue");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req018_ac1_ac2_ac3_search_status_et_erreurs() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire(
        "mcp_outils",
        &[
            (
                "a.md",
                "---\ntags: [recette]\n---\n# Tartes\n\n## Pommes\n\nrecette de la tarte aux pommes",
            ),
            ("b.md", "# Voyage\n\nbillet de train pour Lyon"),
        ],
    );
    std::fs::create_dir_all(notes.join("sous")).unwrap();
    std::fs::write(notes.join("sous/c.md"), "# Sous\n\nbillet de tarte").unwrap();
    let s = demarrer(&config("mcp_outils", &mock, vec![notes]))
        .await
        .unwrap();
    attendre_repos(&s).await;
    let c = client(&s).await;

    // AC-018-1 : les outils sont annoncés, search rend fichier et chemin de titres.
    let outils = c.list_all_tools().await.unwrap();
    let noms: Vec<_> = outils.iter().map(|o| o.name.to_string()).collect();
    assert!(noms.contains(&"search".to_string()) && noms.contains(&"status".to_string()));
    let r = appeler(
        &c,
        "search",
        json!({"question": "tarte aux pommes", "nombre": 2}),
    )
    .await
    .unwrap();
    let v = json_de(&r);
    let premier = &v["resultats"][0];
    assert!(premier["chemin"].as_str().unwrap().ends_with("a.md"));
    assert!(premier["titres"].as_str().unwrap().contains("Pommes"));
    let r = appeler(
        &c,
        "search",
        json!({"question": "train", "dossier": "inconnu"}),
    )
    .await
    .unwrap();
    assert_eq!(json_de(&r)["resultats"], json!([]));
    let r = appeler(&c, "search", json!({"question": "tarte", "tag": "absent"}))
        .await
        .unwrap();
    assert_eq!(json_de(&r)["resultats"], json!([]));
    // Filtres qui gardent une note sur trois.
    for (filtre, fichier) in [
        (json!({"tag": "recette"}), "a.md"),
        (json!({"dossier": "sous"}), "c.md"),
    ] {
        let mut args = json!({"question": "tarte"});
        args.as_object_mut()
            .unwrap()
            .extend(filtre.as_object().unwrap().clone());
        let v = json_de(&appeler(&c, "search", args).await.unwrap());
        let liste = v["resultats"].as_array().unwrap();
        assert!(!liste.is_empty(), "filtre {filtre} : {v}");
        assert!(liste
            .iter()
            .all(|r| r["chemin"].as_str().unwrap().ends_with(fichier)));
    }

    // AC-018-2 : status = état de l'index.
    let r = appeler(&c, "status", json!({})).await.unwrap();
    let e = serde_json::to_value(s.service.etat().unwrap()).unwrap();
    assert_eq!(json_de(&r), e);
    assert_eq!(e["notes"], 3);

    // AC-018-3 : mêmes messages que l'API.
    for (args, attendu) in [
        (json!({"question": ""}), "question vide"),
        (
            json!({"question": "x", "nombre": 0}),
            "le nombre de résultats doit être supérieur à 0",
        ),
        (
            json!({"question": "x", "nombre": -3}),
            "le nombre de résultats doit être supérieur à 0",
        ),
    ] {
        match appeler(&c, "search", args).await {
            Err(ServiceError::McpError(e)) => {
                assert!(e.message.contains(attendu), "message : {}", e.message)
            }
            autre => panic!("erreur MCP attendue, reçu {autre:?}"),
        }
    }
    let _ = c.cancel().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req018_ac4_deux_clients_cherchent_pendant_une_indexation() {
    let mock = MockProvider::demarrer().await;
    let notes = ecrire(
        "mcp_deux",
        &[("a.md", "# A\n\nrecette de la tarte aux pommes")],
    );
    let mut conf = config("mcp_deux", &mock, vec![notes.clone()]);
    conf.reessai_secondes = 3;
    let s = demarrer(&conf).await.unwrap();
    attendre_repos(&s).await;
    let avant = s.service.etat().unwrap();
    // Une note nouvelle, provider en quota : l'indexation reste en cours pendant les recherches.
    std::fs::write(notes.join("b.md"), "# B\n\nseconde note").unwrap();
    mock.panne(Panne::Quota429);
    s.service.reindexer();
    assert!(s.service.etat().unwrap().en_cours);
    let (c1, c2) = (client(&s).await, client(&s).await);
    mock.panne(Panne::Aucune);
    for _ in 0..3 {
        let (r1, r2) = tokio::join!(
            appeler(&c1, "search", json!({"question": "tarte"})),
            appeler(&c2, "search", json!({"question": "tarte"})),
        );
        for r in [r1.unwrap(), r2.unwrap()] {
            assert_ne!(r.is_error, Some(true));
            assert!(!json_de(&r)["resultats"].as_array().unwrap().is_empty());
        }
    }
    // Les recherches ont bien eu lieu pendant l'indexation.
    assert!(s.service.etat().unwrap().en_cours);
    let st = appeler(&c1, "status", json!({})).await.unwrap();
    assert!(json_de(&st)["notes"].as_u64().unwrap() as usize >= avant.notes);
    attendre_repos(&s).await;
    // Les recherches n'ont rien changé à l'index : les deux notes y sont, une fois chacune.
    let apres = s.service.etat().unwrap();
    assert_eq!(apres.notes, 2);
    let _ = (c1.cancel().await, c2.cancel().await);
}
