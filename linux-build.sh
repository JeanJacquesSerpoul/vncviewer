#!/usr/bin/env bash
# Construit les installateurs Linux de VNC Viewer (.deb, .rpm, .AppImage),
# comme la release GitHub, et peut installer le paquet produit.
#
#   ./linux-build.sh                 construit .deb, .rpm et .AppImage
#   ./linux-build.sh --deps          installe d'abord les dépendances de compilation
#   ./linux-build.sh --install       installe ensuite le paquet adapté au système
#   ./linux-build.sh --bundles deb   ne construit que le .deb
#
# Pour compiler et installer sans paquet (dans ~/.local), voir ./install.sh.

set -euo pipefail

APP_NAME="VNC Viewer"
TRUNK_VERSION="v0.21.14"
# Même version que la CI de release.
TAURI_CLI_VERSION="2.12.3"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOOLS="$ROOT/target/install-tools"

deps=0
install=0
bundles="deb,rpm,appimage"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mattention :\033[0m %s\n' "$*" >&2; }
die() {
    printf '\033[1;31merreur :\033[0m %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Usage : ./linux-build.sh [options]

Construit les installateurs Linux de ${APP_NAME} dans target/release/bundle/.

Options :
  --deps            installe les dépendances système de compilation (apt, dnf,
                    zypper ou pacman, via sudo) et Rust (rustup) s'il est absent
  --bundles LISTE   formats à produire, séparés par des virgules : deb, rpm,
                    appimage (défaut : $bundles)
  --install         installe ensuite le paquet produit (.deb avec apt, .rpm avec
                    dnf ou zypper)
  -h, --help        affiche cette aide
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --deps) deps=1 ;;
        --install) install=1 ;;
        --bundles)
            [[ $# -ge 2 ]] || die "--bundles attend une liste (deb,rpm,appimage)"
            bundles="$2"
            shift
            ;;
        --bundles=*) bundles="${1#*=}" ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "option inconnue : $1 (voir --help)" ;;
    esac
    shift
done

[[ "$(uname -s)" == "Linux" ]] || die "ce script ne fonctionne que sous Linux (Windows : windows-build.ps1)"
[[ $EUID -ne 0 ]] || die "ne lancez pas ce script en root ; sudo est utilisé seulement quand il le faut"

IFS=',' read -ra wanted <<<"$bundles"
[[ ${#wanted[@]} -gt 0 ]] || die "--bundles est vide"
for b in "${wanted[@]}"; do
    case "$b" in
        deb | rpm | appimage) ;;
        *) die "format inconnu : $b (deb, rpm ou appimage)" ;;
    esac
done

# ---- Dépendances ------------------------------------------------------------

install_system_deps() {
    command -v sudo >/dev/null || die "sudo est nécessaire pour installer les dépendances"
    if command -v apt-get >/dev/null; then
        info "Installation des dépendances (apt)"
        sudo apt-get update
        sudo apt-get install -y build-essential pkg-config curl file patchelf \
            libwebkit2gtk-4.1-dev libxdo-dev libssl-dev librsvg2-dev \
            libayatana-appindicator3-dev
    elif command -v dnf >/dev/null; then
        info "Installation des dépendances (dnf)"
        sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config curl file patchelf \
            webkit2gtk4.1-devel libxdo-devel openssl-devel librsvg2-devel \
            libappindicator-gtk3-devel rpm-build
    elif command -v zypper >/dev/null; then
        info "Installation des dépendances (zypper)"
        sudo zypper install -y -t pattern devel_basis
        # Liste de https://tauri.app/start/prerequisites/ (openSUSE).
        sudo zypper install -y curl file patchelf webkit2gtk3-devel \
            libopenssl-devel librsvg-devel libappindicator3-1 rpm-build
    elif command -v pacman >/dev/null; then
        info "Installation des dépendances (pacman)"
        sudo pacman -S --needed --noconfirm base-devel curl file patchelf \
            webkit2gtk-4.1 xdotool openssl librsvg libappindicator-gtk3 rpm-tools
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
            cargo install trunk --locked --version "${TRUNK_VERSION#v}" --root "$TOOLS"
            PATH="$TOOLS/bin:$PATH"
            return
            ;;
    esac
    local dir="$TOOLS/trunk-$TRUNK_VERSION"
    PATH="$dir:$PATH"
    if [[ -x "$dir/trunk" ]]; then return; fi
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

# Tauri CLI : celle de cargo si elle est à la bonne version, sinon npx, sinon
# compilée une fois dans target/.
setup_tauri_cli() {
    if command -v cargo-tauri >/dev/null &&
        [[ "$(cargo-tauri --version 2>/dev/null)" == "tauri-cli $TAURI_CLI_VERSION" ]]; then
        TAURI=(cargo-tauri)
    elif [[ -x "$TOOLS/bin/cargo-tauri" ]]; then
        TAURI=("$TOOLS/bin/cargo-tauri")
    elif command -v npx >/dev/null; then
        TAURI=(npx --yes "@tauri-apps/cli@$TAURI_CLI_VERSION")
    else
        info "Compilation de la Tauri CLI $TAURI_CLI_VERSION (une seule fois)"
        cargo install tauri-cli --locked --version "$TAURI_CLI_VERSION" --root "$TOOLS"
        TAURI=("$TOOLS/bin/cargo-tauri")
    fi
}

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
for b in "${wanted[@]}"; do
    case "$b" in
        rpm) command -v rpmbuild >/dev/null || warn "rpmbuild est absent : le .rpm peut échouer (relancez avec --deps)" ;;
        appimage) command -v file >/dev/null || die "la commande « file » est nécessaire pour l'AppImage (relancez avec --deps)" ;;
    esac
done

if command -v rustup >/dev/null; then
    rustup target add wasm32-unknown-unknown >/dev/null
elif ! rustc --print sysroot | xargs -I{} test -d "{}/lib/rustlib/wasm32-unknown-unknown"; then
    die "la cible wasm32-unknown-unknown est absente ; installez Rust via rustup"
fi

command -v trunk >/dev/null || fetch_trunk
setup_tauri_cli

# ---- Construction -----------------------------------------------------------

cd "$ROOT"
info "Construction des installateurs : ${bundles}"
# Exécute aussi `trunk build --release` (beforeBuildCommand de tauri.conf.json).
# L'outil de création d'AppImage est lui-même une AppImage : sans FUSE, il s'extrait.
APPIMAGE_EXTRACT_AND_RUN=1 "${TAURI[@]}" build --bundles "$bundles"

bundle_dir="$ROOT/target/release/bundle"
shopt -s nullglob
produced=("$bundle_dir"/deb/*.deb "$bundle_dir"/rpm/*.rpm "$bundle_dir"/appimage/*.AppImage)
shopt -u nullglob
[[ ${#produced[@]} -gt 0 ]] || die "aucun installateur produit dans $bundle_dir"

info "Installateurs produits :"
printf '  %s\n' "${produced[@]}"

# ---- Installation -----------------------------------------------------------

# Le plus récent des fichiers correspondant au motif.
latest() {
    local f newest=""
    for f in "$@"; do
        [[ -z "$newest" || "$f" -nt "$newest" ]] && newest="$f"
    done
    printf '%s' "$newest"
}

if [[ $install -eq 1 ]]; then
    shopt -s nullglob
    deb="$(latest "$bundle_dir"/deb/*.deb)"
    rpm="$(latest "$bundle_dir"/rpm/*.rpm)"
    appimage="$(latest "$bundle_dir"/appimage/*.AppImage)"
    shopt -u nullglob
    if command -v apt-get >/dev/null && [[ -n "$deb" ]]; then
        info "Installation de $(basename "$deb") (apt)"
        sudo apt-get install -y "$deb"
    elif command -v dnf >/dev/null && [[ -n "$rpm" ]]; then
        info "Installation de $(basename "$rpm") (dnf)"
        sudo dnf install -y "$rpm"
    elif command -v zypper >/dev/null && [[ -n "$rpm" ]]; then
        info "Installation de $(basename "$rpm") (zypper)"
        sudo zypper install -y --allow-unsigned-rpm "$rpm"
    elif [[ -n "$appimage" ]]; then
        chmod +x "$appimage"
        warn "pas de paquet installable ici ; l'AppImage se lance directement : $appimage"
        echo "  (ou ./install.sh pour une installation dans ~/.local)"
        exit 0
    else
        die "aucun paquet adapté à ce système n'a été produit (voir --bundles)"
    fi
    info "$APP_NAME est installé : lancez-le depuis le menu des applications ou avec « vnc-viewer »."
fi
