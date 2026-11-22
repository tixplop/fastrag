use fastrag::config::Config;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let chemin = match args.as_slice() {
        [] => None,
        [opt, p] if opt == "--config" => Some(PathBuf::from(p)),
        _ => {
            eprintln!("Usage : fastrag [--config <fichier>]");
            return ExitCode::from(2);
        }
    };
    match Config::charger(chemin.as_deref()) {
        Ok(config) => {
            println!("Configuration chargée : port {}", config.port);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
