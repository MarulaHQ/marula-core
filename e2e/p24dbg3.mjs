import { chromium } from "playwright";
const BASE="http://127.0.0.1:15435"; const API="http://127.0.0.1:15433";
const b=await chromium.launch({ args: ["--no-proxy-server"] });
const ctx=await b.newContext({ viewport: { width: 1440, height: 900 } });
const p=await ctx.newPage();
await p.addInitScript(([apiBase])=>{
  window.__MARULA_API_URL__ = apiBase;
  window.fetch = new Proxy(window.fetch, { apply(t,a,ar){ console.log("FETCH", ar[0]); return Reflect.apply(t,a,ar); } });
},[API]);
let respCount=0;
p.on("response", r=>{ if(r.url().includes(API)) { respCount++; console.log("API RESP", r.status(), r.url().slice(0,90)); } });
p.on("requestfailed", r=>console.log("FAILED", r.method(), r.url()));
for (let i=0;i<2;i++){
  await p.goto(`${BASE}/login`,{waitUntil:"networkidle"});
  await p.locator('input[type="email"], input[name*="email" i]').first().fill("owner@demo.local");
  await p.locator('input[type="password"]').first().fill("demo-password-123");
  await p.getByRole("button",{name:/sign in/i}).click();
  await p.waitForTimeout(6000);
}
console.log("url:", p.url());
console.log("FETCHLOG:", await p.evaluate(()=>window.__MARULA_API_URL__));
await b.close();
