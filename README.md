# roinroin

Client de tribune **natif pour Linux**, écrit en Rust avec
[iced](https://github.com/iced-rs/iced). Il parle à la fois :

- à un ou plusieurs serveurs **gb2c** (API HTTP + flux SSE `/gb2c/poll`) ;
- à des **boards bouchot directs**, sans serveur intermédiaire (backend TSV ou
  XML + POST de formulaire), y compris LinuxFR avec OAuth2.

Les deux types de sources peuvent être utilisés **en même temps**. Une même
tribune peut être suivie via gb2c et via son board direct ; elle apparaît alors
comme deux entrées distinctes, avec leur propre état de connexion, leurs
propres posts et leurs propres compteurs de non-lus.

Le client suit le [standard tribune / bouchot](https://rfm.devnewton.fr/standard/)
et reprend les fonctions des clients existants `gb3tui`, `gb3-android` et
`wmcoincoin`, avec une intégration Linux soignée.

## Aperçu des fonctions

### Sources et réseau

- flux **SSE** gb2c en direct, avec reconnexion automatique et backoff ;
- liste des tribunes et identités bloquées découvertes via `/gb2c/list` et
  `/gb2c/config` ;
- boards directs **TSV** et **XML** (`<board><post>…`) tolérants aux réponses
  malformées, avec limites de taille et nettoyage des caractères de contrôle ;
- polling direct toutes les 30 secondes, en parallèle pour chaque board ;
- envoi vers `/gb2c/post` ou directement vers l'URL de post du board ;
- OAuth2 LinuxFR : ouverture de la page d'autorisation gb2c, collage du jeton
  de retour, rafraîchissement automatique du jeton ;
- plusieurs serveurs gb2c configurables ;
- pastilles d'état à droite de chaque serveur et board : vert connecté,
  bleu en cours de connexion/rafraîchissement/envoi, rouge en erreur.

### Lecture des posts

- résolution complète des **norloges** : `#42`, ISO
  `2017-06-29T13:45:01.421`, `2017-06-29 13:45:01`, `06/29#13:45:01`,
  `13:45:01`, `13:45`, suffixes `^2`/`²` et `@bouchot` ;
- graphe de conversation : racine, réponses directes et indirectes, couleur de
  fil stable, compteur de réponses ;
- filtrage des identités bloquées et **propagation aux réponses** ;
- mode `gris` (message masqué) ou `masqué` (post absent) ;
- mise en page compacte sur une seule ligne, comme gb3-android : horloge,
  auteur et message partagent la même taille de police ;
- survoler l'horloge d'un post met en évidence ce post et les norloges qui le
  référencent ; survoler une norloge dans un message met en évidence le post
  référencé (dans les deux sens) ; si le post référencé est hors du viewport,
  un aperçu flottant l'affiche en haut de la tribune, formaté comme un post
  normal ; l'aperçu reste ouvert le temps d'y amener le curseur ;
- colonne de posts alignée : horloge, statut et identité ont des largeurs
  fixes (identité tronquée à 16 caractères), le message commence toujours à la
  même position ; si un login existe c'est lui qui est affiché, et son info/UA
  apparaît au survol ; sinon l'info courte est affichée ;
- références inverses : sélectionner un post met en évidence ses réponses ;
- `Maj+clic droit` sur n'importe quel post ouvre tout son arbre de discussion
  dans un panneau à droite : racine, réponses, réponses aux réponses et toutes
  les branches ; le panneau suit les nouveaux messages et se scinde en deux
  colonnes quand un autre arbre est ouvert, avec les mêmes filtres que la
  tribune ;
- BML : `<b>`, `<i>`, `<u>`, `<s>`, `<tt>`, `<code>`, spoilers cliquables,
  liens bruts, liens `<a>`, entités HTML, bigornos `nick<` ;
- totoz affichés par leur nom en vert ; au survol, l’image ou l’animation
  apparaît à sa taille d’origine 5 pixels au-dessus du nom, sans cadre, et
  reste immobile tant que le pointeur demeure sur le totoz ;
- liens : le titre des pages HTML et un aperçu des images sont récupérés à
  l’arrivée des posts ; au survol du lien, ils apparaissent 5 pixels au-dessus
  du texte du lien, une petite croix rouge signalant un contenu inaccessible
  ou protégé ;
- horloge des posts au format `HH:MM:SS`, colonne fixe ;
- détection des canards `\_o<` / `>o_/` ;
- listes **plop** : les messages des auteurs choisis sont remplacés par
  `pika`, `plop`, `prout`, `grouik` ou `meuh` ;
- compteurs de non-lus et badges bigorno/réponse dans le panneau des tribunes ;
- cloche horaire optionnelle (`hourly_bell`).

### Écriture

- éditeur sur une seule ligne avec brouillon persistant, bouton « Envoyer » à
  droite ; le nom de la tribune et sa source servent d'indication dans le champ
  vide ;
- raccourcis `Alt+…` pour insérer les balises de formatage ;
- fermeture automatique des balises : taper `<b>`, `<i>`, `<u>`, `<s>`,
  `<tt>`, `<code>` ou `<spoiler>` insère aussitôt la balise fermante et laisse
  le curseur entre les deux ;
- citation d'un post au clic sur son horloge, avec bascule automatique du
  suffixe `@tribune` quand on change de tribune ;
- envoi par bouton, `Entrée` ou `Ctrl+Entrée`, restauration du brouillon si
  l'envoi échoue ;
- insertion des totoz et emojis dans le brouillon ; choisir un totoz dans la
  recherche revient à la tribune, l'insère avec un espace et place le curseur
  juste après ;
- autocomplétion des totoz directement dans l'éditeur : dès `[:` suivi de deux
  caractères, les totoz correspondants s'affichent juste au-dessus du champ
  avec un aperçu miniature (aucune liste s'il n'y a pas de correspondance) ;
  `↑`/`↓` pour changer de suggestion, `Entrée` ou `Tab` pour l'insérer,
  `Échap` pour fermer la liste ; le curseur reste dans le champ ;
- le focus et le curseur sont placés dans l'éditeur au démarrage, à la
  sélection d'une tribune et au retour sur celle-ci, ainsi que dans le champ
  de recherche à l'ouverture des écrans ToToz et Emoji.

### Intégration Linux

- **notifications de bureau** freedesktop.org via D-Bus (`notify-rust`) :
  bigorno, réponses à mes posts, ou tous les nouveaux posts ;
- notification seulement quand la fenêtre n'est pas au premier plan (option,
  les bigornos sont toujours notifiés) ;
- demande d'attention de la fenêtre pour les bigornos/réponses ;
- fichier `.desktop`, icône hicolor et `make install` ;
- thème sombre (palette gc2/gb3), clair ou détection système ;
- aucune WebView, aucun JavaScript : toute l'interface est native iced ;
- rendu `wgpu` (Vulkan/OpenGL) quand il est disponible, avec repli logiciel
  `tiny-skia` sinon ; X11/Wayland requis.

## Compilation

Prérequis :

- Rust **1.88 ou plus** (`rustup` recommandé) ;
- un environnement graphique X11 ou Wayland ;
- une police emoji couleur (Noto Color Emoji recommandé) pour les emojis et drapeaux dans les posts ;
- éventuellement `libxkbcommon` et les bibliothèques Wayland/X11 de la
  distribution (winit les utilise) ;
- un démon de notifications pour l'intégration bureau (GNOME, KDE, mako,
  dunst…).

