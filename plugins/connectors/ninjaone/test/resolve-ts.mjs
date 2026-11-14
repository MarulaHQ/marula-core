// Resolve hook: `./x.js` falls back to `./x.ts` when no `./x.js` exists.
// See register-ts.mjs for why the tests need it.

export async function resolve(specifier, context, next) {
  if (specifier.startsWith(".") && specifier.endsWith(".js")) {
    try {
      return await next(specifier, context);
    } catch {
      return next(`${specifier.slice(0, -3)}.ts`, context);
    }
  }
  return next(specifier, context);
}
