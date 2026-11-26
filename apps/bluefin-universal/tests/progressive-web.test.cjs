// Executes the shipped progressive view with unavailable Wasm, IndexedDB and
// workers. Browser decoding/visual parity remain separate device checks.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {webcrypto}=require('node:crypto');
const source=name=>fs.readFileSync(path.join(__dirname,'..',name),'utf8');
class Element{
 constructor(tag){this.tagName=tag.toUpperCase();this.children=[];this.dataset={};this.style={};this.attributes={};this.listeners={};this.value='';this.text='';this.className='';this.classList={add(){},remove(){},toggle(){}};}
 append(...values){this.children.push(...values);}
 replaceChildren(...values){this.children=values;}
 setAttribute(k,v){this.attributes[k]=String(v);}
 getAttribute(k){return this.attributes[k]||null;}
 removeAttribute(k){delete this.attributes[k];if(k==='src')this.src='';}
 addEventListener(k,f){this.listeners[k]=f;}
 remove(){this.removed=true;}
 get lastChild(){return this.children.at(-1);}
 set textContent(v){this.text=String(v);this.children=[];}
 get textContent(){return this.text+this.children.map(c=>c.textContent||'').join(' ');}
}
function setup({meshFailure=false,leaseOrigin=''}={}){
 const root=new Element('div'),video=new Element('video'),labels=new Map(),events={},requests=[];
 Object.assign(video,{paused:true,duration:60,currentTime:0,src:'',
  pause(){this.paused=true;},load(){},play(){this.paused=false;return Promise.resolve();},canPlayType(){return 'probably';}});
 labels.set('app',root);labels.set('media',video);
 const document={baseURI:'https://bluefin.test/web/bluefin/',body:new Element('body'),head:new Element('head'),activeElement:null,
  createElement:tag=>new Element(tag),createElementNS:(_ns,tag)=>new Element(tag),createDocumentFragment:()=>new Element('fragment'),
  getElementById:id=>{if(!labels.has(id))labels.set(id,new Element('div'));return labels.get(id);},
  querySelectorAll:()=>[],addEventListener(){}};
 const user='11111111111141118111111111111111',item={Id:'22222222222242228222222222222222',Name:'Original test tone',Type:'Audio'};
 const fetch=async(url,options={})=>{
  const u=new URL(url);requests.push({url:u.href,options});
  let body;
  if(u.pathname==='/System/Info/Public')body={ServerName:'Bluefin'};
  else if(u.pathname==='/Bluefin/Accounts/Options')body={passkeys:true,quickConnect:true};
  else if(u.pathname==='/Users/AuthenticateByName')body={AccessToken:'synthetic-private-bearer',User:{Id:user,Name:'Listener'}};
  else if(u.pathname==='/Bluefin/Capabilities')body={protocolVersion:1,viewer:{userId:user}};
  else if(u.pathname==='/Bluefin/Web/v1')body={version:1,cookieMediaLease:true};
  else if(u.pathname==='/Bluefin/Web/v1/leases')body={version:1,mediaUrl:leaseOrigin+'/Bluefin/Web/v1/media/'+'a'.repeat(64)+'/0',
   releaseUrl:'/Bluefin/Web/v1/leases/'+'a'.repeat(64),renewUrl:'/Bluefin/Web/v1/leases/'+'a'.repeat(64),
   expiresInSeconds:300,playSessionId:'synthetic-play'};
  else if(u.pathname.endsWith('/Views'))body={Items:[{...item,Type:'CollectionFolder',Name:'Music'}]};
  else if(u.pathname.endsWith('/Latest'))body=[item];
  else if(u.pathname.endsWith('/Resume')||u.pathname==='/Shows/NextUp')body={Items:[]};
  else body={Items:[item]};
  return new Response(JSON.stringify(body),{headers:{'Content-Type':'application/json'}});
 };
 const context=vm.createContext({document,location:new URL(document.baseURI),navigator:{},
  crypto:webcrypto,URL,Uint8Array,Uint32Array,Blob,TextEncoder,TextDecoder,AbortController,Response,fetch,
  setTimeout,clearTimeout,setInterval:()=>1,clearInterval(){},addEventListener:(name,fn)=>events[name]=fn,
  isSecureContext:true,WebAssembly:undefined});
 context.window=context;
 context.BluefinRuntime={engine:async()=>{throw Error(meshFailure?'Blocked optional engine':'Wasm unavailable');},ready:()=>null,
  bounded:promise=>promise};
 Object.defineProperty(context,'indexedDB',{get(){throw Error('Default view touched IndexedDB');}});
 Object.defineProperty(context,'localStorage',{get(){throw Error('Default view touched localStorage');}});
 Object.defineProperty(context,'sessionStorage',{get(){throw Error('Default view touched sessionStorage');}});
 const script=source('native-view.js').replace(/\}\)\(\);\s*$/,'globalThis.contract={state,connect,signIn,navigate,beginMesh,play,signOut,render,nativeLeasePath};})();');
 vm.runInContext(script,context);
 return {context,ui:context.contract,root,video,requests,events,item};
}
test('ordinary HTML controls and HTTPS library work without Wasm, storage, or workers',async()=>{
 const t=setup();assert.match(t.root.textContent,/Connect with QR or invitation/);assert.equal(t.requests.length,0);
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);await t.ui.navigate('Library');t.ui.render();
 assert.match(t.root.textContent,/Collections/);assert.match(t.root.textContent,/Music/);
 assert(!t.root.textContent.includes('synthetic-private-bearer'));
 assert(t.requests.every(r=>!r.url.includes('synthetic-private-bearer')));
 assert(t.requests.every(r=>r.options.redirect==='error'));
 await t.ui.signOut();assert.equal(t.ui.state.session,null);
});
test('blocked PQ capability never redeems a QR over HTTPS or signs in implicitly',async()=>{
 const t=setup({meshFailure:true});
 await assert.rejects(t.ui.beginMesh('MC1-EXAMPLE','',false),/Blocked optional engine/);
 assert.equal(t.requests.length,0);assert.equal(t.ui.state.session,null);
});
test('HTTPS origin and media lease escapes are rejected before playback',async()=>{
 const t=setup({leaseOrigin:'https://unrelated.test'});
 await assert.rejects(t.ui.connect('https://unrelated.test'),/trusted HTTPS Bluefin page/);
 assert.equal(t.requests.length,0);
 for(const raw of ['https://evil.test/Bluefin/Web/v1/media/'+'a'.repeat(64)+'/0',
  '/Bluefin/Web/v1/media/'+'a'.repeat(64)+'/0?token=x',
  '/Bluefin/Web/v1/media/'+'a'.repeat(64)+'/../0'])
  assert.throws(()=>t.ui.nativeLeasePath(raw,'media'));
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await assert.rejects(t.ui.play(t.item),/Invalid playback lease route/);
 assert.equal(t.video.src,'');
});
test('native HLS uses a scoped cookie route without a worker or bearer URL',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);assert.match(t.video.src,/^https:\/\/bluefin.test\/Bluefin\/Web\/v1\/media\/[a-f0-9]{64}\/0$/);
 assert(!t.video.src.includes('synthetic-private-bearer'));assert.equal(t.video.paused,false);
 await t.ui.signOut();assert.equal(t.video.src,'');assert(t.requests.some(r=>r.options.method==='DELETE'&&r.url.includes('/Bluefin/Web/v1/leases/')));
});
test('page retirement erases in-memory HTTP credentials and requests remote cleanup',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 t.events.pagehide();assert.equal(t.ui.state.session,null);assert.equal(t.video.src,'');
 assert(t.requests.some(r=>r.url.endsWith('/Sessions/Logout')&&r.options.keepalive===true));
});
test('mesh journal is temporary by default and enforces exact revision retry',async()=>{
 const events={},context=vm.createContext({crypto:webcrypto,TextEncoder,TextDecoder,Uint8Array,ArrayBuffer,URL,
  location:new URL('https://bluefin.test/web/bluefin/'),isSecureContext:true,navigator:{},
  document:{addEventListener:(n,f)=>events[n]=f},addEventListener(){},dispatchEvent(){},CustomEvent:class{},
  setTimeout,clearTimeout});
 Object.defineProperty(context,'indexedDB',{get(){throw Error('Protected storage blocked');}});
 const script=source('mesh-platform.js').replace(/\bexport /g,'')+'\nglobalThis.journal={loadMeshVault,saveMeshVault,selectMeshStorage,clearEphemeralMeshVault};';
 vm.runInContext(script,context);const j=context.journal;
 assert.deepEqual(JSON.parse(await j.loadMeshVault()),{revision:0,value:null});
 await j.saveMeshVault(0,'saved exact request');
 assert.deepEqual(JSON.parse(await j.loadMeshVault()),{revision:1,value:'saved exact request'});
 await assert.rejects(j.saveMeshVault(0,'different request'),/changed/);
 await assert.rejects(j.selectMeshStorage(true),/already active/);
 j.clearEphemeralMeshVault();assert.equal(JSON.parse(await j.loadMeshVault()).value,null);
 await assert.rejects(j.selectMeshStorage(true),/Protected storage blocked/);
});
test('tiny-Wasm denial does not import the optional Rust module',async()=>{
 const context=vm.createContext({isSecureContext:true,location:new URL('https://bluefin.test/'),WebAssembly:undefined,setTimeout,clearTimeout});
 context.window=context;vm.runInContext(source('bootstrap.js'),context);
 await assert.rejects(context.BluefinRuntime.engine(),/HTTPS library remains available/);
 assert.equal(context.BluefinRuntime.ready(),undefined);
});
