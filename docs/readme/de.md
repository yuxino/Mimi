<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>Live-Untertitel und Übersetzung für Systemaudio oder dein Mikrofon unter macOS 13+ (Apple-Chip und Intel) und Windows / Linux x86_64.</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Neueste Version"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="Downloads insgesamt"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="CI-Status von main"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT-Lizenz"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

<p align="center">Mimi macht Sprache von deinem Computer oder Mikrofon zu live übersetzten Untertiteln. Schau Filme, verfolge Streams oder nimm an Unterricht teil, während die Untertitel über deinem Bildschirm schweben.</p>

![Mimis zweisprachiges Untertitelfenster über einer eigens illustrierten Szene](../assets/readme-preview.png)

## Funktionen

- Wähle Systemaudio, Mikrofon oder beides. Systemaudio ist voreingestellt; das Mikrofon wird nur nach ausdrücklicher Auswahl erfasst.
- Übersetze das Audio einer ausgewählten App unter macOS oder Windows 11.
- Zeige Originaltext, Übersetzung oder beides an.
- Passe Position, Größe und Farbe der Untertitel an oder lass Mausklicks durch das Fenster hindurchgehen.
- Speichere bei Bedarf Untertitel oder Audio lokal und exportiere TXT / WAV. Speichern und Aufnahme sind standardmäßig ausgeschaltet.

## Erste Schritte

Für die erste Einrichtung **empfehlen wir Alibaba Cloud oder Google Gemini**. Mit Alibaba Cloud haben wir mehr Tests unter realen Bedingungen durchgeführt; nach meiner bisherigen Erfahrung liefert Gemini die gleichmäßigsten Untertitelergebnisse.

