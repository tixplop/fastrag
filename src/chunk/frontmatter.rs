//! Front matter YAML en tête d'une note.

use yaml_rust2::{Yaml, YamlLoader};

#[derive(Debug, Default, PartialEq)]
pub struct Meta {
    pub titre: Option<String>,
    pub tags: Vec<String>,
}

/// Sépare le front matter du corps. Renvoie `(yaml, nombre de lignes occupées)`.
/// `None` si le contenu ne commence pas par une ligne `---` suivie d'une ligne de fermeture.
pub fn separer(contenu: &str) -> Option<(String, usize)> {
    let mut lignes = contenu.lines();
    if lignes.next()?.trim_end() != "---" {
        return None;
    }
    let mut yaml = Vec::new();
    for (i, l) in lignes.enumerate() {
        if l.trim_end() == "---" || l.trim_end() == "..." {
            return Some((yaml.join("\n"), i + 2));
        }
        yaml.push(l);
    }
    None
}

pub fn normaliser_tag(t: &str) -> String {
    t.trim().trim_start_matches('#').to_lowercase()
}

/// Lit `title` et `tags`. Erreur : message de l'analyseur YAML.
pub fn lire(yaml: &str) -> Result<Meta, String> {
    let docs = YamlLoader::load_from_str(yaml).map_err(|e| e.to_string())?;
    let mut meta = Meta::default();
    let Some(doc) = docs.first() else {
        return Ok(meta);
    };
    if !matches!(doc, Yaml::Hash(_)) {
        return Err("le front matter doit être une liste de champs".into());
    }
    if let Some(t) = doc["title"].as_str() {
        if !t.trim().is_empty() {
            meta.titre = Some(t.trim().to_string());
        }
    }
    let brut: Vec<String> = match &doc["tags"] {
        Yaml::Array(a) => a
            .iter()
            .filter_map(|v| match v {
                Yaml::String(s) => Some(s.clone()),
                Yaml::Integer(i) => Some(i.to_string()),
                _ => None,
            })
            .collect(),
        Yaml::String(s) => s.split([',', ' ']).map(str::to_string).collect(),
        _ => Vec::new(),
    };
    for t in brut.iter().map(|t| normaliser_tag(t)) {
        if !t.is_empty() && !meta.tags.contains(&t) {
            meta.tags.push(t);
        }
    }
    Ok(meta)
}
