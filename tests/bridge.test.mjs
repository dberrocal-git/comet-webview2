// Execute the injected bridge with a fake firmware document and transports.
// Assertions cover ordered key messages, capture/release and origin isolation;
// they do not exercise DirectInput, WebView2 or actual remote HID delivery.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const source=fs.readFileSync(new URL('../src/keyboard/bridge.js',import.meta.url),'utf8');
const video={tagName:'VIDEO',closest:selector=>selector==='#video-wrapper, #stream-box'?{}:null};
function harness(origin='https://192.0.2.10',allowedOrigin='https://192.0.2.10') {
  const messages=[],ipc=[],listeners=new Map(),documentListeners=new Map();
  const context={
    __cometConnectionOrigin:allowedOrigin,location:{origin},performance:{now:()=>0},
    URL,WebSocket:class{
      constructor(url){this.url=url;this.listeners={};}
      addEventListener(name,callback){this.listeners[name]=callback;}
      send(){}
    },
    document:{readyState:'loading',hasFocus:()=>true,elementFromPoint:()=>video,addEventListener:(name,cb)=>documentListeners.set(name,cb),querySelector:()=>({readyState:2,videoWidth:2560})},
    addEventListener:(name,cb)=>listeners.set(name,cb),
    ipc:{postMessage:value=>ipc.push(JSON.parse(value))},
    postMessage:(value,target)=>messages.push({value:JSON.parse(value),target}),
  };
  context.location.host=new URL(origin).host;
  context.setInterval=callback=>context.tick=callback;
  context.window=context;context.top=context;vm.runInNewContext(source,context);
  return{context,messages,ipc,listeners,documentListeners,native:context.__cometNative};
}
{
  const h=harness();h.native.key('KeyV',true);assert.equal(h.messages.length,0);
  h.native.setCaptured(true);
  for(const[code,down]of[['MetaLeft',true],['KeyV',true],['KeyV',true],['KeyV',false],['MetaLeft',false]])h.native.key(code,down);
  assert.deepEqual(h.messages.map(m=>m.value.payload),[{key:'MetaLeft',state:true},{key:'KeyV',state:true},{key:'KeyV',state:false},{key:'MetaLeft',state:false}]);
  assert.ok(h.messages.every(m=>m.target==='https://192.0.2.10'&&m.value.action==='remote_keyboard_event'));
}
{
  const h=harness();h.documentListeners.get('pointerdown')({target:video,clientX:100,clientY:100});
  assert.equal(h.ipc.at(-1).enabled,true);
  h.native.setCaptured(true);h.native.key('ControlLeft',true);h.native.key('KeyW',true);
  assert.equal(h.documentListeners.has('pointerleave'),false);
  assert.equal(h.documentListeners.has('pointermove'),false);
  assert.equal(h.messages.length,2);
  h.native.setCaptured(false); // Native foreground loss or an explicit click releases both keys.
  assert.deepEqual(h.messages.map(m=>m.value.payload),[{key:'ControlLeft',state:true},{key:'KeyW',state:true},{key:'ControlLeft',state:false},{key:'KeyW',state:false}]);
  assert.equal(h.ipc.at(-1).enabled,true,'leaving a document must not disarm native window capture');
  h.native.key('KeyV',true);assert.equal(h.messages.length,4,'queued downs must be ignored after release');
  h.documentListeners.get('pointerdown')({target:video});
  assert.equal(h.ipc.at(-1).enabled,true);
  h.native.key('KeyV',true);assert.equal(h.messages.length,4,'hover must wait for native permission instead of enabling capture itself');
}
{
  const h=harness();h.documentListeners.get('pointerdown')({target:video});h.native.setCaptured(true);h.native.key('AltLeft',true);
  h.native.setCaptured(false);h.native.key('AltLeft',false);
  assert.equal(h.messages.length,2);assert.equal(h.ipc.at(-1).enabled,true,'child-view blur must not disarm native capture');
}
{
  const h=harness();h.documentListeners.get('pointerdown')({target:video});
  let blocked=0;h.listeners.get('keydown')({target:video,preventDefault(){blocked++},stopImmediatePropagation(){blocked++}});
  assert.equal(blocked,2,'firmware DOM keys must stay blocked when native capture is disabled');
  assert.equal(h.messages.length,0);
  const input={tagName:'INPUT',closest:selector=>selector==='#video-wrapper, #stream-box'?null:{}};
  h.documentListeners.get('pointerdown')({target:input});assert.equal(h.ipc.at(-1).enabled,false);
}
{
  const h=harness();h.native.setCaptured(true);
  for(const code of['ControlLeft','AltLeft','F12'])h.native.key(code,true);
  for(const code of['F12','AltLeft','ControlLeft'])h.native.key(code,false);
  assert.equal(h.messages.length,6,'the former release chord is now ordinary remote keyboard input');
}
{
  const h=harness();h.native.setCaptured(true,true);
  const event={target:video,code:'KeyA',isTrusted:true,preventDefault(){},stopImmediatePropagation(){}};
  h.listeners.get('keydown')(event);h.listeners.get('keydown')(event);h.listeners.get('keyup')(event);
  assert.deepEqual(h.messages.map(m=>m.value.payload),[{key:'KeyA',state:true},{key:'KeyA',state:false}], 'trusted DOM input must survive when a transport bypasses the low-level hook');
  h.listeners.get('keydown')({...event,isTrusted:false});assert.equal(h.messages.length,2);
  h.native.setCaptured(false);h.listeners.get('keydown')(event);assert.equal(h.messages.length,2);
}
{
  const h=harness();h.native.setCaptured(true);
  h.native.batch([['ControlLeft',true],['AltLeft',true],['Delete',true]]);
  // The local security screen can interrupt the chord before the key-up events.
  h.native.setCaptured(false);h.native.setCaptured(true);
  h.native.batch([['Digit1',true],['Digit1',false]]);
  assert.deepEqual(h.messages.map(m=>m.value.payload),[
    {key:'ControlLeft',state:true},{key:'AltLeft',state:true},{key:'Delete',state:true},
    {key:'ControlLeft',state:false},{key:'AltLeft',state:false},{key:'Delete',state:false},
    {key:'Digit1',state:true},{key:'Digit1',state:false}
  ],'interrupted chords must be released before capture resumes');
}
assert.equal(harness('https://example.com').native,undefined);
{
  const h=harness();
  h.context.document.querySelector=()=>null; // KVM online without an HDMI source.
  h.documentListeners.get('DOMContentLoaded')();
  const socket=new h.context.WebSocket('wss://192.0.2.10/api/ws');
  socket.listeners.open();h.context.tick();
  assert.equal(h.ipc.at(-1).connected,true);
  h.native.setCaptured(true);
  h.native.key('Digit1',true);h.context.tick();h.native.key('Digit1',false);
  assert.deepEqual(h.messages.map(m=>m.value.payload),[{key:'Digit1',state:true},{key:'Digit1',state:false}],
    'a missing HDMI signal must not interrupt keyboard capture');
  socket.listeners.close();h.context.tick();
  assert.equal(h.ipc.at(-1).connected,false);
}
{
  const origin='https://kvm2.example.test:8443',h=harness(origin,origin);h.native.setCaptured(true);h.native.key('KeyA',true);
  assert.equal(h.messages[0].target,origin);
  assert.equal(harness('https://kvm2.example.test',origin).native,undefined);
}
{
  const h=harness(); h.native.setCaptured(true);
  const keys=[['AltLeft',true],['Tab',true],['Tab',false],['AltLeft',false],['MetaLeft',true],['KeyV',true],['KeyV',false],['MetaLeft',false]];
  h.native.batch(keys);
  h.listeners.get('keydown')({target:video,code:'KeyV',isTrusted:true,preventDefault(){},stopImmediatePropagation(){}});
  assert.deepEqual(h.messages.map(m=>[m.value.payload.key,m.value.payload.state]), keys);
  h.native.setCaptured(false); h.native.batch(keys); assert.equal(h.messages.length,8);
}
console.log('Bridge: passed (buffered chords, click capture, release, basic typing, origin isolation).');
