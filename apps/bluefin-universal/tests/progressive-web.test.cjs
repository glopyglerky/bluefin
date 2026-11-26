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
function browserFailure(message){return Object.assign(Error(message),{bluefinBrowserCapability:true});}
function setup({meshFailure=false,leaseOrigin='',holdLogin=false,holdLease=false,cookieStatus=200,cookieMalformed=false,cookieCreateStatus=200,tvAdvertisement=true,launchCode='',engine}={}){
 const root=new Element('div'),video=new Element('video'),labels=new Map(),events={},requests=[],pending={};
 Object.assign(video,{paused:true,duration:60,currentTime:0,src:'',
  pause(){this.paused=true;},load(){},play(){this.paused=false;this.playCalls=(this.playCalls||0)+1;return Promise.resolve();},canPlayType(){return 'probably';}});
 labels.set('app',root);labels.set('media',video);
 const document={baseURI:'https://bluefin.test/web/bluefin/',body:new Element('body'),head:new Element('head'),activeElement:null,
  createElement:tag=>new Element(tag),createElementNS:(_ns,tag)=>new Element(tag),createDocumentFragment:()=>new Element('fragment'),
  getElementById:id=>{if(!labels.has(id))labels.set(id,new Element('div'));return labels.get(id);},
  querySelectorAll:()=>[],addEventListener(){}};
 const user='11111111111141118111111111111111',item={Id:'22222222222242228222222222222222',Name:'Original test tone',Type:'Audio'};
 const fetch=async(url,options={})=>{
  const u=new URL(url);requests.push({url:u.href,options});
  let body;
  if(u.pathname==='/Bluefin/Web/v1'&&cookieStatus!==200)return new Response('{}',{status:cookieStatus});
  if(u.pathname==='/Bluefin/Web/v1/leases'&&cookieCreateStatus!==200)return new Response('{}',{status:cookieCreateStatus});
  if(u.pathname==='/Users/AuthenticateByName'&&holdLogin)await new Promise(resolve=>pending.login=resolve);
  if(u.pathname==='/Bluefin/Web/v1/leases'&&options.method==='POST'&&holdLease)await new Promise(resolve=>pending.lease=resolve);
  if(u.pathname==='/System/Info/Public')body={ServerName:'Bluefin'};
  else if(u.pathname==='/Bluefin/Accounts/Options')body={passkeys:true,quickConnect:true};
  else if(u.pathname==='/Users/AuthenticateByName')body={AccessToken:'synthetic-private-bearer',User:{Id:user,Name:'Listener'}};
  else if(u.pathname==='/Bluefin/Capabilities')body={protocolVersion:1,viewer:{userId:user}};
  else if(u.pathname==='/Bluefin/Accounts/Mesh/Invite')body={singleUse:true,intent:'login',code:'MC1-INVITATION',webLink:'https://bluefin.test/web/bluefin/#bluefin-connect=MC1-INVITATION',pngBase64:'iVBORw0KGgo=',handle:'a'.repeat(64)};
  else if(u.pathname==='/Bluefin/Web/v1')body={version:cookieMalformed?99:1,cookieMediaLease:true,expiresInSeconds:300,maxSessionSeconds:7200,idleSeconds:90};
  else if(u.pathname==='/Bluefin/Web/v1/leases')body={version:1,mediaUrl:leaseOrigin+'/Bluefin/Web/v1/media/'+'a'.repeat(64)+'/0',
   releaseUrl:'/Bluefin/Web/v1/leases/'+'a'.repeat(64),renewUrl:'/Bluefin/Web/v1/leases/'+'a'.repeat(64),
   expiresInSeconds:300,idleSeconds:90,playSessionId:'33333333333343338333333333333333'};
  else if(u.pathname==='/Bluefin/TV/v1')body={version:1,mediaLease:tvAdvertisement,absoluteSeconds:7200,idleSeconds:90};
  else if(u.pathname==='/Bluefin/TV/v1/leases'&&options.method==='POST')body={version:1,
   mediaUrl:'/Bluefin/TV/v1/media/'+'b'.repeat(64)+'/0',releaseUrl:'/Bluefin/TV/v1/leases/'+'b'.repeat(64),
   expiresInSeconds:7200,idleSeconds:90,playSessionId:'33333333333343338333333333333333'};
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
 context.takeBluefinLaunch=()=>{const code=launchCode;launchCode='';return code;};
 context.BluefinRuntime={engine:engine||(async()=>{throw browserFailure(meshFailure?'Blocked optional engine':'Wasm unavailable');}),ready:()=>null,
  bounded:promise=>promise};
 Object.defineProperty(context,'indexedDB',{get(){throw Error('Default view touched IndexedDB');}});
 Object.defineProperty(context,'localStorage',{get(){throw Error('Default view touched localStorage');}});
 Object.defineProperty(context,'sessionStorage',{get(){throw Error('Default view touched sessionStorage');}});
 const script=source('native-view.js').replace(/\}\)\(\);\s*$/,'globalThis.contract={state,connect,signIn,navigate,beginMesh,play,signOut,stopPlayback,render,nativeLeasePath,tvLeasePath,leaseRequest,issueInvite};})();');
 vm.runInContext(script,context);
 return {context,ui:context.contract,root,video,requests,events,item,pending};
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
 await assert.rejects(j.selectMeshStorage(true),/Protected connection storage unavailable/);
});
test('tiny-Wasm denial does not import the optional Rust module',async()=>{
 const context=vm.createContext({isSecureContext:true,location:new URL('https://bluefin.test/'),WebAssembly:undefined,setTimeout,clearTimeout});
 context.window=context;vm.runInContext(source('bootstrap.js'),context);
 await assert.rejects(context.BluefinRuntime.engine(),/HTTPS library remains available/);
 assert.equal(context.BluefinRuntime.ready(),undefined);
});

