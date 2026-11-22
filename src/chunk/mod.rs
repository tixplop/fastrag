//! Découpage d'un fichier de notes en note (titre, tags) et passages (REQ-003, REQ-004, REQ-005).

pub mod frontmatter;
pub mod markdown;

use crate::store::Passage;
use std::path::Path;

pub const TAILLE_FICHIER_MAX: u64 = 10 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct NoteDecoupee {
    pub titre: String,
    pub tags: Vec<String>,
    pub passages: Vec<Passage>,
    pub avertissements: Vec<String>,
}

/// Fichier refusé ; le message nomme le fichier.
#[derive(Debug, PartialEq)]
pub enum Refus {
    NonUtf8(String),
    TropGros(String),
    Illisible(String, String),
}

impl std::fmt::Display for Refus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refus::NonUtf8(n) => write!(f, "{n} : fichier ignoré, il n'est pas en UTF-8"),
            Refus::TropGros(n) => write!(f, "{n} : fichier ignoré, il dépasse 10 Mo"),
            Refus::Illisible(n, e) => write!(f, "{n} : fichier ignoré, lecture impossible ({e})"),
        }
    }
}

impl std::error::Error for Refus {}

/// Lit et découpe un fichier `.md` ou `.txt`. La taille est vérifiée avant la lecture.
pub fn decouper_fichier(chemin: &Path) -> Result<NoteDecoupee, Refus> {
    let nom = chemin.display().to_string();
    let illisible = |e: std::io::Error| Refus::Illisible(nom.clone(), e.to_string());
    if std::fs::metadata(chemin).map_err(illisible)?.len() > TAILLE_FICHIER_MAX {
        return Err(Refus::TropGros(nom));
    }
    let octets = std::fs::read(chemin).map_err(illisible)?;
    let contenu = String::from_utf8(octets).map_err(|_| Refus::NonUtf8(nom.clone()))?;
    let markdown = chemin
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    let stem = chemin
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(decouper_texte(&nom, &stem, &contenu, markdown))
}

fn passage(texte: String, titres: &str, debut: usize, fin: usize) -> Passage {
    Passage {
        empreinte: blake3::hash(texte.as_bytes()).to_hex().to_string(),
        texte,
        titres: titres.to_string(),
        ligne_debut: debut as u32 + 1,
        ligne_fin: fin as u32 + 1,
    }
}

