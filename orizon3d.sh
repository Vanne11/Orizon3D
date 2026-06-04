#!/usr/bin/env bash
#
# Orizon3D — script de gestión del proyecto (Rust + egui, captura V4L2).
# Sin argumentos abre un menú interactivo; también acepta comandos directos.
# Uso:  ./orizon3d.sh [comando]
#

set -uo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_NAME="orizon3d"
RELEASE_BIN="$PROJECT_DIR/target/release/$BIN_NAME"

info()    { echo -e "${BLUE}[INFO]${NC} $*"; }
success() { echo -e "${GREEN}[OK]${NC} $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; }
header()  { echo -e "\n${BOLD}${CYAN}=== $* ===${NC}\n"; }

# rustup con varios shims instalados: prioriza ~/.cargo/bin y fija stable si falta.
ensure_cargo() {
    export PATH="$HOME/.cargo/bin:$PATH"
    if ! command -v cargo >/dev/null 2>&1; then
        error "cargo no encontrado. Instala Rust: https://rustup.rs"
        return 1
    fi
    if ! cargo --version >/dev/null 2>&1; then
        warn "rustup sin toolchain por defecto; fijando stable…"
        rustup default stable || return 1
    fi
}

detect_pm() {
    for pm in pacman apt-get dnf zypper; do
        command -v "$pm" >/dev/null 2>&1 && { echo "$pm"; return 0; }
    done
    echo ""
}

# Dependencias de sistema para egui/eframe (X11/Wayland + OpenGL) y un compilador C.
install_system_deps() {
    local pm; pm="$(detect_pm)"
    [[ -z "$pm" ]] && { warn "Gestor de paquetes no reconocido; instala manualmente las deps de egui (X11/Wayland + OpenGL) y un compilador C."; return 0; }
    info "Instalando dependencias de sistema con $pm…"
    case "$pm" in
        pacman)  sudo pacman -S --needed --noconfirm base-devel libxcb libxkbcommon libxkbcommon-x11 wayland mesa vulkan-icd-loader ;;
        apt-get) sudo apt-get update && sudo apt-get install -y build-essential libxcb1-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev pkg-config ;;
        dnf)     sudo dnf install -y @development-tools libxcb-devel libxkbcommon-devel wayland-devel mesa-libGL-devel pkgconf-pkg-config ;;
        zypper)  sudo zypper install -y -t pattern devel_basis && sudo zypper install -y libxcb-devel libxkbcommon-devel wayland-devel Mesa-libGL-devel ;;
    esac || warn "Algunas dependencias no se instalaron; revísalo si la app no compila/arranca."
}

cmd_setup() {
    header "Setup"
    install_system_deps
    ensure_cargo || return 1
    info "Compilando en release…"
    ( cd "$PROJECT_DIR" && cargo build --release ) && success "Listo. Ejecuta: ./orizon3d.sh udev && ./orizon3d.sh start"
}

cmd_udev() {
    header "Reglas udev (acceso USB)"
    sudo "$PROJECT_DIR/scripts/install-udev.sh"
}

cmd_build() {
    header "Build (release)"
    ensure_cargo || return 1
    ( cd "$PROJECT_DIR" && cargo build --release ) && success "Binario: $RELEASE_BIN"
}

cmd_start() {
    ensure_cargo || return 1
    if [[ ! -x "$RELEASE_BIN" ]]; then
        warn "No hay binario release; compilando primero…"
        cmd_build || return 1
    fi
    header "Ejecutando (release)"
    "$RELEASE_BIN" "$@" || true
}

cmd_run() {
    header "Ejecutando (debug)"
    ensure_cargo || return 1
    ( cd "$PROJECT_DIR" && cargo run ) || true
}

cmd_test() {
    header "Tests"
    ensure_cargo || return 1
    ( cd "$PROJECT_DIR" && cargo test )
}

cmd_check() {
    header "Check (fmt + clippy + tests)"
    ensure_cargo || return 1
    cd "$PROJECT_DIR"
    cargo fmt --all -- --check || warn "fmt: hay diferencias (corrige con: cargo fmt --all)"
    cargo clippy --all-targets -- -D warnings || warn "clippy: hay avisos"
    cargo test
}

cmd_clean() {
    header "Clean"
    ensure_cargo || return 1
    ( cd "$PROJECT_DIR" && cargo clean ) && success "target/ limpiado"
}

