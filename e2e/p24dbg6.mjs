import { chromium } from "playwright";
const BASE="http://127.0.0.1:15435"; const API="http://127.0.0.1:15433";
const b=await chromium.launch({ args: ["--no-proxy-server"] });
const ctx=await b.newContext(); const p=await ctx.newPage();
await p.addInitScript(([apiBase])=>{ window.__MERIDIAN_API_URL__=apiBase; },[API]);
await p.goto(`${BASE}/login`,{waitUntil:"networkidle"});
await p.locator('input[type="email"], input[name*="email" i]').first().fill("owner@demo.local");
await p.locator('input[type="password"]').first().fill("demo-password-123");
await p.getByRole("button",{name:/sign in/i}).click();
await p.waitForTimeout(5000);
const r = await p.evaluate(async ()=>{
  const out=[];
  for (const [name,val] of [["appearance.theme_override","org_default"],["appearance.theme_override","slate"]]) {
    const res = await fetch(`${window.__MERIDIAN_API_URL__}/api/v1/settings/user`, {
      method: "PUT", headers: {"content-type":"application/json"},
      body: JSON.stringify({ key: name, value: val }),
    });
    out.push([val, res.status, (await res.text()).slice(0,200)]);
  }
  return out;
});
console.log(JSON.stringify(r));
await b.close();
