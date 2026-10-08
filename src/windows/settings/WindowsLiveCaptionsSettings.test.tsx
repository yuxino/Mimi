// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { openWindowsLiveCaptions } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import type { ServiceProfile } from "../../lib/types";
import { WindowsLiveCaptionsSettings } from "./WindowsLiveCaptionsSettings";

vi.mock("../../lib/ipc", async importOriginal => ({ ...await importOriginal<typeof import("../../lib/ipc")>(), openWindowsLiveCaptions: vi.fn() }));
vi.mock("./AlibabaCredentialEditor", () => ({ AlibabaCredentialEditor: ({ textOnly, disabled }: { textOnly?: boolean; disabled: boolean }) => <div data-testid="translation-editor" data-text-only={textOnly} aria-disabled={disabled} /> }));
const initial = useStore.getState();
const profile: ServiceProfile = { id: "windows", name: "Windows captions", provider: "windowsLiveCaptions", credentialState: "missing" };
let host: HTMLDivElement, root: Root;
let props: ComponentProps<typeof WindowsLiveCaptionsSettings>;
let stop: ReturnType<typeof vi.fn>, consent: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  stop = vi.fn().mockResolvedValue(undefined);
  consent = vi.fn().mockResolvedValue(initial.settings);
  useStore.setState({ ...initial, stop, setWindowsLiveCaptionsConsent: consent }, true);
  vi.mocked(openWindowsLiveCaptions).mockReset().mockResolvedValue(undefined);
  props = { profile, windowsSupport: { available: true, status: "ready" }, loading: false, failed: false,
    sourceLanguage: "en", targetLanguage: "original", inputId: "windows-test", disabled: false, busy: false,
    feedback: null, confirmingDelete: false, onRetry: vi.fn().mockResolvedValue(undefined), onBusyChange: vi.fn(),
    onSave: vi.fn(), onRequestDelete: vi.fn(), onConfirmDelete: vi.fn(), onCancelDelete: vi.fn() };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true); vi.unstubAllGlobals(); });
async function render(overrides: Partial<typeof props> = {}) { props = { ...props, ...overrides }; await act(async () => root.render(<WindowsLiveCaptionsSettings {...props} />)); }
function button(label: string) { return [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === label)!; }
async function click(node: HTMLElement) { await act(async () => node.click()); }

it.each(["en", "zh", "zh-TW", "ja", "de", "fr", "ko"] as const)("requires two explicit unchecked confirmations in %s without opening Windows UI", async locale => {
  setStoredUiLanguage(locale);
  await render();
  const boxes = host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
  expect(boxes).toHaveLength(2);
  expect([...boxes].every(box => !box.checked)).toBe(true);
  expect(button(I18N.settings.windowsLiveCaptionsAllow).disabled).toBe(true);
  expect(consent).not.toHaveBeenCalled(); expect(openWindowsLiveCaptions).not.toHaveBeenCalled();
  await click(boxes[0]);
  expect(button(I18N.settings.windowsLiveCaptionsAllow).disabled).toBe(true);
  await click(boxes[1]);
  expect(button(I18N.settings.windowsLiveCaptionsAllow).disabled).toBe(false);
  await click(button(I18N.settings.windowsLiveCaptionsAllow));
  expect(consent).toHaveBeenCalledExactlyOnceWith(profile.id, true);
  expect(stop).not.toHaveBeenCalled();
  expect(host.querySelector('[data-testid="translation-editor"]')?.getAttribute('data-text-only')).toBe("true");
});

it("keeps Windows setup guidance available during loading and status failures, then retries explicitly", async () => {
  await render({ windowsSupport: null, loading: true });
  await click(button(I18N.settings.windowsLiveCaptionsSetup));
  expect(host.querySelectorAll("ol li")).toHaveLength(3);
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsStepMicrophone);
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsLoading);
  await render({ loading: false, failed: true });
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsLoadFailed);
  await click(button(I18N.settings.windowsLiveCaptionsRefresh));
  expect(props.onRetry).toHaveBeenCalledOnce();
  expect(openWindowsLiveCaptions).not.toHaveBeenCalled();
});

