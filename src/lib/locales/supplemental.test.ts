import { expect, it } from "vitest";
import { SUPPLEMENTAL as zhTW } from "./zh-TW-supplemental";
import { SUPPLEMENTAL_EN } from "./supplemental-schema";
import { SUPPLEMENTAL as de } from "./de-supplemental";
import { SUPPLEMENTAL as fr } from "./fr-supplemental";
import { SUPPLEMENTAL as ko } from "./ko-supplemental";
import { SUPPLEMENTAL as th } from "./th-supplemental";

function assertComplete(actual: object, expected: object, path: string) {
  expect(Object.keys(actual).sort(), path).toEqual(Object.keys(expected).sort());
  for (const key of Object.keys(expected) as (keyof typeof expected)[]) {
    const value = actual[key];
    expect(typeof value, `${path}.${key}`).toBe(typeof expected[key]);
    if (typeof value === "object" && value !== null) assertComplete(value, expected[key], `${path}.${key}`);
    else expect(String(value).trim(), `${path}.${key}`).not.toBe("");
  }
}

it.each([["zh-TW", zhTW], ["de", de], ["fr", fr], ["ko", ko], ["th", th]] as const)("keeps audio, diagnostics and control-panel copy complete in %s", (locale, copy) => {
  assertComplete(copy, SUPPLEMENTAL_EN, locale);
  expect(copy.applicationAudio_copy.title).not.toBe(SUPPLEMENTAL_EN.applicationAudio_copy.title);
  expect(copy.connectionDiagnostics_copy.reasons.credentialsMissing).not.toBe(SUPPLEMENTAL_EN.connectionDiagnostics_copy.reasons.credentialsMissing);
  expect(copy.audio3Errors_copy.summary.authentication).not.toBe(SUPPLEMENTAL_EN.audio3Errors_copy.summary.authentication);
});
