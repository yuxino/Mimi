type Choice = {
  family: string;
  persist: (family: string) => Promise<unknown>;
  failed: () => void;
  isCurrent: () => boolean;
};

/** One WebView owns this preference's bounded write queue, including while its
 * settings page is unmounted. Views own previews and notifications separately. */
export class SubtitleFontChoiceOwner {
  private pendingFamily: string | null = null;
  private queued: Choice | null = null;
  private saving = false;
  private listeners = new Set<() => void>();

  getSnapshot = () => this.pendingFamily;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  choose(family: string, persist: Choice["persist"], failed: Choice["failed"], isCurrent: Choice["isCurrent"]) {
    this.queued = { family, persist, failed, isCurrent };
    this.pendingFamily = family;
    // Reserve the queue before notifying a synchronous/reentrant subscriber.
    const start = !this.saving;
    this.saving = true;
    this.publish();
    if (start) void this.drain();
  }

  private publish() {
    for (const listener of this.listeners) listener();
  }

  private discardSatisfiedChoice(family: string) {
    // A successful old command can have been superseded by a native snapshot.
    // Check the latest choice's live source before treating an ABA as satisfied.
    if (this.queued?.family === family && this.queued.isCurrent()) this.queued = null;
  }

  private async drain() {
    try {
      while (this.queued) {
        const choice = this.queued;
        this.queued = null;
        try {
          const result = await choice.persist(choice.family);
          if (result === false || result === null) throw new Error("font_save_rejected");
          // Returning to the successful in-flight choice needs no second write.
          this.discardSatisfiedChoice(choice.family);
        } catch {
          choice.failed();
        }
      }
    } finally {
      this.saving = false;
      this.pendingFamily = null;
      this.publish();
    }
  }
}

export const subtitleFontChoiceOwner = new SubtitleFontChoiceOwner();
