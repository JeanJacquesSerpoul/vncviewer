# vncviewer

**Français** · [English](README.en.md)

[![CI](https://github.com/JeanJacquesSerpoul/vncviewer/actions/workflows/ci.yml/badge.svg)](https://github.com/JeanJacquesSerpoul/vncviewer/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/JeanJacquesSerpoul/vncviewer)](https://github.com/JeanJacquesSerpoul/vncviewer/releases/latest)

Connexion distante via VNC — visionneuse de bureau écrite en Rust avec
[Tauri 2](https://tauri.app) et [Leptos](https://leptos.dev).

## Télécharger

Les installateurs de la dernière version sont sur la page
**[Releases](https://github.com/JeanJacquesSerpoul/vncviewer/releases/latest)** :

| Système | Fichier |
|---|---|
| Debian, Ubuntu | `.deb` : `sudo apt install ./VNC.Viewer_*_amd64.deb` |
| Fedora, openSUSE | `.rpm` |
| Autres Linux | `.AppImage` (le rendre exécutable puis le lancer) |
| Windows | `-setup.exe` ou `.msi` |
| macOS Apple Silicon / Intel | `aarch64.dmg` / `x64.dmg` |

Les installateurs ne sont pas signés : Windows (SmartScreen) et macOS demandent une
confirmation au premier lancement ; la marche à suivre est indiquée sur la page de
la release. Pour compiler depuis les sources, voir plus bas.

## Fonctionnalités

- Protocole RFB 3.3 / 3.7 / 3.8 implémenté en Rust (aucune bibliothèque VNC externe).
- Sécurité : aucune, ou authentification VNC (mot de passe, DES).
- Encodages : ZRLE, Hextile, RRE, CopyRect, Raw ; pseudo-encodages curseur
  (le curseur distant est affiché localement), redimensionnement du bureau et LastRect.
- Clavier complet (keysyms X11, pavé numérique, AltGr),
  souris 3 boutons et molette, Ctrl+Alt+Suppr.
- Presse-papiers bidirectionnel : le texte copié sur le serveur arrive dans le
  presse-papiers local ; le presse-papiers local est envoyé quand la vue reprend
  le focus (ou via le bouton « Presse-papiers »).
- Mode « Ajuster » (mise à l'échelle dans la fenêtre) ou taille réelle avec défilement,
  plein écran.
- Interface bilingue français / anglais : langue du système par défaut, sélecteur FR/EN
  sur l'écran de connexion et dans la barre d'outils, choix mémorisé.
- Notations d'adresse VNC : `hôte`, `hôte:1` (affichage → port 5901), `hôte::5901`, `[::1]:0`.

## Architecture

```
src/              interface Leptos (WebAssembly, compilée par Trunk)
  app.rs          écran de connexion et barre d'outils
  viewer.rs       rendu canvas, file d'entrées clavier/souris
  keysym.rs       KeyboardEvent → keysym X11
  i18n.rs         textes FR/EN et traduction des erreurs
  wire.rs         décodage des messages binaires du backend
src-tauri/        application Tauri
  src/rfb/        client RFB : handshake, auth, décodeurs, framebuffer
  src/session.rs  connexion TCP, tâches lecture/écriture, contrôle de flux
  src/lib.rs      commandes Tauri
```

Le backend renvoie ses erreurs sous forme de codes (`{code, detail}`) indépendants
de la langue ; l'interface les traduit à l'affichage.

Le backend décode les rectangles dans un framebuffer RGBA et envoie à l'interface,
par un `Channel` Tauri binaire, uniquement les zones modifiées ; l'interface les
dessine avec `putImageData` sans recopie dans la mémoire wasm, puis acquitte la
trame avant que le backend ne demande la suivante (pas d'accumulation de latence).

## Prérequis

- Rust stable avec la cible wasm : `rustup target add wasm32-unknown-unknown`
- [Trunk](https://trunkrs.dev) et la CLI Tauri : `cargo install trunk tauri-cli`
- Sous Linux, les dépendances système de Tauri (WebKitGTK 4.1, etc.), voir
  <https://tauri.app/start/prerequisites/>

## Compiler et installer sous Linux

```bash
./install.sh --deps
```

Le script compile l'application en mode release et l'installe dans `~/.local` :
binaire `vnc-viewer` (nommé ainsi pour ne pas masquer le `vncviewer` de TigerVNC),
entrée de menu et icônes. `--deps` installe d'abord les dépendances de compilation
(apt, dnf ou pacman) et Rust s'il est absent ; inutile si elles sont déjà présentes.

| Option | Effet |
|---|---|
| `--system` | installe pour tous les utilisateurs dans `/usr/local` (sudo) |
| `--prefix DIR` | autre répertoire d'installation |
| `--skip-build` | réinstalle le binaire déjà compilé |
| `--uninstall` | désinstalle (avec les mêmes `--system` / `--prefix`) |

## Lancer en développement

```bash
cargo tauri dev
```

## Construire les paquets

```bash
cargo tauri build
```

## Tests

```bash
cargo test --workspace
```

Les tests du backend jouent une session complète contre un faux serveur VNC en
mémoire (authentification, chaque encodage, curseur, presse-papiers, redimensionnement).

## Limites

- Pas de chiffrement (VeNCrypt/TLS) ni des types de sécurité propriétaires
  (Apple ARD, RealVNC) : utiliser un tunnel SSH pour les connexions non locales,
  par exemple `ssh -L 5901:localhost:5901 serveur` puis se connecter à `localhost:1`.
- Encodage Tight non pris en charge (ZRLE est utilisé à la place).
