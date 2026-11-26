// Executes the shipped progressive view with unavailable Wasm, IndexedDB and
// workers. Browser decoding/visual parity remain separate device checks.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {webcrypto}=require('node:crypto');
const {File}=require('node:buffer');
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
function setup({meshFailure=false,leaseOrigin='',holdLogin=false,holdLease=false,cookieStatus=200,cookieMalformed=false,cookieCreateStatus=200,tvAdvertisement=true,nativeHls=true,audioPlayable=true,launchCode='',engine}={}){
 const root=new Element('div'),video=new Element('video'),labels=new Map(),events={},requests=[],pending={};
 Object.assign(video,{paused:true,duration:60,currentTime:0,src:'',
  pause(){this.paused=true;},load(){},play(){this.paused=false;this.playCalls=(this.playCalls||0)+1;return Promise.resolve();},canPlayType(type){return (type==='audio/mpeg'?audioPlayable:nativeHls)?'probably':'';}});
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
 const register=(name,fn)=>{const previous=events[name];events[name]=(...args)=>{previous?.(...args);fn(...args);};};
 const context=vm.createContext({document,location:new URL(document.baseURI),navigator:{},
  crypto:webcrypto,URL,Uint8Array,Uint32Array,Blob,File,TextEncoder,TextDecoder,AbortController,Response,fetch,
  setTimeout,clearTimeout,setInterval:()=>1,clearInterval(){},addEventListener:register,
  isSecureContext:true,WebAssembly:undefined});
 context.window=context;
 context.takeBluefinLaunch=()=>{const code=launchCode;launchCode='';return code;};
 context.BluefinRuntime={engine:engine||(async()=>{throw browserFailure(meshFailure?'Blocked optional engine':'Wasm unavailable');}),ready:()=>null,
  bounded:promise=>promise};
 Object.defineProperty(context,'indexedDB',{get(){throw Error('Default view touched IndexedDB');}});
 Object.defineProperty(context,'localStorage',{get(){throw Error('Default view touched localStorage');}});
 Object.defineProperty(context,'sessionStorage',{get(){throw Error('Default view touched sessionStorage');}});
 vm.runInContext(source('local-music.js'),context);
 const script=source('native-view.js').replace(/\}\)\(\);\s*$/,'globalThis.contract={state,connect,signIn,navigate,beginMesh,play,signOut,stopPlayback,render,nativeLeasePath,tvLeasePath,leaseRequest,issueInvite,openLocalMusic,addLocalFiles,localItems,localOwner,loadRoute,reportProgress,playbackTimeline,seekPlayback,toggleShuffle,playShuffled,importInvitationImage:readInvitationImage};})();');
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
test('native media uses a scoped cookie route without a worker or bearer URL',async()=>{
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
 const operation=retry.listeners.click();
 assert.equal(t.ui.state.capability,null);
 assert(!t.root.textContent.includes('This attempt has not sent your invitation'));
 await operation;
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

test('temporary local Files play and search without any account, storage, worker or network',async()=>{
 const t=setup();t.ui.openLocalMusic();
 await t.ui.addLocalFiles([new File([Buffer.from('ID3local fixture')],'private-original.mp3')]);
 assert.equal(t.ui.state.rows.length,1);assert.equal(t.requests.length,0);
 const track=t.ui.state.rows[0];assert.match(track.Id,/^local-file-/);
 await t.ui.play(track,[track],0);assert.equal(t.ui.state.player.mode,'local');assert.match(t.video.src,/^blob:/);
 assert.equal(t.requests.length,0);
 await t.ui.navigate('Search');assert.equal(t.requests.length,0);
 t.ui.state.localQuery='private';await t.ui.loadRoute();assert.equal(t.ui.state.rows.length,1);assert.equal(t.requests.length,0);
 t.ui.state.expanded=true;t.ui.render();assert.match(t.root.textContent,/Local file/);
 assert.equal(t.requests.length,0);
 await t.ui.signOut();assert.equal(t.video.src,'');assert.equal(t.ui.state.local,false);assert.equal(t.ui.localItems().length,0);
 assert.equal(t.requests.length,0);
});
test('replacing remote playback with a local File retires its remote lease before local playback',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);const before=t.requests.length;
 t.ui.openLocalMusic();await t.ui.addLocalFiles([new File([Buffer.from('ID3local fixture')],'never-upload-this-title.mp3')]);
 const track=t.ui.state.rows[0];await t.ui.play(track,[track],0);
 assert.equal(t.ui.state.player.mode,'local');assert.equal(t.requests.length,before+1);
 assert.equal(t.requests.at(-1).options.method,'DELETE');
 assert(!t.requests.some(request=>(request.options.body||'').includes('never-upload-this-title')));
 t.events.pagehide();assert.equal(t.video.src,'');assert.equal(t.ui.state.player,null);assert.equal(t.ui.state.session,null);
});
test('local source removal stops the shared player and prevents stale ID replay',async()=>{
 const t=setup();t.ui.openLocalMusic();
 await t.ui.addLocalFiles([new File([Buffer.from('ID3local fixture')],'remove-me.mp3')]);
 const track=t.ui.state.rows[0];await t.ui.play(track,[track],0);
 t.ui.localOwner().remove(track.Id);assert.equal(t.video.src,'');assert.equal(t.ui.state.player,null);
 await assert.rejects(t.ui.play(track,[track],0),/Choose this file again/);assert.equal(t.requests.length,0);
});
test('cropped MP3 resume progress, duration and seek add only the audio stream offset',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const item={...t.item,RunTimeTicks:3600*10000000,UserData:{PlaybackPositionTicks:1800*10000000}};
 t.video.duration=1800;await t.ui.play(item);t.video.currentTime=12;
 await t.ui.reportProgress('progress');
 const body=JSON.parse(t.requests.findLast(request=>request.url.endsWith('/Sessions/Playing/Progress')).options.body);
 assert.equal(body.PositionTicks,1812*10000000);
 const timeline=t.ui.playbackTimeline();assert.equal(timeline.position,1812);assert.equal(timeline.duration,3600);assert.equal(timeline.minimum,1800);
 t.ui.seekPlayback(1820);assert.equal(t.video.currentTime,20);
 t.ui.seekPlayback(1700);assert.equal(t.video.currentTime,0);
 await t.ui.signOut();
});
test('turning shuffle off restores the original queue and keeps the current selection',async()=>{
 const t=setup();t.ui.openLocalMusic();
 await t.ui.addLocalFiles(['one','two','three'].map(name=>new File([Buffer.from('ID3'+name)],name+'.mp3')));
 const order=t.ui.state.rows.slice();await t.ui.play(order[1],order,1);
 t.ui.toggleShuffle();assert.equal(t.ui.state.queue[0].Id,order[1].Id);
 t.ui.toggleShuffle();assert.deepEqual(Array.from(t.ui.state.queue,item=>item.Id),Array.from(order,item=>item.Id));
 assert.equal(t.ui.state.queueIndex,1);assert.equal(t.requests.length,0);await t.ui.signOut();
});

