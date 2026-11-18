/** Only call with an already-safe localized message from selectSessionErrorMessage.
 * Older error adapters use complete local sentences: retain the first reason and
 * leave its following recovery instructions in the detailed panel. Never parse
 * arbitrary provider/native text here, and never truncate at a character count.
 */
export function localizedSessionErrorSummary(localizedMessage: string): string {
  const boundary = /[。！？]|[.!?](?=\s|$)/u.exec(localizedMessage);
  return boundary ? localizedMessage.slice(0, boundary.index + 1) : localizedMessage;
}
