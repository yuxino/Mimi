/** One transient notification: replacing it restarts its lifetime, never stacks. */
export class TransientToast<T> {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly publish: (value: T | null) => void;
  constructor(publish: (value: T | null) => void) { this.publish = publish; }
  show(value: T, failure: boolean) {
    this.clearTimer();
    this.publish(value);
    this.timer = setTimeout(() => { this.timer = undefined; this.publish(null); }, failure ? 8000 : 3000);
  }
  clear() { this.clearTimer(); this.publish(null); }
  dispose() { this.clearTimer(); }
  private clearTimer() { if (this.timer !== undefined) clearTimeout(this.timer); this.timer = undefined; }
}
