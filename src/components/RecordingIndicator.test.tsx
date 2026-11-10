// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ResolvedUiLanguage } from "../lib/i18n";
import type { AudioRecordingStatus } from "../lib/types";
import { recordingStatusLabel } from "../lib/recordingStatus";
import { RecordingIndicator } from "./RecordingIndicator";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

it.each([undefined, null, "off"] as const)("leaves no recording claim or chrome for %s", async status => {
  await act(async () => root.render(<RecordingIndicator status={status} language="en" />));
  expect(host.childElementCount).toBe(0);
  expect(recordingStatusLabel(status, "en")).toBeNull();
});

const recordingLabels: Record<ResolvedUiLanguage, string> = {
  zh: "正在录音", "zh-TW": "正在錄音", en: "Recording audio", ja: "録音中",
  ko: "녹음 중", de: "Audioaufnahme läuft", fr: "Enregistrement audio en cours", th: "กำลังบันทึกเสียง",
  ru: "Идёт запись звука",
};
const states = ["starting", "recording", "paused", "limited", "error"] as const satisfies AudioRecordingStatus[];

it.each(Object.keys(recordingLabels) as ResolvedUiLanguage[])("keeps each state accessible in full and collapsed %s chrome", async language => {
  const meanings = new Set<string>();
  const shortLabels = new Set<string>();
  for (const status of states) {
    await act(async () => root.render(<RecordingIndicator status={status} language={language} />));
    const full = host.querySelector<HTMLElement>('[role="img"]')!;
    const label = full.getAttribute("aria-label")!;
    meanings.add(label);
    expect(label).toBeTruthy();
    expect(full.title).toBe(label);
    expect(full.querySelector(".recording-indicator__label")?.textContent).toBeTruthy();
    shortLabels.add(full.querySelector(".recording-indicator__label")!.textContent!);
    expect(recordingStatusLabel(status, language)).toBe(label);
    if (status === "recording") expect(label).toBe(recordingLabels[language]);
    await act(async () => root.render(<RecordingIndicator status={status} language={language} compact />));
    const compact = host.querySelector<HTMLElement>('[role="img"]')!;
    expect(compact.getAttribute("aria-label")).toBe(label);
    expect(compact.title).toBe(label);
    expect(compact.querySelector(".recording-indicator__label")).toBeNull();
    expect(compact.querySelector(".recording-indicator__mark")).not.toBeNull();
    expect(host.querySelector("button, input, [role=button], [tabindex]")).toBeNull();
  }
  expect(meanings.size).toBe(states.length);
  expect(shortLabels.size).toBe(states.length);
});

it("updates a static paused indicator when its language changes", async () => {
  await act(async () => root.render(<RecordingIndicator status="paused" language="zh" />));
  expect(host.textContent).toBe("录音暂停");
  expect(host.firstElementChild?.getAttribute("aria-label")).toBe("录音已暂停");
  await act(async () => root.render(<RecordingIndicator status="paused" language="ja" />));
  expect(host.textContent).toBe("録音一時停止");
  expect(host.firstElementChild?.getAttribute("aria-label")).toBe("録音を一時停止中");
});

it("makes stopped-by-limit and failed recording distinct from active recording", async () => {
  await act(async () => root.render(<RecordingIndicator status="recording" language="zh" />));
  expect(host.textContent).toBe("录音");
  expect(host.querySelector("svg")).toBeNull();
  await act(async () => root.render(<RecordingIndicator status="limited" language="zh" />));
  expect(host.textContent).toBe("录音已满");
  expect(host.firstElementChild?.getAttribute("aria-label")).toBe("录音已达到保存上限");
  await act(async () => root.render(<RecordingIndicator status="error" language="zh" />));
  expect(host.textContent).toBe("录音异常");
  expect(host.querySelector("svg")).not.toBeNull();
  expect(host.firstElementChild?.getAttribute("aria-label")).toBe("录音出现错误");
});
