import type { DEVELOPMENT as EnglishCopy } from "./en-development";

export const DEVELOPMENT = {
    title: "Entwicklungsdebugger", start: "Ablaufverfolgung starten", evidence: "Audio und Untertitel aufzeichnen", stop: "Ablaufverfolgung stoppen", export: "Fall exportieren",
    help: "Nur Entwicklung. Ablaufverfolgungen enthalten keine Inhalte; die Aufnahme speichert ausdrücklich gesendetes Audio der ausgewählten Eingänge und Untertitelzustände in privaten lokalen Dateien. Nach einem Neustart ausgeschaltet.",
    live: "Aktuelle Erkennung und Übersetzung", timeline: "Ereignisverlauf", all: "Alle Stufen", provider: "Anbieterereignisse", reduced: "Untertitelverarbeitung", frontend: "Oberfläche", pipeline: "Audio und Anfragen", snapshot: "Zustandsaufnahmen", loss: "Fehlende Nachweise", audio: "An den Dienst gesendetes Audio", listen: "Audio laden",
    audioHelp: "PCM aus tatsächlichen Audionachrichten, deren lokaler Socket-Versand abgeschlossen wurde. Dies belegt weder Empfang noch Verarbeitung durch den Server. Prüfe Fehler, Abbrüche und Nachweisverluste. Stoppe vor dem Anhören die Untertitel, um eine erneute Erfassung zu vermeiden.",
    system: "Systemaudio", microphone: "Mikrofon", stopFirst: "Zum Anhören Untertitel stoppen", replay: "Untertitelzustände wiedergeben", previous: "Zurück", next: "Weiter", raw: "Original", translated: "Übersetzung", empty: "Noch keine Aufzeichnungen", idle: "Gestoppt", active: "Ablaufverfolgung läuft", saved: "Fall exportiert", failed: "Vorgang fehlgeschlagen. Erneut versuchen.", details: "Ereignisfelder", workspace: "Arbeitsbereich", route: "Erfasste Konfiguration",
    recordHelp: "Audio- und Untertitelnachweise sind standardmäßig aus und enthalten private Inhalte. Sie werden nie automatisch geteilt. Jede Aufnahme erstellt eigene Dateien und bewahrt ältere Fälle.",
    replayHelp: "Die Offline-Wiedergabe verwendet dieselbe Untertitelprojektion und dasselbe Layout. Einzelne Schritte bilden das ursprüngliche Timing der Stabilisierung nicht nach; verwende die Ablaufverfolgung für ursprüngliche Textübernahmen und abgeschnittene Inhalte.",
    final: "Bestätigte Untertitel", clear: "Leeren", pause: "Pausiert", pending: "Übersetzung ausstehend", events: "Ereignisse", noSubtitles: "Noch keine Erkennung oder Übersetzung",
  } satisfies typeof EnglishCopy;
