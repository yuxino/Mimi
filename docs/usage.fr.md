# Utiliser Mimi

[Retour au README](readme/fr.md)

[English](usage.md) · [简体中文](usage.zh-CN.md) · [繁體中文](usage.zh-TW.md) · [日本語](usage.ja.md) · [ภาษาไทย](usage.th.md) · [한국어](usage.ko.md) · [Français](usage.fr.md) · [Deutsch](usage.de.md)

Les liens d’activation, les identifiants et les modèles actuels sont dans le [guide de configuration des fournisseurs (anglais)](provider-setup.md).

Choisissez la langue de l’interface dans Réglages → Général : chinois simplifié ou traditionnel, anglais, japonais, allemand, coréen, français ou thaï. L’option système suit la langue du système. Changer de langue conserve les saisies en cours et l’état des sous-titres. Les langues de reconnaissance et de traduction se choisissent séparément.

## Installation et mises à jour

1. Téléchargez le DMG macOS pour puce Apple ou Intel, le EXE / MSI / ZIP portable Windows x64, ou le .deb / AppImage Linux x86_64 depuis la [dernière version](https://github.com/yuxino/mimi/releases/latest). Vous pouvez aussi compiler les sources.
2. Ouvrez Réglages → Parole et traduction, ajoutez une configuration, saisissez les identifiants demandés et enregistrez. Choisissez les langues de reconnaissance et de traduction.
3. Lancez un contenu audio et activez les sous-titres en direct dans Sous-titres, ou démarrez depuis l’icône mimi de la barre de menus ou de la zone de notification. Sous macOS 14.2 et ultérieur, toutes les applications et une application sélectionnée peuvent utiliser l’autorisation d’enregistrement de l’audio système uniquement. Les autorisations de capture d’écran existantes sont réutilisées sans autre demande. macOS 13–14.1 exige l’enregistrement de l’écran et de l’audio système.

Vous devez fournir vos propres identifiants API ; leur utilisation peut être facturée. Les identifiants de l’application de bureau sont stockés en clair dans un fichier privé local protégé par les permissions du fichier.

macOS, les installations Windows et Linux AppImage proposent les mises à jour dans Réglages → Général → Mise à jour du logiciel. Mimi affiche la progression puis permet l’installation. Windows rouvre Mimi après l’installation ; macOS et Linux AppImage proposent un redémarrage pour terminer. Les versions antérieures à v1.3.8 nécessitent une installation manuelle avant de pouvoir se mettre à jour dans l’application.

Les exemples WebSocket, la détection automatique, les indications de langue et les langues prises en charge sont décrits dans [la configuration des services vocaux et des langues (anglais)](speech-language-setup.md#english).

### Plateformes

- macOS 13+ (puce Apple et Intel) : choisissez `_aarch64.dmg` pour une puce Apple, `_x64.dmg` pour Intel. Les paquets Intel sont disponibles depuis v1.4.4 ; compilation et signature sont vérifiées, mais la capture audio et les autorisations sur un Mac Intel restent à vérifier. Les DMG ne sont pas notarisés par Apple. Si le premier lancement est bloqué, choisissez Ouvrir quand même dans Réglages Système → Confidentialité et sécurité. Consultez les indications ci-dessous pour une ancienne installation.
- Windows x64 : installateurs de préversion EXE / MSI non signés et ZIP portable depuis v1.4.3. SmartScreen peut afficher un avertissement. Extrayez `mimi_<version>_x64-portable.zip` et lancez `mimi.exe`. WebView2 doit être installé (généralement présent sous Windows 11). Réglages et fichiers privés d’identifiants restent dans le répertoire utilisateur de l’application, et les exports à l’emplacement choisi ; le ZIP ne les déplace pas dans son dossier. Pour mettre à jour, quittez Mimi et remplacez-le par le nouveau ZIP de Releases. La version portable n’exécute pas l’installateur de mise à jour intégré.
- Linux x86_64 en préversion (base Ubuntu 22.04+) : `.deb` et AppImage nécessitent PulseAudio ou PipeWire avec `pipewire-pulse`, une sortie audio par défaut fonctionnelle et l’accès au fichier privé d’identifiants. L’audio système provient uniquement du moniteur de sortie. Redémarrez la session après un changement de sortie. X11 est recommandé. Wayland peut limiter le positionnement, le maintien au premier plan et le passage des clics. Utilisez les commandes affichées dans Mimi pour créer des raccourcis clavier système. Sans icône de notification, utilisez Réglages. Réduire la fenêtre laisse Mimi actif ; fermer Réglages quitte Mimi sous Linux. Aucun paquet Linux ARM64 n’est fourni. Voir [installation et vérification Linux (anglais)](development/linux.md).

### Autorisations audio sous macOS

Les nouvelles installations sous macOS 14.2 et ultérieur demandent l’enregistrement de l’audio système uniquement, pour toutes les applications ou une application sélectionnée. Pour changer une autorisation existante, quittez Mimi, supprimez ou désactivez son entrée d’enregistrement de l’écran et de l’audio système dans Réglages Système → Confidentialité et sécurité, puis rouvrez Mimi et démarrez les sous-titres. Autorisez l’audio système uniquement à la demande, ou activez Mimi dans cette catégorie. Redémarrez si macOS le demande. Ce changement est facultatif : les anciennes autorisations d’écran fonctionnent toujours. Si une autorisation manque, ouvrez la page système depuis l’erreur, autorisez Mimi et suivez les demandes de redémarrage avant de réessayer.

<a id="macos-permissions-after-an-update"></a>

### macOS redemande une autorisation après une mise à jour

Les versions macOS utilisent désormais le même certificat autosigné fixe. Jusqu’à v1.4.1, les signatures ad hoc changeaient à chaque compilation ; le premier passage à l’identité fixe peut nécessiter une nouvelle autorisation d’enregistrement. Modifier les sources ne change pas une application déjà installée. La signature fixe évite les changements d’identité par compilation, mais ne garantit pas que macOS ne demandera plus jamais d’autorisation. Supprimer un certificat local ne change pas l’identité installée et ne résout pas ce problème. Utilisez `/Applications/mimi.app` pour la version publiée et `/Applications/mimi-dev.app` pour le développement. Chaque application conserve ses propres réglages de signature et autorisations ; la version de développement ne doit pas reprendre une identité modifiée de la version publiée.

Si l’autorisation est activée mais la capture refusée, quittez et rouvrez d’abord Mimi et suivez la demande normale. Si cela ne suffit pas :

1. Quittez l’application concernée. Dans Réglages Système → Confidentialité et sécurité → Enregistrement de l’écran et de l’audio système (le nom varie selon macOS), supprimez uniquement son ancienne entrée : **mimi** pour la version publiée, **mimi-dev** pour le développement.
2. Avec +, ajoutez le bon `/Applications/mimi.app` ou `/Applications/mimi-dev.app` et activez l’autorisation. Effectuez vous-même l’authentification système demandée. Conservez l’autre version de Mimi et les autres applications.
3. Rouvrez la même application, démarrez les sous-titres par raccourci ou bouton et jouez un audio contenant de la parole. Vérifiez que des sous-titres apparaissent réellement ; un interrupteur activé ne suffit pas à confirmer le rétablissement.

Cela supprime l’ancienne autorisation d’enregistrement, pas un certificat ou une clé API. Si une tentative ne suffit pas, ne répétez pas les réinitialisations. [Signalez le problème](https://github.com/yuxino/mimi/issues) avec les versions de macOS et de Mimi, la source d’installation et l’erreur, sans clés API ni contenu des sous-titres.

Les anciens identifiants du système sont importés une seule fois, puis supprimés après vérification de leur sauvegarde dans le fichier privé en clair. L’importation peut demander l’autorisation de les lire. Ensuite, changement de configuration, enregistrement et mise à jour utilisent uniquement le fichier. Un fichier absent ou endommagé ne provoque pas de retour au Trousseau. Une demande de `codesign` pour la clé privée de signature est une demande distincte lors de la compilation.

## Configurations des services

Chaque configuration de bureau peut mémoriser les langues de reconnaissance et de traduction. Activez la configuration, choisissez les langues, puis ouvrez ses détails dans Réglages → Parole et traduction et choisissez **Mémoriser les langues actuelles**. Pour modifier ou supprimer cette paire, choisissez explicitement **Utiliser les langues actuelles** ou **Supprimer la paire enregistrée**.

Changer ou réappliquer une configuration restaure sa paire enregistrée. Une modification temporaire ne l’écrase pas. Les menus de notification et les commandes flottantes affichent la paire sous le nom de la configuration. Cliquez sur les informations du service en haut des sous-titres pour ouvrir ses détails.

## Audio et sessions enregistrées

L’audio du système est utilisé par défaut. Dans Réglages ou les commandes flottantes, choisissez l’audio du système, le microphone ou les deux. Le microphone utilise l’entrée par défaut et demande l’autorisation au début de la capture si nécessaire. Avec deux sources, connexions de reconnaissance, sous-titres et consommation du service sont indépendants. Changer de source désactive l’enregistrement ; réactivez-le si nécessaire dans Enregistrer et exporter.

Sous macOS et Windows build 20348+, le sélecteur d’applications permet de choisir le son d’une application. Linux capture le moniteur de sortie système et n’offre pas ce sélecteur.

Dans Réglages → Enregistrer et exporter, choisissez d’enregistrer les sous-titres ou les sources audio sélectionnées. Les deux options sont désactivées par défaut. Les contenus sont écrits progressivement dans des fichiers privés locaux, avec des fichiers audio séparés pour le système et le microphone. Désactiver une option efface son contenu de la session en cours. Les sessions déjà enregistrées nécessitent une suppression explicite.

Une nouvelle session conserve les sous-titres confirmés déjà à l’écran, dans la limite d’affichage. Effacer les retire. Cela n’active pas la sauvegarde, ne copie pas les anciennes lignes dans la nouvelle session et ne restaure pas les sous-titres non enregistrés après avoir quitté Mimi.

Les limites sont 10 000 paires confirmées / 2 MiB de texte et 64 MiB d’audio. La sauvegarde s’arrête avec une notification à la limite. Les horodatages des sous-titres indiquent la confirmation ; le WAV omet les pauses et les interruptions de connexion. Leurs chronologies ne sont donc pas directement synchronisées.

## Affichage des sous-titres

Dans Réglages, choisissez l’une des cinq couleurs (blanc par défaut), ou ouvrez le sélecteur système depuis la couleur personnalisée placée après elles. Les couleurs du système et du microphone sont distinctes. L’aperçu et les sous-titres flottants changent immédiatement, y compris en mode immersif. En affichage bilingue, l’original est visuellement plus discret que la traduction.

Afficher l’heure, dans Réglages et les commandes flottantes, est désactivé par défaut. Activé, il affiche l’heure locale de confirmation (HH:mm:ss) à côté des sous-titres confirmés des deux sources, aussi en mode immersif. Ce n’est pas le début de la parole. Les anciennes lignes du microphone conservent une petite icône de source lorsqu’on revient à l’audio du système. Les commandes flottantes permettent aussi de mettre en pause et de reprendre.

Choisissez Traduction seule, Original + traduction ou Original seul dans Réglages, les commandes flottantes ou la zone de notification. ⌘⇧B sous macOS ou Ctrl+Shift+B sous Windows/Linux X11 change immédiatement l’affichage sans interrompre la traduction. Sous Wayland, associez `mimi --cycle-subtitle-display` à un raccourci système. L’affichage bilingue associe les originaux confirmés à leur traduction et montre l’original reconnu pendant l’attente.

Afficher les sous-titres plus tôt montre des brouillons susceptibles de changer. Désactivé, il attend la confirmation, ce qui peut prendre plus longtemps lors d’une parole continue. Cette option ne transforme pas les brouillons en historique enregistré.

## Erreurs de reconnaissance

Les sous-titres existants restent visibles avec la cause de l’erreur. Ouvrez les commandes flottantes pour les détails et les actions de récupération. Une erreur de configuration reconnue propose Parole et traduction ; une erreur temporaire de connexion ou de service propose Réessayer. Corriger ou changer la configuration ramène la session en attente sans démarrer la capture. Réactivez les sous-titres en direct lorsque vous êtes prêt.

## Autres documents

[Contribuer (chinois/anglais)](../.github/CONTRIBUTING.md) · [Sécurité et confidentialité (chinois/anglais)](../.github/SECURITY.md) · [Différences entre plateformes et vérifications (anglais)](development/platform-parity.md)
