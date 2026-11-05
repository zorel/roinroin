# Journal des modifications

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
