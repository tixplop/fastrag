//! Serveur de releases simulé (port 0) : JSON de la dernière release et fichier du binaire.

use axum::{routing::get, Router};

pub struct MockReleases {
    pub url: String,
    tache: tokio::task::JoinHandle<()>,
}

impl MockReleases {
    /// `json` : corps de la réponse de l'API ; `{ASSET}` y est remplacé par le lien de l'asset.
    /// `asset` : contenu servi sur `/asset`.
    pub async fn demarrer(json: &str, asset: &str) -> Self {
        let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", ecoute.local_addr().unwrap());
        let json = json.replace("{ASSET}", &format!("{base}/asset"));
        let asset = asset.to_string();
        let app = Router::new()
            .route("/latest", get(move || async move { json }))
            .route("/asset", get(move || async move { asset }));
        let tache = tokio::spawn(async move {
            axum::serve(ecoute, app).await.unwrap();
        });
        Self {
            url: format!("{base}/latest"),
            tache,
        }
    }

    /// Arrête le serveur et attend que son port soit fermé.
    pub async fn arreter(&mut self) {
        self.tache.abort();
        let _ = (&mut self.tache).await;
    }
}

impl Drop for MockReleases {
    fn drop(&mut self) {
        self.tache.abort();
    }
}

/// JSON d'une release `tag` avec l'asset `fastrag-linux-x86_64` (ou `nom`).
pub fn release(tag: &str, nom: &str) -> String {
    format!(
        "{{\"tag_name\":\"{tag}\",\"assets\":[{{\"name\":\"{nom}\",\"browser_download_url\":\"{{ASSET}}\"}}]}}"
    )
}

pub fn script(version: &str) -> String {
    format!("#!/bin/sh\necho \"fastrag {version}\"\n")
}
