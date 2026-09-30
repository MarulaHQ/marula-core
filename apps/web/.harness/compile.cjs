
const path=require('path'), fs=require('fs');
const pluginPath = require.resolve('@tailwindcss/postcss');
const postcss = require(require.resolve('postcss', {paths:[path.dirname(pluginPath)]}));
const tw = require(pluginPath).default ?? require(pluginPath);
const ui = path.resolve('../../packages/ui/src');
const dir = path.resolve('.harness'); fs.mkdirSync(dir,{recursive:true});
let css = fs.readFileSync('app/globals.css','utf8');
css = css.replace('@import "@marula-core/ui/tokens.css";', '@import "'+ui+'/tokens.css";')
         .replace('@import "@marula-core/ui/components.css";', '@import "'+ui+'/components.css";')
         .replace(/@import "@marula-core\/layout\/styles.css";/, '')
         .replace(/@import "\.\/shell.css";/, '');
fs.writeFileSync(path.join(dir,'in.css'), css);
postcss([tw()]).process(css,{from: path.join(dir,'in.css')}).then(r=>{
  fs.writeFileSync(path.join(dir,'app.css'), r.css);
  console.log('css bytes', r.css.length);
}).catch(e=>{console.error(String(e)); process.exit(1);});