/// Découpe un contenu déjà lu. `nom` sert aux avertissements, `stem` de titre par défaut.
pub fn decouper_texte(nom: &str, stem: &str, contenu: &str, markdown: bool) -> NoteDecoupee {
    let mut note = NoteDecoupee::default();
    let mut decalage = 0;
    let mut meta = frontmatter::Meta::default();
    let (sections, tags_texte, titre_h1);
    if markdown {
        let mut corps = contenu;
        if let Some((yaml, n)) = frontmatter::separer(contenu) {
            decalage = n;
            corps = contenu.splitn(n + 1, '\n').nth(n).unwrap_or("");
            match frontmatter::lire(&yaml) {
                Ok(m) => meta = m,
                Err(e) => note
                    .avertissements
                    .push(format!("{nom} : front matter ignoré, YAML invalide ({e})")),
            }
        }
        (sections, tags_texte, titre_h1) = markdown::sections(&markdown::remplacer_liens(corps));
    } else {
        sections = vec![markdown::Section {
            titres: String::new(),
            ligne: 0,
            texte: contenu.to_string(),
        }];
        tags_texte = Vec::new();
        titre_h1 = None;
    }
    note.titre = meta
        .titre
        .or(titre_h1)
        .filter(|t| !t.is_empty())
        .unwrap_or(stem.to_string());
    note.tags = meta.tags;
    for t in tags_texte {
        if !note.tags.contains(&t) {
            note.tags.push(t);
        }
    }
    for s in sections {
        for (texte, d, f) in markdown::redecouper(&s.texte) {
            let base = decalage + s.ligne;
            note.passages
                .push(passage(texte, &s.titres, base + d, base + f));
        }
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(c: &str) -> NoteDecoupee {
        decouper_texte("n.md", "n", c, true)
    }

    fn fixture(nom: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(nom)
    }

    fn tmp(nom: &str) -> std::path::PathBuf {
        let base = std::env::var_os("TMPDIR")
            .map(Into::into)
            .unwrap_or_else(std::env::temp_dir);
        let d = base.join(format!("chunk_{nom}_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn req003_ac1_trois_sections_et_chemin_de_titres() {
        let n = decouper_fichier(&fixture("cuisine.md")).unwrap();
        let t: Vec<&str> = n.passages.iter().map(|p| p.titres.as_str()).collect();
        assert_eq!(
            t,
            [
                "Cuisine > Tartes",
                "Cuisine > Soupes",
                "Cuisine > Soupes > Velouté"
            ]
        );
        let n = md("# Projet\n\n## Réunions\n\n### 12 mars\n\ntexte\n");
        assert_eq!(n.passages.len(), 1);
        assert_eq!(n.passages[0].titres, "Projet > Réunions > 12 mars");
        let n = md("avant\n\n## A\n\na\n\n## B\n\nb\n\n## C\n\nc\n");
        assert_eq!(n.passages.len(), 4);
        assert_eq!(n.passages[0].titres, "");
    }

    #[test]
    fn req003_ac2_numeros_de_ligne() {
        let n = md("# T\n\nun\n\n## A\n\nligne a\nsuite a\n\n## B\n\nb\n");
        let l: Vec<(u32, u32)> = n
            .passages
            .iter()
            .map(|p| (p.ligne_debut, p.ligne_fin))
            .collect();
        assert_eq!(l, [(1, 3), (5, 8), (10, 12)]);
        // Le front matter compte dans les lignes du fichier.
        let n = md("---\ntitle: X\n---\n# T\n\ntexte\n");
        assert_eq!((n.passages[0].ligne_debut, n.passages[0].ligne_fin), (4, 6));
    }

    #[test]
    fn req003_ac3_redecoupage_avec_chevauchement() {
        let phrase = "Une phrase assez courte pour la cuisine. ";
        let long = phrase.repeat(120);
        let n = md(&format!("## Gros\n\n{long}\n"));
        assert!(n.passages.len() >= 3);
        for p in &n.passages {
            assert!(p.texte.chars().count() <= 2000);
            assert_eq!(p.titres, "Gros");
        }
        for w in n.passages.windows(2) {
            let fin: String = w[0]
                .texte
                .chars()
                .rev()
                .take(150)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            assert!(w[1].texte.contains(&fin), "pas de chevauchement");
            assert!(w[1].ligne_debut >= w[0].ligne_debut);
        }
        // Sans aucun espace : coupe nette, toujours au plus 2000 caractères.
        let n = decouper_texte("a.txt", "a", &"x".repeat(5000), false);
        assert!(n.passages.iter().all(|p| p.texte.chars().count() <= 2000));
        assert!(n.passages.len() >= 3);
    }

    #[test]
    fn req003_ac4_fichier_vide() {
        let d = tmp("vide");
        std::fs::write(d.join("vide.md"), "").unwrap();
        let n = decouper_fichier(&d.join("vide.md")).unwrap();
        assert!(n.passages.is_empty() && n.avertissements.is_empty());
        assert!(md("---\ntitle: X\n---\n").passages.is_empty());
    }

    #[test]
    fn req003_txt_par_taille_seulement() {
        let n = decouper_texte("a.txt", "a", "# pas un titre #tag\n[[x]]\n", false);
        assert_eq!(n.passages.len(), 1);
        assert_eq!(n.passages[0].titres, "");
        assert!(n.tags.is_empty());
        assert!(n.passages[0].texte.contains("[[x]]"));
    }

    #[test]
    fn req003_diese_dans_un_bloc_de_code_ignore() {
        let n = md("# Vrai\n\ntexte\n\n```\n# faux titre #faux\n```\n");
        assert_eq!(n.passages.len(), 1);
        assert_eq!(n.passages[0].titres, "Vrai");
        assert!(n.tags.is_empty());
    }

    #[test]
    fn req004_ac1_front_matter_metadonnee() {
        let n = md("---\ntitle: Ma recette\ntags: [a]\n---\n# Autre\n\ncorps\n");
        assert_eq!(n.titre, "Ma recette");
        assert!(n
            .passages
            .iter()
            .all(|p| !p.texte.contains("title:") && !p.texte.contains("---")));
    }

    #[test]
    fn req004_ac2_tags_sans_doublon() {
        let n = md("---\ntags: [recette, Dessert]\n---\nun #dessert et #Gâteau/choco, #12 non\n");
        assert_eq!(n.tags, ["recette", "dessert", "gâteau/choco"]);
        let n = decouper_fichier(&fixture("cuisine.md")).unwrap();
        assert_eq!(n.tags, ["recette", "dessert"]);
    }

    #[test]
    fn req004_ac3_liens_remplaces() {
        let n = md("voir [[Pâte brisée|la pâte]] et [[Pâte brisée]]\n");
        assert_eq!(n.passages[0].texte, "voir la pâte et Pâte brisée");
        assert_eq!(
            markdown::remplacer_liens("[[ouvert et [[a]]"),
            "[[ouvert et a"
        );
    }

    #[test]
    fn req004_ac4_front_matter_invalide() {
        let n = md("---\ntitle: [oups\n  : :\n---\n# T\n\ncorps\n");
        assert_eq!(n.passages.len(), 1);
        assert!(!n.passages[0].texte.contains("oups"));
        assert_eq!(n.avertissements.len(), 1);
        assert!(n.avertissements[0].contains("n.md"));
    }

    #[test]
    fn req005_ac1_non_utf8() {
        let d = tmp("utf8");
        let f = d.join("binaire.md");
        std::fs::write(&f, [0xff, 0xfe, 0x41, 0xc3]).unwrap();
        let e = decouper_fichier(&f).unwrap_err();
        assert!(matches!(e, Refus::NonUtf8(_)));
        assert!(e.to_string().contains("binaire.md"));
    }

    #[test]
    fn req005_ac2_plus_de_10_mo() {
        let d = tmp("gros");
        let f = d.join("enorme.txt");
        let h = std::fs::File::create(&f).unwrap();
        h.set_len(TAILLE_FICHIER_MAX + 1).unwrap(); // fichier creux, rien n'est lu
        let e = decouper_fichier(&f).unwrap_err();
        assert!(matches!(e, Refus::TropGros(_)));
        assert!(e.to_string().contains("enorme.txt"));
        h.set_len(TAILLE_FICHIER_MAX).unwrap();
        assert!(decouper_fichier(&f).is_ok());
    }
}
