// Playwright CLI run-code harness. Serve the actual SupportDiagnostics component
// with mocked Tauri IPC and a clipboard stub. Open /scripts/frontend/fixtures/diagnostic-toast.html
// on the local Vite server before running this code with Playwright CLI.
async (page) => {
  const copy = page.getByRole("button", { name: "复制诊断信息" });
  const section = page.getByRole("region", { name: "遇到问题？" });
  const before = await section.boundingBox();
  await copy.click();
  const toast = page.getByRole("status");
  await toast.waitFor();
  const after = await section.boundingBox();
  const placement = await toast.evaluate(el => getComputedStyle(el).position);
  if (before.height !== after.height || placement !== "fixed") throw new Error("inline layout regression");
  await page.waitForTimeout(1800);
  await copy.click();
  await toast.waitFor();
  if (await toast.count() !== 1) throw new Error("stacked notifications");
  await page.waitForTimeout(1800);
  if (!(await toast.isVisible())) throw new Error("repeat did not reset expiry");
  await toast.waitFor({ state: "detached" });
  await copy.click();
  await toast.waitFor();
  await page.evaluate(() => { location.hash = "application-settings"; });
  await toast.waitFor({ state: "detached" });
  await copy.click();
  await toast.waitFor();
  await page.getByRole("button", { name: "切换设置页" }).click();
  await toast.waitFor({ state: "detached" });
  await copy.click();
  await toast.waitFor();
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await toast.waitFor({ state: "detached" });
  const fixtureUrl = new URL(page.url());
  fixtureUrl.hash = "";
  fixtureUrl.search = "?delayMs=500";
  await page.goto(fixtureUrl.href);
  await copy.click();
  await page.evaluate(() => { location.hash = "another-page"; });
  await page.waitForTimeout(700);
  if (await page.getByRole("status").count()) throw new Error("late operation resurfaced");
  fixtureUrl.search = "?clipboardFailure=1";
  await page.goto(fixtureUrl.href);
  await copy.click();
  const error = page.getByRole("alert");
  await error.waitFor();
  if (await page.getByRole("status").count()) throw new Error("failure reported success");
  await page.getByRole("button", { name: "关闭提示" }).click();
  await error.waitFor({ state: "detached" });
  return { noInline: true, repeat: true, autoDismiss: true, navigationDismiss: true, closeDismiss: true, noLateFeedback: true, failure: true };
}
