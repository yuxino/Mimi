import { traceMethod } from "./historyProbe";

type ScrollIntent =
  | { kind: "down" | "pointer"; top: number }
  | { kind: "scrollbar"; top: number; range: number };

function maxScrollTop(element: HTMLElement) {
  return Math.max(0, element.scrollHeight - element.clientHeight);
}

/** Owns follow/read intent and sentence anchors; never changes subtitle text. */
export class TimelineScroll {
  private following = true;
  private userScrolling: ScrollIntent | null = null;
  private modeAnchor = false;
  private reading: { id: string; offset: number } | null = null;
  private readingStart = false;
  private smoothFollowing = false;
  private pointerInterruptedGlide = false;

  pointerDown(element: HTMLElement, scrollbar = false) {
    traceMethod("pointerDown", "enter", this.following, this.userScrolling, element);
    try {
    this.pointerInterruptedGlide = this.smoothFollowing;
    if (this.pointerInterruptedGlide) {
      // Freeze automatic movement before attributing any scroll to a pointer.
      // A plain click resumes the glide on release; a real drag enters reading.
      this.smoothFollowing = false;
      this.move(element, element.scrollTop, "instant");
    }
    this.userIntent(element, scrollbar ? "scrollbar" : "pointer");

    } finally {
      traceMethod("pointerDown", "exit", this.following, this.userScrolling, element);
    }
  }

  pointerMove(element: HTMLElement) {
    traceMethod("pointerMove", "enter", this.following, this.userScrolling, element);
    try {
    if (this.userScrolling?.kind !== "scrollbar") this.userIntent(element);

    } finally {
      traceMethod("pointerMove", "exit", this.following, this.userScrolling, element);
    }
  }

  pointerUp(element: HTMLElement) {
    traceMethod("pointerUp", "enter", this.following, this.userScrolling, element);
    try {
    const scrollbar = this.userScrolling?.kind === "scrollbar";
    // A quick release can precede the last scroll event from the browser.
    if (scrollbar) this.scrolled(element);
    this.endUserIntent();
    const resume = this.pointerInterruptedGlide && this.following;
    this.pointerInterruptedGlide = false;
    if (resume) this.contentChanged(element, "smooth");
    else if (scrollbar && this.following && !this.atTail(element)) this.contentChanged(element, "instant");

    } finally {
      traceMethod("pointerUp", "exit", this.following, this.userScrolling, element);
    }
  }

  userIntent(element: HTMLElement, kind: ScrollIntent["kind"] = "pointer") {
    traceMethod("userIntent", "enter", this.following, this.userScrolling, element);
    try {
    this.smoothFollowing = false;
    this.readingStart = false;
    const top = element.scrollTop;
    this.userScrolling = kind === "scrollbar" ? { kind, top, range: maxScrollTop(element) } : { kind, top };
    this.rememberReading(element);

    } finally {
      traceMethod("userIntent", "exit", this.following, this.userScrolling, element);
    }
  }

  endUserIntent() {
    traceMethod("endUserIntent", "enter", this.following, this.userScrolling);
    try {
    this.userScrolling = null;

    } finally {
      traceMethod("endUserIntent", "exit", this.following, this.userScrolling);
    }
  }

  /** Enter full confirmed-history reading even if the compact rows do not
   * overflow yet, so an upward wheel/key can reveal a single long sentence. */
  beginReading(element: HTMLElement) {
    traceMethod("beginReading", "enter", this.following, this.userScrolling, element);
    try {
    this.pointerInterruptedGlide = false;
    this.smoothFollowing = false;
    this.readingStart = false;
    this.endUserIntent();
    this.following = false;
    this.rememberReading(element);

    } finally {
      traceMethod("beginReading", "exit", this.following, this.userScrolling, element);
    }
  }

  /** Explicit Home outranks the old compact-row anchor while full rows finish
   * measuring. A new gesture releases it back to ordinary sentence reading. */
  readFromStart(element: HTMLElement) {
    traceMethod("readFromStart", "enter", this.following, this.userScrolling, element);
    try {
    this.beginReading(element);
    this.modeAnchor = false;
    this.readingStart = true;
    this.move(element, 0, "instant");
    this.rememberReading(element);

    } finally {
      traceMethod("readFromStart", "exit", this.following, this.userScrolling, element);
    }
  }

  isFollowing(): boolean {
    return this.following;
  }

  followTail(element: HTMLElement) {
    traceMethod("followTail", "enter", this.following, this.userScrolling, element);
    try {
    this.endUserIntent();
    this.pointerInterruptedGlide = false;
    this.smoothFollowing = false;
    this.following = true;
    this.modeAnchor = false;
    this.reading = null;
    this.readingStart = false;
    this.move(element, element.scrollHeight, "instant");

    } finally {
      traceMethod("followTail", "exit", this.following, this.userScrolling, element);
    }
  }

  /** Called only by a new downward gesture; short text may emit no scroll. */
  followIfAtTail(element: HTMLElement) {
    traceMethod("followIfAtTail", "enter", this.following, this.userScrolling, element);
    try {
    if (this.atTail(element)) this.followTail(element);

    } finally {
      traceMethod("followIfAtTail", "exit", this.following, this.userScrolling, element);
    }
  }

