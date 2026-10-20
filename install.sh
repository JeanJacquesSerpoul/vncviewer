#!/usr/bin/env bash
# Compile et installe VNC Viewer sous Linux.
#
#   ./install.sh                  installation pour l'utilisateur courant (~/.local)
#   ./install.sh --deps           installe d'abord les dépendances de compilation
#   ./install.sh --system         installation pour tous les utilisateurs (/usr/local, sudo)
#   ./install.sh --uninstall      désinstallation
#
# Voir ./install.sh --help.

set -euo pipefail

APP_ID="io.github.vncviewer"
APP_NAME="VNC Viewer"
# Distinct de `vncviewer` (TigerVNC, RealVNC...) pour ne pas le masquer.
BIN_NAME="vnc-viewer"
TRUNK_VERSION="v0.21.14"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

prefix=""
system=0
deps=0
uninstall=0
skip_build=0
custom_prefix=0

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mattention :\033[0m %s\n' "$*" >&2; }
die() {
    printf '\033[1;31merreur :\033[0m %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Usage : ./install.sh [options]

Compile ${APP_NAME} depuis les sources et l'installe.

Options :
  --deps          installe les dépendances système de compilation (apt, dnf ou
                  pacman, via sudo) et Rust (rustup) s'il est absent
  --system        installe pour tous les utilisateurs dans /usr/local (via sudo)
  --prefix DIR    répertoire d'installation (défaut : ~/.local, ou /usr/local
                  avec --system)
  --skip-build    installe le binaire déjà compilé (target/release/$BIN_NAME)
  --uninstall     supprime les fichiers installés
  -h, --help      affiche cette aide

Fichiers installés :
  PREFIX/bin/${BIN_NAME}
  PREFIX/share/applications/${APP_ID}.desktop
  PREFIX/share/icons/hicolor/*/apps/${APP_ID}.png
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --deps) deps=1 ;;
        --system) system=1 ;;
        --prefix)
            [[ $# -ge 2 ]] || die "--prefix attend un répertoire"
            prefix="$2"
            shift
            ;;
        --prefix=*) prefix="${1#*=}" ;;
        --skip-build) skip_build=1 ;;
        --uninstall) uninstall=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "option inconnue : $1 (voir --help)" ;;
    esac
    shift
done

[[ "$(uname -s)" == "Linux" ]] || die "ce script ne fonctionne que sous Linux"
[[ $EUID -ne 0 || $system -eq 1 ]] || die "ne lancez pas ce script en root ; utilisez --system pour une installation globale"

if [[ -n "$prefix" ]]; then
    custom_prefix=1
else
    if [[ $system -eq 1 ]]; then prefix="/usr/local"; else prefix="$HOME/.local"; fi
fi
prefix="${prefix%/}"

# sudo uniquement pour écrire dans un préfixe qui n'appartient pas à l'utilisateur.
SUDO=""
if [[ $EUID -ne 0 ]]; then
    probe="$prefix"
    while [[ ! -e "$probe" ]]; do probe="$(dirname "$probe")"; done
    if [[ ! -w "$probe" ]]; then
        command -v sudo >/dev/null || die "pas de droit d'écriture dans $prefix et sudo est absent"
        SUDO="sudo"
    fi
fi

bin_path="$prefix/bin/$BIN_NAME"
desktop_path="$prefix/share/applications/$APP_ID.desktop"
icon_dir="$prefix/share/icons/hicolor"
# Taille d'icône -> fichier source dans src-tauri/icons.
ICONS=("32x32:32x32.png" "64x64:64x64.png" "128x128:128x128.png" "256x256:128x128@2x.png")

refresh_caches() {
    if command -v update-desktop-database >/dev/null; then
        $SUDO update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null && [[ -f "$icon_dir/index.theme" ]]; then
        $SUDO gtk-update-icon-cache -q -t "$icon_dir" 2>/dev/null || true
    fi
}

# ---- Désinstallation --------------------------------------------------------

if [[ $uninstall -eq 1 ]]; then
    info "Désinstallation depuis $prefix"
    files=("$bin_path" "$desktop_path")
    for entry in "${ICONS[@]}"; do files+=("$icon_dir/${entry%%:*}/apps/$APP_ID.png"); done
    removed=0
    for f in "${files[@]}"; do
        if [[ -e "$f" ]]; then
            $SUDO rm -f "$f"
            echo "  supprimé : $f"
            removed=1
        fi
    done
    [[ $removed -eq 1 ]] || warn "rien à supprimer dans $prefix"
    refresh_caches
    info "Terminé. Les préférences éventuelles restent dans ~/.local/share/$APP_ID."
    exit 0
fi

# ---- Dépendances ------------------------------------------------------------

install_system_deps() {
    command -v sudo >/dev/null || [[ $EUID -eq 0 ]] || die "sudo est nécessaire pour installer les dépendances"
    local s=""
    [[ $EUID -eq 0 ]] || s="sudo"
    if command -v apt-get >/dev/null; then
        info "Installation des dépendances (apt)"
        $s apt-get update
        $s apt-get install -y build-essential pkg-config curl file \
            libwebkit2gtk-4.1-dev libxdo-dev libssl-dev librsvg2-dev
    elif command -v dnf >/dev/null; then
        info "Installation des dépendances (dnf)"
        $s dnf install -y gcc gcc-c++ make pkgconf-pkg-config curl file \
            webkit2gtk4.1-devel libxdo-devel openssl-devel librsvg2-devel
    elif command -v pacman >/dev/null; then
        info "Installation des dépendances (pacman)"
        $s pacman -S --needed --noconfirm base-devel curl file \
            webkit2gtk-4.1 xdotool openssl librsvg
    else
        die "gestionnaire de paquets non reconnu ; installez les dépendances indiquées sur https://tauri.app/start/prerequisites/"
    fi
}

install_rust() {
    command -v curl >/dev/null || die "curl est nécessaire pour installer Rust"
    info "Installation de Rust (rustup)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
}

# Trunk précompilé (version figée, somme de contrôle vérifiée), placé dans target/.
fetch_trunk() {
    local arch
    case "$(uname -m)" in
        x86_64 | amd64) arch="x86_64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *)
            info "Compilation de Trunk (architecture $(uname -m))"
            cargo install trunk --locked --version "${TRUNK_VERSION#v}"
            TRUNK="trunk"
            return
            ;;
    esac
    local dir="$ROOT/target/install-tools/trunk-$TRUNK_VERSION"
    TRUNK="$dir/trunk"
    if [[ -x "$TRUNK" ]]; then return; fi
    command -v curl >/dev/null || die "curl est nécessaire pour télécharger Trunk"
    info "Téléchargement de Trunk $TRUNK_VERSION"
    local file="trunk-$arch-unknown-linux-gnu.tar.gz"
    local base="https://github.com/trunk-rs/trunk/releases/download/$TRUNK_VERSION"
    mkdir -p "$dir"
    curl -fsSL -o "$dir/$file" "$base/$file"
    local expected actual
    expected="$(curl -fsSL "$base/$file.sha256" | awk '{print $1}')"
    actual="$(sha256sum "$dir/$file" | awk '{print $1}')"
    [[ -n "$expected" && "$expected" == "$actual" ]] || die "somme de contrôle de Trunk invalide"
    tar -xzf "$dir/$file" -C "$dir" trunk
    rm -f "$dir/$file"
}

if [[ $skip_build -eq 0 ]]; then
    [[ $deps -eq 0 ]] || install_system_deps

    if ! command -v cargo >/dev/null && [[ -f "$HOME/.cargo/env" ]]; then
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
    fi
    if ! command -v cargo >/dev/null; then
        [[ $deps -eq 1 ]] || die "Rust est introuvable ; relancez avec --deps ou installez-le depuis https://rustup.rs"
        install_rust
    fi

    if ! command -v pkg-config >/dev/null || ! pkg-config --exists webkit2gtk-4.1; then
        die "WebKitGTK 4.1 (paquet de développement) est introuvable ; relancez avec --deps"
    fi

    if command -v rustup >/dev/null; then
        rustup target add wasm32-unknown-unknown >/dev/null
    elif ! rustc --print sysroot | xargs -I{} test -d "{}/lib/rustlib/wasm32-unknown-unknown"; then
        die "la cible wasm32-unknown-unknown est absente ; installez Rust via rustup"
    fi

    if command -v trunk >/dev/null; then TRUNK="trunk"; else fetch_trunk; fi

    cd "$ROOT"
    info "Compilation de l'interface (Trunk)"
    "$TRUNK" build --release
    info "Compilation de l'application (cargo, mode release)"
    # `custom-protocol` embarque l'interface dans le binaire (comme `cargo tauri build`).
    cargo build --release --locked -p vncviewer --features tauri/custom-protocol
fi

binary="$ROOT/target/release/$BIN_NAME"
[[ -x "$binary" ]] || die "binaire introuvable : $binary (lancez le script sans --skip-build)"

# ---- Installation -----------------------------------------------------------

info "Installation dans $prefix${SUDO:+ (via sudo)}"
$SUDO install -Dm755 "$binary" "$bin_path"
echo "  $bin_path"

for entry in "${ICONS[@]}"; do
    size="${entry%%:*}"
    $SUDO install -Dm644 "$ROOT/src-tauri/icons/${entry#*:}" "$icon_dir/$size/apps/$APP_ID.png"
done
echo "  $icon_dir/*/apps/$APP_ID.png"

desktop_tmp="$(mktemp)"
trap 'rm -f "$desktop_tmp"' EXIT
cat >"$desktop_tmp" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
GenericName=Remote Desktop Viewer
GenericName[fr]=Visionneuse de bureau distant
Comment=Connect to a remote desktop over VNC
Comment[fr]=Se connecter à un bureau distant via VNC
Exec=$bin_path
Icon=$APP_ID
Terminal=false
Categories=Network;RemoteAccess;
Keywords=vnc;rfb;remote;desktop;bureau;distant;
EOF
$SUDO install -Dm644 "$desktop_tmp" "$desktop_path"
echo "  $desktop_path"

refresh_caches

info "$APP_NAME est installé."
echo "  Lancez-le depuis le menu des applications ou avec : $BIN_NAME"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) warn "$prefix/bin n'est pas dans votre PATH ; ajoutez-le pour lancer « $BIN_NAME » depuis un terminal." ;;
esac
uninstall_cmd="./install.sh --uninstall"
[[ $system -eq 0 ]] || uninstall_cmd+=" --system"
[[ $custom_prefix -eq 0 ]] || uninstall_cmd+=" --prefix $prefix"
echo "  Désinstallation : $uninstall_cmd"