it.each(["closed", "setupRequired", "unreadable"] as const)("explains %s without treating it as an unavailable OS or auto-opening captions", async status => {
  await render({ windowsSupport: { available: true, status } });
  const message = status === "closed" ? I18N.settings.windowsLiveCaptionsClosed : status === "setupRequired" ? I18N.settings.windowsLiveCaptionsSetupRequired : I18N.settings.windowsLiveCaptionsUnreadable;
  expect(host.textContent).toContain(message);
  expect(button(I18N.settings.windowsLiveCaptionsOpen).disabled).toBe(false);
  expect(openWindowsLiveCaptions).not.toHaveBeenCalled();
  await click(button(I18N.settings.windowsLiveCaptionsOpen));
  expect(openWindowsLiveCaptions).toHaveBeenCalledOnce(); expect(props.onRetry).toHaveBeenCalledOnce();
});

it("keeps imported configurations actionable on unsupported systems without allowing consent", async () => {
  await render({ windowsSupport: { available: false, status: "unsupported" } });
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsUnsupported);
  expect(button(I18N.settings.windowsLiveCaptionsOpen)).toBeUndefined();
  expect(host.querySelector("fieldset")!.disabled).toBe(true);
  await click(button(I18N.settings.windowsLiveCaptionsRefresh));
  expect(props.onRetry).toHaveBeenCalledOnce(); expect(consent).not.toHaveBeenCalled();
});

it("stops an active session before revoking permission and serializes duplicate actions", async () => {
  let stopped!: () => void;
  stop.mockImplementation(() => new Promise<void>(resolve => { stopped = resolve; }));
  await render({ profile: { ...profile, windowsLiveCaptionsConsent: true }, disabled: true, requiresStop: true });
  const revoke = button(I18N.settings.windowsLiveCaptionsStopAndRevoke);
  await act(async () => { revoke.click(); revoke.click(); });
  expect(stop).toHaveBeenCalledOnce(); expect(consent).not.toHaveBeenCalled();
  expect(revoke.disabled).toBe(true);
  await act(async () => stopped());
  expect(consent).toHaveBeenCalledExactlyOnceWith(profile.id, false);
});

it("preserves consent if stopping fails and displays safe retry copy", async () => {
  stop.mockRejectedValue(new Error("synthetic-private-native-error"));
  await render({ profile: { ...profile, windowsLiveCaptionsConsent: true }, requiresStop: true });
  await click(button(I18N.settings.windowsLiveCaptionsStopAndRevoke));
  expect(consent).not.toHaveBeenCalled();
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsConsentFailed);
  expect(host.textContent).not.toContain("synthetic-private-native-error");
  expect(button(I18N.settings.windowsLiveCaptionsStopAndRevoke).disabled).toBe(false);
});

it("makes original-only privacy and missing independent translation explicit", async () => {
  await render();
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsOriginalPrivacy);
  expect(host.textContent).not.toContain(I18N.settings.windowsLiveCaptionsNoTranslator);
  expect(host.querySelector('input[type="password"]')).toBeNull();
  await render({ targetLanguage: "zh" });
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsPrivacy);
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsNoTranslator);
  await render({ profile: { ...profile, textTranslation: "deepL" } });
  expect(host.textContent).not.toContain(I18N.settings.windowsLiveCaptionsNoTranslator);
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsAudioUnavailable);
});

it("reports launch failures without claiming Windows is ready or granting consent", async () => {
  vi.mocked(openWindowsLiveCaptions).mockRejectedValue(new Error("synthetic-error"));
  await render({ windowsSupport: { available: true, status: "closed" } });
  await click(button(I18N.settings.windowsLiveCaptionsOpen));
  expect(host.textContent).toContain(I18N.settings.windowsLiveCaptionsOpenFailed);
  expect(host.textContent).not.toContain(I18N.settings.windowsLiveCaptionsReady);
  expect(props.onRetry).not.toHaveBeenCalled(); expect(consent).not.toHaveBeenCalled();
});

it("checks Windows recognition without sending an Apple-only source language override", async () => {
  const connectionCheck = vi.fn().mockReturnValue(<button>Check Windows recognition</button>);
  await render({ profile: { ...profile, windowsLiveCaptionsConsent: true }, connectionCheck });
  expect(connectionCheck).toHaveBeenCalledExactlyOnceWith(undefined);
});