cmd_doctor() {
    header "Doctor"
    echo -e "${BOLD}Rust:${NC}   $(cargo --version 2>/dev/null || echo 'N/A — instala Rust')"
    echo -e "${BOLD}rustc:${NC}  $(rustc --version 2>/dev/null || echo 'N/A')"
    echo ""
    # ¿uvcvideo cargado?
    if lsmod 2>/dev/null | grep -q '^uvcvideo'; then
        success "uvcvideo cargado"
    else
        warn "uvcvideo no parece cargado (necesario para /dev/video*)"
    fi
    # ¿nodos de vídeo del escáner Revopoint?
    local found=0
    if command -v v4l2-ctl >/dev/null 2>&1; then
        v4l2-ctl --list-devices 2>/dev/null | grep -iqE 'revo|depthcam|3dcamera' && found=1
    fi
    shopt -s nullglob
    local vids=(/dev/video*)
    shopt -u nullglob
    if [[ ${#vids[@]} -eq 0 ]]; then
        warn "No hay nodos /dev/video* (¿escáner conectado? ¿uvcvideo?)"
    else
        echo -e "${BOLD}Nodos:${NC}  ${vids[*]}"
        [[ $found -eq 1 ]] && success "Escáner Revopoint detectado" || warn "No identifiqué un escáner Revopoint (instala v4l-utils para más detalle)"
    fi
    # ¿reglas udev instaladas?
    [[ -f /etc/udev/rules.d/cs_uvc.rules ]] && success "Reglas udev instaladas" || warn "Reglas udev no instaladas (./orizon3d.sh udev)"
}

cmd_status() {
    header "Orizon3D"
    echo -e "${BOLD}Directorio:${NC} $PROJECT_DIR"
    echo -e "${BOLD}Rust:${NC}       $(cargo --version 2>/dev/null || echo 'N/A')"
    echo -e "${BOLD}Rama:${NC}       $(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null || echo 'N/A')"
    echo -e "${BOLD}Commit:${NC}     $(git -C "$PROJECT_DIR" log --oneline -1 2>/dev/null || echo 'N/A')"
    [[ -x "$RELEASE_BIN" ]] && echo -e "${BOLD}Binario:${NC}    $RELEASE_BIN" || echo -e "${BOLD}Binario:${NC}    (sin compilar)"
}

cmd_help() {
    echo -e "${BOLD}${CYAN}Orizon3D${NC} — escáner 3D Revopoint para Linux"
    echo ""
    echo -e "${BOLD}Uso:${NC} ./orizon3d.sh [comando]      (sin comando = menú interactivo)"
    echo ""
    echo "  setup     dependencias de sistema + compila (release)"
    echo "  udev      instala reglas udev de acceso USB (sudo, una vez)"
    echo "  start     ejecuta la versión release"
    echo "  run       ejecuta en debug"
    echo "  build     compila release"
    echo "  test      tests unitarios"
    echo "  check     fmt + clippy + tests (estilo CI)"
    echo "  clean     cargo clean"
    echo "  doctor    diagnostica entorno y detecta el escáner"
    echo "  status    versiones, rama y commit"
    echo "  help      esta ayuda"
}

menu_loop() {
    while true; do
        clear
        echo -e "${BOLD}${CYAN}Orizon3D${NC} ${DIM}· $(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null || echo '-')${NC}\n"
        echo -e "  ${GREEN}1${NC}) start      ${DIM}ejecuta release${NC}"
        echo -e "  ${GREEN}2${NC}) run        ${DIM}ejecuta debug${NC}"
        echo -e "  ${YELLOW}3${NC}) build      ${DIM}compila release${NC}"
        echo -e "  ${YELLOW}4${NC}) test       ${DIM}tests${NC}"
        echo -e "  ${YELLOW}5${NC}) check      ${DIM}fmt + clippy + tests${NC}"
        echo -e "  ${BLUE}6${NC}) setup      ${DIM}deps + compila${NC}"
        echo -e "  ${BLUE}7${NC}) udev       ${DIM}reglas USB${NC}"
        echo -e "  ${BLUE}8${NC}) doctor     ${DIM}diagnóstico${NC}"
        echo -e "  ${BLUE}9${NC}) status     ${DIM}info${NC}"
        echo -e "  ${RED}0${NC}) salir\n"
        echo -ne "${BOLD}  Opción: ${NC}"; read -r c
        case "${c// /}" in
            1) cmd_start ;; 2) cmd_run ;; 3) cmd_build ;; 4) cmd_test ;;
            5) cmd_check ;; 6) cmd_setup ;; 7) cmd_udev ;; 8) cmd_doctor ;;
            9) cmd_status ;; 0|q|exit|salir) echo -e "\n${GREEN}Hasta luego${NC}"; exit 0 ;;
            "") ;; *) error "Opción inválida: $c"; sleep 1 ;;
        esac
        echo ""; echo -e "${DIM}ENTER para volver al menú…${NC}"; read -r
    done
}

main() {
    [[ $# -eq 0 ]] && { menu_loop; exit 0; }
    case "$1" in
        setup)          cmd_setup ;;
        udev)           cmd_udev ;;
        start)          shift; cmd_start "$@" ;;
        run)            cmd_run ;;
        build)          cmd_build ;;
        test)           cmd_test ;;
        check)          cmd_check ;;
        clean)          cmd_clean ;;
        doctor)         cmd_doctor ;;
        status)         cmd_status ;;
        help|--help|-h) cmd_help ;;
        *)              error "Comando desconocido: $1"; echo; cmd_help; exit 1 ;;
    esac
}

main "$@"
