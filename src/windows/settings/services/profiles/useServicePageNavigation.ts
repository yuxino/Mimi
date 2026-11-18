import { useLayoutEffect, useRef, type RefObject } from "react";

/** Subpages share a viewport, but not a reading position or keyboard focus. */
export function useServicePageNavigation(root: RefObject<HTMLDivElement | null>, page: string, visible: boolean, request: number) {
  const previousPage = useRef<string | null>(null);
  const returnTo = useRef<{ page: string; top: number; focusId: string } | null>(null);

  const rememberPagePosition = (focusId: string) => {
    const scroll = root.current?.closest<HTMLElement>(".settings-console__scroll");
    returnTo.current = { page, top: scroll?.scrollTop ?? 0, focusId };
  };

  useLayoutEffect(() => {
    if (!visible) { previousPage.current = null; return; }
    const previous = previousPage.current;
    previousPage.current = page;
    if (previous === null && page === "list") return;
    const scroll = root.current?.closest<HTMLElement>(".settings-console__scroll");
    const returning = returnTo.current?.page === page && previous !== null && previous !== page;
    if (scroll) scroll.scrollTop = returning ? returnTo.current?.top ?? 0 : 0;
    // Re-entering a category keeps focus on its sidebar button. Native deep
    // links manage their own destination focus after this layout reset.
    if (previous === null) return;
    const focus = returning
      ? document.getElementById(returnTo.current?.focusId ?? "") ?? document.getElementById("service-add-profile")
      : page === "list" ? document.getElementById("service-add-profile")
      : root.current?.querySelector<HTMLElement>(".service-detail h2, [data-service-page-heading]");
    focus?.focus({ preventScroll: true });
    if (returning) returnTo.current = null;
  }, [page, visible, request, root]);

  return rememberPagePosition;
}
