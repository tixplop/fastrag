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
