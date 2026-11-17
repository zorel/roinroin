# Journal des modifications

## [0.5.0] - 2026-11-17

### Ajouts

- authentification OAuth2 LinuxFR **directe**, sans serveur gb2c : renseigner
  l'identifiant et le secret d'une application LinuxFR dans les réglages, le
  client ouvre la page d'autorisation, récupère le code affiché par LinuxFr
  (redirection native `urn:ietf:wg:oauth:2.0:oob`, car LinuxFr refuse les
  redirections http), échange ce code et rafraîchit ensuite le jeton
  directement sur `linuxfr.org` ; gb2c reste le repli quand aucun identifiant
  d'application n'est configuré.

### Modifications

- le réglage de filtrage devient la **boitakon (bàk)** : deux choix explicites,
  « Garder la ligne, en gris » et « Masquer la ligne », remplacent les boutons
  qui changeaient de libellé ; la case « Afficher aussi les posts masqués »,
  sans effet réel et redondante avec le mode gris, est supprimée ;
- l'option `hungry` de la bàk filtre aussi les réponses aux posts filtrés ;
  sans elle, seuls les posts de l'identité bloquée sont filtrés. `hungry`
  vaut `true` par défaut, ce qui conserve le comportement précédent.

### Corrections

- les posts qui répondent avec une horloge courte (`HH:mm`) à l'un de nos
  posts sont désormais reconnus comme des réponses : ils rejoignent le fil de
  discussion et déclenchent le badge, la notification et le marqueur `↩` ; une
  horloge courte pointant vers le post d'un autre auteur reste une simple
  norloge navigable, trop ambiguë pour créer une arête ;
- les emojis et l'icône de recherche s'affichent à nouveau : les
  distributions récentes ne fournissent plus qu'une police emoji COLRv1
  (`Noto Color Emoji`), format que le moteur de rendu d'iced ne sait pas
  tracer ; une police couleur COLRv0 (Twemoji Mozilla) est désormais
  embarquée dans le binaire et utilisée pour les emojis, les drapeaux, les
  pastilles et les boutons d'interface.

## [0.4.1] - 2026-11-07

### Corrections

- la connexion SSE gb2c est surveillée : une inactivité de 75 s force une
  reconnexion au lieu de laisser l'interface connectée à un flux mort ;
- les fermetures normales du flux (limite d'une heure côté gb2c, timeout d'un
  proxy ou d'un NAT) ne font plus clignoter l'état du serveur en erreur ;
- les posts rejoués par gb2c à chaque reconnexion sont filtrés, et les posts
  identiques reçus à nouveau (SSE ou board direct) ne déclenchent plus une
  réanalyse complète de la conversation.

## [0.4.0] - 2026-11-05

### Ajouts

- sélection du texte des posts dans la tribune, avec surlignage, copie par
  `Ctrl+C` et prise en charge d'une sélection qui traverse plusieurs posts ;
- copie des liens : un lien sélectionné est copié avec son URL complète plutôt
  qu'avec son libellé, et un clic droit sur un lien copie directement cette URL ;
- les références horaires datées rejoignent désormais l'arbre de discussion
  correspondant ;
- option `--profile` (`ROINROIN_PROFILE=1`) : résumé périodique des mises à
  jour, rendus et analyses, avec journalisation des opérations lentes ;
- option `--debug-http` : affichage des statuts et en-têtes HTTP, sans jamais
  afficher les corps de réponse ;
- capture d'écran dans le README.

### Modifications

- le survol de l'horloge utilise `on_enter`/`on_exit` au lieu de suivre chaque
  déplacement de souris, ce qui évite de reconstruire toute la tribune ;
- le curseur du champ de composition se base sur la vraie valeur du brouillon,
  et non sur le texte indicatif lorsqu'il est vide ;
- les posts reçus par SSE sont regroupés par paquet réseau puis ingérés par
  lots, afin de réduire les reconstructions de l'interface.

### Corrections

- le curseur du champ de composition ne saute plus à la fin du texte indicatif
  quand le brouillon est vide ;
- le curseur reste la flèche du système au-dessus des posts ; le curseur texte
  est réservé au champ de composition ;
- la sélection de texte est correctement effacée au changement de tribune,
  d'écran ou lors d'un clic ailleurs.

## [0.3.0] - 2026-11-01

- panneaux de conversation à droite avec `Maj+clic droit` ;
- autocomplétion des totoz dans le champ de composition ;
- insertion des citations et des totoz à la position du curseur ;
- aperçus des liens et des totoz au survol ;
- thème clair, sombre ou système et notifications de bureau.

## [0.2] - 2026-10-29

- premier client gb2c et boards directs avec lecture, envoi et notifications.

## [0.1] - 2026-10-29

- version initiale.
