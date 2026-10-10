# Mimi verwenden

[Zurück zur README](readme/de.md)

[English](usage.md) · [简体中文](usage.zh-CN.md) · [繁體中文](usage.zh-TW.md) · [日本語](usage.ja.md) · [ภาษาไทย](usage.th.md) · [한국어](usage.ko.md) · [Français](usage.fr.md) · [Deutsch](usage.de.md)

Links zur Freischaltung, Zugangsdaten und aktuelle Modelle findest du in der [Anleitung zur Anbietereinrichtung (Englisch)](provider-setup.md).

Wähle die Oberflächensprache unter Einstellungen → Allgemein: vereinfachtes oder traditionelles Chinesisch, Englisch, Japanisch, Deutsch, Koreanisch, Französisch oder Thai. Die Systemoption folgt der Systemsprache. Ein Sprachwechsel erhält aktuelle Eingaben und den Untertitelzustand. Erkennungs- und Übersetzungssprache werden getrennt gewählt.

## Installation und Updates

1. Lade die macOS-DMG für Apple-Chip oder Intel, Windows x64 EXE / MSI / portable ZIP oder Linux x86_64 .deb / AppImage von der [neuesten Veröffentlichung](https://github.com/yuxino/mimi/releases/latest). Du kannst Mimi auch aus dem Quellcode bauen.
2. Öffne Einstellungen → Sprache & Übersetzung, füge eine Konfiguration hinzu, gib die angeforderten Zugangsdaten ein und speichere. Wähle Erkennungs- und Übersetzungssprache.
3. Spiele Audio ab und aktiviere die Live-Untertitel unter Untertitel oder starte über das mimi-Symbol in der Menüleiste bzw. Taskleiste. Ab macOS 14.2 unterstützen alle Anwendungen und eine ausgewählte Anwendung die Berechtigung nur für Systemaudio. Bestehende Bildschirmaufnahme-Berechtigungen werden ohne zusätzliche Audio-Berechtigung weiterverwendet. macOS 13–14.1 benötigt Bildschirm- und Systemaudioaufnahme.

Du benötigst eigene API-Zugangsdaten; die Nutzung kann kostenpflichtig sein. Desktop-Zugangsdaten liegen in einer privaten Klartextdatei, geschützt durch lokale Dateiberechtigungen.

macOS, Windows-Installationen und Linux AppImage lassen sich unter Einstellungen → Allgemein → Softwareupdate aktualisieren. Mimi zeigt den Downloadfortschritt und bietet danach die Installation an. Windows öffnet Mimi nach der Installation erneut; macOS und Linux AppImage bieten einen gesonderten Neustart zum Abschluss an. Versionen vor v1.3.8 benötigen einmal eine manuelle Installation, um Updates in der App zu ermöglichen.

WebSocket-Beispiele, automatische Erkennung gegenüber Sprachhinweisen und unterstützte Sprachbereiche stehen unter [Sprachdienste und Sprachparameter (Englisch)](speech-language-setup.md#english).

### Plattformen

- macOS 13+ (Apple-Chip und Intel): Wähle `_aarch64.dmg` für Apple-Chip und `_x64.dmg` für Intel. Intel-Pakete gibt es seit v1.4.4. Build und Signatur sind geprüft; Audioerfassung und Berechtigungen auf Intel-Hardware sind noch nicht verifiziert. Die DMG ist nicht von Apple notarisiert. Wird der erste Start blockiert, wähle Trotzdem öffnen unter Systemeinstellungen → Datenschutz & Sicherheit. Beachte bei älteren Installationen die Berechtigungshinweise unten.
- Windows x64: Unsignierte Vorschau-Installer als EXE / MSI sowie seit v1.4.3 eine portable ZIP. SmartScreen kann warnen. Entpacke `mimi_<version>_x64-portable.zip` und starte `mimi.exe`. WebView2 muss installiert sein (unter Windows 11 normalerweise vorhanden). Einstellungen und private Zugangsdaten bleiben im Benutzerverzeichnis der App, Exporte am gewählten Speicherort; nichts wird in den ZIP-Ordner verschoben. Beende Mimi zum Update und ersetze die portable Kopie durch eine neue ZIP aus Releases. Sie führt keine Installer-Updates innerhalb der App aus.
- Linux x86_64 Vorschau (Ubuntu 22.04+ als Basis): `.deb` und AppImage benötigen PulseAudio oder PipeWire mit `pipewire-pulse`, ein funktionierendes Standardausgabegerät und Zugriff auf die private Zugangsdaten-Datei. Systemaudio wird nur vom Ausgabemonitor erfasst. Starte die Sitzung nach einem Gerätewechsel neu. X11 wird empfohlen. Wayland kann Positionierung, Vordergrundanzeige und Klickdurchleitung beschränken. Richte Systemtastenkürzel mit den in Mimi angezeigten Befehlen ein. Ohne Taskleistensymbol kannst du Einstellungen verwenden. Minimieren lässt Mimi weiterlaufen; Schließen der Einstellungen beendet Mimi unter Linux. Es gibt kein Linux-ARM64-Paket. Siehe [Linux-Einrichtung und Prüfung (Englisch)](development/linux.md).

### macOS-Systemaudio-Berechtigung

Neue Installationen ab macOS 14.2 fragen bei allen Anwendungen und ausgewählten Anwendungen nach der Berechtigung nur für Systemaudio. Zum Umstellen einer bestehenden Installation: Beende Mimi, entferne oder deaktiviere dessen Bildschirm- und Systemaudioaufnahme unter Systemeinstellungen → Datenschutz & Sicherheit, öffne Mimi erneut und starte Untertitel. Erlaube die reine Systemaudioaufnahme auf Nachfrage oder aktiviere Mimi in dieser Gruppe. Starte Mimi neu, wenn macOS es verlangt. Die Umstellung ist optional; bestehende Bildschirmberechtigungen funktionieren weiter. Bei fehlender Berechtigung kannst du die Systemseite aus der Fehlermeldung öffnen, den Zugriff erlauben und nach einem angeforderten Neustart erneut versuchen.

<a id="macos-permissions-after-an-update"></a>

### macOS fragt nach einem Update erneut nach Erlaubnis

macOS-Veröffentlichungen verwenden jetzt dasselbe feste selbstsignierte Zertifikat. Bis v1.4.1 wechselte die Ad-hoc-Signatur bei jedem Build. Beim ersten Wechsel zur festen Identität kann eine erneute Aufnahmeerlaubnis nötig sein. Eine Quellcodeänderung verändert keine bereits installierte App. Die feste Signatur verhindert wechselnde Build-Identitäten, garantiert aber nicht, dass macOS nie wieder fragt. Das Löschen eines lokalen Zertifikats ändert die installierte Identität nicht und hilft hier nicht. Verwende `/Applications/mimi.app` für die Veröffentlichung und `/Applications/mimi-dev.app` für Entwicklung. Beide behalten getrennte Signaturvorgaben und Aufnahmeberechtigungen; die Entwicklungsapp darf eine geänderte Release-Identität nicht übernehmen.

Ist die Berechtigung aktiviert, aber die Erfassung gesperrt, beende und öffne Mimi zuerst erneut und folge der normalen Nachfrage. Hilft das nicht:

1. Beende die betroffene App. Entferne unter Systemeinstellungen → Datenschutz & Sicherheit → Bildschirm- & Systemaudioaufnahme (der Name hängt von macOS ab) ausschließlich den alten Eintrag: **mimi** für die Veröffentlichung, **mimi-dev** für Entwicklung.
2. Füge mit + die passende `/Applications/mimi.app` oder `/Applications/mimi-dev.app` hinzu und aktiviere sie. Führe eine angeforderte Systemauthentifizierung selbst aus. Lass die andere Mimi-App und fremde Einträge unverändert.
3. Öffne dieselbe App, starte Untertitel über Tastenkürzel oder Start und spiele Audio mit Sprache ab. Prüfe, ob tatsächlich Untertitel erscheinen; ein aktivierter Schalter allein belegt keine Wiederherstellung.

Entfernt wird die alte Aufnahmeerlaubnis, kein Zertifikat oder API-Schlüssel. Wenn ein Versuch nicht hilft, wiederhole den Reset nicht ständig. [Melde den Fehler](https://github.com/yuxino/mimi/issues) mit macOS- und Mimi-Version, Installationsquelle und Fehlermeldung, ohne API-Schlüssel oder Untertitelinhalt.

Alte Betriebssystem-Zugangsdaten werden einmal importiert und erst nach Prüfung der gespeicherten privaten Klartextdatei gelöscht. Der Import kann eine Freigabe zum Lesen verlangen. Danach verwenden Wechsel, Speichern und Updates ausschließlich die Datei. Fehlende oder beschädigte Daten führen nicht zum Rückgriff auf den Schlüsselbund. Eine `codesign`-Nachfrage nach dem Signaturschlüssel ist eine separate Build-Nachfrage.

## Dienstkonfigurationen

Jede Desktop-Konfiguration kann ein Erkennungs-/Übersetzungssprachpaar speichern. Aktiviere die Konfiguration, wähle die Sprachen und öffne ihre Details unter Einstellungen → Sprache & Übersetzung. Wähle **Aktuelle Sprachen merken**. Änderungen oder Entfernen erfolgen ausdrücklich mit **Mit aktuellen Sprachen aktualisieren** oder **Gespeichertes Paar entfernen**.

Beim Wechsel oder erneuten Anwenden wird das gespeicherte Paar wiederhergestellt. Vorübergehende Sprachänderungen überschreiben es nicht. Taskleisten- und schwebende Menüs zeigen das Paar unter dem Konfigurationsnamen. Über die Dienstinformation oben im Untertitel öffnest du die aktuellen Konfigurationsdetails.

## Audio und gespeicherte Sitzungen

Systemaudio ist voreingestellt. Wähle in Einstellungen oder den schwebenden Bedienelementen Systemaudio, Mikrofon oder beide. Das Mikrofon verwendet das Standard-Eingabegerät und fordert nötige Berechtigungen erst beim Erfassungsstart an. Bei zwei Eingängen sind Erkennungsverbindungen, Untertitel und Dienstverbrauch unabhängig. Ein Eingangswechsel schaltet die Aufnahme aus; aktiviere sie bei Bedarf unter Speichern & Exportieren erneut.

Unter macOS und Windows build 20348+ kannst du mit der App-Auswahl das Audio einer bestimmten Anwendung wählen. Linux erfasst den Systemausgabemonitor und hat diese Auswahl nicht.

Unter Einstellungen → Speichern & Exportieren kannst du Untertitel speichern oder die ausgewählten Audioeingänge aufnehmen. Beide Optionen sind standardmäßig aus. Aktivierte Inhalte werden fortlaufend in private lokale Sitzungsdateien geschrieben; Systemaudio und Mikrofon bleiben getrennt. Ausschalten löscht den entsprechenden Inhalt der aktuellen Sitzung. Bereits gespeicherte Aufzeichnungen musst du ausdrücklich löschen.

Eine neue Desktop-Sitzung behält die begrenzten bestätigten Untertitel auf dem Bildschirm. Leeren entfernt sie. Das aktiviert keine Speicherung, kopiert keine alten Zeilen in die neue Aufzeichnung und stellt ungespeicherte Untertitel nach dem Beenden nicht wieder her.

Die Grenzen sind 10.000 bestätigte Untertitelpaare / 2 MiB Text und 64 MiB Audio. Beim Erreichen stoppt die Speicherung mit einem Hinweis. Untertitelzeiten markieren die Bestätigung; WAV lässt Pausen und Verbindungslücken aus. Beide Zeitachsen sind daher nicht direkt synchronisiert.

## Untertitelanzeige

Wähle in Einstellungen eine von fünf Farben (standardmäßig Weiß) oder öffne die Systemfarbauswahl über das benutzerdefinierte Farbfeld dahinter. Systemaudio und Mikrofon haben getrennte Farben. Vorschau und schwebende Untertitel ändern sich sofort, auch im immersiven Modus. Zweisprachige Untertitel zeigen das Original dezenter als die Übersetzung.

Uhrzeit anzeigen ist in Einstellungen und schwebenden Bedienelementen standardmäßig aus. Aktiviert zeigt es die lokale Bestätigungszeit (HH:mm:ss) neben bestätigten Untertiteln beider Eingänge, auch im immersiven Modus. Dies ist nicht der Sprechbeginn. Nach dem Wechsel zu reinem Systemaudio behalten vorhandene Mikrofonzeilen ein kleines Quellensymbol. Die schwebenden Bedienelemente bieten auch Pause/Fortsetzen.

Wähle Nur Übersetzung, Original + Übersetzung oder Nur Original in Einstellungen, Untertitelsteuerung oder Taskleiste. ⌘⇧B unter macOS bzw. Ctrl+Shift+B unter Windows/Linux X11 wechselt sofort, ohne die Übersetzung zu unterbrechen. Unter Wayland binde `mimi --cycle-subtitle-display` in den Systemeinstellungen. Zweisprachig werden bestätigtes Original und Übersetzung zusammen angezeigt; während die Übersetzung aussteht, erscheint das erkannte Original vorab.

Untertitel früher anzeigen zeigt veränderliche Entwürfe. Ausgeschaltet wartet die Anzeige auf bestätigte Ergebnisse; durchgehendes Sprechen kann länger dauern. Entwürfe werden dadurch nicht zu gespeicherter Historie.

## Erkennungsfehler

Bei Erkennungsfehlern bleiben vorhandene Untertitel und der Fehlergrund sichtbar. Öffne die schwebende Steuerung für Details und Wiederherstellung. Erkannte Konfigurationsfehler bieten Sprache & Übersetzung, vorübergehende Verbindungs- oder Dienstfehler bieten Wiederholen. Das Korrigieren oder Wechseln der Konfiguration bringt die fehlgeschlagene Sitzung in den Leerlauf, ohne Audio zu starten. Aktiviere Live-Untertitel erneut, wenn du bereit bist.

## Weitere Dokumentation

[Mitwirken (Chinesisch/Englisch)](../.github/CONTRIBUTING.md) · [Sicherheit & Datenschutz (Chinesisch/Englisch)](../.github/SECURITY.md) · [Plattformunterschiede und Prüfstand (Englisch)](development/platform-parity.md)
