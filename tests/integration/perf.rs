use crate::common::dossier_test;
use crate::common::mock_provider::MockProvider;
use crate::common::GardeEnfant;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const NOTES: usize = 10_000;
const MOTS: [&str; 16] = [
    "jardin",
    "recette",
    "voyage",
    "budget",
    "projet",
    "lecture",
    "musique",
    "atelier",
    "cuisine",
    "marché",
    "réunion",
    "carnet",
    "randonnée",
    "photo",
    "voiture",
    "santé",
];

fn contenu_note(i: usize) -> String {
    let mut s = format!("# Note {i}\n\n");
    for j in 0..5 {
        let a = MOTS[(i + j) % MOTS.len()];
        let b = MOTS[(i / 7 + j * 3) % MOTS.len()];
        s.push_str(&format!(
            "## Partie {j}\n\n{a} {b} numéro {i} section {j}\n\n"
        ));
    }
    s
}

fn rss_mo(pid: u32) -> f64 {
    let t = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let ko: f64 = t
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .expect("VmRSS introuvable");
    ko / 1024.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serie_req020_ac1_ac2_dix_mille_notes_memoire_et_recherche() {
    let mock = MockProvider::demarrer_dim(1024).await;
    let notes = dossier_test("perf_notes");
    for i in 0..NOTES {
        std::fs::write(notes.join(format!("n{i}.md")), contenu_note(i)).unwrap();
    }
    let donnees = dossier_test("perf_donnees");
    let conf = dossier_test("perf_conf").join("config.toml");
    std::fs::write(
        &conf,
        format!(
            "dossiers = [{:?}]\ndossier_donnees = {:?}\nport = 0\nreessai_secondes = 1\n[embedding]\nurl = {:?}\nmodele = \"m\"\ncle_env = \"TEMOIN_CLE\"\n",
            notes.display().to_string(),
            donnees.display().to_string(),
            mock.url
        ),
    )
    .unwrap();
    let mut enfant = GardeEnfant::new(
        Command::new(env!("CARGO_BIN_EXE_fastrag"))
            .arg("--config")
            .arg(&conf)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut ligne = String::new();
    BufReader::new(enfant.stdout.as_mut().unwrap())
        .read_line(&mut ligne)
        .unwrap();
    let adresse = ligne
        .trim()
        .strip_prefix("Service prêt sur http://")
        .expect("le service n'a pas démarré")
        .to_string();
    let url = |c: &str| format!("http://{adresse}{c}");

    let debut = Instant::now();
    loop {
        let e: serde_json::Value = reqwest::get(url("/status"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if e["notes"] == NOTES && e["en_cours"] == false {
            println!(
                "indexation : {:?}, passages : {}",
                debut.elapsed(),
                e["passages"]
            );
            break;
        }
        assert!(
            debut.elapsed() < Duration::from_secs(45),
            "indexation trop lente : {e}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // AC-020-2 : mémoire au repos.
    let rss = rss_mo(enfant.id());
    println!("VmRSS : {rss:.0} Mo");

    // AC-020-1 : série de recherches.
    let client = reqwest::Client::new();
    let mut durees = Vec::new();
    for i in 0..20 {
        let q = format!(
            r#"{{"question":"{} {} numéro {}"}}"#,
            MOTS[i % 16],
            MOTS[(i * 5) % 16],
            i * 400
        );
        let t = Instant::now();
        let r = client.post(url("/search")).body(q).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let v: serde_json::Value = r.json().await.unwrap();
        durees.push(t.elapsed());
        println!("recherche {i} : {:?}", durees[i]);
        assert!(!v["resultats"].as_array().unwrap().is_empty());
    }
    durees.sort();
    println!(
        "recherche : médiane {:?}, maximum {:?}",
        durees[durees.len() / 2],
        durees[durees.len() - 1]
    );
    let rss_apres = rss_mo(enfant.id());
    println!("VmRSS après recherches : {rss_apres:.0} Mo");
    enfant.kill().unwrap();
    enfant.wait().unwrap();
    assert!(rss < 300.0, "VmRSS {rss:.0} Mo");
    assert!(
        rss_apres < 300.0,
        "VmRSS après recherches {rss_apres:.0} Mo"
    );
    assert!(
        *durees.last().unwrap() < Duration::from_millis(200),
        "recherche trop lente : {durees:?}"
    );
}