test('new explicit playback resets shuffled queues when crossing local and server music',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const remote=[t.item,{...t.item,Id:'44444444444444448444444444444444',Name:'Second server song'}];
 await t.ui.playShuffled(remote);assert.equal(t.ui.state.shuffle,true);
 t.ui.openLocalMusic();await t.ui.addLocalFiles([new File([Buffer.from('ID3local fixture')],'independent.mp3')]);
 const local=t.ui.state.rows[0];await t.ui.play(local,[local],0);
 assert.equal(t.ui.state.shuffle,false);assert.equal(t.ui.state.unshuffledQueue[0].Id,local.Id);
 t.ui.toggleShuffle();t.ui.toggleShuffle();assert.equal(t.ui.state.queue[0].Id,local.Id);
 await t.ui.playShuffled([local]);await t.ui.play(t.item,remote,0);
 assert.equal(t.ui.state.shuffle,false);assert(t.ui.state.unshuffledQueue.every(item=>!item.LocalSource));
 await t.ui.signOut();
});
test('a QR image finishing after page retirement cannot repopulate the invitation',async()=>{
 const t=setup();let resolve,parsed=0;
 t.context.BluefinRuntime.engine=async()=>({webMeshImage(){parsed++;return 'MC1-SECRET';}});
 t.ui.state.sheet='invite';
 const attempt=t.ui.importInvitationImage({size:20,arrayBuffer:()=>new Promise(r=>resolve=r)});
 const outcome=assert.rejects(attempt,/page or account changed/);await until(()=>resolve);
 t.events.pagehide();resolve(new ArrayBuffer(20));await outcome;
 assert.equal(parsed,0);assert.equal(t.ui.state.code,'');assert.equal(t.requests.length,0);
});

