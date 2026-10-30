<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>Des sous-titres traduits en direct pour les vidéos que vous regardez.</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Dernière version"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="Nombre total de téléchargements"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="État de la CI sur main"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="Licence MIT"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>Télécharger Mimi</strong></a> · <a href="https://mimi.yuxino.cn/fr/">Site web</a> · <a href="../usage.fr.md">Mode d’emploi</a></p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md">ภาษาไทย</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

![Illustration des sous-titres bilingues de Mimi avec un dialogue fictif](../assets/readme-preview.png)

Illustration de l’interface avec un dialogue d’exemple. [Voir les sous-titres et le mode immersif (ancienne interface)](../demos/README.md).

## Ce que vous pouvez faire

- **Suivez dans deux langues** : pour les vidéos, les directs et les cours en langue étrangère, affichez les deux langues, la traduction seule ou l’original seul.
- **Gardez seulement les sous-titres** : déplacez-les et réglez la taille du texte et sa couleur. Le mode immersif garde seulement le texte et laisse passer les clics.
- **Choisissez la source audio** : sur ordinateur, sélectionnez une application sous macOS / Windows 11 ou un flux de lecture sous Linux.
- **Gardez une copie** : sur ordinateur, activez la sauvegarde des sous-titres ou l’enregistrement audio pour exporter en TXT / Markdown ou WAV. Ces deux options sont désactivées par défaut.

## Premiers pas

Mimi est gratuit et open source, sans abonnement. Les services cloud nécessitent vos propres identifiants API et peuvent être payants. L’audio ou le texte reconnu est envoyé au service choisi.

Sur un Mac compatible avec puce Apple et macOS 26+, les [services locaux Apple](../provider-setup.md#apple-speech) fonctionnent sans clé API après le téléchargement des ressources linguistiques.

1. **Téléchargez et installez Mimi** : choisissez le fichier adapté sur la [page de téléchargement](https://github.com/yuxino/mimi/releases/latest) : DMG pour macOS 13+ (puce Apple / Intel), EXE pour Windows x64 ou AppImage / DEB pour Linux x86_64. Pour les téléphones sous Android 10 ou ultérieur, suivez le [guide Android (anglais)](../../android/README.md#start).
2. **Configurez un service** dans Réglages → Modèles et services. Ajoutez OpenAI, Gemini, xAI ou un autre service pris en charge, saisissez vos identifiants API, puis enregistrez. Le [guide de configuration (anglais)](../provider-setup.md#choose) explique où les obtenir et quoi saisir.
3. **Choisissez le son et les langues** dans les réglages audio : langue parlée et langue de traduction. Le son de l’ordinateur est sélectionné par défaut. Choisissez le microphone pour votre propre voix, ou activez les deux sources.
4. **Lancez les sous-titres** depuis la page Sous-titres. Activez les sous-titres en direct, accordez les autorisations demandées, puis lancez une vidéo, une diffusion ou un autre contenu avec des voix.

## Besoin d’aide ?

Le [guide d’utilisation](../usage.fr.md) couvre l’installation, les autorisations, l’affichage et les sessions enregistrées. Si les sous-titres manquent ou si la connexion échoue, consultez les [erreurs de reconnaissance](../usage.fr.md#errors). Si le problème persiste, [signalez-le](https://github.com/yuxino/mimi/issues).

<details>
<summary>Contributeurs et communauté</summary>

## Contributeurs

Pour participer, consultez le [guide de contribution (anglais/chinois)](../../.github/CONTRIBUTING.md).

Merci à toutes les personnes qui écrivent du code, signalent des problèmes, essaient Mimi ou le font connaître (๑•̀ㅂ•́)و✧

Un grand merci à [@yebuwudong](https://github.com/yebuwudong) pour l’[application Android](https://github.com/yuxino/mimi/pull/37), ainsi qu’à [@LLLin000](https://github.com/LLLin000) pour l’[animation des sous-titres](https://github.com/yuxino/mimi/pull/67) et les [améliorations audio sous Windows](https://github.com/yuxino/mimi/pull/89).

Merci aussi à [@Chtholly000](https://github.com/Chtholly000) pour les améliorations des [sous-titres continus de Gemini et du renouvellement planifié des connexions](https://github.com/yuxino/Mimi/pull/176).

Merci à [@EmmetZ](https://github.com/EmmetZ) pour la [correction des commandes sous Wayland](https://github.com/yuxino/Mimi/pull/242), la [capture de flux de lecture individuels sous Linux](https://github.com/yuxino/Mimi/pull/275) et les [icônes des applications dans la liste des flux](https://github.com/yuxino/Mimi/pull/304).

Merci à [@wangmian0](https://github.com/wangmian0) d’avoir [désactivé le mode de raisonnement de DeepSeek pour la traduction des sous-titres en direct](https://github.com/yuxino/Mimi/pull/270).

Merci à [@hubumulity](https://github.com/hubumulity) pour le [réglage du nombre de sous-titres affichés](https://github.com/yuxino/Mimi/pull/303).

Merci à [@ryannyui](https://github.com/ryannyui) pour les [correctifs de compatibilité graphique de l’AppImage Linux et la mise à jour des outils de compilation](https://github.com/yuxino/Mimi/pull/312).

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="40" height="40" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="40" height="40" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="40" height="40" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="40" height="40" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="40" height="40" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="40" height="40" alt="@EmmetZ"></a>
  <a href="https://github.com/wangmian0"><img src="../assets/contributors/wangmian0.svg" width="40" height="40" alt="@wangmian0"></a>
  <a href="https://github.com/hubumulity"><img src="../assets/contributors/hubumulity.svg" width="40" height="40" alt="@hubumulity"></a>
  <a href="https://github.com/ryannyui"><img src="../assets/contributors/ryannyui.svg" width="40" height="40" alt="@ryannyui"></a>
</p>

[Tous les contributeurs](https://github.com/yuxino/mimi/graphs/contributors)

## Communauté

Merci aux membres de [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305) et [Eleduck](https://eleduck.com/) d’avoir essayé Mimi, partagé leurs retours et fait connaître le projet.

</details>

[MIT](../../LICENSE) © 2026 yuxino
