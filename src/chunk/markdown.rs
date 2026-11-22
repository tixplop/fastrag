//! Découpage d'un texte Markdown en sections puis en passages.

use super::frontmatter::normaliser_tag;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

pub const TAILLE_MAX: usize = 2000;
pub const CHEVAUCHEMENT: usize = 200;

/// Une section : chemin de titres, première ligne (0-based dans le corps) et texte.
pub struct Section {
    pub titres: String,
    pub ligne: usize,
    pub texte: String,
}

/// Remplace `[[cible|alias]]` par l'alias et `[[cible]]` par la cible.
pub fn remplacer_liens(texte: &str) -> String {
    let mut sortie = String::with_capacity(texte.len());
    let mut reste = texte;
    while let Some(d) = reste.find("[[") {
        let apres = &reste[d + 2..];
        match apres.find("]]") {
            Some(f) if !apres[..f].contains(['\n', '[']) => {
                let lien = &apres[..f];
                sortie.push_str(&reste[..d]);
                sortie.push_str(lien.split_once('|').map_or(lien, |(_, a)| a));
                reste = &apres[f + 2..];
            }
            _ => {
                sortie.push_str(&reste[..d + 2]);
                reste = apres;
            }
        }
    }
    sortie.push_str(reste);
    sortie
}

fn niveau(l: HeadingLevel) -> usize {
    l as usize
}

/// Sections du corps (liens déjà remplacés) et tags `#mot` trouvés hors blocs de code.
pub fn sections(corps: &str) -> (Vec<Section>, Vec<String>, Option<String>) {
    let debuts_lignes: Vec<usize> = std::iter::once(0)
        .chain(corps.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let ligne_de = |octet: usize| debuts_lignes.partition_point(|&d| d <= octet) - 1;

    let mut tags = Vec::new();
    // (niveau, titre, ligne de début, octet de début)
    let mut titres: Vec<(usize, String, usize, usize)> = Vec::new();
    let mut courant: Option<(usize, String, usize, usize)> = None;
    let mut dans_code = false;
    for (ev, plage) in Parser::new_ext(corps, Options::empty()).into_offset_iter() {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                courant = Some((
                    niveau(level),
                    String::new(),
                    ligne_de(plage.start),
                    plage.start,
                ));
            }
            Event::End(TagEnd::Heading(_)) => titres.extend(courant.take()),
            Event::Start(Tag::CodeBlock(_)) => dans_code = true,
            Event::End(TagEnd::CodeBlock) => dans_code = false,
            Event::Text(t) | Event::Code(t) if courant.is_some() => {
                if let Some(c) = courant.as_mut() {
                    c.1.push_str(&t);
                }
            }
            Event::Text(t) if !dans_code => extraire_tags(&t, &mut tags),
            _ => {}
        }
    }

    let lignes: Vec<&str> = corps.lines().collect();
    let mut bornes: Vec<(String, usize)> = vec![(String::new(), 0)];
    let mut pile: Vec<(usize, String)> = Vec::new();
    for (n, t, l, _) in &titres {
        while pile.last().is_some_and(|(p, _)| p >= n) {
            pile.pop();
        }
        pile.push((*n, t.trim().to_string()));
        let chemin = pile
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" > ");
        bornes.push((chemin, *l));
    }
    let titre_h1 = titres
        .iter()
        .find(|t| t.0 == 1)
        .map(|t| t.1.trim().to_string());

    let mut res = Vec::new();
    for (i, (chemin, debut)) in bornes.iter().enumerate() {
        let fin = bornes
            .get(i + 1)
            .map_or(lignes.len(), |b| b.1)
            .min(lignes.len());
        if *debut >= fin {
            continue;
        }
        // Une section sans autre texte que son titre ne donne pas de passage.
        let corps_section = if i == 0 {
            &lignes[*debut..fin]
        } else {
            &lignes[*debut + 1..fin]
        };
        if corps_section.iter().all(|l| l.trim().is_empty()) {
            continue;
        }
        res.push(Section {
            titres: chemin.clone(),
            ligne: *debut,
            texte: lignes[*debut..fin].join("\n"),
        });
    }
    (res, tags, titre_h1)
}

fn extraire_tags(texte: &str, tags: &mut Vec<String>) {
    let c: Vec<char> = texte.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '#' && (i == 0 || c[i - 1].is_whitespace()) {
            let mut j = i + 1;
            while j < c.len() && (c[j].is_alphanumeric() || matches!(c[j], '_' | '-' | '/')) {
                j += 1;
            }
            let mot: String = c[i + 1..j].iter().collect();
            if mot
                .chars()
                .any(|ch| !ch.is_ascii_digit() && ch != '/' && ch != '-')
            {
                let t = normaliser_tag(&mot);
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
}

/// Coupe `texte` en morceaux d'au plus `TAILLE_MAX` caractères, avec `CHEVAUCHEMENT`.
/// Renvoie `(morceau, ligne de début, ligne de fin)` relatives au texte (0-based).
pub fn redecouper(texte: &str) -> Vec<(String, usize, usize)> {
    let c: Vec<char> = texte.chars().collect();
    let mut morceaux = Vec::new();
    let mut debut = 0;
    // Curseur monotone : lignes comptées jusqu'à `pos`.
    let (mut pos, mut lignes) = (0, 0);
    loop {
        let fin = if c.len() - debut <= TAILLE_MAX {
            c.len()
        } else {
            point_de_coupe(&c, debut)
        };
        let mut d = debut;
        let mut f = fin;
        while d < f && c[d].is_whitespace() {
            d += 1;
        }
        while f > d && c[f - 1].is_whitespace() {
            f -= 1;
        }
        if d < f {
            lignes += c[pos..d].iter().filter(|&&x| x == '\n').count();
            pos = d;
            let ld = lignes;
            let lf = ld + c[d..f].iter().filter(|&&x| x == '\n').count();
            morceaux.push((c[d..f].iter().collect(), ld, lf));
        }
        if fin >= c.len() {
            break;
        }
        debut = fin - CHEVAUCHEMENT;
    }
    morceaux
}

/// Fin du morceau commençant à `debut` : paragraphe, sinon phrase, sinon espace, sinon coupe nette.
fn point_de_coupe(c: &[char], debut: usize) -> usize {
    let cible = debut + TAILLE_MAX;
    let plancher = debut + TAILLE_MAX / 2;
    let cherche = |ok: &dyn Fn(usize) -> bool| (plancher..=cible).rev().find(|&p| ok(p));
    cherche(&|p| p >= 2 && c[p - 1] == '\n' && c[p - 2] == '\n')
        .or_else(|| {
            cherche(&|p| p < c.len() && c[p].is_whitespace() && matches!(c[p - 1], '.' | '!' | '?'))
        })
        .or_else(|| cherche(&|p| p < c.len() && c[p].is_whitespace()))
        .unwrap_or(cible)
}
