//! Client du provider : embeddings (format OpenAI) et reranking (format Cohere/Jina) (REQ-006, REQ-015).

use serde::Deserialize;
use std::fmt;
use std::time::Duration;

/// Nombre maximal de textes par appel à `/embeddings`.
pub const TAILLE_LOT: usize = 64;

/// Erreur du provider. Le message nomme la cause et ne contient jamais la clé.
#[derive(Debug, PartialEq)]
pub enum ErreurProvider {
    Reseau(String),
    CleInvalide(u16),
    Quota,
    ReponseInvalide(String),
}

impl fmt::Display for ErreurProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reseau(d) => write!(f, "provider injoignable (réseau) : {d}"),
            Self::CleInvalide(c) => write!(f, "clé du provider refusée (clé invalide, code {c})"),
            Self::Quota => write!(f, "quota du provider dépassé (code 429)"),
            Self::ReponseInvalide(d) => write!(f, "réponse invalide du provider : {d}"),
        }
    }
}

impl std::error::Error for ErreurProvider {}

pub struct Provider {
    http: reqwest::Client,
    url: String,
    cle: Option<String>,
    modele: String,
}

#[derive(Deserialize)]
struct RepEmbeddings {
    data: Vec<LigneEmbedding>,
}
#[derive(Deserialize)]
struct LigneEmbedding {
    index: usize,
    embedding: Vec<f32>,
}
#[derive(Deserialize)]
struct RepRerank {
    results: Vec<LigneRerank>,
}
#[derive(Deserialize)]
struct LigneRerank {
    index: usize,
    relevance_score: f32,
}

impl Provider {
    /// `url` : base du provider (sans `/embeddings`). `cle` : valeur de la clé, si elle existe.
    pub fn new(url: &str, cle: Option<String>, modele: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("client HTTP");
        Self {
            http,
            url: url.trim_end_matches('/').to_string(),
            cle: cle.filter(|c| !c.is_empty()),
            modele: modele.to_string(),
        }
    }

    /// Vecteurs des textes, dans le même ordre, par lots de 64 au plus.
    pub async fn embeddings(&self, textes: &[String]) -> Result<Vec<Vec<f32>>, ErreurProvider> {
        let mut sortie = Vec::with_capacity(textes.len());
        for lot in textes.chunks(TAILLE_LOT) {
            let corps = serde_json::json!({"model": self.modele, "input": lot});
            let rep: RepEmbeddings = self.appeler("embeddings", corps).await?;
            if rep.data.len() != lot.len() {
                return Err(ErreurProvider::ReponseInvalide(format!(
                    "{} vecteurs reçus pour {} textes",
                    rep.data.len(),
                    lot.len()
                )));
            }
            let mut vecteurs: Vec<Option<Vec<f32>>> = vec![None; lot.len()];
            for l in rep.data {
                match vecteurs.get_mut(l.index) {
                    Some(c @ None) if !l.embedding.is_empty() => *c = Some(l.embedding),
                    _ => {
                        return Err(ErreurProvider::ReponseInvalide(
                            "index ou vecteur incorrect".into(),
                        ))
                    }
                }
            }
            sortie.extend(vecteurs.into_iter().flatten());
        }
        Ok(sortie)
    }

    /// Scores de pertinence `(index du document, score)`, dans l'ordre renvoyé par le reranker.
    pub async fn rerank(
        &self,
        requete: &str,
        documents: &[String],
    ) -> Result<Vec<(usize, f32)>, ErreurProvider> {
        let corps = serde_json::json!({
            "model": self.modele, "query": requete, "documents": documents,
        });
        let rep: RepRerank = self.appeler("rerank", corps).await?;
        if rep.results.iter().any(|r| r.index >= documents.len()) {
            return Err(ErreurProvider::ReponseInvalide("index hors limites".into()));
        }
        Ok(rep
            .results
            .into_iter()
            .map(|r| (r.index, r.relevance_score))
            .collect())
    }

    async fn appeler<T: serde::de::DeserializeOwned>(
        &self,
        chemin: &str,
        corps: serde_json::Value,
    ) -> Result<T, ErreurProvider> {
        let mut req = self
            .http
            .post(format!("{}/{chemin}", self.url))
            .json(&corps);
        if let Some(c) = &self.cle {
            req = req.bearer_auth(c);
        }
        let rep = req
            .send()
            .await
            .map_err(|e| ErreurProvider::Reseau(e.without_url().to_string()))?;
        let code = rep.status().as_u16();
        match code {
            401 | 403 => return Err(ErreurProvider::CleInvalide(code)),
            429 => return Err(ErreurProvider::Quota),
            200..=299 => {}
            _ => return Err(ErreurProvider::ReponseInvalide(format!("code HTTP {code}"))),
        }
        rep.json::<T>()
            .await
            .map_err(|e| ErreurProvider::ReponseInvalide(e.without_url().to_string()))
    }
}
