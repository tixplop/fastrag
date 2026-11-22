//! API HTTP : `POST /search`, `POST /reindex`, `GET /status` (REQ-016).

use crate::search::{EntreeRecherche, ErreurRecherche};
use crate::service::{Reindexation, Service};
use crate::web::{ErreurVue, EtatVue, Page, PassageVue, Repartition, ResultatsVue};
use askama::Template;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub fn routeur(service: Arc<Service>) -> Router {
    Router::new()
        .route("/search", post(chercher))
        .route("/reindex", post(reindexer))
        .route("/status", get(etat))
        .route("/", get(page))
        .route("/style.css", get(style))
        .with_state(service)
}

/// Erreur renvoyée en JSON `{"error", "field"}`.
fn erreur(code: StatusCode, message: String, champ: Option<&str>) -> Response {
    (code, Json(json!({"error": message, "field": champ}))).into_response()
}

fn entree_invalide(message: &str, champ: &str) -> ErreurRecherche {
    ErreurRecherche::Entree {
        message: message.into(),
        champ: champ.into(),
    }
}

/// Lit le corps d'une recherche et nomme le champ fautif (JSON invalide : champ `corps`).
pub fn lire_entree(corps: &[u8]) -> Result<EntreeRecherche, ErreurRecherche> {
    let v: Value = serde_json::from_slice(corps)
        .map_err(|e| entree_invalide(&format!("JSON invalide : {e}"), "corps"))?;
    let Some(o) = v.as_object() else {
        return Err(entree_invalide("le corps doit être un objet JSON", "corps"));
    };
    let texte = |nom: &str| -> Result<Option<String>, ErreurRecherche> {
        match o.get(nom) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(entree_invalide("doit être un texte", nom)),
        }
    };
    let question = match o.get("question") {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => return Err(entree_invalide("question manquante", "question")),
        Some(_) => return Err(entree_invalide("doit être un texte", "question")),
    };
    let nombre = match o.get("nombre") {
        None | Some(Value::Null) => None,
        Some(n) => Some(
            n.as_i64()
                .ok_or_else(|| entree_invalide("doit être un nombre entier", "nombre"))?,
        ),
    };
    Ok(EntreeRecherche {
        question,
        nombre,
        dossier: texte("dossier")?,
        tag: texte("tag")?,
    })
}

async fn chercher(State(service): State<Arc<Service>>, corps: Bytes) -> Response {
    let resultat = match lire_entree(&corps) {
        Ok(e) => service.chercher(&e).await,
        Err(e) => Err(e),
    };
    match resultat {
        Ok(r) => Json(json!({ "resultats": r })).into_response(),
        Err(ErreurRecherche::Entree { message, champ }) => {
            erreur(StatusCode::BAD_REQUEST, message, Some(&champ))
        }
        Err(e @ ErreurRecherche::Provider(_)) => {
            tracing::warn!("recherche impossible : {e}");
            erreur(StatusCode::BAD_GATEWAY, e.to_string(), None)
        }
        Err(e) => {
            tracing::error!("recherche impossible : {e}");
            erreur(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), None)
        }
    }
}

async fn reindexer(State(service): State<Arc<Service>>) -> Response {
    let texte = match service.reindexer() {
        Reindexation::Lancee => "lancée",
        Reindexation::DejaEnCours => "déjà en cours",
    };
    Json(json!({ "statut": texte })).into_response()
}

async fn etat(State(service): State<Arc<Service>>) -> Response {
    match service.etat() {
        Ok(e) => Json(e).into_response(),
        Err(e) => erreur(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("erreur de l'index : {e}"),
            None,
        ),
    }
}

async fn style() -> Response {
    (
        [(CONTENT_TYPE, "text/css; charset=utf-8")],
        crate::web::STYLE_CSS,
    )
        .into_response()
}

/// Page de test (REQ-021, REQ-022) : paramètres lus tels quels dans l'adresse.
async fn page(
    State(service): State<Arc<Service>>,
    Query(p): Query<HashMap<String, String>>,
) -> Response {
    let texte = |nom: &str| p.get(nom).cloned().unwrap_or_default();
    let etat = match service.etat() {
        Ok(e) => e,
        Err(e) => {
            return erreur(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("erreur de l'index : {e}"),
                None,
            )
        }
    };
    let mut vue = Page {
        question: texte("question"),
        nombre: p.get("nombre").cloned().unwrap_or_else(|| "5".into()),
        dossier: texte("dossier"),
        tag: texte("tag"),
        erreur: None,
        resultats: None,
        etat: EtatVue::new(etat),
    };
    if p.contains_key("question") {
        match rechercher(&service, &vue).await {
            Ok(r) => vue.resultats = Some(r),
            Err(e) => {
                vue.erreur = Some(match e {
                    ErreurRecherche::Entree { message, champ } => {
                        ErreurVue::new(message, Some(&champ))
                    }
                    e => ErreurVue::new(e.to_string(), None),
                })
            }
        }
    }
    match vue.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => erreur(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("page illisible : {e}"),
            None,
        ),
    }
}

async fn rechercher(service: &Service, vue: &Page) -> Result<ResultatsVue, ErreurRecherche> {
    let nombre = match vue.nombre.trim() {
        "" => None,
        n => Some(
            n.parse::<i64>()
                .map_err(|_| entree_invalide("doit être un nombre entier", "nombre"))?,
        ),
    };
    let filtre = |v: &str| (!v.is_empty()).then(|| v.to_string());
    let detail = service
        .chercher_detail(&EntreeRecherche {
            question: vue.question.clone(),
            nombre,
            dossier: filtre(&vue.dossier),
            tag: filtre(&vue.tag),
        })
        .await?;
    let d = detail.durees;
    let attente = d.embedding + d.reranker.unwrap_or_default();
    Ok(ResultatsVue {
        resume: crate::web::resume(detail.passages.len(), d.reranker.is_some()),
        total: crate::web::duree(d.total),
        attente: crate::web::duree(attente),
        repartition: Repartition::new(d.total, d.embedding, d.reranker),
        passages: detail
            .passages
            .into_iter()
            .enumerate()
            .map(|(i, p)| PassageVue {
                rang: i + 1,
                titres: p.resultat.titres,
                fichier: p.resultat.chemin,
                debut: p.resultat.ligne_debut as usize,
                fin: p.resultat.ligne_fin as usize,
                tags: p.resultat.tags,
                fusion: p.fusion,
                rang_sens: p.rang_sens,
                rang_mots: p.rang_mots,
                reranker: p.reranker,
                texte: p.resultat.texte,
            })
            .collect(),
    })
}
