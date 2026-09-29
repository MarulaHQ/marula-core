import { chromium } from "playwright";
const BASE="http://127.0.0.1:15435"; const API="http://127.0.0.1:15433";
const b=await chromium.launch({ args: ["--no-proxy-server"] });
const ctx=await b.newContext(); const p=await ctx.newPage();
await p.addInitScript(([apiBase,mode,preset])=>{
  window.__MARULA_API_URL__=apiBase;
  localStorage.setItem("marula-theme",mode);
  localStorage.setItem("marula-theme-preset",preset);
},[API,"light","slate"]);
await p.goto(`${BASE}/login`,{waitUntil:"networkidle"});
await p.locator('input[type="email"], input[name*="email" i]').first().fill("owner@demo.local");
await p.locator('input[type="password"]').first().fill("demo-password-123");
await p.getByRole("button",{name:/sign in/i}).click();
await p.waitForTimeout(5000);
await p.goto(`${BASE}/settings/appearance`,{waitUntil:"networkidle"});
await p.waitForTimeout(1500);
const info = await p.evaluate(()=>{
  const cs=getComputedStyle(document.documentElement);
  return {
    preset: document.documentElement.dataset.preset,
    theme: document.documentElement.dataset.theme,
    mCanvas: cs.getPropertyValue("--m-canvas").trim(),
    mSurface: cs.getPropertyValue("--m-surface").trim(),
    mAccent: cs.getPropertyValue("--m-accent").trim(),
    bodyBg: getComputedStyle(document.body).backgroundColor,
    hasStyle: !!document.getElementById("m-theme-presets"),
  };
});
console.log(JSON.stringify(info,null,1));
await b.close();