async function until(check){for(let n=0;n<100;n++){if(check())return;await new Promise(setImmediate);}assert.fail('pending boundary was not reached');}
test('a login minted after pagehide is revoked instead of adopted',async()=>{
 const t=setup({holdLogin:true});await t.ui.connect('https://bluefin.test');
 const login=t.ui.signIn('Listener','example-password',false);const result=assert.rejects(login,/page or account changed/);
 await until(()=>t.pending.login);t.events.pagehide();t.pending.login();await result;
 assert.equal(t.ui.state.session,null);
 assert(t.requests.some(r=>r.url.endsWith('/Sessions/Logout')&&r.options.keepalive===true));
 assert(!t.root.textContent.includes('Original test tone'));
});
test('a delayed optional runtime cannot begin pairing after pagehide',async()=>{
 const t=setup();let resolve,begin=0,retire=0;
 t.context.BluefinRuntime.engine=()=>new Promise(r=>resolve=r);
 const attempt=t.ui.beginMesh('MC1-EXAMPLE','',false);const result=assert.rejects(attempt,/page or account changed/);
 await until(()=>resolve);t.events.pagehide();
 resolve({webMeshBegin(){begin++;return Promise.resolve('{"kind":"pairing"}');},webMeshPageExit(){retire++;}});
 await result;assert.equal(begin,0);assert.equal(retire,0);assert.equal(t.requests.length,0);
});
test('pending mesh worker setup cannot start a retired player',async()=>{
 const t=setup();let resolve,plays=0,retire=0;
 const engine={webMeshPlay(){plays++;},webMeshControl(){},webMeshPageExit(){retire++;}};
 t.context.BluefinRuntime.engine=async()=>engine;t.context.BluefinRuntime.ready=()=>engine;
 t.context.BluefinRuntime.mediaWorker=()=>new Promise(r=>resolve=r);
 t.ui.state.session={handle:'bluefin-mesh-v1:'+'a'.repeat(64),user:'1'.repeat(32)};t.ui.state.mode='mesh';
 const playing=t.ui.play(t.item),result=assert.rejects(playing,/page or account changed/);
 await until(()=>resolve);t.events.pagehide();resolve();await result;
 assert.equal(plays,0);assert.equal(retire,1);assert.equal(t.video.src,'');
});
test('a late HTTP media lease is released without starting playback',async()=>{
 const t=setup({holdLease:true});await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const playing=t.ui.play(t.item),result=assert.rejects(playing,/page or account changed/);
 await until(()=>t.pending.lease);t.events.pagehide();t.pending.lease();await result;
 assert.equal(t.video.src,'');assert.equal(t.video.playCalls||0,0);
 assert(t.requests.some(r=>r.options.method==='DELETE'&&r.options.keepalive===true&&r.url.includes('/Bluefin/Web/v1/leases/')));
});
test('a throwing decoder still loses its local source and playback state',async()=>{
 const t=setup();t.video.src='blob:synthetic';t.video.paused=false;
 t.ui.state.player={mode:'https',hls:{destroy(){throw Error('decoder failure');}}};
 await assert.rejects(t.ui.stopPlayback(),/decoder failure/);
 assert.equal(t.video.src,'');assert.equal(t.video.paused,true);assert.equal(t.ui.state.player,null);
});

