/** Initiate write during the click gesture. WebKit accepts a promised Blob,
 * whereas awaiting IPC before writeText loses its transient activation. */
export async function writeDiagnosticClipboard(report: Promise<string>): Promise<void> {
  if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
    const blob = report.then((value) => new Blob([value], { type: "text/plain" }));
    // A denied write may finish before IPC fails. Keep that later rejection
    // handled while preserving its failure for a clipboard consumer.
    void blob.catch(() => {});
    const item = new ClipboardItem({ "text/plain": blob });
    await navigator.clipboard.write([item]);
  } else {
    await navigator.clipboard.writeText(await report);
  }
}
