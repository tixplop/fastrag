//! Provider simulé : `/embeddings` et `/rerank`, vecteurs déterministes, pannes programmables.

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const DIM: usize = 8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Panne {
    Aucune,
    Cle401,
    Quota429,
    /// Corps qui n'est pas du JSON attendu.
    Invalide,
}

#[derive(Default)]
struct Etat {
    dim: usize,
    panne: Option<Panne>,
    /// Taille de chaque lot reçu par `/embeddings`.
    lots: Vec<usize>,
    textes: Vec<String>,
    appels_rerank: usize,
    vecteurs: HashMap<String, Vec<f32>>,
    scores: HashMap<String, f32>,
    autorisations: Vec<String>,
}

pub struct MockProvider {
    pub url: String,
    etat: Arc<Mutex<Etat>>,
    adresse: std::net::SocketAddr,
    tache: Mutex<tokio::task::JoinHandle<()>>,
}

/// Vecteur déterministe tiré de l'empreinte du texte.
pub fn vecteur_defaut(texte: &str) -> Vec<f32> {
    vecteur_dim(texte, DIM)
}

/// Vecteur déterministe de `dim` dimensions (flux étendu de l'empreinte).
pub fn vecteur_dim(texte: &str, dim: usize) -> Vec<f32> {
    let mut octets = vec![0u8; dim];
    blake3::Hasher::new()
        .update(texte.as_bytes())
        .finalize_xof()
        .fill(&mut octets);
    octets.iter().map(|b| *b as f32 / 255.0 + 0.01).collect()
}

async fn gerer_panne(
    e: &Arc<Mutex<Etat>>,
    h: &axum::http::HeaderMap,
) -> Option<axum::response::Response> {
    let mut g = e.lock().unwrap();
    g.autorisations.push(
        h.get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string(),
    );
    match g.panne.unwrap_or(Panne::Aucune) {
        Panne::Aucune => None,
        Panne::Cle401 => Some(StatusCode::UNAUTHORIZED.into_response()),
        Panne::Quota429 => Some(StatusCode::TOO_MANY_REQUESTS.into_response()),
        Panne::Invalide => Some((StatusCode::OK, "pas du json").into_response()),
    }
}

async fn embeddings(
    State(e): State<Arc<Mutex<Etat>>>,
    h: axum::http::HeaderMap,
    Json(corps): Json<Value>,
) -> axum::response::Response {
    let entrees: Vec<String> = corps["input"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    {
        let mut g = e.lock().unwrap();
        g.lots.push(entrees.len());
        g.textes.extend(entrees.iter().cloned());
    }
    if let Some(r) = gerer_panne(&e, &h).await {
        return r;
    }
    let g = e.lock().unwrap();
    let data: Vec<Value> = entrees
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let v = g
                .vecteurs
                .get(t)
                .cloned()
                .unwrap_or_else(|| vecteur_dim(t, g.dim));
            json!({"index": i, "embedding": v})
        })
        .collect();
    Json(json!({"data": data})).into_response()
}

async fn rerank(
    State(e): State<Arc<Mutex<Etat>>>,
    h: axum::http::HeaderMap,
    Json(corps): Json<Value>,
) -> axum::response::Response {
    e.lock().unwrap().appels_rerank += 1;
    if let Some(r) = gerer_panne(&e, &h).await {
        return r;
    }
    let g = e.lock().unwrap();
    let docs: Vec<&str> = corps["documents"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let mut res: Vec<(usize, f32)> = docs
        .iter()
        .enumerate()
        .map(|(i, d)| (i, g.scores.get(*d).copied().unwrap_or(0.0)))
        .collect();
    res.sort_by(|a, b| b.1.total_cmp(&a.1));
    let results: Vec<Value> = res
        .into_iter()
        .map(|(i, s)| json!({"index": i, "relevance_score": s}))
        .collect();
    Json(json!({"results": results})).into_response()
}

fn construire(etat: Arc<Mutex<Etat>>) -> Router {
    Router::new()
        .route("/embeddings", post(embeddings))
        .route("/rerank", post(rerank))
        // Pas de connexion réutilisée : après `arreter()`, le port refuse les connexions.
        .layer(axum::middleware::map_response(
            |mut r: axum::response::Response| async move {
                r.headers_mut()
                    .insert("connection", "close".parse().unwrap());
                r
            },
        ))
        .with_state(etat)
}

impl MockProvider {
    pub async fn demarrer() -> Self {
        Self::demarrer_dim(DIM).await
    }

    /// Provider dont les vecteurs par défaut ont `dim` dimensions.
    pub async fn demarrer_dim(dim: usize) -> Self {
        let etat = Arc::new(Mutex::new(Etat {
            dim,
            ..Etat::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = listener.local_addr().unwrap();
        let app = construire(etat.clone());
        let tache = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{adresse}"),
            etat,
            adresse,
            tache: Mutex::new(tache),
        }
    }

    /// Panne réseau : le serveur n'écoute plus.
    pub fn arreter(&self) {
        self.tache.lock().unwrap().abort();
    }

    /// Retour du provider : écoute de nouveau sur la même adresse.
    pub async fn relancer(&self) {
        self.arreter();
        let mut essais = 0;
        let listener = loop {
            match tokio::net::TcpListener::bind(self.adresse).await {
                Ok(l) => break l,
                Err(e) if essais >= 250 => panic!("port {} non libéré : {e}", self.adresse),
                Err(_) => {
                    essais += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await
                }
            }
        };
        let app = construire(self.etat.clone());
        *self.tache.lock().unwrap() = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    pub fn panne(&self, p: Panne) {
        self.etat.lock().unwrap().panne = Some(p);
    }

    pub fn imposer_vecteur(&self, texte: &str, v: Vec<f32>) {
        self.etat.lock().unwrap().vecteurs.insert(texte.into(), v);
    }

    pub fn imposer_score(&self, texte: &str, s: f32) {
        self.etat.lock().unwrap().scores.insert(texte.into(), s);
    }

    pub fn tailles_lots(&self) -> Vec<usize> {
        self.etat.lock().unwrap().lots.clone()
    }

    pub fn textes_recus(&self) -> Vec<String> {
        self.etat.lock().unwrap().textes.clone()
    }

    pub fn appels_rerank(&self) -> usize {
        self.etat.lock().unwrap().appels_rerank
    }

    pub fn autorisations(&self) -> Vec<String> {
        self.etat.lock().unwrap().autorisations.clone()
    }
}
