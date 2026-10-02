import { Languages } from "lucide-react";
import type { ServiceProvider, TextTranslation } from "../lib/types";
import alibabaCloud from "../assets/providers/alibaba-cloud.svg";
import openAI from "../assets/providers/openai-black.svg";
import openAIDark from "../assets/providers/openai-white.svg";
import gemini from "../assets/providers/gemini.svg";
import azure from "../assets/providers/azure.svg";
import volcanoEngine from "../assets/providers/volcano-engine.png";
import tencentCloud from "../assets/providers/tencent-cloud.svg";
import baiduTranslate from "../assets/providers/baidu-translate.jpg";
import xAI from "../assets/providers/xai.png";
import deepL from "../assets/providers/deepl-blue.svg";
import deepLDark from "../assets/providers/deepl-white.svg";
import deepLX from "../assets/providers/deeplx.svg";
import "./provider-icon.css";

type IconProvider = ServiceProvider | Exclude<TextTranslation, "followService">;
const PROVIDER_ASSETS: Record<Exclude<IconProvider, "openAICompatible">, string> = {
  alibabaCloud,
  openAIRealtime: openAI,
  googleGeminiLive: gemini,
  azureOpenAIRealtime: azure,
  volcanoEngine,
  tencentCloud,
  baiduTranslate,
  xAIRealtime: xAI,
  deepL,
  deepLX,
};

interface ProviderIconProps {
  provider: IconProvider;
  size?: 32 | 36;
  className?: string;
}

/** Decorative service marks; adjacent text labels the actual provider or route. */
export function ProviderIcon({ provider, size = 36, className }: ProviderIconProps) {
  const darkAsset = provider === "openAIRealtime" ? openAIDark : provider === "deepL" ? deepLDark : null;
  return (
    <span
      className={["provider-icon", className].filter(Boolean).join(" ")}
      data-provider={provider}
      aria-hidden="true"
      style={{ width: size, height: size }}
    >
      {provider === "openAICompatible" ? (
        <Languages className="provider-icon__generic" size={28} strokeWidth={1.5} />
      ) : <>
        <img
          className={["provider-icon__image", darkAsset && "provider-icon__image--light", provider === "deepLX" && "provider-icon__image--deeplx"].filter(Boolean).join(" ")}
          src={PROVIDER_ASSETS[provider]}
          width={32}
          height={32}
          alt=""
          draggable={false}
        />
        {darkAsset && (
          <img className="provider-icon__image provider-icon__image--dark" src={darkAsset} width={32} height={32} alt="" draggable={false} />
        )}
      </>}
    </span>
  );
}
