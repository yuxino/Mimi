import { describe, expect, it } from "vitest";
import type { SessionStateEvent, SubtitleSnapshot } from "../../lib/types";
import {
  actionErrorMessage,
  deriveTrayPresentation,
  hasSubtitleContent,
} from "./trayModel";

type SessionStatus = SessionStateEvent["status"];

function presentation(
  status: SessionStatus,
  options: {
    paused?: boolean;
    credential?: "present" | "missing" | "unavailable";
    hasContent?: boolean;
  } = {},
) {
  return deriveTrayPresentation({
    statusKind: status.kind,
    isPaused: options.paused ?? false,
    credentialState: options.credential ?? "present",
    hasSubtitleContent: options.hasContent ?? false,
  });
}

describe("deriveTrayPresentation", () => {
  it("offers start when idle and the active credential is ready", () => {
    const model = presentation({ kind: "idle" });

    expect(model.visualState).toBe("ready");
    expect(model.statusKind).toBe("ready");
    expect(model.primaryAction).toEqual({ action: "start", disabled: false });
    expect(model.secondaryAction).toBeNull();
    expect(model.canShowOverlay).toBe(false);
    expect(model.canChangeSourceLanguage).toBe(true);
  });

  it.each(["missing", "unavailable", undefined] as const)(
    "routes an idle %s credential state to settings",
    (credential) => {
      const model = deriveTrayPresentation({
        statusKind: "idle",
        isPaused: false,
        credentialState: credential,
        hasSubtitleContent: false,
      });

      expect(model.visualState).toBe("setup");
      expect(model.statusKind).toBe("setupRequired");
      expect(model.primaryAction.action).toBe("configure");
    },
  );

  it("offers pause and stop while listening", () => {
    const model = presentation(
      { kind: "listening" },
      { hasContent: true },
    );

    expect(model.visualState).toBe("listening");
    expect(model.primaryAction.action).toBe("pause");
    expect(model.secondaryAction?.action).toBe("stop");
    expect(model.canShowOverlay).toBe(true);
    expect(model.canClearSubtitles).toBe(true);
    expect(model.canChangeSourceLanguage).toBe(true);
  });

  it("allows language changes while keeping the paused session's resume and stop actions", () => {
    const model = presentation(
      { kind: "listening" },
      { paused: true },
    );

    expect(model.visualState).toBe("paused");
    expect(model.primaryAction.action).toBe("resume");
    expect(model.secondaryAction?.action).toBe("stop");
    expect(model.canShowOverlay).toBe(true);
    expect(model.canChangeSourceLanguage).toBe(true);
  });

  it.each([
    ["connecting", { kind: "connecting" } as const],
    ["stopping", { kind: "stopping" } as const],
  ])("makes %s an explicit disabled state", (action, status) => {
    const model = presentation(status, { paused: true });

    expect(model.primaryAction).toEqual({ action, disabled: true });
    expect(model.secondaryAction).toBeNull();
    expect(model.canChangeSourceLanguage).toBe(false);
    expect(model.canShowOverlay).toBe(false);
  });

  it("keeps a retry path and the error state after a session failure", () => {
    const model = presentation({ kind: "error", message: "Connection failed" });

    expect(model.visualState).toBe("error");
    expect(model.statusKind).toBe("error");
    expect(model.primaryAction.action).toBe("start");
    expect(model.canChangeSourceLanguage).toBe(true);
  });

  it("offers subtitle tools after a stopped session leaves content", () => {
    const model = presentation({ kind: "idle" }, { hasContent: true });

    expect(model.canShowOverlay).toBe(true);
    expect(model.canClearSubtitles).toBe(true);
  });
});

describe("tray helpers", () => {
  const empty: SubtitleSnapshot = {
    source: { text: "", isFinal: false },
    translation: { text: "", isFinal: false },
    history: [],
  };

  it("detects drafts, translations, and durable history as clearable content", () => {
    expect(hasSubtitleContent(empty)).toBe(false);
    expect(
      hasSubtitleContent({
        ...empty,
        source: { text: "  hello ", isFinal: false },
      }),
    ).toBe(true);
    expect(
      hasSubtitleContent({
        ...empty,
        translation: { text: "译文", isFinal: false },
      }),
    ).toBe(true);
    expect(
      hasSubtitleContent({
        ...empty,
        history: [{ source: "a", translation: "b", createdAt: 1 }],
      }),
    ).toBe(true);
  });

  it("keeps useful asynchronous errors and otherwise uses a safe fallback", () => {
    expect(actionErrorMessage(new Error("No connection"), "Try again")).toBe(
      "No connection",
    );
    expect(actionErrorMessage("Native call failed", "Try again")).toBe(
      "Native call failed",
    );
    expect(actionErrorMessage(null, "Try again")).toBe("Try again");
  });
});
