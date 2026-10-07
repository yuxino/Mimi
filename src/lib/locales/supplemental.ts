import { SUPPLEMENTAL as de } from "./de-supplemental";
import { SUPPLEMENTAL as fr } from "./fr-supplemental";
import { SUPPLEMENTAL as ko } from "./ko-supplemental";
import { SUPPLEMENTAL as zhTW } from "./zh-TW-supplemental";
import type { SupplementalCopy } from "./supplemental-schema";

export const supplemental = Object.fromEntries(
  (Object.keys(de) as (keyof SupplementalCopy)[]).map(key => [key, { de: de[key], fr: fr[key], ko: ko[key], "zh-TW": zhTW[key] }]),
) as { [K in keyof SupplementalCopy]: { de: SupplementalCopy[K]; fr: SupplementalCopy[K]; ko: SupplementalCopy[K]; "zh-TW": SupplementalCopy[K] } };
