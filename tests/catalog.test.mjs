import assert from 'node:assert/strict';
import fs from 'node:fs';
const catalog=JSON.parse(fs.readFileSync(new URL('../locales/en.json',import.meta.url),'utf8'));
const directory=new URL('../src/',import.meta.url);
let checked=0;
for(const name of fs.readdirSync(directory,{recursive:true})) {
  if(!/\.(rs|html|js)$/.test(name))continue;
  const source=fs.readFileSync(new URL(name.replaceAll('\\','/'),directory),'utf8');
  for(const match of source.matchAll(/["']((?:app|manager|field|password|action|trust|footer|toolbar|capture|theme|profile|notice|error)\.[a-z_]+)["']/g)) {
    if(/\.(css|js|rs|html)$/.test(match[1]))continue;
    assert.equal(typeof catalog[match[1]],'string',`${name}: missing ${match[1]}`);checked++;
  }
}
const toolbar=fs.readFileSync(new URL('ui/toolbar.html',directory),'utf8');
assert.ok(!toolbar.includes('class="theme-toggle fullscreen"'),'theme and full-screen must remain separate controls');
for(const name of ['ui/manager.html','ui/toolbar.html']) {
  assert.ok(!/Guardados|Añadir|Contraseña|Capturar|Conectar|Cancelar|Pantalla completa/.test(fs.readFileSync(new URL(name.replaceAll('\\','/'),directory),'utf8')),`${name}: untranslated legacy label`);
}
assert.ok(!fs.readFileSync(new URL('keyboard/mod.rs',directory),'utf8').includes('let emergency'),'there must be no reserved release hotkey');
console.log(`Catalog: ${checked} UI and native string references resolved.`);
