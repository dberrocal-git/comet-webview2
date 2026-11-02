import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

class Element {
  constructor(){this.value='';this.textContent='';this.children=[];this.dataset={};this.attributes={};}
  append(...elements){this.children.push(...elements);}
  replaceChildren(...elements){this.children=elements;}
  setAttribute(key,value){this.attributes[key]=value;}
  reportValidity(){return true;}
  contains(target){return this===target||this.children.some(child=>child.contains?.(target));}
  set innerHTML(value){throw new Error('Profile values must never be assigned to innerHTML');}
}
function load(filename){
  const html=fs.readFileSync(new URL('../src/ui/'+filename,import.meta.url),'utf8');
  const elements=new Map(), events=new Map(), messages=[];
  const get=id=>{if(!elements.has(id))elements.set(id,new Element());return elements.get(id);};
  const catalog=JSON.parse(fs.readFileSync(new URL('../locales/en.json',import.meta.url),'utf8'));
  const context={setTimeout:cb=>{events.set('timer',cb);return 1;},clearTimeout:()=>events.delete('timer'),cometSettings:{theme:'dark',keyboard_capture:true},cometI18n:{language:'en',strings:catalog},document:{readyState:'loading',documentElement:{dataset:{},matches:()=>false},getElementById:get,querySelector:get,querySelectorAll:()=>[],createElement:()=>new Element(),addEventListener:(name,cb)=>events.set(name,cb)},ipc:{postMessage:message=>messages.push(JSON.parse(message))}};
  context.window=context;
  const scripts = [...html.matchAll(/<script src="\/([^"]+)"><\/script>/g)].map(match => match[1]);
  assert.deepEqual(scripts, ['controls.js', filename.replace('.html', '.js')]);
  for (const script of scripts) {
    vm.runInNewContext(fs.readFileSync(new URL('../src/ui/'+script, import.meta.url),'utf8'),context);
  }
  return {context,get,events,messages};
}
const profile={id:'profile-a',name:'<img src=x onerror=alert(1)>',url:'https://kvm.example.test:8443/',has_password:true,certificate_sha256:null};
const state={profiles:[profile],selected_id:profile.id,form_version:1,busy:false,notice:null,trust:null,can_return:false,manager_visible:true};
{
  const h=load('manager.html');h.context.cometManager.update(state);
  assert.equal(h.get('name').value,profile.name,'stored names must be displayed as literal text');
  assert.equal(h.get('password').value,'','a saved password must never be revealed to the form');
  assert.equal(h.get('forget').hidden,false);
  h.get('password').value='synthetic-password';h.get('remember').checked=true;
  h.get('form').onsubmit({preventDefault(){}});
  assert.equal(h.messages.at(-1).kind,'save_profile');
  assert.equal(h.messages.at(-1).password,'synthetic-password');
  assert.equal(h.messages.at(-1).connect,true);
  assert.equal('username' in h.messages.at(-1),false,'the UI must not send or ask for an account');
  assert.equal(h.get('password').value,'','clear plaintext immediately after submission');
  h.get('password').value='new-entry';h.get('show-password').onclick();
  assert.equal(h.get('password').type,'text');h.get('show-password').onclick();
  assert.equal(h.get('password').type,'password');
  h.context.cometManager.update({...state,busy:true,trust:{attempt:9,url:profile.url,fingerprint:'ab'.repeat(32),previous:null}});
  assert.equal(h.get('fields').disabled,true);
  h.get('trust-accept').onclick();
  assert.equal(h.messages.at(-1).attempt,9);
  assert.equal(h.messages.at(-1).fingerprint,'ab'.repeat(32));
  assert.equal(h.get('editor').hidden,true,'certificate review should not duplicate the edit form');
}
{
  const h=load('manager.html');
  const other={...profile,id:'profile-b',name:'Second',url:'https://second.example/'};
  const first={...state,profiles:[profile,other]};
  h.context.cometManager.update(first);
  const rows=h.get('profiles').children.slice();
  h.context.cometManager.update({...first,selected_id:other.id,form_version:2});
  assert.equal(h.get('profiles').children[0],rows[0],'selection must retain existing rows for a smooth transition');
  assert.equal(h.get('profiles').children[1],rows[1]);
  assert.equal(rows[0].attributes['aria-selected'],'false');
  assert.equal(rows[1].attributes['aria-selected'],'true');
  h.context.cometManager.leave();
  assert.equal(h.context.document.documentElement.dataset.pagePhase,'leaving');
  h.context.cometManager.prepare();
  h.context.cometManager.enter();
  assert.equal(h.context.document.documentElement.dataset.pagePhase,'entering');
}
{
  const h=load('toolbar.html');h.context.cometToolbar.profiles(state);
  assert.equal(h.get('remote-controls').hidden,true,'video controls stay out of the connection screen');
  h.context.cometToolbar.profiles({...state,manager_visible:false});
  assert.equal(h.get('remote-controls').hidden,false);
  assert.equal(h.get('profiles').children.length,1);
  assert.equal(h.get('profiles').children[0].textContent,profile.name);
  assert.equal(h.get('profiles').value,profile.id);
  h.get('profiles').value=profile.id;h.get('connect').onclick();
  assert.equal(h.messages.at(-1).id,profile.id);
  h.context.cometToolbar.update({captured:true,capture_enabled:true,width:2560,height:1440,fps:60});
  assert.equal(h.get('capture-label').textContent,'Keyboard captured');
  h.get('.capture').onclick();
  assert.equal(h.messages.at(-1).enabled,false,'the manual switch must disable automatic capture');
  h.context.cometToolbar.update({captured:false,capture_enabled:false,width:2560,height:1440,fps:60});
  assert.equal(h.get('capture-label').textContent,'Keyboard off');
  h.get('theme-toggle').onclick();assert.equal(h.messages.at(-1).kind,'toggle_theme');
  h.context.cometUI.setTheme('light');assert.equal(h.context.document.documentElement.dataset.theme,'light');
  assert.equal(h.get('.stats').textContent,'2560 × 1440 · 60 FPS');
}
assert.ok(!fs.readFileSync(new URL('../src/ui/manager.html',import.meta.url),'utf8').includes('id="username"'));
{
  const html=fs.readFileSync(new URL('../src/ui/toolbar.html',import.meta.url),'utf8');
  const body=html.slice(html.indexOf('<body>'),html.indexOf('</body>'));
  assert.ok(!body.includes('class="brand"'),'the window title replaces toolbar branding');
  assert.ok(body.indexOf('id="theme-toggle"') < body.indexOf('id="connection-group"'));
  assert.ok(body.indexOf('id="pin"') > body.indexOf('id="remote-controls"'));
  assert.ok(body.indexOf('id="pin"') < body.indexOf('class="fullscreen"'));
}
{
  const h=load('toolbar.html'), toolbar=h.context.cometToolbar;
  const session={...state,manager_visible:false,viewer_id:profile.id,profiles:[profile,{id:'second',name:'Lab'}]};
  const select=h.get('profiles'),connect=h.get('connect');
  toolbar.profiles(session);
  assert.deepEqual(select.children.map(option=>option.value),[profile.id,'second']);
  toolbar.update({online:true});
  assert.equal(connect.textContent,'Connected');
  assert.equal(connect.disabled,true);
  const count=h.messages.length;
  connect.onclick();
  assert.equal(h.messages.length,count,'the current KVM must not be reconnected');
  select.value='second';select.onchange();
  const pendingOption = select.children[1];
  toolbar.profiles(session); // An unrelated update must preserve the open selector.
  assert.equal(select.children[1],pendingOption);
  toolbar.update({online:true});
  assert.equal(select.value,'second');
  assert.equal(connect.textContent,'Connect');
  assert.equal(connect.disabled,false);
  connect.onclick();
  assert.equal(h.messages.at(-1).id,'second');
  select.value=profile.id;select.onchange();
  assert.equal(connect.dataset.connected,'true');
  toolbar.update({online:false});
  assert.equal(connect.disabled,false,'a dropped connection can be reopened');
  toolbar.profiles({...session,viewer_id:null,selected_id:'missing',profiles:[]});
  assert.equal(select.children.length,0);
  assert.equal(select.value,'');
  assert.equal(select.disabled,true);
  assert.equal(connect.disabled,true);
  toolbar.profiles({...session,viewer_id:null,selected_id:'missing'});
  assert.equal(select.value,profile.id,'select the first saved device if no valid selection exists');
  const doc=h.context.document.documentElement;
  toolbar.layout({autoHide:true,collapsed:true,fullscreen:true,pinned:false});
  assert.equal(doc.dataset.collapsed,'true');
  doc.onpointerenter();
  assert.equal(h.messages.at(-1).visible,true);
  toolbar.layout({autoHide:true,collapsed:false,fullscreen:true,pinned:false});
  doc.onpointerleave();h.events.get('timer')();
  assert.equal(h.messages.at(-1).visible,false);
  toolbar.layout({autoHide:false,collapsed:false,fullscreen:true,pinned:true});
  assert.equal(h.get('pin').attributes['aria-pressed'],'true');
  doc.onpointerleave();
  assert.equal(h.events.has('timer'),false);
  h.get('pin').onclick();
  assert.equal(h.messages.at(-1).kind,'toggle_toolbar_pin');
}
console.log('Profiles UI: passed (English catalog, theme switch, manual capture, saved credentials, contextual controls).');
