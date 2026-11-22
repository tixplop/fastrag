use super::*;
use askama::Template;
use scraper::{Html, Selector};
use std::collections::HashMap;

const COMPOSANTS: &str = include_str!("templates/composants.html");
const DEMO: &str = include_str!("templates/demo.html");

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn passage(
    rang: usize,
    titres: &str,
    tags: &[&str],
    reranker: Option<f64>,
    sens: Option<usize>,
) -> PassageVue {
    PassageVue {
        rang,
        titres: titres.to_string(),
        fichier: "/notes/cuisine/recettes.md".to_string(),
        debut: 3,
        fin: 9,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        fusion: 0.032_258,
        rang_sens: sens,
        rang_mots: None,
        reranker,
        texte: "<script>alert(1)</script> <b>gras</b>".to_string(),
    }
}

#[derive(Template)]
#[template(path = "demo.html")]
struct Demo {
    texte_hostile: String,
    rep_complete: Repartition,
    rep_sans_reranker: Repartition,
    rep_nulle: Repartition,
    passage_reranker: PassageVue,
    passage_simple: PassageVue,
}

fn demo() -> Html {
    let html = Demo {
        texte_hostile: "<img src=x onerror=alert(1)>".to_string(),
        rep_complete: Repartition::new(ms(200), ms(50), Some(ms(100))),
        rep_sans_reranker: Repartition::new(ms(168), ms(0), None),
        rep_nulle: Repartition::new(Duration::ZERO, Duration::ZERO, None),
        passage_reranker: passage(
            1,
            "Recettes > Tarte",
            &["cuisine", "dessert"],
            Some(0.9876),
            Some(2),
        ),
        passage_simple: passage(2, "", &[], None, None),
    }
    .render()
    .unwrap();
    Html::parse_document(&html)
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).unwrap()
}

