use fastrag::config::Config;
use fastrag::index::{pending::synchroniser_avec_reprise, Indexeur};
use fastrag::provider::Provider;
use fastrag::store::{Store, Verrou};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let chemin = match args.as_slice() {
        [] => None,
        [opt, p] if opt == "--config" => Some(PathBuf::from(p)),
        _ => {
            eprintln!("Usage : fastrag [--config <fichier>]");
            return ExitCode::from(2);
        }
    };
    let config = match Config::charger(chemin.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Configuration chargée : port {}", config.port);
    match indexer(&config).await {
        Ok(()) => {
            println!("Indexation terminée");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Rattrapage complet au démarrage (REQ-008 AC-3).
async fn indexer(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let _verrou = Verrou::prendre(&config.dossier_donnees)?;
    let store = Store::ouvrir(&config.dossier_donnees, &config.embedding.modele)?;
    let cle = std::env::var(&config.embedding.cle_env).ok();
    let provider = Provider::new(&config.embedding.url, cle, &config.embedding.modele);
    let mut indexeur = Indexeur::new(store, provider, config.dossiers.clone());
    synchroniser_avec_reprise(&mut indexeur, Duration::from_secs(config.reessai_secondes)).await?;
    Ok(())
}
