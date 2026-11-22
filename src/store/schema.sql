CREATE TABLE IF NOT EXISTS meta (
    cle TEXT PRIMARY KEY,
    valeur TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notes (
    id INTEGER PRIMARY KEY,
    chemin TEXT NOT NULL UNIQUE,
    empreinte TEXT NOT NULL,
    titre TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS note_tags (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS note_tags_note ON note_tags(note_id);
CREATE INDEX IF NOT EXISTS note_tags_tag ON note_tags(tag);
CREATE TABLE IF NOT EXISTS passages (
    id INTEGER PRIMARY KEY,
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    ordre INTEGER NOT NULL,
    texte TEXT NOT NULL,
    titres TEXT NOT NULL,
    ligne_debut INTEGER NOT NULL,
    ligne_fin INTEGER NOT NULL,
    empreinte TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS passages_note ON passages(note_id);
CREATE INDEX IF NOT EXISTS passages_empreinte ON passages(empreinte);
-- Vecteurs (f32 little-endian) rattachés à l'empreinte du texte ; le modèle est celui du fichier (meta).
CREATE TABLE IF NOT EXISTS vecteurs (
    empreinte TEXT PRIMARY KEY,
    vecteur BLOB NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts USING fts5(
    texte,
    tokenize = 'unicode61 remove_diacritics 2'
);
