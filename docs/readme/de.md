<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>Live übersetzte Untertitel für den Ton deines Computers oder Mikrofons.</p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>Mimi herunterladen</strong></a> · <a href="https://mimi.yuxino.cn/de/">Website</a> · <a href="../usage.de.md">Anleitung</a></p>
  <p>macOS 13+ (Apple silicon / Intel) · Windows x64 · Linux x86_64 · <a href="../../android/README.md#start">Android 10+</a></p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Neueste Version"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT-Lizenz"></a>
  </p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md">ภาษาไทย</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

## Erste Schritte

Mimi ist kostenlos und Open Source. Für Cloud-Dienste brauchst du eigene API-Zugangsdaten; Gebühren können anfallen. Audio oder erkannter Text wird an den gewählten Dienst gesendet.

Auf einem unterstützten Mac mit Apple-Chip und macOS 26+ kannst du nach dem Laden der Sprachressourcen [lokale Apple-Dienste ohne API-Schlüssel](../provider-setup.md#apple-speech) verwenden.

1. **Herunterladen und installieren**: Wähle auf der [Downloadseite](https://github.com/yuxino/mimi/releases/latest) das Paket für dein System: DMG für Mac, EXE für Windows oder AppImage / DEB für Linux. Für Handys gibt es die [Android-Anleitung (Englisch)](../../android/README.md#start).
2. **Dienst einrichten**: Füge unter Einstellungen → Modelle & Dienste OpenAI, Gemini, xAI oder einen anderen unterstützten Dienst hinzu. Trage deine API-Zugangsdaten ein und speichere. Die [Einrichtungsanleitung (Englisch)](../provider-setup.md#choose) erklärt, wo du sie bekommst und was du eintragen musst.
3. **Audio und Sprachen wählen**: Wähle in den Audio- und Spracheinstellungen die gesprochene Sprache und die Zielsprache. Computerton ist vorausgewählt. Wähle das Mikrofon für deine eigene Stimme oder aktiviere beide Quellen.
4. **Untertitel starten**: Aktiviere Live-Untertitel auf der Untertitelseite, erteile die angefragten Berechtigungen und spiele ein Video, einen Stream oder andere Inhalte mit Sprache ab.

![Mimis zweisprachige Untertitel mit fiktivem Beispieldialog](../assets/readme-preview.png)

Oberflächenillustration mit Beispieldialog. [Untertitel und immersiven Modus ansehen (ältere Oberfläche)](../demos/README.md).

## Was Mimi kann

- Zeige beide Sprachen, nur die Übersetzung oder nur das Original.
- Verschiebe die Untertitel und ändere Textgröße und Farbe. Im immersiven Modus bleibt nur der Text; Klicks erreichen die App darunter.
- Am Desktop kannst du unter macOS / Windows 11 eine App oder unter Linux einen Wiedergabestream auswählen.
- Aktiviere am Desktop die Untertitelspeicherung oder Audioaufnahme, um TXT / Markdown oder WAV zu exportieren. Beide Optionen sind zunächst ausgeschaltet.

## Hilfe benötigt?

Die [Nutzungsanleitung](../usage.de.md) erklärt Installation, Berechtigungen, Untertitelanzeige und gespeicherte Sitzungen. Bei fehlenden Untertiteln oder Verbindungsproblemen beginne mit [Erkennungsfehlern](../usage.de.md#errors). Bleibt das Problem bestehen, [melde es](https://github.com/yuxino/mimi/issues).

<details>
<summary>Mitwirkende und Community</summary>

## Mitwirkende

Du möchtest mithelfen? Beginne mit der [Beitragsanleitung (Englisch/Chinesisch)](../../.github/CONTRIBUTING.md).

Danke an alle, die Code schreiben, Probleme melden, Mimi ausprobieren oder weiterempfehlen (๑•̀ㅂ•́)و✧

Besonderer Dank gilt [@yebuwudong](https://github.com/yebuwudong) für die [Android-App](https://github.com/yuxino/mimi/pull/37) und [@LLLin000](https://github.com/LLLin000) für die [Untertitelanimation](https://github.com/yuxino/mimi/pull/67) und [Verbesserungen am Windows-Audio](https://github.com/yuxino/mimi/pull/89).

Danke auch an [@Chtholly000](https://github.com/Chtholly000) für die Verbesserungen an [Geminis fortlaufenden Untertiteln und dem geplanten Verbindungswechsel](https://github.com/yuxino/Mimi/pull/176).

Danke an [@EmmetZ](https://github.com/EmmetZ) für die [Korrektur der Wayland-Untertitelsteuerung](https://github.com/yuxino/Mimi/pull/242), die [Aufnahme einzelner Wiedergabestreams unter Linux](https://github.com/yuxino/Mimi/pull/275) und die [App-Symbole für Wiedergabestreams](https://github.com/yuxino/Mimi/pull/304).

Danke an [@wangmian0](https://github.com/wangmian0) für die [Deaktivierung des Denkmodus von DeepSeek bei der Live-Untertitelübersetzung](https://github.com/yuxino/Mimi/pull/270).

Danke an [@hubumulity](https://github.com/hubumulity) für die [Einstellung der Anzahl sichtbarer Untertitel](https://github.com/yuxino/Mimi/pull/303).

Danke an [@ryannyui](https://github.com/ryannyui) für die [Grafikkompatibilitätskorrekturen für das Linux-AppImage und die Aktualisierung der Build-Werkzeuge](https://github.com/yuxino/Mimi/pull/312).

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

[Alle Mitwirkenden](https://github.com/yuxino/mimi/graphs/contributors)

## Community

Danke an die Menschen bei [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305) und [Eleduck](https://eleduck.com/), die Mimi ausprobieren, Rückmeldungen geben und weiterempfehlen.

</details>

[MIT](../../LICENSE) © 2026 yuxino
