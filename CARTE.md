# CARTE — fastrag

Où se trouve quoi. Mise à jour à chaque tâche qui crée, renomme ou change le rôle d'un fichier.
Format : `chemin` — rôle (REQ).

- `Cargo.toml` — crate lib + bin `fastrag`, binaire de test `integration`.
- `src/lib.rs` — racine de la bibliothèque.
- `src/main.rs` — binaire mince : lit `--config <fichier>`, charge la configuration (REQ-001).
- `src/config.rs` — configuration TOML, valeurs par défaut, erreur nommant fichier et champ (REQ-001 AC-1, AC-2).
- `.config/nextest.toml` — timeouts (10 s unitaire, 60 s intégration), groupe `serie` (tests nommés `serie_*`).
- `tests/integration/main.rs` — binaire unique d'intégration, un module par thème.
- `tests/integration/common/mod.rs` — aide commune (dossier de test sous `TMPDIR`).
- `tests/integration/socle.rs` — test de socle.
- `src/provider.rs` — client `Provider` : `embeddings` (OpenAI, lots de 64) et `rerank` (Cohere/Jina), `ErreurProvider` (réseau, clé, quota, réponse invalide) sans la clé (REQ-006 AC-1, REQ-015 AC-5).
- `tests/integration/common/mock_provider.rs` — provider simulé partagé (vecteurs déterministes ou imposés, pannes, compteurs).
- `tests/integration/provider.rs` — tests du client.
- `src/store/mod.rs` — `Store` SQLite (un fichier par modèle, WAL) : remplacer/supprimer/renommer une note en transaction, vecteurs connus, `Verrou` exclusif du dossier de données (REQ-010).
- `src/store/query.rs` — candidats : 50 voisins (`vec_distance_cosine`) et 50 meilleurs FTS5 (bm25), filtres dossier et tag (REQ-014).
- `src/store/schema.sql` — tables notes, note_tags, passages, vecteurs, FTS5 `unicode61 remove_diacritics 2`.
- `tests/integration/store.rs` — tests du stockage (REQ-010 AC-1) ; AC-2 de REQ-014 : test unitaire dans `src/store/query.rs`.
- `src/chunk/mod.rs` — `decouper_fichier` / `decouper_texte` : fichier → titre, tags, passages, avertissements ; refus non UTF-8 et > 10 Mo ; tests unitaires (REQ-003, REQ-004, REQ-005 AC-1, AC-2).
- `src/chunk/markdown.rs` — sections par titres (`pulldown-cmark`), tags, liens `[[...]]`, redécoupage 2 000 / 200 caractères.
- `src/chunk/frontmatter.rs` — front matter YAML (`yaml-rust2`) : titre, tags.
- `tests/fixtures/` — notes d'exemple (`cuisine.md`, `courses.md`, `pense-bete.txt`).
- `src/index/mod.rs` — `Indexeur::synchroniser` : rapprochement dossiers/index (nouvelles, modifiées, supprimées, renommées), empreinte `blake3`, envoi au provider des seuls textes sans vecteur connu, indicateur « en cours » (REQ-002, REQ-006 AC-2 à AC-4).
- `src/index/scan.rs` — parcours des dossiers : extensions `.md/.markdown/.txt`, dossiers cachés ignorés, dossier introuvable → avertissement (REQ-002).
- `tests/integration/indexation.rs` — tests de l'indexeur (REQ-002, REQ-006).