Verwende für Google Gemini eine stabile Netzwerkverbindung. Prüfe dein verfügbares Kontingent, die Abrechnung und deine API-Schlüssel in [Google AI Studio](https://aistudio.google.com/).

Links zur Freischaltung, Hinweise zu Zugangsdaten und die derzeit von Mimi verwendeten Modelle findest du in der **[Anleitung zur Anbietereinrichtung](../provider-setup.md)**.

1. Öffne Einstellungen → Sprache & Übersetzung, füge eine Konfiguration hinzu, gib die angeforderten Anbieterzugangsdaten ein und speichere.
2. Wähle die Erkennungs- und Übersetzungssprachen.
3. Spiele etwas ab und aktiviere unter Untertitel die Live-Untertitel. Erlaube unter macOS die Bildschirm- und Systemaudioaufnahme, wenn du dazu aufgefordert wirst.

Cloud-Sprachdienste erfordern deine eigenen Zugangsdaten und erhalten dein Audio; dabei können Nutzungsgebühren entstehen. Apple Speech erkennt Audio auf unterstützten Macs lokal. Eine externe Textübersetzung sendet den erkannten Text an den von dir gewählten Dienst; mit Apple Translation bleibt der Text auf dem Mac.

[Einrichtung & Hilfe](../usage.md) · [Android](../../android/README.md) · [Fehler melden](https://github.com/yuxino/mimi/issues) · [Mitwirken](../../.github/CONTRIBUTING.md)

<a id="apple-local-recognition"></a>

### Lokale Erkennung mit Apple

**Apple Speech** wird angezeigt, wenn das System die Voraussetzungen erfüllt: Apple-Chip, macOS 26 oder neuer und eine verfügbare Systemtranskription. Ein API-Schlüssel für die Spracherkennung ist nicht nötig. Stoppe die Untertitel und wähle unter **Erkennungssprache** die Sprache. Falls sie fehlt, klicke auf **Laden und verwenden**; Mimi lädt die Ressourcen von Apple und wählt die Sprache aus, sobald sie bereit ist. Klicke bei einer bereits geladenen Sprache auf **Erkennungssprache festlegen** und starte anschließend die Untertitel. Du musst dafür nicht in die Systemeinstellungen wechseln. Eine automatische Spracherkennung wird nicht angeboten. Siehe [Apple Speech einrichten](../provider-setup.md#apple-speech).

**Apple Translation** ist ein separater lokaler Textübersetzungsdienst auf unterstützten Macs mit Apple-Chip und macOS 26 oder neuer. Wähle ihn unter **Textübersetzung**, speichere die Konfiguration und wähle eine konkrete Ausgangs- und Zielsprache. Mimi zeigt an, ob das Paar bereit ist; mit **Sprachen laden oder aktivieren** öffnest du Apples Einrichtungsbestätigung. Übersetzungsmodelle sind von Spracherkennungsmodellen getrennt. Vorhandene Modelle werden wiederverwendet; fehlende lädt Apple nach deiner Bestätigung. Verbindungsprüfungen und Untertitelsitzungen fordern nie einen Download an. Ein Text-API-Schlüssel oder Textproxy ist nicht nötig.

Wähle **Keine Übersetzung (nur Original)**, um mit Apple Speech nur den Originaltext anzuzeigen. Du kannst auch einen externen Textübersetzer wie [Index-Translate](#try-index-translate) verwenden; dieser Dienst erhält den erkannten Text.

<a id="try-index-translate"></a>

### Index-Translate ausprobieren

Bilibilis [Index-Translate](https://github.com/bilibili/Index-Translate#inference) bietet derzeit eine kostenlose öffentliche Übersetzungs-API an (Stand: 5. Oktober 2026). Sie kann die Textübersetzung für die Erkennung mit Alibaba Cloud oder Apple Speech übernehmen.

Öffne unter **Einstellungen → Sprache & Übersetzung** eine Konfiguration für **Alibaba Cloud** oder **Apple Speech** und wähle unter **Textübersetzung** die **OpenAI-kompatible API**. Verwende die Werte aus dem [offiziellen Beispiel](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41):

| Feld | Wert |
| --- | --- |
| Dienstadresse | `https://index-translate.bilibili.com/v1` |
| Modellname | `Index-Translate-35B-A3B` |
| API-Schlüssel | Leer lassen; die öffentliche API erfordert derzeit keine Authentifizierung. |

Speichere und starte dann die Verbindungsprüfung neben **Textübersetzung**. Falls für diese Adresse bereits ein Schlüssel gespeichert ist, entferne ihn mit **Übersetzungsschlüssel entfernen** und speichere erneut.

Index-Translate übernimmt ausschließlich die Textübersetzung. Lass bei Alibaba Cloud dessen Zugangsdaten für die Spracherkennung eingerichtet; für die Erkennung können weiterhin Gebühren entstehen. Apple Speech benötigt keinen Erkennungsschlüssel. Die Verfügbarkeit der kostenlosen API hängt vom Anbieter ab.

## Häufige Fragen

**macOS fragt trotz aktivierter Berechtigung immer wieder nach der Aufnahmeerlaubnis?** Beende Mimi und entferne ausschließlich seinen Eintrag unter Systemeinstellungen → Datenschutz & Sicherheit → Bildschirm- & Systemaudioaufnahme. Füge ihn anschließend erneut hinzu. Verwende für die veröffentlichte App `/Applications/mimi.app` oder für die Entwicklung `/Applications/mimi-dev.app`, aktiviere den Eintrag und öffne dieselbe App erneut. Siehe [Berechtigungen wiederherstellen](../usage.md#macos-permissions-after-an-update).

## Mitwirkende

Danke an alle, die Code schreiben, Probleme melden, Mimi ausprobieren oder weiterempfehlen (๑•̀ㅂ•́)و✧

Besonderer Dank gilt [@yebuwudong](https://github.com/yebuwudong) für die [Android-App](https://github.com/yuxino/mimi/pull/37) und [@LLLin000](https://github.com/LLLin000) für die [Untertitelanimation](https://github.com/yuxino/mimi/pull/67) und [Verbesserungen am Windows-Audio](https://github.com/yuxino/mimi/pull/89).

Danke auch an [@Chtholly000](https://github.com/Chtholly000) für die Verbesserungen an [Geminis fortlaufenden Untertiteln und dem geplanten Verbindungswechsel](https://github.com/yuxino/Mimi/pull/176).

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
</p>

[Alle Mitwirkenden](https://github.com/yuxino/mimi/graphs/contributors)

## Community

Danke an die Menschen bei [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305) und [Eleduck](https://eleduck.com/), die Mimi ausprobieren, Rückmeldungen geben und weiterempfehlen.

[MIT](../../LICENSE) © 2026 yuxino