Sur Debian/Ubuntu, les en-têtes utiles sont :

```bash
sudo apt install build-essential pkg-config libxkbcommon-dev \
    libwayland-dev libx11-dev
```

`notify-rust` parle D-Bus directement (zbus), il n'y a pas besoin de la
bibliothèque C libdbus.

```bash
cargo build --release
./target/release/roinroin
```

Options utiles :

```bash
roinroin --gb2c http://localhost:16666 --nickname dave
roinroin --no-notifications
roinroin --help
```

Ou :

```bash
make
make run
```

Tests :

```bash
cargo test
```

## Téléchargements

Les [releases GitHub](https://github.com/zorel/roinroin/releases) fournissent
des archives précompilées pour Linux :

- `x86_64-unknown-linux-gnu` (PC 64 bits) ;
- `aarch64-unknown-linux-gnu` (ARM 64 bits, Raspberry Pi 64 bits).

Chaque archive contient le binaire `roinroin`, le fichier `.desktop`, l'icône
et la licence.

Installation dans `~/.local` :

```bash
make install
# binaire   : ~/.local/bin/roinroin
# .desktop  : ~/.local/share/applications/roinroin.desktop
# icône     : ~/.local/share/icons/hicolor/scalable/apps/roinroin.svg
```

`make install PREFIX=/usr/local` installe ailleurs (avec les droits adaptés).

## Configuration

Au premier lancement, roinroin crée :

- `${XDG_CONFIG_HOME:-~/.config}/roinroin/config.toml`
- `${XDG_DATA_HOME:-~/.local/share}/roinroin/state.json`

Un exemple commenté est fourni : [`roinroin.example.toml`](roinroin.example.toml).

Les champs importants :

| Champ | Rôle |
| --- | --- |
| `nickname` | pseudo envoyé avec les posts et utilisé pour les bigornos/réponses |
| `servers` | liste de serveurs gb2c (`name`, `url`, `enabled`) |
| `selected_boards` | boards directs actifs (prédéfinis ou personnalisés) |
| `hidden_tribunes` | tribunes gb2c masquées (clé `gb2c:URL\|nom`) |
| `custom_boards` | boards directs supplémentaires |
| `blocked` | identités masquées (login ou info) |
| `blocked_mode` | `gray` ou `hide` |
| `plop_list` | auteurs dont le message est remplacé par un plop |
| `totoz_server` | serveur totoz (`https://nsfw.totoz.eu` par défaut) |
| `notifications` | activation, bigorno, réponses, tous les posts, focus |
| `history_limit` | nombre de posts conservés par tribune (2000 par défaut) |
| `hourly_bell` | notification à chaque heure pile |
| `theme` | `dark`, `light` ou `system` |
| `language` | langue de l'interface (`french` pour l'instant) |