test('only advertised cookie-capability absence selects the old per-item TV capability',async()=>{
 const t=setup({cookieStatus:404});await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);assert.equal(t.ui.state.player.lease.kind,'tv');
 assert.match(t.video.src,/^https:\/\/bluefin.test\/Bluefin\/TV\/v1\/media\/[a-f0-9]{64}\/0$/);
 assert(!t.video.src.includes('synthetic-private-bearer'));assert(!t.root.textContent.includes('b'.repeat(64)));
 await assert.rejects(t.ui.leaseRequest(t.ui.state.player.lease.releaseUrl,'POST',t.ui.state.session,false,'tv'),/Invalid playback lease operation/);
 await t.ui.signOut();
 assert(t.requests.some(r=>r.options.method==='DELETE'&&r.url.includes('/Bluefin/TV/v1/leases/')));
 assert(!t.requests.some(r=>r.options.method==='POST'&&r.url.includes('/Bluefin/TV/v1/leases/')));
});
test('permission failures and malformed cookie advertisements never select TV compatibility',async()=>{
 for(const settings of [{cookieStatus:401},{cookieStatus:403},{cookieStatus:503},{cookieMalformed:true},{cookieCreateStatus:403}]){
  const t=setup(settings);await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
  await assert.rejects(t.ui.play(t.item));assert.equal(t.video.src,'');
  assert(!t.requests.some(r=>new URL(r.url).pathname.startsWith('/Bluefin/TV/')));
 }
});
test('legacy compatibility requires its complete version and rejects secret-route escapes',async()=>{
 const t=setup({cookieStatus:404,tvAdvertisement:false});await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await assert.rejects(t.ui.play(t.item),/does not advertise/);
 assert(!t.requests.some(r=>r.options.method==='POST'&&new URL(r.url).pathname==='/Bluefin/TV/v1/leases'));
 const secret='b'.repeat(64);
 for(const raw of ['https://evil.test/Bluefin/TV/v1/media/'+secret+'/0','/Bluefin/TV/v1/media/'+secret+'/0?api_key=x',
 '/Bluefin/TV/v1/media/'+secret+'/%30','/Bluefin/TV/v1/media/'+secret+'/../0','/Bluefin/Web/v1/media/'+secret+'/0'])
  assert.throws(()=>t.ui.tvLeasePath(raw,'media',secret));
});