  scrolled(element: HTMLElement) {
    traceMethod("scrolled", "enter", this.following, this.userScrolling, element);
    try {
    const atTail = this.atTail(element);
    if (atTail) this.smoothFollowing = false;
    const intent = this.userScrolling;
    if (intent && (intent.kind !== "scrollbar" || intent.range === maxScrollTop(element))) {
      const downward = element.scrollTop > intent.top ||
        (intent.kind === "down" && element.scrollTop === intent.top);
      if (atTail && downward) this.following = true;
      else if (!atTail && element.scrollTop !== intent.top) this.following = false;
    }
    this.syncScrollPosition(element);
    // Native layout/anchor corrections may emit scroll without user input.
    // Keep the actual visible anchor, but never use them to leave reading.
    if (!this.following) this.rememberReading(element);

    } finally {
      traceMethod("scrolled", "exit", this.following, this.userScrolling, element);
    }
  }

  displayChanged(element: HTMLElement) {
    traceMethod("displayChanged", "enter", this.following, this.userScrolling, element);
    try {
    this.pointerInterruptedGlide = false;
    this.smoothFollowing = false;
    this.modeAnchor = true;
    this.reflow(element);

    } finally {
      traceMethod("displayChanged", "exit", this.following, this.userScrolling, element);
    }
  }

  contentChanged(element: HTMLElement, behavior: ScrollBehavior, motionEnabled = true) {
    traceMethod("contentChanged", "enter", this.following, this.userScrolling, element);
    try {
    this.modeAnchor = false;
    if (!motionEnabled) this.pointerInterruptedGlide = false;
    if (this.following && (this.pointerInterruptedGlide || this.userScrolling?.kind === "scrollbar")) return;
    // Streaming revisions can arrive before a new sentence reaches the tail.
    // Keep retargeting that glide until it finishes or motion is disabled.
    const followBehavior = motionEnabled && (behavior === "smooth" || this.smoothFollowing) ? "smooth" : "instant";
    this.smoothFollowing = this.following && followBehavior === "smooth" && !this.atTail(element);
    if (this.following) this.move(element, element.scrollHeight, followBehavior);
    else this.restoreReading(element);

    } finally {
      traceMethod("contentChanged", "exit", this.following, this.userScrolling, element);
    }
  }

  reflow(element: HTMLElement) {
    traceMethod("reflow", "enter", this.following, this.userScrolling, element);
    try {
    if (!this.following) {
      this.restoreReading(element);
      return;
    }
    if (this.pointerInterruptedGlide || this.userScrolling?.kind === "scrollbar") return;
    // Keep the newest sentence's original in view after a display-mode change.
    // New content resumes ordinary tail following.
    const latest = element.lastElementChild as HTMLElement | null;
    const top = this.modeAnchor && latest ? this.top(element, latest) : element.scrollHeight;
    // Wrapping and lane-budget changes can resize rows during a glide.
    // Retarget it instead of snapping to the final tail.
    if (this.atTail(element)) this.smoothFollowing = false;
    this.move(element, top, this.smoothFollowing ? "smooth" : "instant");

    } finally {
      traceMethod("reflow", "exit", this.following, this.userScrolling, element);
    }
  }

  private rememberReading(element: HTMLElement) {
    const top = element.getBoundingClientRect().top;
    const first = Array.from(element.children).find(child => child.getBoundingClientRect().bottom > top);
    const id = (first as HTMLElement | undefined)?.dataset.utteranceId;
    this.reading = first && id ? { id, offset: first.getBoundingClientRect().top - top } : null;
  }

  private restoreReading(element: HTMLElement) {
    this.syncScrollPosition(element);
    if (this.readingStart) {
      this.move(element, 0, "instant");
      this.rememberReading(element);
      return;
    }
    if (!this.reading) return;
    const block = Array.from(element.children).find(child => (child as HTMLElement).dataset.utteranceId === this.reading!.id) as HTMLElement | undefined;
    // A bounded history may evict the old sentence; preserve the current
    // reading position rather than jumping to the newest sentence in that case.
    if (block) {
      // Removing an original lane can make a sentence shorter than the
      // former reading offset. Reveal that same sentence's start rather than
      // leaving it entirely above the viewport and showing its successor.
      const rect = block.getBoundingClientRect();
      const offset = -this.reading.offset >= rect.bottom - rect.top ? 0 : this.reading.offset;
      this.reading.offset = offset;
      this.move(element, this.top(element, block) - offset, "instant");
    }
  }

  private top(element: HTMLElement, child: HTMLElement) {
    return element.scrollTop + child.getBoundingClientRect().top - element.getBoundingClientRect().top;
  }

  private move(element: HTMLElement, top: number, behavior: ScrollBehavior) {
    element.scrollTo({ top: Math.max(0, Math.min(top, maxScrollTop(element))), behavior });
    this.syncScrollPosition(element);
  }

  private syncScrollPosition(element: HTMLElement) {
    traceMethod("syncScrollPosition", "enter", this.following, this.userScrolling, element);
    try {
    // Native thumb drags can send many scroll events without a pointermove.
    // Retain only that gesture, rebasing it after our own moves and reflows.
    if (this.userScrolling?.kind === "scrollbar") {
      this.userScrolling.top = element.scrollTop;
      this.userScrolling.range = maxScrollTop(element);
    } else {
      this.endUserIntent();
    }

    } finally {
      traceMethod("syncScrollPosition", "exit", this.following, this.userScrolling, element);
    }
  }

  private atTail(element: HTMLElement): boolean {
    return maxScrollTop(element) - element.scrollTop <= 2;
  }
}
