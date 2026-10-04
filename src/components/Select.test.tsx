// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Select } from "./Select";

const options = [
  { value: "translation", label: "仅译文" },
  { value: "bilingual", label: "双语" },
  { value: "original", label: "仅原文" },
];
const languages = [
  { value: "zh", label: "中文" },
  { value: "en", label: "英语" },
  { value: "ja", label: "日语" },
  { value: "de", label: "德语" },
  { value: "fr", label: "法语" },
  { value: "pt", label: "葡萄牙语" },
];
let host: HTMLDivElement;
let root: Root;
const onChange = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount());
  host.remove();
  onChange.mockReset();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
async function render(value: string) {
  await act(() => root.render(<Select label="字幕显示" value={value} options={options} onChange={onChange} />));
}
function trigger() { return host.querySelector<HTMLButtonElement>('[role="combobox"]')!; }

const translatorOptions = [
  { value: "default", label: "Alibaba", icon: <svg width="32" height="32"><circle cx="16" cy="16" r="8" /></svg> },
  { value: "deepL", label: "DeepL", icon: <img src="synthetic-deepl.svg" alt="" /> },
  { value: "deepLX", label: "DeepLX" },
];

it("keeps decorative option icons beside unchanged labels and preserves keyboard typeahead", async () => {
  await act(() => root.render(<Select label="文字翻译" value="default" options={translatorOptions} onChange={onChange} />));
  expect(trigger().textContent).toBe("Alibaba");
  expect(trigger().getAttribute("aria-label")).toBe("文字翻译");
  expect(trigger().querySelector('.mimi-select__icon[aria-hidden="true"] svg')).not.toBeNull();
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "D", bubbles: true })));
  expect(visibleLabels()).toEqual(["Alibaba", "DeepL", "DeepLX"]);
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("DeepL");
  expect(document.querySelectorAll('.mimi-select__menu .mimi-select__icon[aria-hidden="true"]')).toHaveLength(2);
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(onChange).toHaveBeenCalledExactlyOnceWith("deepL");
  await act(() => root.render(<Select label="文字翻译" value="deepL" options={translatorOptions} onChange={onChange} />));
  expect(trigger().textContent).toBe("DeepL");
  expect(trigger().querySelector('.mimi-select__icon[aria-hidden="true"] img')).not.toBeNull();
});

it("uses a fallback icon only while the saved selection is absent from the options", async () => {
  const props = { label: "Saved choice", options: translatorOptions, valueLabel: "Saved service", valueIcon: <svg data-fallback="true" />, onChange };
  await act(() => root.render(<Select {...props} value="missing" />));
  expect(trigger().textContent).toBe("Saved service");
  expect(trigger().querySelector('.mimi-select__icon[aria-hidden="true"] [data-fallback]')).not.toBeNull();
  await act(() => root.render(<Select {...props} value="deepL" />));
  expect(trigger().querySelector('.mimi-select__icon img')).not.toBeNull();
  expect(trigger().querySelector('[data-fallback]')).toBeNull();
  await act(() => root.render(<Select {...props} value="deepLX" />));
  expect(trigger().querySelector('.mimi-select__icon')).toBeNull();
});

it("filters icon-bearing options by label and value without changing searchable keyboard selection", async () => {
  await act(() => root.render(<Select label="文字翻译" value="default" options={translatorOptions}
    searchLabel="搜索翻译服务" emptyMessage="没有匹配服务" onChange={onChange} />));
  await act(() => trigger().click());
  await typeQuery("deep");
  expect(visibleLabels()).toEqual(["DeepL", "DeepLX"]);
  await key("End");
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("DeepLX");
  await key("Enter");
  expect(onChange).toHaveBeenCalledExactlyOnceWith("deepLX");
  expect(document.activeElement).toBe(trigger());
});

it("passes the trigger's provider theme variables to portalled option icons", async () => {
  await act(() => root.render(<Select label="文字翻译" value="default" options={translatorOptions} onChange={onChange} />));
  trigger().style.setProperty("--provider-light-display", "none");
  trigger().style.setProperty("--provider-dark-display", "block");
  trigger().style.setProperty("--provider-backing-background", "#fff");
  await act(() => trigger().click());
  const popup = document.querySelector<HTMLElement>(".mimi-select__menu")!;
  expect(popup.parentElement).toBe(document.body);
  expect(popup.style.getPropertyValue("--provider-light-display")).toBe("none");
  expect(popup.style.getPropertyValue("--provider-dark-display")).toBe("block");
  expect(popup.style.getPropertyValue("--provider-backing-background")).toBe("#fff");
});

