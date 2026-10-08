// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LocalProgramSettings } from "./LocalProgramSettings";
import { setStoredUiLanguage } from "../../lib/i18n";
import { LOCAL_PROGRAM_COPY as copy } from "../../lib/localProgramI18n";
import { pickLocalProgramPath } from "../../lib/ipc";
import type { LocalProgramConfiguration, ServiceProfile } from "../../lib/types";
vi.mock("../../lib/ipc", () => ({ pickLocalProgramPath: vi.fn(), setOverlayPointerCursor: vi.fn() }));
vi.mock("./AlibabaCredentialEditor", () => ({ AlibabaCredentialEditor: () => <div data-translation-editor /> }));
let host: HTMLDivElement, root: Root;
const save = vi.fn(); const check = vi.fn(() => <button>Check</button>);
const program: LocalProgramConfiguration = { engine: "whisperCpp", executable: "/apps/whisper-cli", modelPath: "/models/own model.bin", arguments: [] };
const profile: ServiceProfile = { id: "own", name: "Own model", provider: "localProgram", credentialState: "missing" };
const props = { inputId: "own", profile, disabled: false, busy: false, feedback: null, onSaveProgram: save, connectionCheck: check, onSave: vi.fn(), onRequestDelete: vi.fn(), onConfirmDelete: vi.fn(), confirmingDelete: false, onCancelDelete: vi.fn() };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); setStoredUiLanguage("en");
  vi.mocked(pickLocalProgramPath).mockReset(); save.mockReset().mockResolvedValue(undefined); check.mockClear();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.unstubAllGlobals(); });
const button = (label: string) => [...host.querySelectorAll("button")].find(button => button.textContent === label)!;
const render = (override: Partial<typeof props> = {}) => act(async () => root.render(<LocalProgramSettings {...props} {...override} />));
it("selects program and model independently, saves once and checks only the saved configuration", async () => {
  await render(); expect(check).toHaveBeenLastCalledWith(null);
  expect(pickLocalProgramPath).not.toHaveBeenCalled();
  vi.mocked(pickLocalProgramPath).mockResolvedValueOnce(program.executable).mockResolvedValueOnce(program.modelPath);
  await act(async () => button(copy.chooseProgram).click()); await act(async () => button(copy.chooseModel).click());
  expect(host.querySelector<HTMLInputElement>("#own-program")!.value).toBe(program.executable);
  expect(host.querySelector<HTMLInputElement>("#own-model")!.value).toBe(program.modelPath);
  let finish!: () => void; save.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
  await act(async () => { button(copy.save).click(); button(copy.save).click(); });
  expect(save).toHaveBeenCalledExactlyOnceWith(program); expect(button(copy.chooseModel).disabled).toBe(true);
  await act(async () => finish()); await render({ profile: { ...profile, localProgram: program, speechCredentialState: "present" } });
  expect(check).toHaveBeenLastCalledWith(undefined); expect(button(copy.save).disabled).toBe(true);
  expect(host.querySelector("[data-translation-editor]")).not.toBeNull();
});
it("keeps a failed save editable for retry without exposing native paths or process errors", async () => {
  await render({ profile: { ...profile, localProgram: program } });
  vi.mocked(pickLocalProgramPath).mockResolvedValue("/models/new.bin");
  await act(async () => button(copy.chooseModel).click()); save.mockRejectedValueOnce("local_program_model_missing");
  await act(async () => button(copy.save).click()); expect(host.textContent).toContain(copy.modelError);
  expect(host.querySelector<HTMLInputElement>("#own-model")!.value).toBe("/models/new.bin");
  expect(check).toHaveBeenLastCalledWith(null);
  await act(async () => button(copy.save).click()); expect(save).toHaveBeenCalledTimes(2);
});
it.each(["zh", "zh-TW", "en", "ja", "ko", "fr", "de"] as const)("keeps owned files and launch controls disabled during a session in %s", async locale => {
  setStoredUiLanguage(locale); await render({ disabled: true, profile: { ...profile, localProgram: program } });
  expect(button(copy.chooseProgram).disabled).toBe(true); expect(button(copy.chooseModel).disabled).toBe(true);
  expect(host.querySelector<HTMLInputElement>("#own-program")!.disabled).toBe(true);
  expect(host.textContent).not.toContain("undefined"); expect(host.querySelector('[aria-label*="Delete model"]')).toBeNull();
});
it("cancelling a picker retains the saved path and never saves automatically", async () => {
  await render({ profile: { ...profile, localProgram: program } }); vi.mocked(pickLocalProgramPath).mockResolvedValue(null);
  await act(async () => button(copy.chooseModel).click()); expect(host.querySelector<HTMLInputElement>("#own-model")!.value).toBe(program.modelPath);
  expect(save).not.toHaveBeenCalled(); expect(check).toHaveBeenLastCalledWith(undefined);
});
