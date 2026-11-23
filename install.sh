#!/bin/sh
# Installe fastrag (Linux x86_64) depuis la dernière release GitHub.
set -eu

NOM="fastrag-linux-x86_64"
BASE="${FASTRAG_DOWNLOAD_URL:-https://github.com/tixplop/fastrag/releases/latest/download}"
DOSSIER="${FASTRAG_INSTALL_DIR:-$HOME/.local/bin}"

systeme=$(uname -s)
archi=$(uname -m)
if [ "$systeme" != "Linux" ] || [ "$archi" != "x86_64" ]; then
    echo "Aucun binaire publié pour $systeme $archi : compilez fastrag depuis les sources (voir le README)." >&2
    exit 1
fi

mkdir -p "$DOSSIER"
tmp=$(mktemp "$DOSSIER/.fastrag.XXXXXX")
trap 'rm -f "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

adresse="$BASE/$NOM"
if ! curl -fsSL -o "$tmp" "$adresse"; then
    echo "Téléchargement impossible : $adresse" >&2
    exit 1
fi
chmod 755 "$tmp"
mv -f "$tmp" "$DOSSIER/fastrag"

sortie=$("$DOSSIER/fastrag" --version)
version=${sortie#fastrag }
echo "fastrag $version est installé dans $DOSSIER/fastrag."
case ":$PATH:" in
    *":$DOSSIER:"*) ;;
    *) echo "Ajoutez $DOSSIER à votre PATH." ;;
esac
echo "Configuration : voir le README (section Configuration)."
