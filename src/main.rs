use fastrag::config::Config;
use std::path::PathBuf;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let chemin = match args.as_slice() {
        [] => None,
        [a] if a == "--version" => {
            println!("fastrag {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        [a] if a == "update" => return ExitCode::from(fastrag::update::lancer().await),
        [opt, p] if opt == "--config" => Some(PathBuf::from(p)),
        _ => {
            eprintln!("Usage : fastrag [--config <fichier>] | fastrag update | fastrag --version");
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
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    match fastrag::service::demarrer(&config).await {
        Ok(mut service) => {
            println!("Service prêt sur http://{}", service.adresse);
            service.attendre().await;
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("Démarrage impossible : {e}");
            ExitCode::FAILURE
        }
    }
}
