// @vitest-environment jsdom
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import type { ServiceProvider } from "../lib/types";
import openAILight from "../assets/providers/openai-black.svg";
import openAIDark from "../assets/providers/openai-white.svg";
import deepL from "../assets/providers/deepl-blue.svg";
import deepLDark from "../assets/providers/deepl-white.svg";
import deepLX from "../assets/providers/deeplx.svg";
import { ProviderIcon } from "./ProviderIcon";

it("bundles distinct official assets for the eight speech providers", () => {
  const providers: ServiceProvider[] = [
    "alibabaCloud", "openAIRealtime", "googleGeminiLive", "azureOpenAIRealtime",
    "volcanoEngine", "tencentCloud", "baiduTranslate", "xAIRealtime",
  ];
  const sources = providers.map((provider) => {
    const host = document.createElement("div");
    host.innerHTML = renderToStaticMarkup(<ProviderIcon provider={provider} />);
    const icon = host.querySelector(".provider-icon")!;
    const image = icon.querySelector("img")!;
    expect(icon.getAttribute("aria-hidden")).toBe("true");
    expect(image.getAttribute("alt")).toBe("");
    expect(image.getAttribute("src")).not.toMatch(/^https?:/);
    return image.getAttribute("src");
  });
  expect(new Set(sources).size).toBe(providers.length);
});

it("uses official light and dark OpenAI artwork without recoloring it", () => {
  const html = renderToStaticMarkup(<ProviderIcon provider="openAIRealtime" size={32} className="picker-icon" />);
  const host = document.createElement("div");
  host.innerHTML = html;
  expect(host.querySelector(".provider-icon__image--light")?.getAttribute("src")).toBe(openAILight);
  expect(host.querySelector(".provider-icon__image--dark")?.getAttribute("src")).toBe(openAIDark);
  expect(html).toContain("width:32px;height:32px");
  expect(html).toContain("picker-icon");
});

it("uses official DeepL artwork for text translation in both themes", () => {
  const host = document.createElement("div");
  host.innerHTML = renderToStaticMarkup(<ProviderIcon provider="deepL" />);
  expect(host.querySelector(".provider-icon__image--light")?.getAttribute("src")).toBe(deepL);
  expect(host.querySelector(".provider-icon__image--dark")?.getAttribute("src")).toBe(deepLDark);
  expect(host.querySelector(".provider-icon")?.getAttribute("aria-hidden")).toBe("true");
});

it("uses the legacy DeepLX documentation mark for its text translation route", () => {
  const host = document.createElement("div");
  host.innerHTML = renderToStaticMarkup(<ProviderIcon provider="deepLX" />);
  expect(host.querySelector(".provider-icon__image--deeplx")?.getAttribute("src")).toBe(deepLX);
  expect(host.querySelector("img")?.getAttribute("alt")).toBe("");
  expect(host.querySelector(".provider-icon")?.getAttribute("aria-hidden")).toBe("true");
});

it("uses a neutral decorative glyph for OpenAI-compatible third parties", () => {
  const host = document.createElement("div");
  host.innerHTML = renderToStaticMarkup(<ProviderIcon provider="openAICompatible" size={32} />);
  expect(host.querySelector("svg.provider-icon__generic")).not.toBeNull();
  expect(host.querySelector("img")).toBeNull();
  expect(host.querySelector(".provider-icon")?.getAttribute("aria-hidden")).toBe("true");
});
