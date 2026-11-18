/** Keep the latest subtitle visible as the viewport or its rows reflow, even
 * when compact-lane measurement changes only scrollHeight after returning. */
export function observeTimelineResize(element: HTMLElement, onResize = () => {
  element.scrollTo({ top: element.scrollHeight, behavior: "instant" });
}): () => void {
  const observer = new ResizeObserver(() => {
    onResize();
  });
  observer.observe(element);
  for (const row of Array.from(element.children)) observer.observe(row);
  return () => observer.disconnect();
}