it("updates the visible label on an external value change without losing trigger focus", async () => {
  await render("translation");
  const button = trigger();
  button.focus();
  for (const value of ["bilingual", "original", "translation", "bilingual"]) {
    await render(value);
    expect(button.textContent).toBe(options.find(option => option.value === value)!.label);
    expect(trigger()).toBe(button);
    expect(document.activeElement).toBe(button);
    expect(onChange).not.toHaveBeenCalled();
  }
});

it("repaints repeated label edits for the same route without replacing the focused trigger", async () => {
  const renderName = async (label: string) => act(() => root.render(
    <Select label="Translator" value="custom" options={[{ value: "custom", label }]} onChange={onChange} />,
  ));
  await renderName("b");
  const button = trigger();
  button.focus();
  let content = button.querySelector(".mimi-select__content");
  for (const label of ["B 站", "B 站 · @home / (测试) 😀", "Another service"]) {
    await renderName(label);
    expect(trigger()).toBe(button);
    expect(document.activeElement).toBe(button);
    expect(button.textContent).toBe(label);
    expect(button.querySelector(".mimi-select__content")).not.toBe(content);
    content = button.querySelector(".mimi-select__content");
  }
  expect(onChange).not.toHaveBeenCalled();
});

it("follows an external value while the menu is open, including its keyboard choice", async () => {
  await render("translation");
  await act(() => trigger().click());
  await render("bilingual");
  const selected = document.querySelector('[role="option"][aria-selected="true"]')!;
  expect(selected.textContent).toBe("双语");
  expect(trigger().getAttribute("aria-activedescendant")).toBe(selected.id);
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(onChange).toHaveBeenCalledExactlyOnceWith("original");
  expect(trigger().getAttribute("aria-expanded")).toBe("false");
});

it("applies rapid closed-menu arrows only when opted in, keeps focus and wraps the complete list", async () => {
  await act(() => root.render(<Select label="Choice" value="bilingual" options={options} closedArrowSelection onChange={onChange} />));
  const button = trigger();
  button.focus();
  await act(() => {
    for (const key of ["ArrowDown", "ArrowDown", "ArrowUp"]) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      button.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
    }
  });
  expect(onChange.mock.calls).toEqual([["original"], ["translation"], ["original"]]);
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(document.activeElement).toBe(button);
  // Other selects retain their standard open-and-confirm arrow behavior.
  onChange.mockClear();
  await render("bilingual");
  await act(() => button.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
  expect(onChange).not.toHaveBeenCalled();
});

it.each([
  ["ArrowDown", "translation"], ["ArrowUp", "original"],
])("starts an unavailable closed selection at a defined boundary for %s", async (key, expected) => {
  await act(() => root.render(<Select label="Choice" value="removed" options={options} closedArrowSelection onChange={onChange} />));
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })));
  expect(onChange).toHaveBeenCalledExactlyOnceWith(expected);
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("uses all options for a closed arrow after a filtered manual choice and follows an external reset", async () => {
  const renderChoice = async (value: string) => act(() => root.render(<Select label="Language" value={value} options={languages}
    searchLabel="Search" closedArrowSelection onChange={onChange} />));
  await renderChoice("zh");
  await act(() => trigger().click());
  await typeQuery("de");
  await key("Enter");
  expect(onChange).toHaveBeenLastCalledWith("de");
  await renderChoice("de");
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })));
  expect(onChange).toHaveBeenLastCalledWith("ja");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  await renderChoice("pt");
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  expect(onChange).toHaveBeenLastCalledWith("zh");
});

async function renderSearch(value = "zh", list = languages, disabled = false) {
  await act(() => root.render(<Select label="识别语言" value={value} options={list} disabled={disabled}
    searchLabel="搜索语言" emptyMessage="没有匹配语言" onChange={onChange} />));
}
function input() { return document.querySelector<HTMLInputElement>(".mimi-select__search")!; }
function visibleLabels() { return [...document.querySelectorAll('[role="option"]')].map(option => option.textContent); }
async function typeQuery(text: string) {
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), text);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function key(key: string, extra: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...extra });
  await act(() => input().dispatchEvent(event));
  return event;
}

