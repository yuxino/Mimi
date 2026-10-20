import { createContext } from "react";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { I18N } from "../../lib/i18n";
import type { ServiceProfile, TextTranslationNameDraft } from "../../lib/types";

export type NameDraftKey = "profile" | "recognition" | `translation:${TextTranslationNameDraft["route"]}`;
type NameError = { message: string; retry: boolean } | null;
type Options = { readOnly: boolean; allowEmpty: boolean; onSave: (name: string) => Promise<unknown> };

/** Only display metadata lives here; credentials remain owned by their editor. */
export class NameDraft {
  draft: string;
  confirmed: string;
  source: string;
  revision = 0;
  failedRevision = -1;
  composing = false;
  private interruptedComposition = false;
  focused = false;
  inFlight = false;
  flushRequested = false;
  views = 0;
  discarded = false;
  ownValues = new Set<string>();
  options: Options | null = null;
  private submittedName: string | null = null;
  private pendingSource: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private listeners = new Set<() => void>();
  private snapshot: { draft: string; error: NameError };
  private release: () => void;

  constructor(value: string, release: () => void = () => {}) {
    this.draft = this.confirmed = this.source = value;
    this.snapshot = { draft: value, error: null };
    this.release = release;
  }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  private publish(draft = this.draft, error = this.snapshot.error) {
    if (draft === this.snapshot.draft && error === this.snapshot.error) return;
    this.snapshot = { draft, error };
    for (const listener of this.listeners) listener();
  }
  clearTimer() {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }
  schedule() {
    this.clearTimer();
    if (!this.composing && !this.discarded) this.timer = setTimeout(() => { void this.flush(); }, 350);
  }
  change(next: string) {
    if (!this.composing) this.interruptedComposition = false;
    this.draft = next.replace(/[\r\n]/g, "");
    this.revision += 1;
    this.publish(this.draft, null);
    this.schedule();
  }
  receive(value: string) {
    if (this.source === value) return;
    this.source = value;
    const wasClean = this.draft.trim() === this.confirmed;
    const ownAcknowledgement = this.ownValues.delete(value);
    if (this.inFlight) {
      if (!ownAcknowledgement) this.pendingSource = value;
      else if (value === this.submittedName) this.pendingSource = null;
    }
    if (!this.inFlight && this.draft.trim() === value) {
      this.confirmed = value;
      this.publish(this.draft, null);
    } else if (!(ownAcknowledgement && this.focused) && !this.inFlight) {
      // A late acknowledgement cannot replace newer typing or move the caret.
      // Once clean, accept external changes, including a previously used name.
      if (wasClean && !this.composing && !this.interruptedComposition) {
        this.confirmed = this.draft = value;
        this.publish(this.draft, null);
      } else if (!ownAcknowledgement) {
        this.confirmed = value;
      }
    }
    this.release();
  }
  retry() {
    this.failedRevision = -1;
    this.publish(this.draft, null);
    void this.flush();
  }
  confirm() {
    this.interruptedComposition = false;
    void this.flush();
  }
  detach() {
    this.views -= 1;
    // The old native composer cannot finish in a replacement input. Preserve
    // its text, but require another edit/composition end or explicit Enter
    // before treating preedit as confirmed display metadata.
    if (this.composing) this.interruptedComposition = true;
    this.focused = this.composing = false;
    this.clearTimer();
    void this.flush();
    this.release();
  }
  discard() {
    this.discarded = true;
    this.clearTimer();
  }
  canRelease() {
    return this.views === 0 && !this.inFlight && !this.composing && !this.interruptedComposition && this.snapshot.error === null
      && this.draft.trim() === this.confirmed && this.source === this.confirmed;
  }
  flush = async () => {
    this.clearTimer();
    const options = this.options;
    // Disabling new edits must not drop an owned draft. Native guards validate
    // its write; deleting its profile cancels any subsequent coalesced write.
    if (this.discarded || !options || options.readOnly || this.composing || this.interruptedComposition) return;
    if (this.inFlight) { this.flushRequested = true; return; }
    const name = this.draft.trim();
    if (!options.allowEmpty && !name) {
      this.publish(this.draft, { message: I18N.settings.nameRequired, retry: false });
      return;
    }
    if (name === this.confirmed || this.failedRevision === this.revision) return;
    const revision = this.revision;
    this.inFlight = true;
    this.flushRequested = false;
    this.submittedName = name;
    this.pendingSource = null;
    this.ownValues.add(name);
    if (this.ownValues.size > 32) this.ownValues.delete(this.ownValues.values().next().value!);
    try {
      const result = await options.onSave(name);
      if (result === null || result === false) throw new Error("name_save_rejected");
      // The reply acknowledges this write; a newer received source owns the
      // baseline, including when a local ABA edit must still be saved again.
      this.confirmed = this.pendingSource ?? name;
      if (revision === this.revision && !this.composing && !this.interruptedComposition) {
        if (this.pendingSource !== null && this.draft.trim() !== this.pendingSource) this.draft = this.pendingSource;
        this.publish(this.draft, null);
      }
    } catch (cause) {
      if (this.pendingSource !== null) this.confirmed = this.pendingSource;
      this.ownValues.delete(name);
      this.failedRevision = revision;
      if (revision === this.revision) this.publish(this.draft, { message: profileErrorMessage(cause), retry: true });
    } finally {
      this.inFlight = false;
      this.submittedName = this.pendingSource = null;
      if (this.flushRequested) {
        this.flushRequested = false;
        void this.flush();
      }
      this.release();
    }
  };
}

export class NameDraftOwner {
  private profiles = new Map<string, Map<NameDraftKey, NameDraft>>();
  get(profileId: string, key: NameDraftKey, value: string) {
    let fields = this.profiles.get(profileId);
    if (!fields) { fields = new Map(); this.profiles.set(profileId, fields); }
    let draft = fields.get(key);
    if (!draft) {
      draft = new NameDraft(value, () => {
        // StrictMode rehearses effect cleanup before reattaching the same view.
        // Release only after the commit, and only if this generation still owns
        // the key: a deleted/recreated profile must not lose its new controller.
        queueMicrotask(() => {
          const ownedFields = this.profiles.get(profileId);
          if (draft!.canRelease() && ownedFields && ownedFields.get(key) === draft) {
            ownedFields.delete(key);
            if (ownedFields.size === 0) this.profiles.delete(profileId);
          }
        });
      });
      fields.set(key, draft);
    }
    return draft;
  }
  discard(profileId: string) {
    const fields = this.profiles.get(profileId);
    this.profiles.delete(profileId);
    for (const draft of fields?.values() ?? []) draft.discard();
  }
  prune(profiles: ServiceProfile[]) {
    const ids = new Set(profiles.map(profile => profile.id));
    for (const id of this.profiles.keys()) if (!ids.has(id)) this.discard(id);
  }
  sync(profiles: ServiceProfile[]) {
    const saved = new Map(profiles.map(profile => [profile.id, profile]));
    for (const [id, fields] of this.profiles) {
      const profile = saved.get(id);
      if (!profile) { this.discard(id); continue; }
      for (const [key, draft] of fields) {
        const value = key === "profile" ? profile.name : key === "recognition" ? profile.speechRecognitionName ?? ""
          : profile.textTranslationNames?.[key.slice("translation:".length) as TextTranslationNameDraft["route"]] ?? "";
        draft.receive(value);
      }
    }
  }
}

export const NameDraftContext = createContext<{ owner: NameDraftOwner; profileId: string } | null>(null);
