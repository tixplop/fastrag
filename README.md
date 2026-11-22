# fastrag

Recherche sémantique dans vos notes Markdown et texte. fastrag indexe vos dossiers, garde l'index à jour et répond par une API HTTP et par MCP (pour Claude Code et autres clients).

## Installation

```sh
cargo build --release
```

Le binaire est `target/release/fastrag`.

## Configuration

Copiez `config.example.toml` vers `~/.config/fastrag/config.toml` et adaptez-le (ou passez `--config <fichier>`). Sans fichier, les valeurs par défaut s'appliquent : port 7878, modèle `bge-m3`, reranker `bge-reranker-v2-m3` désactivé, aucun dossier.

Les embeddings viennent d'un service compatible OpenAI (route `/embeddings`) : OpenRouter, LiteLLM, Ollama… Indiquez son `url` et son `modele` dans `[embedding]`. La clé se donne par variable d'environnement, dont le nom est `cle_env` (par défaut `FASTRAG_CLE_EMBEDDING`) :

```sh
export FASTRAG_CLE_EMBEDDING=sk-...
```

Le reranker facultatif (`[reranker]`, `actif = true`) utilise une route `/rerank` de type Cohere ou Jina, avec sa propre clé (`FASTRAG_CLE_RERANKER`).

Changer de modèle recalcule l'index automatiquement ; l'ancien index reste utilisé jusqu'à la fin.

## Lancement

```sh
export FASTRAG_CLE_EMBEDDING=sk-...
target/release/fastrag --config ~/.config/fastrag/config.toml
```

Le service écoute sur `127.0.0.1:7878`, indexe vos dossiers puis surveille les changements. Une seule instance peut utiliser un dossier de données.

## API HTTP

```sh
# Recherche (nombre : 1 à 50, 5 par défaut ; dossier et tag facultatifs)
curl -s http://127.0.0.1:7878/search -H 'Content-Type: application/json' \
  -d '{"question": "recette de crêpes", "nombre": 5, "tag": "cuisine"}'

# État de l'index et avertissements
curl -s http://127.0.0.1:7878/status

# Réindexation complète
curl -s -X POST http://127.0.0.1:7878/reindex
```

Une entrée invalide renvoie une erreur 400 : `{"error": "...", "field": "..."}`.

## Page de test

Ouvrez `http://127.0.0.1:7878/` (même port que l'API) : une page, sans JavaScript, pour essayer la recherche. Elle propose les réglages de `POST /search` (question, nombre de résultats, dossier, tag) et montre les passages trouvés avec leur score de fusion, leur rang par le sens et par les mots (et le score du reranker s'il est activé), les temps de réponse (durée totale, attente du provider) et l'état de l'index (notes, passages, dernière mise à jour, avertissements). L'état est lu au chargement : rechargez la page pour le mettre à jour.

## Client MCP

Claude Code :

```sh
claude mcp add --transport http fastrag http://127.0.0.1:7878/mcp
```

Deux outils sont proposés : `search` et `status`.

## Dossiers Windows sous WSL

Indiquez-les par leur chemin `/mnt/c/...`. Sous `/mnt/`, l'inotify n'est pas fiable : fastrag relit alors ces dossiers toutes les 5 secondes (`releve_secondes`), donc une modification peut mettre quelques secondes à apparaître. Les dossiers du système Linux sont surveillés en direct.
