//! Page de test `GET /` (REQ-021, REQ-022) : service en processus, provider simulé.

use crate::api::{config, ecrire};
use crate::common::mock_provider::MockProvider;
use fastrag::service::{demarrer, ServiceEnCours};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::time::{Duration, Instant};

fn sel(s: &str) -> Selector {
    Selector::parse(s).unwrap()
}

async fn texte(s: &ServiceEnCours, chemin: &str) -> String {
    let r = reqwest::get(format!("http://{}{chemin}", s.adresse))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.text().await.unwrap()
}

async fn page(s: &ServiceEnCours, chemin: &str) -> Html {
    Html::parse_document(&texte(s, chemin).await)
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

async fn monter(
    nom: &str,
    mock: &MockProvider,
    notes: &[(&str, &str)],
    reranker: bool,
) -> ServiceEnCours {
    let dossier = ecrire(nom, notes);
    let mut c = config(nom, mock, vec![dossier]);
    if reranker {
        c.reranker.actif = true;
        c.reranker.url = mock.url.clone();
        c.reranker.modele = "r".into();
    }
    let s = demarrer(&c).await.unwrap();
    attendre_repos(&s).await;
    s
}

/// Texte d'un élément, espaces réduits.
fn t(e: ElementRef) -> String {
    e.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Valeur de la mesure `libelle` (dt/dd) dans `racine`.
fn mesure(racine: ElementRef, libelle: &str) -> String {
    for c in racine.select(&sel(".chiffre")) {
        let dt = t(c.select(&sel("dt")).next().unwrap());
        if dt == libelle {
            return t(c.select(&sel("dd")).next().unwrap());
        }
    }
    panic!("mesure « {libelle} » absente");
}

fn notes_tartes() -> Vec<(&'static str, &'static str)> {
    vec![
        ("a.md", "# Tartes\n\nrecette de la tarte aux pommes"),
        ("b.md", "# Voyage\n\nbillet de train pour Lyon"),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac1_ac8_accueil_formulaire_et_feuille_de_style() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_accueil", &mock, &notes_tartes(), false).await;
    let brut = texte(&s, "/").await;
    let h = Html::parse_document(&brut);
    assert_eq!(h.root_element().value().attr("lang"), Some("fr"));
    assert_eq!(
        t(h.select(&sel("title")).next().unwrap()),
        "FastRAG — page de test"
    );
    let h1: Vec<_> = h.select(&sel("h1")).collect();
    assert_eq!(h1.len(), 1);
    assert_eq!(t(h1[0]), "FastRAG");
    let form = h.select(&sel("form")).next().unwrap();
    assert_eq!(form.value().attr("method"), Some("get"));
    assert_eq!(form.value().attr("action"), Some("/"));
    // Ordre de tabulation : Question, Chercher, Nombre, Dossier, Tag, rien d'autre.
    let focalisables: Vec<_> = h
        .select(&sel("input, button, select, textarea, a[href], [tabindex]"))
        .map(|e| {
            e.value()
                .attr("name")
                .map(str::to_string)
                .unwrap_or_else(|| t(e))
        })
        .collect();
    assert_eq!(
        focalisables,
        ["question", "Chercher", "nombre", "dossier", "tag"]
    );
    // Libellés reliés, aucun attribut de validation.
    for (nom, libelle) in [
        ("question", "Question"),
        ("nombre", "Nombre de résultats"),
        ("dossier", "Dossier (facultatif)"),
        ("tag", "Tag (facultatif)"),
    ] {
        let i = h
            .select(&sel(&format!("input[name={nom}]")))
            .next()
            .unwrap();
        let v = i.value();
        let id = v.attr("id").unwrap();
        let l = h.select(&sel(&format!("label[for={id}]"))).next().unwrap();
        assert_eq!(t(l), libelle);
        assert_eq!(v.attr("type"), Some("text"));
        for a in ["required", "min", "max"] {
            assert!(v.attr(a).is_none(), "{nom} porte {a}");
        }
        assert_eq!(v.attr("autofocus").is_some(), nom == "question");
    }
    let nombre = h.select(&sel("input[name=nombre]")).next().unwrap();
    assert_eq!(nombre.value().attr("value"), Some("5"));
    let b = h.select(&sel("button")).next().unwrap();
    assert_eq!(b.value().attr("type"), Some("submit"));
    assert!(brut.contains("Saisissez une question, puis choisissez « Chercher »."));
    assert_eq!(h.select(&sel("ol.passages, li.passage")).count(), 0);
    assert_eq!(h.select(&sel(".resume")).count(), 0);
    // Feuille de style servie par le binaire, aucune ressource externe.
    let css = reqwest::get(format!("http://{}/style.css", s.adresse))
        .await
        .unwrap();
    assert_eq!(css.status(), 200);
    assert!(css.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    assert!(css.text().await.unwrap().contains(":root"));
    assert!(brut.contains("href=\"/style.css\""));
    assert!(!brut.contains("http://") && !brut.contains("https://"));
    assert_eq!(h.select(&sel("script")).count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac2_ac9_memes_passages_que_post_search_et_formulaire_garde() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_passages", &mock, &notes_tartes(), false).await;
    let r: Value = reqwest::Client::new()
        .post(format!("http://{}/search", s.adresse))
        .body(r#"{"question":"tarte","nombre":2,"tag":null}"#)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let h = page(&s, "/?question=tarte&nombre=2&dossier=&tag=").await;
    let li: Vec<_> = h.select(&sel("ol.passages > li.passage")).collect();
    let attendu = r["resultats"].as_array().unwrap();
    assert_eq!(li.len(), attendu.len());
    assert_eq!(li.len(), 2);
    for (i, (e, a)) in li.iter().zip(attendu).enumerate() {
        assert_eq!(
            t(e.select(&sel(".passage__rang")).next().unwrap()),
            format!("#{}", i + 1)
        );
        assert_eq!(
            t(e.select(&sel("h3")).next().unwrap()),
            a["titres"].as_str().unwrap()
        );
        assert_eq!(
            t(e.select(&sel("code")).next().unwrap()),
            a["chemin"].as_str().unwrap()
        );
        assert_eq!(
            t(e.select(&sel(".passage__source span")).next().unwrap()),
            format!("lignes {} à {}", a["ligne_debut"], a["ligne_fin"])
        );
        assert_eq!(
            e.select(&sel(".passage__texte"))
                .next()
                .unwrap()
                .text()
                .collect::<String>(),
            a["texte"].as_str().unwrap()
        );
    }
    assert_eq!(
        t(h.select(&sel(".resume")).next().unwrap()),
        "2 passages · ordre : score de fusion"
    );
    // Le formulaire garde les valeurs reçues, telles quelles.
    let h = page(
        &s,
        "/?question=a%22%3Cb%3E&nombre=7&dossier=travail%2F&tag=x",
    )
    .await;
    let val = |n: &str| {
        h.select(&sel(&format!("input[name={n}]")))
            .next()
            .unwrap()
            .value()
            .attr("value")
            .unwrap()
            .to_string()
    };
    assert_eq!(val("question"), "a\"<b>");
    assert_eq!(val("nombre"), "7");
    assert_eq!(val("dossier"), "travail/");
    assert_eq!(val("tag"), "x");
    // Singulier, tags et chemin de titres vide.
    let h = page(&s, "/?question=tarte&nombre=1").await;
    assert_eq!(
        t(h.select(&sel(".resume")).next().unwrap()),
        "1 passage · ordre : score de fusion"
    );
    // Dossier inconnu : aucun passage, sans erreur.
    let h = page(&s, "/?question=tarte&nombre=5&dossier=inconnu&tag=").await;
    assert_eq!(h.select(&sel("li.passage")).count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac2_tags_et_titre_sans_titre() {
    let mock = MockProvider::demarrer().await;
    let s = monter(
        "web_tags",
        &mock,
        &[
            ("nu.md", "texte sans titre ni tag"),
            (
                "t.md",
                "---\ntags: [recette, dessert]\n---\n# Tarte\n\nrecette de tarte",
            ),
        ],
        false,
    )
    .await;
    let h = page(&s, "/?question=tarte&nombre=5&dossier=&tag=recette").await;
    let li = h.select(&sel("li.passage")).next().unwrap();
    let mut tags: Vec<_> = li.select(&sel(".etiquette")).map(t).collect();
    tags.sort();
    assert_eq!(tags, ["dessert", "recette"]);
    let h = page(&s, "/?question=texte&nombre=5&dossier=&tag=").await;
    let titres: Vec<_> = h.select(&sel("li.passage h3")).map(t).collect();
    assert!(
        titres.contains(&"nu.md (sans titre)".to_string()),
        "{titres:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac3_rangs_scores_et_absent() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_scores", &mock, &notes_tartes(), false).await;
    let d = s
        .service
        .chercher_detail(&fastrag::search::EntreeRecherche {
            question: "tarte".into(),
            nombre: Some(5),
            dossier: None,
            tag: None,
        })
        .await
        .unwrap();
    let h = page(&s, "/?question=tarte&nombre=5&dossier=&tag=").await;
    let li: Vec<_> = h.select(&sel("li.passage")).collect();
    assert_eq!(li.len(), d.passages.len());
    let mut vu_absent = false;
    for (e, p) in li.iter().zip(&d.passages) {
        assert_eq!(mesure(*e, "Fusion"), format!("{:.4}", p.fusion));
        let rang = |r: Option<usize>| r.map_or("absent".to_string(), |r| r.to_string());
        assert_eq!(mesure(*e, "Rang sens"), rang(p.rang_sens));
        assert_eq!(mesure(*e, "Rang mots"), rang(p.rang_mots));
        vu_absent |= p.rang_mots.is_none() && mesure(*e, "Rang mots") == "absent";
        assert!(e
            .select(&sel(".chiffre--principal dt"))
            .map(t)
            .eq(["Fusion".to_string()]));
    }
    assert!(
        vu_absent,
        "un passage devait être absent de la recherche par mots"
    );

    // Reranker activé : score en premier, ordre du reranker.
    let mock = MockProvider::demarrer().await;
    let s = monter(
        "web_scores_rr",
        &mock,
        &[
            ("a.md", "recette de la tarte aux pommes"),
            ("b.md", "billet de train pour Lyon"),
        ],
        true,
    )
    .await;
    mock.imposer_score("recette de la tarte aux pommes", 0.1);
    mock.imposer_score("billet de train pour Lyon", 0.9);
    let h = page(&s, "/?question=tarte&nombre=5&dossier=&tag=").await;
    let li: Vec<_> = h.select(&sel("li.passage")).collect();
    assert_eq!(li.len(), 2);
    assert_eq!(mesure(li[0], "Reranker"), "0.9000");
    assert_eq!(mesure(li[1], "Reranker"), "0.1000");
    let ordre: Vec<_> = li[0].select(&sel(".chiffre dt")).map(t).collect();
    assert_eq!(ordre, ["Reranker", "Fusion", "Rang sens", "Rang mots"]);
    assert!(li[0]
        .select(&sel(".chiffre--principal dt"))
        .map(t)
        .eq(["Reranker".to_string()]));
    assert_eq!(
        t(h.select(&sel(".resume")).next().unwrap()),
        "2 passages · ordre : score du reranker"
    );
    assert_ne!(
        mesure(h.select(&sel(".repartition")).next().unwrap(), "Reranker"),
        "désactivé"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac4_ac10_durees_et_repartition() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_durees", &mock, &notes_tartes(), false).await;
    mock.retarder(60);
    let h = page(&s, "/?question=tarte&nombre=5&dossier=&tag=").await;
    let ms = |txt: String| -> u64 {
        if txt == "< 1 ms" {
            0
        } else {
            txt.trim_end_matches(" ms").parse().unwrap()
        }
    };
    let section = h
        .select(&sel("section[aria-labelledby=titre-temps]"))
        .next()
        .unwrap();
    assert_eq!(
        t(section.select(&sel("h3")).next().unwrap()),
        "Temps de réponse"
    );
    let total = ms(mesure(section, "Durée totale"));
    let attente = ms(mesure(section, "Attente du provider"));
    let emb = ms(mesure(section, "Embedding de la question"));
    let local = ms(mesure(section, "Recherche locale"));
    assert!(emb >= 60 && attente >= 60, "emb {emb} attente {attente}");
    assert!(total >= attente);
    // Chaque durée est tronquée à la milliseconde : un écart d'une unité au plus.
    assert!(
        local.abs_diff(total - attente) <= 1,
        "local {local} total {total} attente {attente}"
    );
    assert_eq!(mesure(section, "Reranker"), "désactivé");
    assert_eq!(
        section
            .select(&sel(".repartition__barre"))
            .next()
            .unwrap()
            .value()
            .attr("aria-hidden"),
        Some("true")
    );
    assert!(section.select(&sel(".repartition__barre span")).count() >= 1);
    assert!(t(section)
        .ends_with("Mesurées par le service, de la réception de la question à la réponse."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac5_ac11_erreurs_champ_et_provider() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_erreurs", &mock, &notes_tartes(), false).await;
    let cas = [
        (
            "/?question=&nombre=5",
            "question",
            "question vide",
            "Corrigez le champ « Question », puis relancez la recherche.",
        ),
        (
            "/?question=tarte&nombre=0",
            "nombre",
            "le nombre de résultats doit être supérieur à 0",
            "Corrigez le champ « Nombre de résultats », puis relancez la recherche.",
        ),
        (
            "/?question=tarte&nombre=abc",
            "nombre",
            "doit être un nombre entier",
            "Corrigez le champ « Nombre de résultats », puis relancez la recherche.",
        ),
    ];
    for (adresse, champ, message, marche) in cas {
        let h = page(&s, adresse).await;
        let alerte = h.select(&sel("[role=alert]")).next().unwrap();
        let txt = t(alerte);
        assert!(txt.contains("La recherche n'a pas abouti"), "{txt}");
        assert!(txt.contains(marche), "{txt}");
        assert!(
            t(h.select(&sel("#erreur-recherche")).next().unwrap()).contains(message),
            "{adresse}"
        );
        assert_eq!(h.select(&sel("[aria-invalid=true]")).count(), 1);
        let i = h.select(&sel("[aria-invalid=true]")).next().unwrap();
        assert_eq!(i.value().attr("name"), Some(champ));
        assert!(i.value().attr("autofocus").is_some());
        assert!(i
            .value()
            .attr("aria-describedby")
            .unwrap()
            .split(' ')
            .any(|x| x == "erreur-recherche"));
        assert_eq!(
            h.select(&sel(".resume, li.passage, #titre-temps")).count(),
            0
        );
    }
    // Le message de l'API est repris mot pour mot, et la valeur invalide reste dans le champ.
    let h = page(&s, "/?question=tarte&nombre=abc").await;
    assert_eq!(
        h.select(&sel("input[name=nombre]"))
            .next()
            .unwrap()
            .value()
            .attr("value"),
        Some("abc")
    );
    // Provider arrêté : autre erreur, aucun champ en erreur.
    mock.arreter();
    let h = page(&s, "/?question=tarte&nombre=5").await;
    let txt = t(h.select(&sel("[role=alert]")).next().unwrap());
    assert!(txt.contains("La recherche n'a pas abouti"));
    assert!(txt.contains("provider injoignable"), "{txt}");
    assert!(txt.contains("Relancez la recherche. Si l'erreur revient, vérifiez que le provider répond et que sa clé est valide."));
    assert_eq!(h.select(&sel("[aria-invalid]")).count(), 0);
    assert_eq!(
        h.select(&sel(".resume, li.passage, #titre-temps")).count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac6_ac12_aucun_passage() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_vide", &mock, &notes_tartes(), false).await;
    let h = page(&s, "/?question=tarte&nombre=5&dossier=inconnu&tag=").await;
    let st = t(h.select(&sel("[role=status]")).next().unwrap());
    assert!(st.contains("Aucun passage trouvé."));
    assert!(st.contains("L'index est vide, ou aucun passage ne passe les filtres. Retirez un filtre ou reformulez la question."));
    assert_eq!(
        t(h.select(&sel(".resume")).next().unwrap()),
        "0 passage · ordre : score de fusion"
    );
    assert_eq!(h.select(&sel("#titre-temps")).count(), 1);
    assert_eq!(h.select(&sel("[role=alert], li.passage")).count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req021_ac7_html_du_passage_et_des_avertissements_non_interprete() {
    let mock = MockProvider::demarrer().await;
    let s = monter(
        "web_html",
        &mock,
        &[(
            "x.md",
            "# Piège\n\nvoici <script>alert(1)</script> et <b>gras</b> piege",
        )],
        false,
    )
    .await;
    let brut = texte(&s, "/?question=piege&nombre=5&dossier=&tag=").await;
    assert!(!brut.contains("<script"));
    assert!(!brut.contains("<b>gras"));
    let h = Html::parse_document(&brut);
    assert_eq!(h.select(&sel("script, .passage__texte b")).count(), 0);
    let txt = h
        .select(&sel(".passage__texte"))
        .next()
        .unwrap()
        .text()
        .collect::<String>();
    assert!(txt.contains("<script>alert(1)</script> et <b>gras</b>"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req022_ac1_ac2_ac3_etat_identique_a_status() {
    let mock = MockProvider::demarrer().await;
    let s = monter("web_etat", &mock, &notes_tartes(), false).await;
    // Un avertissement : dossier introuvable ajouté à la configuration.
    let dossier = ecrire("web_etat_avert", &[("ok.md", "# Ok\n\ntexte")]);
    let mut c = config(
        "web_etat2",
        &mock,
        vec![dossier, "/inexistant/dossier-<b>".into()],
    );
    c.reessai_secondes = 1;
    let s2 = demarrer(&c).await.unwrap();
    attendre_repos(&s2).await;
    for serv in [&s, &s2] {
        let j: Value = reqwest::get(format!("http://{}/status", serv.adresse))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let h = page(serv, "/").await;
        let sec = h
            .select(&sel("section[aria-labelledby=titre-etat]"))
            .next()
            .unwrap();
        assert_eq!(t(sec.select(&sel("h2")).next().unwrap()), "État de l'index");
        assert_eq!(mesure(sec, "Notes"), j["notes"].to_string());
        assert_eq!(mesure(sec, "Passages"), j["passages"].to_string());
        assert_eq!(
            mesure(sec, "Indexation en cours"),
            if j["en_cours"] == true { "Oui" } else { "Non" }
        );
        let tm = sec.select(&sel("time")).next().unwrap();
        let brut = j["derniere_mise_a_jour"].as_str().unwrap();
        assert_eq!(tm.value().attr("datetime"), Some(brut));
        assert_eq!(t(tm), format!("{} {} UTC", &brut[..10], &brut[11..19]));
        assert!(t(sec).ends_with(
            "État lu au chargement de la page. Rechargez la page pour le mettre à jour."
        ));
        let avert: Vec<String> = j["avertissements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        let titre = t(sec.select(&sel("h3")).next().unwrap());
        assert_eq!(titre, format!("Avertissements ({})", avert.len()));
        let li: Vec<_> = sec.select(&sel("ul > li")).map(t).collect();
        assert_eq!(li, avert);
        if avert.is_empty() {
            assert!(t(sec).contains("Aucun avertissement."));
        }
    }
    let j = s2.service.etat().unwrap();
    assert!(
        !j.avertissements.is_empty(),
        "le dossier introuvable doit donner un avertissement"
    );
}
