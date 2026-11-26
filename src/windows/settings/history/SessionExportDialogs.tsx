import type { ReactNode } from "react";
import { Switch } from "../../../components/inputs/Switch";
import { I18N } from "../../../lib/i18n";
import type { SessionHistoryItem, TranscriptExportOptions } from "../../../lib/types";
import { SettingsConfirmation } from "../controls/SettingsConfirmation";
import { InlineFeedback, SettingsRow, SettingsSelect } from "../controls/SettingsPrimitives";
import { SettingsHelp } from "../feedback/SettingsHelp";
import type { SessionWorkspaceStrings } from "./SessionHistoryLibrary";

export type PendingBatchDelete = { mode: "selected" | "all"; ids: string[] } | null;

interface SessionExportDialogsProps {
  visible: boolean;
  readerOpen: boolean;
  busy: boolean;
  selected: SessionHistoryItem | undefined;
  copy: SessionWorkspaceStrings;
  confirmDelete: boolean;
  onCancelDelete: () => void;
  onConfirmDelete: (id: string) => void;
  pendingBatchDelete: PendingBatchDelete;
  onCancelBatchDelete: () => void;
  onConfirmBatchDelete: (mode: "selected" | "all", ids: string[]) => void;
  renameOpen: boolean;
  nameDraft: string;
  renameError: boolean;
  onNameDraftChange: (value: string) => void;
  onCancelRename: () => void;
  onConfirmRename: (id: string) => void;
  exportOpen: boolean;
  exportDisabled: boolean;
  exportAudio: boolean;
  canExportAudio: boolean;
  canExportTranscript: boolean;
  audioSourceCount: number;
  sourceControl: ReactNode;
  exportOptions: TranscriptExportOptions;
  contentOptions: { value: string; label: string }[];
  onExportAudioChange: (exportAudio: boolean) => void;
  onExportOptionsChange: (updater: (current: TranscriptExportOptions) => TranscriptExportOptions) => void;
  onCancelExport: () => void;
  onConfirmExport: () => void;
  manualCopy: string | null;
  onCloseManualCopy: () => void;
}

export function SessionExportDialogs({
  visible,
  readerOpen,
  busy,
  selected,
  copy,
  confirmDelete,
  onCancelDelete,
  onConfirmDelete,
  pendingBatchDelete,
  onCancelBatchDelete,
  onConfirmBatchDelete,
  renameOpen,
  nameDraft,
  renameError,
  onNameDraftChange,
  onCancelRename,
  onConfirmRename,
  exportOpen,
  exportDisabled,
  exportAudio,
  canExportAudio,
  canExportTranscript,
  audioSourceCount,
  sourceControl,
  exportOptions,
  contentOptions,
  onExportAudioChange,
  onExportOptionsChange,
  onCancelExport,
  onConfirmExport,
  manualCopy,
  onCloseManualCopy,
}: SessionExportDialogsProps) {
  if (!visible) return null;

  return (
    <>
      {readerOpen && confirmDelete && selected && (
        <SettingsConfirmation
          message={I18N.settings.historyDeleteConfirm}
          confirmLabel={I18N.settings.historyDelete}
          disabled={busy}
          onCancel={onCancelDelete}
          onConfirm={() => onConfirmDelete(selected.id)}
        />
      )}
      {pendingBatchDelete && pendingBatchDelete.ids.length > 0 && (
        <SettingsConfirmation
          message={
            pendingBatchDelete.mode === "all"
              ? copy.clearAllConfirm(pendingBatchDelete.ids.length)
              : copy.deleteSelectedConfirm(pendingBatchDelete.ids.length)
          }
          confirmLabel={
            pendingBatchDelete.mode === "all"
              ? copy.clearAll
              : copy.deleteSelected(pendingBatchDelete.ids.length)
          }
          disabled={busy}
          onCancel={onCancelBatchDelete}
          onConfirm={() => onConfirmBatchDelete(pendingBatchDelete.mode, pendingBatchDelete.ids)}
        />
      )}
      {renameOpen && selected && (
        <SettingsConfirmation
          message={copy.rename}
          variant="default"
          confirmIcon="checkmark"
          confirmLabel={copy.save}
          disabled={busy || [...nameDraft.trim()].length > 120}
          onCancel={onCancelRename}
          onConfirm={() => onConfirmRename(selected.id)}
        >
          <label className="session-workspace__name-field">
            {copy.name}
            <SettingsHelp label={I18N.settings.helpLabel} text={copy.nameHelp} />
            <input
              value={nameDraft}
              maxLength={240}
              disabled={busy}
              onChange={(event) => onNameDraftChange(event.target.value)}
            />
          </label>
          {[...nameDraft.trim()].length > 120 && <InlineFeedback tone="error">{copy.nameHelp}</InlineFeedback>}
          {renameError && <InlineFeedback tone="error">{copy.renameFailed}</InlineFeedback>}
        </SettingsConfirmation>
      )}
      {exportOpen && (
        <SettingsConfirmation
          message={copy.export}
          variant="default"
          confirmIcon="download"
          confirmLabel={copy.chooseFile}
          disabled={exportDisabled || (exportAudio ? !canExportAudio : !canExportTranscript)}
          onCancel={onCancelExport}
          onConfirm={onConfirmExport}
        >
          <p className="session-workspace__export-scope">{copy.whole}</p>
          <div className="session-workspace__export-options">
            <SettingsRow label={copy.format}>
              <SettingsSelect
                label={copy.format}
                value={exportAudio ? "wav" : exportOptions.format}
                onChange={(value) => {
                  onExportAudioChange(value === "wav");
                  if (value !== "wav") {
                    onExportOptionsChange((options) => ({
                      ...options,
                      format: value as TranscriptExportOptions["format"],
                    }));
                  }
                }}
                options={[
                  ...(canExportTranscript
                    ? [{ value: "txt", label: copy.plain }, { value: "markdown", label: copy.markdown }]
                    : []),
                  ...(canExportAudio ? [{ value: "wav", label: copy.wav }] : []),
                ]}
              />
            </SettingsRow>
            {exportAudio ? (
              audioSourceCount > 1 && <SettingsRow label={I18N.settings.recordingSource}>{sourceControl}</SettingsRow>
            ) : (
              <>
                <SettingsRow label={copy.content}>
                  <SettingsSelect
                    label={copy.content}
                    value={exportOptions.content}
                    onChange={(value) =>
                      onExportOptionsChange((options) => ({
                        ...options,
                        content: value as TranscriptExportOptions["content"],
                      }))}
                    options={contentOptions}
                  />
                </SettingsRow>
                <SettingsRow label={copy.timestamps} description={copy.timing}>
                  <Switch
                    checked={exportOptions.includeTimestamps}
                    aria-label={copy.timestamps}
                    onChange={(includeTimestamps) =>
                      onExportOptionsChange((options) => ({ ...options, includeTimestamps }))}
                  />
                </SettingsRow>
              </>
            )}
          </div>
        </SettingsConfirmation>
      )}
      {manualCopy !== null && (
        <SettingsConfirmation
          message={copy.copyFailed}
          variant="default"
          hideConfirm
          onCancel={onCloseManualCopy}
          onConfirm={() => {}}
        >
          <textarea
            className="session-workspace__manual-copy"
            readOnly
            value={manualCopy}
            aria-label={copy.copy}
            onFocus={(event) => event.currentTarget.select()}
          />
        </SettingsConfirmation>
      )}
    </>
  );
}