test('mesh-to-local replacement waits for its own Rust stop receipt before acquiring playback',async()=>{
 const t=setup();let acknowledge;
 t.context.BluefinRuntime.ready=()=>({webMeshStop:()=>new Promise(resolve=>acknowledge=resolve)});
 t.ui.openLocalMusic();await t.ui.addLocalFiles([new File([Buffer.from('ID3local fixture')],'after-mesh.mp3')]);
 const item=t.ui.state.rows[0];t.ui.state.player={mode:'mesh',handle:'bluefin-mesh-v1:'+'a'.repeat(64),item:t.item};
 t.video.src='/old-mesh';t.video.paused=false;
 const replacement=t.ui.play(item,[item],0);await until(()=>acknowledge);
 assert.equal(t.video.src,'');assert.equal(t.video.playCalls||0,0);assert.equal(t.ui.state.player,null);
 acknowledge();await replacement;assert.equal(t.ui.state.player.mode,'local');assert.match(t.video.src,/^blob:/);
 assert.equal(t.requests.length,0);await t.ui.signOut();
});
test('an unacknowledged mesh stop cannot be bypassed by a second local Play',async()=>{
 const t=setup();t.context.BluefinRuntime.ready=()=>({webMeshStop:async()=>{throw Error('retirement pending');}});
 t.ui.openLocalMusic();await t.ui.addLocalFiles([new File([Buffer.from('ID3local fixture')],'held-until-cleanup.mp3')]);
 const item=t.ui.state.rows[0];t.ui.state.player={mode:'mesh',handle:'bluefin-mesh-v1:'+'a'.repeat(64),item:t.item};
 await assert.rejects(t.ui.play(item,[item],0),/retirement pending/);
 await assert.rejects(t.ui.play(item,[item],0),/retirement pending/);
 assert.equal(t.video.src,'');assert.equal(t.video.playCalls||0,0);assert.equal(t.requests.length,0);t.events.pagehide();
});
test('local music exposes local search even when a server account is connected',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 t.ui.openLocalMusic();t.ui.render();assert.match(t.root.textContent,/Music on this device/);
 // Inspect the rendered control tree, not a CSS inference.
 function descendants(element){return [element,...element.children.flatMap(descendants)];}
 assert(descendants(t.root).some(element=>element.tagName==='INPUT'&&element.attributes['aria-label']==='Search music on this device'));
 const before=t.requests.length;t.ui.state.localQuery='local';t.ui.state.route={kind:'local_search'};await t.ui.loadRoute();
 assert.equal(t.requests.length,before);await t.ui.signOut();
});

test('forced MP3 HTTP leases use native audio even when HLS and MSE are unavailable',async()=>{
 const t=setup({nativeHls:false});await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);assert.equal(t.ui.state.player.mode,'https');assert.equal(t.ui.state.player.hls,undefined);
 assert.equal(t.context.document.head.children.length,0);assert.equal(t.video.paused,false);await t.ui.signOut();
 const unsupported=setup({nativeHls:false,audioPlayable:false});await unsupported.ui.connect('https://bluefin.test');await unsupported.ui.signIn('Listener','example-password',false);
 const before=unsupported.requests.length;await assert.rejects(unsupported.ui.play(unsupported.item),/MP3 audio/);
 assert.equal(unsupported.requests.length,before);assert.equal(unsupported.video.src,'');
});

