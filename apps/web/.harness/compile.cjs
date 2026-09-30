
const path=require('path'), fs=require('fs');
const pluginPath = require.resolve('@tailwindcss/postcss');
const postcss = require(require.resolve('postcss', {paths:[path.dirname(pluginPath)]}));
const tw = require(pluginPath).default ?? require(pluginPath);
const ui = path.resolve('../../packages/ui/src');
const appDir = path.resolve('app');
const dir = path.resolve('.harness'); fs.mkdirSync(dir,{recursive:true});
let css = fs.readFileSync('app/globals.css','utf8');
// Point every workspace import at its real file, and every app-relative import
// at app/. Dropping the app-relative ones is what broke this script when main
// added ./ledger.css: the import still pointed at a sibling that only exists
// next to globals.css, so the compile failed and left a STALE app.css behind.
css = css.replace('@import "@marula-core/ui/tokens.css";', '@import "'+ui+'/tokens.css";')
         .replace('@import "@marula-core/ui/components.css";', '@import "'+ui+'/components.css";')
         .replace(/@import "@marula-core\/layout\/styles.css";/, '')
         .replace(/@import "\.\/([A-Za-z0-9._-]+\.css)";/g, (_m, f) => '@import "'+path.join(appDir, f)+'";');
fs.writeFileSync(path.join(dir,'in.css'), css);
postcss([tw()]).process(css,{from: path.join(dir,'in.css')}).then(r=>{
  fs.writeFileSync(path.join(dir,'app.css'), r.css);
  console.log('css bytes', r.css.length);
}).catch(e=>{console.error(String(e)); process.exit(1);});
