import type { ComponentProps } from "react";
import { OPENAI_COMPATIBLE_PRESETS } from "../../lib/generated/providerCopy";
import { ProviderIcon } from "./ProviderIcon";
import deepseek from "../../assets/providers/deepseek.svg";
import kimi from "../../assets/providers/kimi.svg";
import kimiDark from "../../assets/providers/kimi-dark.svg";
import siliconflow from "../../assets/providers/siliconflow.svg";
import siliconflowDark from "../../assets/providers/siliconflow-white.svg";
import openrouter from "../../assets/providers/openrouter.svg";
import openrouterDark from "../../assets/providers/openrouter-white.svg";
import bilibili from "../../assets/providers/bilibili.png";
import ollama from "../../assets/providers/ollama.svg";
import lmStudio from "../../assets/providers/lm-studio.svg";
import lmStudioDark from "../../assets/providers/lm-studio-white.svg";
import baiduQianfan from "../../assets/providers/baidu-qianfan.png";
import qwen from "../../assets/providers/qwen.png";

type PresetId = typeof OPENAI_COMPATIBLE_PRESETS[number]["id"];
type Mark = { provider: ComponentProps<typeof ProviderIcon>["provider"] }
  | { light: string; dark?: string; backing?: boolean };

const MARKS: Record<PresetId, Mark> = {
  openai: { provider: "openAIRealtime" },
  deepseek: { light: deepseek },
  qwen: { light: qwen, backing: true },
  gemini: { provider: "googleGeminiLive" },
  kimi: { light: kimi, dark: kimiDark },
  siliconflow: { light: siliconflow, dark: siliconflowDark },
  openrouter: { light: openrouter, dark: openrouterDark },
  indexTranslate: { light: bilibili },
  tencentTokenHub: { provider: "tencentCloud" },
  baiduQianfan: { light: baiduQianfan },
  volcanoArk: { provider: "volcanoEngine" },
  ollama: { light: ollama, backing: true },
  lmStudio: { light: lmStudio, dark: lmStudioDark },
};

/** A setup preset is a UI destination, not a separate saved provider identity. */
export function TextTranslationPresetIcon({ presetId }: { presetId: PresetId }) {
  const mark = MARKS[presetId];
  return <span className="provider-icon translation-preset-icon" data-preset={presetId} aria-hidden="true">
    {"provider" in mark ? <ProviderIcon provider={mark.provider} size={32} /> : <>
      <img className={["provider-icon__image", mark.dark && "provider-icon__image--light", mark.backing && "provider-icon__image--backed"].filter(Boolean).join(" ")} src={mark.light} width={32} height={32} alt="" draggable={false} />
      {mark.dark && <img className="provider-icon__image provider-icon__image--dark" src={mark.dark} width={32} height={32} alt="" draggable={false} />}
    </>}
  </span>;
}
