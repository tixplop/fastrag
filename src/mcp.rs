//! Serveur MCP (Streamable HTTP) monté sur `/mcp` : outils `search` et `status` (REQ-018).
//! Ils appellent les mêmes fonctions que l'API, avec les mêmes messages d'erreur.

use crate::search::{EntreeRecherche, ErreurRecherche};
use crate::service::Service;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
pub struct ServeurMcp {
    service: Arc<Service>,
    tool_router: ToolRouter<Self>,
}

impl ServeurMcp {
    pub fn new(service: Arc<Service>) -> Self {
        Self {
            service,
            tool_router: Self::tool_router(),
        }
    }
}

fn texte_json(v: serde_json::Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        v.to_string(),
    )]))
}

#[tool_router]
impl ServeurMcp {
    #[tool(
        description = "Cherche dans les notes les passages qui répondent à une question. Rend le fichier, le chemin de titres et le texte de chaque passage."
    )]
    async fn search(
        &self,
        Parameters(entree): Parameters<EntreeRecherche>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.service.chercher(&entree).await {
            Ok(r) => texte_json(json!({ "resultats": r })),
            Err(e @ ErreurRecherche::Entree { .. }) => {
                Err(ErrorData::invalid_params(e.to_string(), None))
            }
            Err(e) => {
                tracing::warn!("recherche impossible : {e}");
                Err(ErrorData::internal_error(e.to_string(), None))
            }
        }
    }

    #[tool(
        description = "État de l'index : nombre de notes et de passages, dernière mise à jour, indexation en cours, avertissements."
    )]
    async fn status(&self) -> Result<CallToolResult, ErrorData> {
        match self.service.etat() {
            Ok(e) => match serde_json::to_value(e) {
                Ok(v) => texte_json(v),
                Err(e) => Err(ErrorData::internal_error(e.to_string(), None)),
            },
            Err(e) => Err(ErrorData::internal_error(
                ErreurRecherche::Store(e).to_string(),
                None,
            )),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ServeurMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Service HTTP à monter sur `/mcp`.
pub fn service_http(
    service: Arc<Service>,
) -> StreamableHttpService<ServeurMcp, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(ServeurMcp::new(service.clone())),
        Default::default(),
        StreamableHttpServerConfig::default(),
    )
}