fn texte(e: scraper::ElementRef) -> String {
    e.text()
        .flat_map(|t| t.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

// --- Formats ---

#[test]
fn formats_duree_score_largeur() {
    assert_eq!(duree(ms(168)), "168 ms");
    assert_eq!(duree(Duration::from_micros(900)), "< 1 ms");
    assert_eq!(duree(Duration::ZERO), "< 1 ms");
    assert_eq!(score(0.032_258), "0.0323");
    assert_eq!(score(1.0), "1.0000");
    assert_eq!(largeur(ms(839), ms(1000)), "83.9%");
    assert_eq!(largeur(ms(1), ms(3)), "33.3%");
    assert_eq!(largeur(ms(0), ms(0)), "0.0%");
}

// --- AC-021-1 : aucune ressource externe ---

#[test]
fn req021_ac1_feuille_embarquee_sans_ressource_externe() {
    assert!(!STYLE_CSS.is_empty());
    for (nom, contenu) in [
        ("style.css", STYLE_CSS),
        ("composants.html", COMPOSANTS),
        ("demo.html", DEMO),
    ] {
        for interdit in ["http", "@import", "url(", "//cdn"] {
            assert!(!contenu.contains(interdit), "{nom} contient {interdit}");
        }
    }
    let page = demo().html();
    assert!(!page.contains("http"));
    assert!(!page.contains("<script src") && !page.contains("<link") && !page.contains("<img"));
}

// --- Feuille de style ---

fn racine() -> HashMap<String, String> {
    let debut = STYLE_CSS.find(":root {").unwrap();
    let fin = debut + STYLE_CSS[debut..].find("\n}").unwrap();
    STYLE_CSS[debut..fin]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--"))
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (format!("--{k}"), v.trim().trim_end_matches(';').to_string()))
        .collect()
}

fn rgb(hex: &str) -> [f64; 3] {
    let h = hex.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
    [c(0), c(2), c(4)]
}

fn luminance(hex: &str) -> f64 {
    let [r, g, b] = rgb(hex).map(|v| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn contrastes_wcag_des_paires_de_tokens() {
    let t = racine();
    let paires: &[(&str, &str, f64)] = &[
        ("texte", "fond", 4.5),
        ("texte", "surface", 4.5),
        ("texte", "surface-2", 4.5),
        ("texte-attenue", "surface", 4.5),
        ("texte-attenue", "fond", 4.5),
        ("texte-attenue", "surface-2", 4.5),
        ("bordure-champ", "surface", 3.0),
        ("bordure-champ", "fond", 3.0),
        ("sur-accent", "accent", 4.5),
        ("sur-accent", "accent-survol", 4.5),
        ("accent", "surface", 4.5),
        ("accent", "accent-fond", 4.5),
        ("focus", "fond", 3.0),
        ("focus", "surface", 3.0),
        ("alerte", "alerte-fond", 4.5),
        ("texte", "alerte-fond", 4.5),
        ("erreur", "erreur-fond", 4.5),
        ("erreur", "surface", 4.5),
        ("texte", "erreur-fond", 4.5),
        ("temps-embedding", "surface", 3.0),
        ("temps-reranker", "surface", 3.0),
        ("temps-local", "surface", 3.0),
    ];
    for (a, b, min) in paires {
        let (va, vb) = (&t[&format!("--couleur-{a}")], &t[&format!("--couleur-{b}")]);
        let r = ratio(va, vb);
        assert!(r >= *min, "{a} ({va}) / {b} ({vb}) : {r:.2} < {min}");
    }
    // Valeurs du tableau du design system (tolérance d'arrondi).
    assert!((ratio(&t["--couleur-texte"], &t["--couleur-fond"]) - 15.11).abs() < 0.02);
    assert!((ratio(&t["--couleur-accent"], &t["--couleur-surface"]) - 6.39).abs() < 0.02);
}

#[test]
fn feuille_regles_d_accessibilite_et_rupture() {
    assert!(STYLE_CSS.contains(":focus-visible"));
    assert!(STYLE_CSS.contains("@media (prefers-reduced-motion: reduce)"));
    assert!(STYLE_CSS.contains("--duree-courte: 0ms"));
    assert!(STYLE_CSS.contains("@media (max-width: 720px)"));
}

#[test]
fn feuille_aucune_valeur_en_dur_hors_des_tokens() {
    let debut = STYLE_CSS.find(":root {").unwrap();
    let fin = debut + STYLE_CSS[debut..].find("\n}").unwrap() + 2;
    let reste = format!("{}{}", &STYLE_CSS[..debut], &STYLE_CSS[fin..]);
    for l in reste.lines() {
        let l = l.trim();
        if l.starts_with("@media") || l.starts_with("--duree-courte") || l.starts_with("/*") {
            continue;
        }
        assert!(
            !l.contains(": #") && !l.contains(" #"),
            "couleur en dur : {l}"
        );
        assert!(!l.contains("rgb"), "couleur en dur : {l}");
        for mot in l.split(|c: char| !c.is_ascii_alphanumeric() && c != '.') {
            let chiffres = mot.trim_end_matches(|c: char| c.is_ascii_alphabetic());
            let unite = &mot[chiffres.len()..];
            let est_nombre = !chiffres.is_empty() && chiffres.parse::<f64>().is_ok();
            if est_nombre && ["px", "em", "rem", "ms", "s"].contains(&unite) {
                assert!(
                    chiffres.parse::<f64>().unwrap() == 0.0,
                    "taille en dur : {l}"
                );
            }
        }
    }
    // Les variables utilisées existent toutes.
    let t = racine();
    let mut reste_vars = STYLE_CSS;
    while let Some(i) = reste_vars.find("var(--") {
        reste_vars = &reste_vars[i + 4..];
        let nom = &reste_vars[..reste_vars.find(')').unwrap()];
        assert!(t.contains_key(nom), "variable inconnue {nom}");
    }
}

// --- Composants ---

#[test]
fn composant_bouton_et_champs() {
    let d = demo();
    let b = d.select(&sel("button.bouton")).next().unwrap();
    assert_eq!(b.value().attr("type"), Some("submit"));
    assert_eq!(texte(b), "Chercher");

    let champs: Vec<_> = d.select(&sel("#champs .champ")).collect();
    assert_eq!(champs.len(), 4);
    let inp = |i: usize| {
        champs[i]
            .select(&sel("input"))
            .next()
            .unwrap()
            .value()
            .clone()
    };
    // repos, sans aide
    assert_eq!(inp(0).attr("aria-invalid"), None);
    assert_eq!(inp(0).attr("aria-describedby"), None);
    assert_eq!(inp(0).attr("value"), Some("ma valeur"));
    assert_eq!(inp(0).attr("inputmode"), None);
    let label = champs[0].select(&sel("label")).next().unwrap();
    assert_eq!(label.value().attr("for"), Some("q"));
    assert_eq!(inp(0).attr("id"), Some("q"));
    // avec aide, numérique
    assert_eq!(inp(1).attr("aria-describedby"), Some("n-aide"));
    assert_eq!(inp(1).attr("inputmode"), Some("numeric"));
    assert!(champs[1]
        .select(&sel("p#n-aide.champ__aide"))
        .next()
        .is_some());
    // en erreur avec aide : les deux ids
    assert_eq!(inp(2).attr("aria-invalid"), Some("true"));
    assert_eq!(inp(2).attr("aria-describedby"), Some("n2-aide erreur-n2"));
    // en erreur sans aide
    assert_eq!(inp(3).attr("aria-invalid"), Some("true"));
    assert_eq!(inp(3).attr("aria-describedby"), Some("erreur-d"));
    for i in 0..4 {
        let v = inp(i);
        for interdit in ["required", "min", "max"] {
            assert_eq!(v.attr(interdit), None);
        }
        assert_eq!(v.attr("type"), Some("text"));
    }
}

#[test]
fn composant_messages() {
    let d = demo();
    let m: Vec<_> = d.select(&sel("#messages > .message")).collect();
    assert_eq!(m.len(), 3);
    assert_eq!(m[0].value().attr("role"), Some("status"));
    assert_eq!(
        texte(m[0].select(&sel(".message__titre")).next().unwrap()),
        "Aucun passage trouvé."
    );
    assert!(!m[0].value().classes().any(|c| c == "message--erreur"));
    assert_eq!(m[1].value().attr("role"), Some("alert"));
    assert!(m[1].value().classes().any(|c| c == "message--erreur"));
    assert_eq!(m[1].value().attr("id"), Some("erreur-n2"));
    // texte venu des notes échappé
    assert!(m[1].select(&sel("img")).next().is_none());
    assert_eq!(
        texte(m[1].select(&sel("p:not(.message__titre)")).next().unwrap()),
        "<img src=x onerror=alert(1)>"
    );
    assert_eq!(m[2].value().attr("role"), None);
    assert!(m[2].value().classes().any(|c| c == "message--alerte"));
    assert_eq!(m[2].select(&sel("ul li")).count(), 2);
}

#[test]
fn composant_etiquettes_et_chiffres() {
    let d = demo();
    let e: Vec<_> = d.select(&sel("#etiquettes .etiquette")).collect();
    assert_eq!(e.len(), 2);
    assert!(!e[0].value().classes().any(|c| c == "etiquette--info"));
    assert!(e[1].value().classes().any(|c| c == "etiquette--info"));
    assert_eq!(texte(e[1]), "Oui");

    let c: Vec<_> = d.select(&sel("dl#chiffres > .chiffre")).collect();
    assert_eq!(c.len(), 4);
    let a = |i: usize, s: &str| c[i].value().classes().any(|x| x == s);
    assert!(!a(0, "chiffre--compact") && !a(0, "chiffre--principal"));
    assert!(a(1, "chiffre--compact"));
    assert!(a(2, "chiffre--principal"));
    assert_eq!(texte(c[2].select(&sel("dt")).next().unwrap()), "Fusion");
    assert_eq!(texte(c[2].select(&sel("dd")).next().unwrap()), "0.0325");
    let dd = c[3].select(&sel("dd")).next().unwrap();
    assert!(dd.value().classes().any(|x| x == "est-absent"));
    assert_eq!(texte(dd), "absent");
    assert!(c[0].select(&sel("dd.est-absent")).next().is_none());
}

#[test]
fn composant_repartition() {
    let d = demo();
    let r: Vec<_> = d.select(&sel("#repartitions > .repartition")).collect();
    assert_eq!(r.len(), 3);
    // complète : 50 + 100 + 50 sur 200 ms
    let barre = r[0].select(&sel(".repartition__barre")).next().unwrap();
    assert_eq!(barre.value().attr("aria-hidden"), Some("true"));
    let segs: Vec<_> = barre
        .select(&sel("span"))
        .map(|s| {
            (
                s.value().attr("class").unwrap(),
                s.value().attr("style").unwrap(),
            )
        })
        .collect();
    assert_eq!(
        segs,
        vec![
            ("temps-embedding", "width:25.0%"),
            ("temps-reranker", "width:50.0%"),
            ("temps-local", "width:25.0%")
        ]
    );
    let leg: Vec<String> = r[0]
        .select(&sel(".repartition__legende .chiffre"))
        .map(texte)
        .collect();
    assert_eq!(
        leg,
        vec![
            "Embedding de la question 50 ms",
            "Reranker 100 ms",
            "Recherche locale 50 ms"
        ]
    );
    // sans reranker, embedding nul : pas de segment nul, légende à trois entrées
    let segs: Vec<_> = r[1]
        .select(&sel(".repartition__barre span"))
        .map(|s| s.value().attr("class").unwrap())
        .collect();
    assert_eq!(segs, vec!["temps-local"]);
    let leg: Vec<String> = r[1]
        .select(&sel(".repartition__legende .chiffre"))
        .map(texte)
        .collect();
    assert_eq!(
        leg,
        vec![
            "Embedding de la question < 1 ms",
            "Reranker désactivé",
            "Recherche locale 168 ms"
        ]
    );
    assert!(r[1].select(&sel("dd.est-absent")).next().is_some());
    // durée totale nulle : pas de barre
    assert!(r[2].select(&sel(".repartition__barre")).next().is_none());
    assert_eq!(
        r[2].select(&sel(".repartition__legende .chiffre")).count(),
        3
    );
    assert_eq!(r[2].select(&sel(".pastille")).count(), 3);
}

#[test]
fn composant_passage() {
    let d = demo();
    assert_eq!(
        d.select(&sel("ol.passages[role=list] > li.passage"))
            .count(),
        2
    );
    let p: Vec<_> = d.select(&sel("li.passage")).collect();
    // avec reranker, tags
    assert_eq!(
        texte(p[0].select(&sel(".passage__rang")).next().unwrap()),
        "#1"
    );
    assert_eq!(
        texte(p[0].select(&sel("h3")).next().unwrap()),
        "Recettes > Tarte"
    );
    let scores: Vec<String> = p[0]
        .select(&sel("dl.chiffres .chiffre"))
        .map(texte)
        .collect();
    assert_eq!(
        scores,
        vec![
            "Reranker 0.9876",
            "Fusion 0.0323",
            "Rang sens 2",
            "Rang mots absent"
        ]
    );
    let premier = p[0].select(&sel("dl.chiffres .chiffre")).next().unwrap();
    assert!(premier.value().classes().any(|c| c == "chiffre--principal"));
    assert_eq!(
        texte(p[0].select(&sel(".passage__source code")).next().unwrap()),
        "/notes/cuisine/recettes.md"
    );
    assert!(texte(p[0].select(&sel(".passage__source")).next().unwrap()).contains("lignes 3 à 9"));
    let tags: Vec<String> = p[0]
        .select(&sel(".etiquettes .etiquette"))
        .map(texte)
        .collect();
    assert_eq!(tags, vec!["cuisine", "dessert"]);
    // texte complet, échappé
    let pre = p[0].select(&sel("pre.passage__texte")).next().unwrap();
    assert_eq!(
        pre.text().collect::<String>(),
        "<script>alert(1)</script> <b>gras</b>"
    );
    assert!(pre.select(&sel("script, b")).next().is_none());
    // sans reranker, sans tag, sans titre
    assert_eq!(
        texte(p[1].select(&sel("h3")).next().unwrap()),
        "recettes.md (sans titre)"
    );
    let scores: Vec<String> = p[1]
        .select(&sel("dl.chiffres .chiffre"))
        .map(texte)
        .collect();
    assert_eq!(
        scores,
        vec!["Fusion 0.0323", "Rang sens absent", "Rang mots absent"]
    );
    let premier = p[1].select(&sel("dl.chiffres .chiffre")).next().unwrap();
    assert!(premier.value().classes().any(|c| c == "chiffre--principal"));
    assert!(p[1].select(&sel(".etiquettes")).next().is_none());
    assert_eq!(p[1].select(&sel(".chiffre dd.est-absent")).count(), 2);
}

#[test]
fn composant_section() {
    let d = demo();
    let s: Vec<_> = d.select(&sel("section.section")).collect();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].value().attr("aria-labelledby"), Some("s1"));
    let h = s[0].select(&sel("h2#s1")).next().unwrap();
    assert_eq!(texte(h), "État de l'index");
    assert_eq!(
        texte(s[0].select(&sel("p.section__pied")).next().unwrap()),
        "Pied de section."
    );
    assert!(s[1].select(&sel("h3#s2")).next().is_some());
    assert!(s[1].select(&sel(".section__pied")).next().is_none());
}