Les boards prédéfinis sont : `dlfp`, `euromussels`, `gabuzomeu`, `moules`,
`devnewton`, `ototu`, `reveildutroll`, `sveetch`. Il suffit de les mettre dans
`selected_boards` ; ils sont déjà connus du client.

### LinuxFR

gb2c termine l'OAuth LinuxFR par une redirection vers `/linuxfr.html#<jeton
base64>` et ne peut pas transmettre ce fragment à un client natif. Dans les
réglages, bouton **Ouvrir l'autorisation**, puis coller l'URL de retour (ou le
jeton) dans le champ LinuxFR et appliquer. roinroin accepte aussi un jeton
d'accès brut ; le rafraîchissement se fait ensuite via
`/gb2c/linuxfr/refresh`.

## Raccourcis

| Raccourci | Action |
| --- | --- |
| `Entrée` / `Ctrl+Entrée` / `Alt+Entrée` | envoyer |
| `Alt+B` / `I` / `U` / `S` | gras, italique, souligné, barré |
| `Alt+T` / `C` / `P` | télétexte, code, spoiler |
| `Alt+A` / `M` | BLAM, Moment |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | tribune suivante / précédente |
| `Ctrl+1` … `Ctrl+9` | tribune par position |
| `Ctrl+R` / `F5` | tout actualiser |
| `Ctrl+,` | réglages |
| `Ctrl+M` | tout marquer comme lu |
| `Ctrl+T` / `Ctrl+E` | ToToz / Emoji |
| `Échap` | fermer un panneau ou désélectionner |

## Architecture

```
src/
  main.rs          démarrage iced, icône de fenêtre
  app.rs           état, update Elm, abonnements SSE/temps/clavier
  ui.rs            arbre de widgets, posts, composition, réglages
  hover_text.rs    texte riche avec liens cliquables et survol
  visible.rs       suivi des posts réellement visibles dans le viewport
  net.rs           gb2c, boards directs, SSE, totoz, emoji, pièces jointes
  tsv.rs           parseurs TSV et XML bornés et tolérants
  message.rs       scanner BML, norloges, bigornos, totoz
  conversation.rs  résolution des références, graphe, filtres, statistiques
  model.rs         clés de source/tribune/post, temps, identités
  config.rs        configuration TOML, état JSON, boards prédéfinis
  notify.rs        notifications D-Bus
  theme.rs         palette et styles iced
```

Le client est volontairement sans base de données : les posts vivent en
mémoire, l'état persistant se limite aux derniers posts lus, aux tribunes
connues, à la tribune active, au brouillon et à la taille de la fenêtre et du
panneau des tribunes. Les réponses distantes sont toujours traitées comme non
fiables (bornes de taille, UTF-8 tolérant, caractères de contrôle filtrés).

## Différences assumées avec les clients existants

- roinroin peut afficher **gb2c et des boards directs simultanément** ; les
  autres clients choisissent généralement l'un ou l'autre.
- L'interface est en français pour l'instant ; la structure de configuration
  prévoit déjà une traduction.
- Pas de tray icon ni de mode dockapp : l'intégration passe par les
  notifications, le `.desktop` et la fenêtre native.
- Tous les posts conservés sont affichés (jusqu'à `history_limit` par tribune),
  sans fenêtre glissante ni bouton « charger les précédents ».
- Les totoz inline sont des pastilles cliquables (recherche/prévisualisation)
  plutôt que des images chargées automatiquement dans chaque post, pour éviter
  de télécharger des dizaines d'images à l'ouverture d'une tribune.

## Licence

MIT. Voir [`LICENSE`](LICENSE).
