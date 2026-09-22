import { chromium } from "playwright";
const b=await chromium.launch({ args: ["--no-proxy-server"] });
const p=await b.newPage();
p.on("requestfailed", r=>console.log("FAILED", r.url(), r.failure()?.errorText));
p.on("pageerror", e=>console.log("PAGEERR", String(e).slice(0,300)));
await p.goto("http://127.0.0.1:15435/login", { waitUntil: "networkidle" });
await b.close();