function findElement(root,predicate) {
 if(predicate(root))return root;
 for(const child of root.children||[]){const found=findElement(child,predicate);if(found)return found;}
 return null;
}
test('QR capability failure offers optional site guidance before sending an invitation',async()=>{
 const code='MC1-EXAMPLE',t=setup({launchCode:code,meshFailure:true});
 assert.equal(t.requests.length,0);assert.equal(t.ui.state.sheet,'invite');
 assert.equal(t.ui.state.capability,null);
 await assert.rejects(t.ui.beginMesh(code,'',false),/Blocked optional engine/);
 t.ui.render();
 const dialog=findElement(t.root,e=>e.tagName==='DIALOG');
 assert(dialog);assert.equal(dialog.attributes['aria-modal'],'true');
 assert.equal(dialog.attributes['aria-labelledby'],'capability-title');
 assert.match(dialog.textContent,/cannot determine whether it is enabled/);
 assert.match(dialog.textContent,/has not sent your invitation/);
 assert.match(dialog.textContent,/reopen it after a reload/);
 assert.match(dialog.textContent,/Page Menu/);assert.match(dialog.textContent,/Website Settings/);
 assert.match(dialog.textContent,/reduces Lockdown Mode protection for that site/);
 assert.match(dialog.textContent,/Media and account services remain on/);
 assert(!dialog.textContent.includes('Lockdown Mode is enabled'));
 const native=findElement(dialog,e=>e.tagName==='A'&&e.href?.startsWith('bluefin:'));
 assert.equal(native.href,'bluefin://connect/'+code);
 const guidance=findElement(dialog,e=>e.tagName==='A'&&e.href?.startsWith('https://support.apple.com/'));
 assert.equal(guidance.href,'https://support.apple.com/en-us/105120');
 assert.equal(guidance.rel,'noopener noreferrer');
 assert.equal(t.requests.length,0);assert.equal(t.ui.state.session,null);
 let prevented=false;dialog.listeners.cancel({preventDefault(){prevented=true;}});
 assert(prevented);assert.equal(t.ui.state.capability,null);assert.equal(t.ui.state.code,code);
 assert.equal(t.ui.state.sheet,'invite');assert.equal(t.requests.length,0);
});
test('capability retry keeps the same invitation and invokes Rust only after explicit retry',async()=>{
 const code='MC1-EXAMPLE',attempts=[];let probes=0;
 const t=setup({launchCode:code,engine:async()=>{
  probes++;if(probes===1)throw browserFailure('Optional engine blocked');
  return {webMeshBegin:async(...args)=>{attempts.push(args);return JSON.stringify({kind:'pairing',needsBeginRetry:true});}};
 }});
 assert.equal(probes,0);assert.equal(attempts.length,0);
 await assert.rejects(t.ui.beginMesh(code,'Viewer',false),/Optional engine blocked/);
 t.ui.render();assert.equal(attempts.length,0);
 const retry=findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent==='Try again');
 await retry.listeners.click();
 assert.equal(probes,2);assert.deepEqual(attempts,[[code,'Viewer',false]]);
 assert.equal(t.ui.state.capability,null);assert.equal(t.ui.state.pairing.needsBeginRetry,true);
 assert.equal(t.requests.length,0);
});
test('an invitation or account rejection after Rust starts does not suggest a browser exception',async()=>{
 const t=setup({engine:async()=>({webMeshBegin:async()=>{throw Error('Invitation was already used');}})});
 await assert.rejects(t.ui.beginMesh('MC1-EXAMPLE','',false),/Invitation was already used/);
 assert.equal(t.ui.state.capability,null);assert.equal(t.requests.length,0);
});
test('leaving the capability sheet for HTTPS sign-in does not redeem the invitation',async()=>{
 const t=setup({launchCode:'MC1-EXAMPLE',meshFailure:true});
 await assert.rejects(t.ui.beginMesh('MC1-EXAMPLE','',false),/Blocked optional engine/);
 t.ui.render();const choice=findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent==='Use HTTPS sign-in');
 choice.listeners.click();
 assert.equal(t.ui.state.capability,null);assert.equal(t.ui.state.sheet,null);
 assert.equal(t.ui.state.phase,'connect');assert.equal(t.ui.state.session,null);
 assert.equal(t.requests.length,0);
});
test('blocked mesh media preserves its session and warns that claimed temporary keys cannot survive reload',async()=>{
 const t=setup(),session={handle:'bluefin-mesh-v1:'+'b'.repeat(64),user:'11111111111141118111111111111111',name:'Viewer'};
 t.ui.state.session=session;t.ui.state.mode='mesh';t.ui.state.phase='library';
 t.context.BluefinRuntime.mediaWorker=async()=>{throw browserFailure('Media worker unavailable');};
 await assert.rejects(t.ui.play(t.item),/Media worker unavailable/);
 t.ui.render();
 assert.equal(t.ui.state.session,session);assert.equal(t.ui.state.mode,'mesh');
 assert.equal(t.ui.state.capability.beforeBegin,false);
 const dialog=findElement(t.root,e=>e.tagName==='DIALOG');
 assert.match(dialog.textContent,/Encrypted playback is unavailable/);
 assert.match(dialog.textContent,/original QR cannot restore that key/);
 assert(!dialog.textContent.includes('has not sent your invitation'));
 assert.equal(t.requests.length,0);
});
test('account invitation creation supplies its explicit Login intent and renders the returned QR',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 try {
  t.ui.state.sheet='connections';await t.ui.issueInvite();t.ui.render();
  const request=t.requests.find(r=>new URL(r.url).pathname==='/Bluefin/Accounts/Mesh/Invite');
  assert(request);assert.equal(request.options.method,'POST');
  assert.deepEqual(JSON.parse(request.options.body),{intent:'login',lifetimeSeconds:600});
  assert.equal(t.ui.state.invitation.code,'MC1-INVITATION');
  assert(findElement(t.root,e=>e.tagName==='IMG'&&e.alt==='One-use Bluefin device invitation'));
  assert(!t.root.textContent.includes('synthetic-private-bearer'));
 } finally {await t.ui.signOut();}
});
function journalContext() {
 const context=vm.createContext({crypto:webcrypto,TextEncoder,TextDecoder,Uint8Array,ArrayBuffer,URL,
  location:new URL('https://bluefin.test/web/bluefin/'),isSecureContext:true,navigator:{},
  document:{addEventListener(){}},addEventListener(){},dispatchEvent(){},CustomEvent:class{},
  setTimeout,clearTimeout});
 vm.runInContext(source('mesh-platform.js').replace(/\bexport /g,'')+
  '\nglobalThis.journal={loadMeshVault,selectMeshStorage,meshRemembered};',context);
 return context;
}
test('missing and denied protected storage stay optional and report the same actionable error',async()=>{
 for(const kind of ['missing','getter','open']){
  const context=journalContext();
  if(kind==='getter')Object.defineProperty(context,'indexedDB',{get(){throw Error('SecurityError');}});
  if(kind==='open')context.indexedDB={open(){throw Error('SecurityError');}};
  const j=context.journal;
  assert.deepEqual(JSON.parse(await j.loadMeshVault()),{revision:0,value:null});
  await assert.rejects(j.selectMeshStorage(true),/Protected connection storage unavailable.*temporary connection/);
  assert.equal(j.meshRemembered(),false);
  assert.deepEqual(JSON.parse(await j.loadMeshVault()),{revision:0,value:null});
 }
});
test('a failed protected storage open can be retried after the user permits access',async()=>{
 const context=journalContext(),j=context.journal;let opens=0;
 context.indexedDB={open(){opens++;throw Error('SecurityError');}};
 await assert.rejects(j.selectMeshStorage(true),/Protected connection storage unavailable/);
 context.indexedDB={open(){opens++;const request={result:{close(){}}};setTimeout(()=>request.onsuccess(),0);return request;}};
 await j.selectMeshStorage(true);
 assert.equal(opens,2);assert.equal(j.meshRemembered(),true);
});