it("opens a named search input outside the listbox and filters localized names and wire codes", async () => {
  await renderSearch();
  await act(() => trigger().click());
  expect(document.activeElement).toBe(input());
  expect(input().getAttribute("aria-label")).toBe("搜索语言");
  expect(input().closest('[role="listbox"]')).toBeNull();
  expect(document.getElementById(input().getAttribute("aria-controls")!)?.getAttribute("role")).toBe("listbox");
  await typeQuery("葡萄");
  expect(visibleLabels()).toEqual(["葡萄牙语"]);
  await typeQuery(" DE ");
  expect(visibleLabels()).toEqual(["德语"]);
  await key("Enter");
  expect(onChange).toHaveBeenCalledExactlyOnceWith("de");
  expect(trigger().getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(trigger());
});

it("navigates the filtered list and resets its cursor safely as the query changes", async () => {
  await renderSearch();
  await act(() => trigger().click());
  await typeQuery("e");
  expect(visibleLabels()).toEqual(["英语", "德语"]);
  await key("End");
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("德语");
  await key("ArrowDown");
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("英语");
  await key("ArrowUp");
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("德语");
  await key("Home");
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("英语");
  await typeQuery("pt");
  const option = document.querySelector('[role="option"]')!;
  expect(document.getElementById(input().getAttribute("aria-activedescendant")!)).toBe(option);
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  await key("Enter");
  expect(onChange).toHaveBeenCalledExactlyOnceWith("pt");
});

it("loads choices on open and keeps a saved label without inventing a selectable result", async () => {
  const onOpen = vi.fn();
  await act(() => root.render(<Select label="Application" value="saved.app" valueLabel="Saved Player"
    options={[{ value: "system", label: "All applications" }]} searchLabel="Search applications"
    onOpen={onOpen} onChange={onChange} />));
  expect(trigger().textContent).toBe("Saved Player");
  expect(onOpen).not.toHaveBeenCalled();
  await act(() => trigger().click());
  expect(onOpen).toHaveBeenCalledOnce();
  expect(visibleLabels()).toEqual(["All applications"]);
  expect(Number.parseFloat(document.querySelector<HTMLElement>(".mimi-select__menu")!.style.maxHeight)).toBe(280);
});

it("scrolls only the results for keyboard navigation and never moves the list on hover", async () => {
  await renderSearch();
  await act(() => trigger().click());
  const results = document.querySelector<HTMLElement>(".mimi-select__options")!;
  const popup = document.querySelector<HTMLElement>(".mimi-select__menu")!;
  const rows = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  results.getBoundingClientRect = () => ({ top: 50, bottom: 150 } as DOMRect);
  Object.defineProperty(results, "clientHeight", { value: 100 });
  rows.forEach((row, index) => {
    row.getBoundingClientRect = () => ({ top: 50 + index * 32 - results.scrollTop, bottom: 82 + index * 32 - results.scrollTop } as DOMRect);
  });
  await key("End");
  expect(results.scrollTop).toBe(92);
  expect(popup.scrollTop).toBe(0);
  await act(() => rows[0].dispatchEvent(new MouseEvent("pointermove", { bubbles: true })));
  expect(results.scrollTop).toBe(92);
  await key("Home");
  expect(results.scrollTop).toBe(0);
  expect(popup.scrollTop).toBe(0);
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
});

it("shows caller-supplied no-match feedback without selecting or inventing an active option", async () => {
  await renderSearch();
  await act(() => trigger().click());
  await typeQuery("unknown language");
  expect(visibleLabels()).toEqual([]);
  expect(document.querySelector('[role="status"]')?.textContent).toBe("没有匹配语言");
  expect(input().hasAttribute("aria-activedescendant")).toBe(false);
  expect(trigger().hasAttribute("aria-activedescendant")).toBe(false);
  for (const command of ["ArrowUp", "ArrowDown", "Home", "End", "Enter"]) await key(command);
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  await typeQuery("日语");
  await act(() => document.querySelector<HTMLElement>('[role="option"]')!.click());
  expect(onChange).toHaveBeenCalledExactlyOnceWith("ja");
});

it("keeps external selections current while filtered, including a selected value outside the results", async () => {
  await renderSearch();
  await act(() => trigger().click());
  await typeQuery("e");
  await renderSearch("de");
  const selected = document.querySelector('[role="option"][aria-selected="true"]')!;
  expect(document.getElementById(input().getAttribute("aria-activedescendant")!)).toBe(selected);
  expect(trigger().textContent).toBe("德语");
  await renderSearch("ja");
  expect(trigger().textContent).toBe("日语");
  expect(document.querySelector('[role="option"][aria-selected="true"]')).toBeNull();
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("英语");
  await key("Enter");
  expect(onChange).toHaveBeenCalledExactlyOnceWith("en");
});

it.each([
  { command: "Escape", shiftKey: false },
  { command: "Tab", shiftKey: false },
  { command: "Tab", shiftKey: true },
])("closes search with $command (shift=$shiftKey) and restores the form trigger without selecting", async ({ command, shiftKey }) => {
  await renderSearch();
  await act(() => trigger().click());
  await typeQuery("de");
  const event = await key(command, { shiftKey });
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(document.activeElement).toBe(trigger());
  expect(event.defaultPrevented).toBe(command === "Escape");
  expect(onChange).not.toHaveBeenCalled();
  await act(() => trigger().click());
  expect(input().value).toBe("");
  expect(visibleLabels()).toHaveLength(languages.length);
});

it("leaves text editing and IME Enter alone, and dismisses on an outside pointer", async () => {
  await renderSearch();
  await act(() => trigger().click());
  await typeQuery("de");
  expect((await key(" ")).defaultPrevented).toBe(false);
  expect((await key("Enter", { isComposing: true })).defaultPrevented).toBe(false);
  expect(onChange).not.toHaveBeenCalled();
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
  await act(() => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("does not confirm a searchable selection on WebKit's IME Enter with keyCode 229", async () => {
  await renderSearch();
  await act(() => trigger().click());
  await typeQuery("de");
  const event = await key("Enter", { isComposing: false, keyCode: 229 });
  expect(event.defaultPrevented).toBe(false);
  expect(document.activeElement).toBe(input());
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  await key("Enter");
  expect(onChange).toHaveBeenCalledExactlyOnceWith("de");
});

it("starts an explicit query when typing on the closed trigger and keeps a searchable popup inside the viewport", async () => {
  vi.stubGlobal("innerWidth", 240);
  vi.stubGlobal("innerHeight", 120);
  await renderSearch();
  trigger().getBoundingClientRect = () => ({ left: 210, right: 240, top: 64, bottom: 100, width: 30, height: 36 } as DOMRect);
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true, cancelable: true })));
  expect(input().value).toBe("d");
  expect(visibleLabels()).toEqual(["德语"]);
  const popup = document.querySelector<HTMLElement>(".mimi-select__menu")!;
  const height = Number.parseFloat(popup.style.maxHeight), width = Number.parseFloat(popup.style.width);
  expect(height).toBeGreaterThanOrEqual(87); // Input, spacing and one usable result.
  expect(Number.parseFloat(popup.style.left)).toBeGreaterThanOrEqual(8);
  expect(Number.parseFloat(popup.style.left) + width).toBeLessThanOrEqual(232);
  expect(Number.parseFloat(popup.style.bottom)).toBeGreaterThanOrEqual(8);
  expect(Number.parseFloat(popup.style.bottom) + height).toBeLessThanOrEqual(112);
});

