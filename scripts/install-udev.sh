#!/bin/sh
# Instala las reglas udev que dan acceso de usuario al escáner Revopoint por USB.
# El SDK lib3DCamera habla UVC directamente por libusb, así que necesita permiso
# de lectura/escritura sobre el nodo del dispositivo (MODE=0666).

set -e

if [ "$(id -u)" != "0" ]; then
    echo "Ejecútalo con sudo:  sudo $0"
    exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
RULES_SRC="$SCRIPT_DIR/../vendor/3DCamera/cs_uvc.rules"

cp "$RULES_SRC" /etc/udev/rules.d/cs_uvc.rules
udevadm control --reload-rules
udevadm trigger

echo "Reglas udev instaladas. Reconecta el escáner por USB."
