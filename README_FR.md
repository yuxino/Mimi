<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi <sup>みみ</sup></h1>
  <p>Sous-titres et traduction en direct pour l’audio du système ou le microphone sur macOS 13+ (puces Apple et Intel), Windows et Linux x86_64.</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Dernière version"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="Nombre total de téléchargements"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="État de la CI sur main"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="Licence MIT"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p>
    <a href="README.md">English</a> · <a href="README_ZH.md">简体中文</a> · <a href="README_ZH-TW.md">繁體中文</a> · <a href="README_KO.md">한국어</a> · <a href="README_FR.md">Français</a> · <a href="README_DE.md">Deutsch</a>
  </p>
</div>

<p align="center">Mimi transforme les paroles provenant de votre ordinateur ou de votre microphone en sous-titres traduits en direct. Regardez des films, suivez des streams ou des cours avec les sous-titres flottant au-dessus de votre écran.</p>

![Fenêtre de sous-titres bilingues de Mimi sur une illustration originale](docs/assets/readme-preview.png)

## Fonctionnalités

- Choisissez l’audio du système, le microphone ou les deux. L’audio du système est sélectionné par défaut ; le microphone doit être choisi explicitement.
- Traduisez l’audio d’une application sélectionnée sur macOS ou Windows 11.
- Affichez le texte original, la traduction ou les deux.
- Ajustez la position, la taille et la couleur des sous-titres, ou laissez les clics de souris traverser la fenêtre.
- Sauvegardez les sous-titres ou l’audio localement et exportez-les en TXT / WAV au besoin. La sauvegarde et l’enregistrement sont désactivés par défaut.

L’interface de bureau est disponible en **简体中文 · 繁體中文 · English · 日本語 · Deutsch · 한국어 · Français**. Changez-la dans **Réglages → Général**. Les langues de l’interface, de reconnaissance et de traduction se choisissent séparément.

Sur ordinateur, vous pouvez enregistrer, mettre à jour ou supprimer une paire de langues de reconnaissance et de traduction pour chaque configuration. Sélectionner ou réappliquer cette configuration rétablit la paire enregistrée ; les changements temporaires de langue ne la remplacent pas. La paire apparaît sous le nom de la configuration dans les menus de la zone de notification et de la fenêtre flottante. Cliquez sur les informations du service dans l’en-tête des sous-titres pour ouvrir les détails de la configuration actuelle.

## Premiers pas

Pour votre première configuration, **nous recommandons Alibaba Cloud ou Google Gemini**. Nous avons davantage testé Alibaba Cloud en conditions réelles ; dans mon expérience jusqu’ici, Gemini a fourni les sous-titres les plus réguliers.