it("keeps non-searchable typeahead and empty/disabled controls unchanged", async () => {
  await render("translation");
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "双", bubbles: true })));
  expect(document.querySelector(".mimi-select__search")).toBeNull();
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(onChange).toHaveBeenCalledExactlyOnceWith("bilingual");
  await renderSearch("zh", [], false);
  expect(trigger().disabled).toBe(true);
  await act(() => trigger().click());
  expect(document.querySelector(".mimi-select__search")).toBeNull();
  await renderSearch("zh", languages, true);
  expect(trigger().disabled).toBe(true);
});

it("keeps the non-searchable typeahead timeout based on keyboard event timing", async () => {
  await act(() => root.render(<Select label="Language" value="de" options={[
    { value: "de", label: "German" }, { value: "en", label: "English" }, { value: "et", label: "Estonian" },
  ]} onChange={onChange} />));
  const press = async (character: string, at: number) => {
    const event = new KeyboardEvent("keydown", { key: character, bubbles: true });
    Object.defineProperty(event, "timeStamp", { value: at });
    await act(() => trigger().dispatchEvent(event));
  };
  await press("E", 1000);
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("English");
  await press("s", 1200);
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("Estonian");
  await press("E", 2000);
  expect(document.querySelector('[data-active="true"]')?.textContent).toBe("English");
});
