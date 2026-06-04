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

# Acceso de usuario al nodo USB (MODE=0666). Para la captura V4L2, además, el
# acceso a /dev/video* lo concede normalmente el ACL de uaccess de systemd-logind
# a la sesión activa; si no, añade tu usuario al grupo «video».
cp "$RULES_SRC" /etc/udev/rules.d/cs_uvc.rules

# Importante (vía V4L2): NO desvincular uvcvideo. Si quedó instalada la antigua
# regla de unbind, la quitamos para que uvcvideo cree los nodos /dev/video*.
rm -f /etc/udev/rules.d/99-revopoint-unbind.rules

udevadm control --reload-rules
udevadm trigger

echo "Reglas udev instaladas. Reconecta el escáner por USB."