Pour Google Gemini, utilisez une connexion réseau stable. Consultez votre quota disponible, la facturation et vos clés API dans [Google AI Studio](https://aistudio.google.com/).

Pour les liens d’activation, les instructions concernant les identifiants et les modèles actuellement utilisés par Mimi, consultez le **[guide de configuration des fournisseurs](docs/provider-setup.md)**.

1. Ouvrez Réglages → Parole et traduction, ajoutez une configuration, saisissez les identifiants demandés par le fournisseur et enregistrez.
2. Choisissez les langues de reconnaissance et de traduction.
3. Lancez du son et activez Sous-titres en direct dans Sous-titres. Sur macOS, autorisez l’enregistrement de l’écran et de l’audio du système lorsque cela vous est demandé.

Les services vocaux cloud nécessitent vos propres identifiants et reçoivent votre audio ; des frais d’utilisation peuvent s’appliquer. Apple Speech reconnaît l’audio localement sur les Mac compatibles. La traduction du texte à distance envoie le texte reconnu au service de votre choix ; Apple Translation garde le texte sur le Mac.

[Configuration et aide](docs/usage.md) · [Android](android/README.md) · [Signaler un bug](https://github.com/yuxino/mimi/issues) · [Contribuer](CONTRIBUTING.md)

<a id="apple-local-recognition"></a>

### Reconnaissance locale Apple

**Apple Speech** apparaît lorsque le système le permet : puce Apple, macOS 26 ou version ultérieure et moteur de transcription système disponible. Aucune clé API de reconnaissance vocale n’est nécessaire. Arrêtez les sous-titres et choisissez la langue dans **Langue de reconnaissance**. Si elle est manquante, cliquez sur **Télécharger et utiliser** ; Mimi télécharge les ressources Apple et sélectionne cette langue une fois prête. Pour une langue déjà téléchargée, cliquez sur **Définir la langue de reconnaissance**, puis démarrez les sous-titres. Inutile d’ouvrir les Réglages Système. La détection automatique de la langue n’est pas proposée. Consultez la [configuration d’Apple Speech](docs/provider-setup.md#apple-speech).

**Apple Translation** est un service distinct de traduction du texte sur l’appareil, disponible sur les Mac compatibles avec puce Apple sous macOS 26 ou version ultérieure. Sélectionnez-le dans **Traduction du texte**, enregistrez la configuration, puis choisissez une langue source et une langue cible précises. Mimi indique si la paire est prête ; **Télécharger ou activer les langues** ouvre la confirmation de configuration d’Apple. Les modèles de traduction sont séparés des modèles de reconnaissance vocale. Les modèles existants sont réutilisés ; Apple télécharge les modèles manquants après votre confirmation. Les vérifications et les sessions de sous-titres ne demandent jamais de téléchargement. Aucune clé API ni aucun proxy de traduction du texte n’est nécessaire.

Pour afficher uniquement l’original avec Apple Speech, choisissez **Sans traduction (original uniquement)**. Vous pouvez aussi utiliser un service de traduction du texte à distance, comme [Index-Translate](#try-index-translate) ; ce service reçoit le texte reconnu.

<a id="try-index-translate"></a>

### Essayer Index-Translate

Le projet [Index-Translate](https://github.com/bilibili/Index-Translate#inference) de Bilibili propose actuellement une API publique de traduction gratuite (au 5 octobre 2026). Elle peut traduire le texte reconnu par Alibaba Cloud ou Apple Speech.

Dans **Réglages → Parole et traduction**, ouvrez une configuration **Alibaba Cloud** ou **Apple Speech** et choisissez **API compatible OpenAI** dans **Traduction du texte**. Utilisez les valeurs de l’[exemple officiel](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41) :

| Champ | Valeur |
| --- | --- |
| Adresse du service | `https://index-translate.bilibili.com/v1` |
| Nom du modèle | `Index-Translate-35B-A3B` |
| Clé API | Laissez vide ; l’API publique ne nécessite actuellement aucune authentification. |

Enregistrez, puis lancez la vérification de connexion à côté de **Traduction du texte**. Si une clé est déjà enregistrée pour cette adresse, supprimez-la avec **Supprimer la clé de traduction** et enregistrez de nouveau.

Index-Translate assure uniquement la traduction du texte. Avec Alibaba Cloud, conservez les identifiants de reconnaissance vocale configurés ; la reconnaissance peut toujours être facturée. Apple Speech ne nécessite aucune clé de reconnaissance. La disponibilité de l’API gratuite dépend du service en amont.

## Questions fréquentes

**macOS redemande l’autorisation d’enregistrement alors qu’elle est activée ?** Quittez Mimi, puis supprimez et ajoutez de nouveau uniquement son entrée dans Réglages Système → Confidentialité et sécurité → Enregistrement de l’écran et de l’audio du système. Utilisez `/Applications/mimi.app` pour la version publique ou `/Applications/mimi-dev.app` pour le développement, activez l’autorisation et rouvrez la même application. Consultez la [récupération des autorisations](docs/usage.md#macos-permissions-after-an-update).

## Contributeurs

Merci à toutes les personnes qui écrivent du code, signalent des problèmes, essaient Mimi ou le font connaître (๑•̀ㅂ•́)و✧

Un grand merci à [@yebuwudong](https://github.com/yebuwudong) pour l’[application Android](https://github.com/yuxino/mimi/pull/37), ainsi qu’à [@LLLin000](https://github.com/LLLin000) pour l’[animation des sous-titres](https://github.com/yuxino/mimi/pull/67) et les [améliorations audio sous Windows](https://github.com/yuxino/mimi/pull/89).

Merci aussi à [@Chtholly000](https://github.com/Chtholly000) pour les améliorations des [sous-titres continus de Gemini et du renouvellement planifié des connexions](https://github.com/yuxino/Mimi/pull/176).

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="docs/assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
</p>

[Tous les contributeurs](https://github.com/yuxino/mimi/graphs/contributors)

## Communauté

Merci aux membres de [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305) et [Eleduck](https://eleduck.com/) d’avoir essayé Mimi, partagé leurs retours et fait connaître le projet.

[MIT](LICENSE) © 2026 yuxino
