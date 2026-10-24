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
- Sécurité :
  - aucune, authentification VNC (mot de passe, DES) ou Apple (Partage d'écran
    de macOS, nom d'utilisateur et mot de passe) ;
  - **VeNCrypt avec TLS** (X509None, X509Vnc, X509Plain) : un certificat signé
    par une autorité connue est accepté directement ; un certificat auto-signé
    est affiché avec son empreinte SHA-256 et mémorisé une fois approuvé ;
  - **tunnel SSH** intégré (`utilisateur@passerelle[:port]`) : clé d'hôte
    vérifiée dans `~/.ssh/known_hosts` (ajoutée après approbation), authentification
    par l'agent SSH, les clés de `~/.ssh` ou un mot de passe ;
  - l'icône de cadenas de la barre d'outils indique si la session est chiffrée.
- Le mot de passe n'est demandé que si le serveur l'exige ; il peut être mémorisé
  dans le trousseau du système (jamais dans les préférences).
- Encodages : Tight (JPEG, palette, gradient), ZRLE, Hextile, RRE, CopyRect, Raw ;
  trois niveaux de qualité (sans perte, bonne, économique) ; pseudo-encodages
  curseur, redimensionnement du bureau (dont ExtendedDesktopSize) et LastRect.
- Clavier complet (keysyms X11, pavé numérique, AltGr), souris 3 boutons et molette ;
  menu « Touches » : Ctrl+Alt+Suppr, Alt+Tab, touche Windows, Impr. écran, F1–F12…
- Presse-papiers bidirectionnel, en UTF-8 avec les serveurs qui gèrent le
  presse-papiers étendu (Latin-1 sinon) : le texte copié sur le serveur arrive
  dans le presse-papiers local ; le presse-papiers local est envoyé quand la vue
  reprend le focus (ou via le bouton dédié).
- Affichage : ajusté à la fenêtre ou zoom de 50 à 200 %, lissage désactivable,
  résolution distante adaptée à la fenêtre (si le serveur le permet).
- Plein écran avec barre d'outils masquée : elle réapparaît en haut de l'écran ou
  avec F8 ; Ctrl+Alt+Entrée bascule le plein écran.
- Mode lecture seule, capture d'écran PNG, images/s et débit dans la barre d'outils.
- Reconnexion automatique quand la connexion est perdue ; connexion annulable.
- Historique des connexions et favoris ; plusieurs fenêtres (une session chacune).
- Thème clair ou sombre selon le système ; taille et position des fenêtres mémorisées.
- Interface bilingue français / anglais : langue du système par défaut, sélecteur FR/EN
  sur l'écran de connexion et dans la barre d'outils, choix mémorisé.
- Notations d'adresse VNC : `hôte`, `hôte:1` (affichage → port 5901), `hôte::5901`, `[::1]:0`.

## Architecture

```
src/              interface Leptos (WebAssembly, compilée par Trunk)
  app.rs          état, cycle de connexion, reconnexion
  connect.rs      écran de connexion, historique, demandes d'identifiants
  toolbar.rs      vue distante, barre d'outils et menus
  viewer.rs       rendu canvas, file d'entrées clavier/souris
  keysym.rs       KeyboardEvent → keysym X11
  i18n.rs         textes FR/EN et traduction des erreurs
  prefs.rs        préférences et historique (localStorage)
  wire.rs         décodage des messages binaires du backend
src-tauri/        application Tauri
  src/rfb/        client RFB : handshake, sécurité (VNC, ARD, VeNCrypt/TLS),
                  décodeurs (dont Tight), presse-papiers étendu, framebuffer
  src/ssh.rs      tunnel SSH
  src/session.rs  connexion, tâches lecture/écriture, contrôle de flux
  src/lib.rs      commandes Tauri (une session par fenêtre)
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

## Construire les installateurs

Les scripts produisent les mêmes installateurs que la release GitHub, dans
`target/release/bundle/`, et téléchargent les outils manquants (Trunk, Tauri CLI).

Linux (`.deb`, `.rpm`, `.AppImage`) :

```bash
./linux-build.sh --deps            # dépendances système + Rust, puis construction
./linux-build.sh --install         # construit puis installe le .deb (apt) ou le .rpm (dnf, zypper)
./linux-build.sh --bundles deb     # un seul format
```

Windows (setup NSIS et `.msi`), dans PowerShell :

```powershell
.\windows-build.ps1 -Deps         # Rust et outils C++ de Visual Studio (winget), puis construction
.\windows-build.ps1 -Install      # construit puis lance l'installateur
.\windows-build.ps1 -Bundles nsis # un seul format
```

Si PowerShell refuse d'exécuter le script :
`powershell -ExecutionPolicy Bypass -File .\windows-build.ps1`.

Directement avec la Tauri CLI : `cargo tauri build`.

## Tests

```bash
cargo test --workspace
```

Les tests du backend jouent une session complète contre un faux serveur VNC en
mémoire (authentification, VeNCrypt/TLS, chaque encodage, curseur, presse-papiers
classique et étendu, redimensionnement).

Un test supplémentaire, ignoré par défaut, se connecte à un vrai serveur ; les
paramètres sont ceux de la commande `connect`, en JSON :

```bash
VNC_LIVE='{"host":"localhost","port":5901,"password":"secret","shared":true}' \
  cargo test -p vncviewer live -- --ignored --nocapture
```

## Limites

- TLS anonyme (sous-types VeNCrypt TLSNone / TLSVnc / TLSPlain) non pris en charge :
  si le serveur ne propose que celui-ci, la connexion se rabat sur les autres types
  de sécurité (non chiffrés). Configurer un certificat X.509 sur le serveur, ou
  passer par le tunnel SSH.
- Types de sécurité propriétaires RealVNC (RA2) non pris en charge.
- Tunnel SSH : les clés protégées par une phrase de passe passent par l'agent SSH ;
  les certificats d'hôte SSH ne sont pas acceptés.