test('full HLS video seeks to the bookmark and never reports a doubled or premature zero position',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const item={...t.item,Type:'Movie',RunTimeTicks:3600*10000000,UserData:{PlaybackPositionTicks:1800*10000000}};
 t.video.duration=3600;await t.ui.play(item);
 assert.equal(t.ui.state.player.offset,0);
 assert(!t.requests.some(request=>request.url.endsWith('/Sessions/Playing')));
 t.video.listeners.loadedmetadata();assert.equal(t.video.currentTime,1800);
 t.video.currentTime=1812;await t.ui.reportProgress('progress');
 const body=JSON.parse(t.requests.findLast(request=>request.url.endsWith('/Sessions/Playing/Progress')).options.body);
 assert.equal(body.PositionTicks,1812*10000000);
 assert.equal(t.ui.playbackTimeline().duration,3600);assert.equal(t.ui.playbackTimeline().minimum,0);
 t.ui.seekPlayback(1700);assert.equal(t.video.currentTime,1700);
 await t.ui.signOut();
});

test('an out-of-range video bookmark restarts at zero with reporting enabled',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const item={...t.item,Type:'Movie',RunTimeTicks:3600*10000000,UserData:{PlaybackPositionTicks:1800*10000000}};
 t.video.duration=900;await t.ui.play(item);t.video.listeners.loadedmetadata();
 assert.equal(t.video.currentTime,0);assert.equal(t.ui.state.player.resumeApplied,true);assert.equal(t.ui.state.player.reportingReady,true);
 assert.match(t.ui.state.notice,/Starting from the beginning/);await t.ui.reportProgress('progress');
 const body=JSON.parse(t.requests.findLast(request=>request.url.endsWith('/Sessions/Playing/Progress')).options.body);
 assert.equal(body.PositionTicks,0);await t.ui.signOut();
});
test('unsupported HTTPS live TV does not allocate a lease or stop current library playback',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);const before=t.requests.length,source=t.video.src,player=t.ui.state.player;
 await assert.rejects(t.ui.play({...t.item,Type:'TvChannel'}),/Live TV playback/);
 assert.equal(t.requests.length,before);assert.equal(t.video.src,source);assert.equal(t.ui.state.player,player);
 await t.ui.signOut();
});

test('page retirement during optional startup prevents the later engine from claiming an invitation',async()=>{
 let resolveEngine;const engine=new Promise(resolve=>{resolveEngine=resolve;});let claims=0;
 const t=setup({launchCode:'MC1-EXAMPLE',engine:()=>engine});
 const pending=t.ui.beginMesh('MC1-EXAMPLE','',false);
 const cancelled=assert.rejects(pending,/page or account changed/);
 t.events.pagehide();
 resolveEngine({webMeshBegin:async()=>{claims++;return JSON.stringify({kind:'pairing'});}});
 await cancelled;
 assert.equal(claims,0);assert.equal(t.requests.length,0);
 assert.equal(t.ui.state.code,'');assert.equal(t.ui.state.capability,null);
 assert.equal(t.ui.state.session,null);
});
test('a late capability rejection cannot reopen its dialog after page retirement',async()=>{
 let rejectEngine;const engine=new Promise((_resolve,reject)=>{rejectEngine=reject;});
 const t=setup({launchCode:'MC1-EXAMPLE',engine:()=>engine});
 const pending=t.ui.beginMesh('MC1-EXAMPLE','',false);
 const rejected=assert.rejects(pending,/Late engine failure/);
 t.events.pagehide();rejectEngine(browserFailure('Late engine failure'));await rejected;
 assert.equal(t.ui.state.capability,null);assert.equal(t.ui.state.code,'');
 assert.equal(t.requests.length,0);
});
test('a Begin reply arriving after page retirement cannot restore the cleared pairing view',async()=>{
 let resolveBegin,started;const startedPromise=new Promise(resolve=>{started=resolve;});
 const reply=new Promise(resolve=>{resolveBegin=resolve;});
 const t=setup({engine:async()=>({webMeshBegin:()=>{started();return reply;}})});
 const pending=t.ui.beginMesh('MC1-EXAMPLE','',false);
 const cancelled=assert.rejects(pending,/page or account changed/);
 await startedPromise;t.events.pagehide();
 resolveBegin(JSON.stringify({kind:'pairing',needsBeginRetry:false}));await cancelled;
 assert.equal(t.ui.state.pairing,null);assert.equal(t.ui.state.session,null);
 assert.equal(t.ui.state.code,'');assert.equal(t.requests.length,0);
});

