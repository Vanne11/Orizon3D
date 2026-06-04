#!/bin/sh
# Descarga la librería oficial lib3DCamera.so (Linux x64) del repositorio
# open-source de Revopoint. No la versionamos en este repo por su tamaño (~94 MB).
#
# Uso:  ./scripts/fetch-sdk.sh

set -e

# Commit fijado contra el que se desarrolló este proyecto (reproducibilidad).
COMMIT="516ee9b64ed9a36f7db2b1303d4c9364e39ac8d7"
SO_PATH="thirdparty/3DCamera/linux/x64/lib3DCamera.so"
URL="https://raw.githubusercontent.com/Revopoint/3DViewer/${COMMIT}/${SO_PATH}"

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
DEST_DIR="$SCRIPT_DIR/../vendor/3DCamera/lib"
DEST="$DEST_DIR/lib3DCamera.so"

mkdir -p "$DEST_DIR"

if [ -f "$DEST" ]; then
    echo "Ya existe $DEST — nada que hacer."
    exit 0
fi

echo "Descargando lib3DCamera.so (~94 MB)…"
if command -v curl >/dev/null 2>&1; then
    curl -L --fail -o "$DEST" "$URL"
elif command -v wget >/dev/null 2>&1; then
    wget -O "$DEST" "$URL"
else
    echo "Necesitas curl o wget para descargar el SDK." >&2
    exit 1
fi

chmod 0755 "$DEST"
echo "SDK instalado en $DEST"