function passkeyContext() {
 const calls=[],credential={id:'synthetic-credential',type:'public-key',response:{clientDataJSON:'e30'}};
 const context=vm.createContext({crypto:webcrypto,TextEncoder,TextDecoder,Uint8Array,ArrayBuffer,URL,
  atob:value=>Buffer.from(value,'base64').toString('binary'),
  btoa:value=>Buffer.from(value,'binary').toString('base64'),
  location:new URL('https://bluefin.test/web/bluefin/'),isSecureContext:true,
  PublicKeyCredential:class{},
  navigator:{credentials:{
   get:async options=>{calls.push({method:'get',options});return {toJSON:()=>credential};},
   create:async options=>{calls.push({method:'create',options});return {toJSON:()=>credential};}
  }},document:{addEventListener(){}},addEventListener(){},dispatchEvent(){},CustomEvent:class{},
  setTimeout,clearTimeout});
 context.window=context;
 vm.runInContext(source('mesh-platform.js').replace(/\bexport /g,'')+
  '\nglobalThis.passkeyAdapter=meshPasskey;',context);
 return {context,calls,credential};
}
test('same-origin Login accepts the server assertion kind and requests an existing passkey',async()=>{
 const t=passkeyContext();
 // Exact BeginResult passkey_options shape emitted by bluefin_mesh_accounts.
 const options={kind:'assertion',rpId:'bluefin.test',origin:'https://bluefin.test',
  publicKey:{challenge:'AQIDBA',timeout:60000,rpId:'bluefin.test',
   allowCredentials:[{type:'public-key',id:'BQYHCA'}],userVerification:'required'},
  browserUrl:'https://bluefin.test/Bluefin/Accounts/Mesh/Passkey#'+'a'.repeat(64)+'.'+'b'.repeat(64)};
 assert.deepEqual(JSON.parse(await t.context.passkeyAdapter(JSON.stringify(options))),t.credential);
 assert.equal(t.calls.length,1);assert.equal(t.calls[0].method,'get');
 const publicKey=t.calls[0].options.publicKey;
 assert.deepEqual(Array.from(publicKey.challenge),[1,2,3,4]);
 assert.deepEqual(Array.from(publicKey.allowCredentials[0].id),[5,6,7,8]);
 assert.equal(publicKey.rpId,'bluefin.test');assert.equal(publicKey.userVerification,'required');
});
test('same-origin registration uses creation options and decodes the server user and exclusion IDs',async()=>{
 const t=passkeyContext();
 const options={kind:'registration',rpId:'bluefin.test',origin:'https://bluefin.test',
  publicKey:{challenge:'AQIDBA',timeout:60000,rp:{id:'bluefin.test',name:'Bluefin'},
   user:{id:'BQYHCA',name:'Listener',displayName:'Listener'},
   pubKeyCredParams:[{type:'public-key',alg:-7}],
   authenticatorSelection:{residentKey:'required',userVerification:'required'},
   excludeCredentials:[{type:'public-key',id:'CQoL'}]},
  browserUrl:'https://bluefin.test/Bluefin/Accounts/Mesh/Passkey#'+'a'.repeat(64)+'.'+'b'.repeat(64)};
 await t.context.passkeyAdapter(JSON.stringify(options));
 assert.equal(t.calls.length,1);assert.equal(t.calls[0].method,'create');
 const publicKey=t.calls[0].options.publicKey;
 assert.deepEqual(Array.from(publicKey.challenge),[1,2,3,4]);
 assert.deepEqual(Array.from(publicKey.user.id),[5,6,7,8]);
 assert.deepEqual(Array.from(publicKey.excludeCredentials[0].id),[9,10,11]);
 assert.equal(publicKey.rp.id,'bluefin.test');
 assert.equal(publicKey.authenticatorSelection.userVerification,'required');
});
test('the passkey adapter rejects wrong-origin requests and unsupported kind aliases before WebAuthn',async()=>{
 const t=passkeyContext();
 for(const options of [
  {kind:'authentication',origin:'https://bluefin.test',publicKey:{}},
  {kind:'unknown',origin:'https://bluefin.test',publicKey:{}},
  {kind:'assertion',origin:'https://other.test',publicKey:{}},
  {kind:'registration',origin:'https://other.test',publicKey:{}}
 ])await assert.rejects(t.context.passkeyAdapter(JSON.stringify(options)),/paired server passkey page/);
 assert.equal(t.calls.length,0);
});
