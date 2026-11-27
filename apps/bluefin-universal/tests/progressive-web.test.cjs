// Executes the shipped progressive view with unavailable Wasm, IndexedDB and
// workers. Browser decoding/visual parity remain separate device checks.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {webcrypto}=require('node:crypto');
const {File}=require('node:buffer');
const source=name=>fs.readFileSync(path.join(__dirname,'..',name),'utf8');
class Element{
 constructor(tag){this.tagName=tag.toUpperCase();this.children=[];this.dataset={};this.style={setProperty(name,value){this[name]=String(value);},getPropertyValue(name){return this[name]||'';}};this.attributes={};this.listeners={};this.value='';this.text='';this.className='';this.classList={add(){},remove(){},toggle(){}};}
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
function setup({meshFailure=false,leaseOrigin='',holdLogin=false,holdLease=false,cookieStatus=200,cookieMalformed=false,cookieCreateStatus=200,tvAdvertisement=true,nativeHls=true,audioPlayable=true,launchCode='',engine,apiResponse}={}){
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
  const custom=await apiResponse?.(u,options);if(custom!==undefined)return custom;
  let body;
  if(u.pathname==='/Bluefin/Web/v1'&&cookieStatus!==200)return new Response('{}',{status:cookieStatus});
  if(u.pathname==='/Bluefin/Web/v1/leases'&&cookieCreateStatus!==200)return new Response('{}',{status:cookieCreateStatus});
  if(u.pathname==='/Users/AuthenticateByName'&&holdLogin)await new Promise(resolve=>pending.login=resolve);
  if(u.pathname==='/Bluefin/Web/v1/leases'&&options.method==='POST'&&holdLease)await new Promise(resolve=>pending.lease=resolve);
  if(u.pathname==='/System/Info/Public')body={ServerName:'Bluefin'};
  else if(u.pathname==='/Bluefin/Accounts/Options')body={passkeys:true,quickConnect:true};
  else if(u.pathname==='/Users/AuthenticateByName')body={AccessToken:'synthetic-private-bearer',User:{Id:user,Name:'Listener'}};
  else if(u.pathname==='/Bluefin/Home/Recommendations')return new Response('{}',{status:404});
  else if(u.pathname==='/Movies/Recommendations')body=[];
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
 vm.runInContext(source('browse-window.js'),context);
 const script=source('native-view.js').replace(/\}\)\(\);\s*$/,'globalThis.contract={loadLyrics,pumpArtwork,clearImages,state,connect,signIn,navigate,openItem,beginMesh,play,signOut,stopPlayback,render,nativeLeasePath,tvLeasePath,leaseRequest,issueInvite,openLocalMusic,addLocalFiles,localItems,localOwner,loadRoute,reportProgress,playbackTimeline,seekPlayback,toggleShuffle,playShuffled,startSearch,openSearch,searchCall,loadMoreSearch,loadSearchFacets,recordHomeSearch,clearHomeSearchHistory,importInvitationImage:readInvitationImage};})();');
 vm.runInContext(script,context);
 return {context,ui:context.contract,root,video,requests,events,item,pending};
}
test('ordinary HTML controls and HTTPS library work without Wasm, storage, or workers',async()=>{
 const t=setup();assert.match(t.root.textContent,/Connect with QR and passkey/);assert.equal(t.requests.length,0);
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
 assert.equal(t.ui.state.code,'');assert.equal(t.ui.state.capability,null);assert.equal(t.ui.state.session,null);
});
test('a late capability rejection cannot reopen its dialog after pagehide',async()=>{
 const t=setup({launchCode:'MC1-EXAMPLE'});let rejectEngine;
 t.context.BluefinRuntime.engine=()=>new Promise((_resolve,reject)=>{rejectEngine=reject;});
 const attempt=t.ui.beginMesh('MC1-EXAMPLE','',false);const result=assert.rejects(attempt,/Late engine failure/);
 await until(()=>rejectEngine);t.events.pagehide();rejectEngine(browserFailure('Late engine failure'));
 await result;assert.equal(t.ui.state.capability,null);assert.equal(t.ui.state.code,'');assert.equal(t.requests.length,0);
});
test('a Begin reply arriving after pagehide cannot restore the cleared pairing view',async()=>{
 const t=setup({launchCode:'MC1-EXAMPLE'});let resolveBegin;
 t.context.BluefinRuntime.engine=async()=>({webMeshBegin:()=>new Promise(resolve=>{resolveBegin=resolve;})});
 const attempt=t.ui.beginMesh('MC1-EXAMPLE','',false);const result=assert.rejects(attempt,/page or account changed/);
 await until(()=>resolveBegin);t.events.pagehide();
 resolveBegin(JSON.stringify({kind:'pairing',needsBeginRetry:false}));await result;
 assert.equal(t.ui.state.pairing,null);assert.equal(t.ui.state.session,null);
 assert.equal(t.ui.state.code,'');assert.equal(t.requests.length,0);
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
test('opening an invitation exposes installation without sending or persisting its code',()=>{
 const code='MC1-EXAMPLE',t=setup({launchCode:code});
 assert.equal(t.ui.state.sheet,'invite');
 assert.equal(t.ui.state.code,code);
 const sheet=findElement(t.root,e=>e.attributes.role==='dialog');
 assert(sheet);
 assert(!sheet.textContent.includes('Your account name'));
 const install=findElement(sheet,e=>e.tagName==='A'&&e.textContent==='Install Bluefin');
 assert.equal(install?.href,'https://bluefin.test/web/bluefin/install.html');
 assert.equal(install.target,'_blank');
 assert.equal(install.rel,'noopener noreferrer');
 assert.equal(t.requests.length,0);
});
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
test('capability guidance warns against reloading after an earlier Begin reached Rust',async()=>{
 const t=setup({engine:async()=>({webMeshBegin:async()=>{throw Error('Connection interrupted');}})});
 await assert.rejects(t.ui.beginMesh('MC1-EXAMPLE','',false),/Connection interrupted/);
 t.context.BluefinRuntime.engine=async()=>{throw browserFailure('Optional engine blocked');};
 await assert.rejects(t.ui.beginMesh('MC1-EXAMPLE','',false),/Optional engine blocked/);
 assert.equal(t.ui.state.capability.beforeBegin,false);assert.equal(t.requests.length,0);
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

const jsonReply=(value,status=200)=>new Response(JSON.stringify(value),{status,headers:{'Content-Type':'application/json'}});
test('moonlit six primary categories keep utilities outside the category bar',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);t.ui.render();
 const nav=findElement(t.root,e=>e.tagName==='NAV');
 assert.deepEqual(nav.children.filter(e=>e.tagName==='BUTTON').map(e=>e.textContent.trim()),
  ['Home','Sports','TV Shows','Movies','Live TV','Music']);
 await t.ui.navigate('Movies');
 assert(t.requests.some(r=>new URL(r.url).searchParams.get('IncludeItemTypes')==='Movie'));
 await t.ui.navigate('TV Shows');
 assert(t.requests.some(r=>new URL(r.url).searchParams.get('IncludeItemTypes')==='Series'));
 await t.ui.openSearch();assert.equal(t.ui.state.route.filters.scope,'all');assert.equal(t.ui.state.tab,'TV Shows');
 t.ui.render();assert(findElement(t.root,e=>e.attributes['aria-label']==='Collections'));
});
test('moonlit refined search sends exact server filters again after duplicate rows',async()=>{
 const studio='44444444444444448444444444444444',collection='55555555555545558555555555555555';
 const a={Id:'66666666666646668666666666666666',Type:'Movie',Name:'First title'},b={Id:'77777777777747778777777777777777',Type:'Movie',Name:'Next title'};
 const t=setup({apiResponse:(u)=>{
  if(u.searchParams.get('SearchTerm')==='ocean'&&u.pathname.endsWith('/Items')){
   const start=Number(u.searchParams.get('StartIndex')||0);
   return jsonReply({Items:start===0?[a,a]:start===2?[b]:[],TotalRecordCount:3});
  }
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.startSearch('ocean',{scope:'movies',studioID:studio,tag:'Service: Acme+',collectionID:collection});
 assert.deepEqual(Array.from(t.ui.state.search.pages[0].items,x=>x.Id),[a.Id]);
 await t.ui.loadMoreSearch('library');
 assert.deepEqual(Array.from(t.ui.state.search.pages[0].items,x=>x.Id),[a.Id,b.Id]);
 const searched=t.requests.filter(r=>new URL(r.url).searchParams.get('SearchTerm')==='ocean');
 assert.equal(searched.length,2);
 for(const r of searched){const q=new URL(r.url).searchParams;assert.equal(q.get('StudioIds'),studio);assert.equal(q.get('Tags'),'Service: Acme+');assert.equal(q.get('ParentId'),collection);assert.equal(q.get('IncludeItemTypes'),'Movie');}
 assert.equal(new URL(searched[1].url).searchParams.get('StartIndex'),'2');
});
test('moonlit home uses genuine recommendation rows and only 404 selects legacy fallback',async()=>{
 const title={Id:'66666666666646668666666666666666',Type:'Movie',Name:'From this account'};
 for(const status of [200,404,403]){
  const t=setup({apiResponse:u=>{
   if(u.pathname==='/Bluefin/Home/Recommendations')return jsonReply(status===200?{recommendations:[{id:'liked:one',title:'Because you like a favorite',reason:'liked',items:[title]}],searchHistorySupported:true,searchHistoryGeneration:0}:{},status);
   if(u.pathname==='/Movies/Recommendations')return jsonReply([{RecommendationType:'SimilarToLikedItem',BaselineItemName:'Favorite movie',Items:[title]}]);
  }});
  await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
  const legacy=t.requests.filter(r=>new URL(r.url).pathname==='/Movies/Recommendations');
  assert.equal(legacy.length,status===404?1:0);
  assert.equal(t.ui.state.shelves.some(s=>s.items.some(i=>i.Id===title.Id)),status!==403);
  if(status===200){assert.equal(t.ui.state.featured.Id,title.Id);assert.equal(t.ui.state.homeHistory.generation,0);}
 }
});
test('moonlit search cancellation discards a delayed former route response',async()=>{
 let finish,heldSignal;
 const t=setup({apiResponse:async(u,o)=>{
  if(u.searchParams.get('SearchTerm')==='old'&&u.pathname.endsWith('/Items')){
   heldSignal=o.signal;await new Promise(r=>finish=r);
   return jsonReply({Items:[{Id:'66666666666646668666666666666666',Type:'Movie',Name:'Private old result'}],TotalRecordCount:1});
  }
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const pending=t.ui.startSearch('old',{scope:'movies'});await until(()=>finish);
 const next=t.ui.navigate('Music');assert.equal(heldSignal.aborted,true);
 finish();await Promise.all([pending,next]);t.ui.render();
 assert.equal(t.ui.state.route.kind,'music');assert(!t.root.textContent.includes('Private old result'));
});
test('moonlit all media includes server artists channels and programmes',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.startSearch('match',{scope:'all'});
 const searched=t.requests.filter(r=>new URL(r.url).searchParams.get('SearchTerm')==='match');
 assert(searched.some(r=>new URL(r.url).pathname==='/Artists'));
 assert(searched.some(r=>new URL(r.url).pathname==='/LiveTv/Channels'));
 assert(searched.some(r=>new URL(r.url).pathname==='/LiveTv/Programs'));
 assert.equal(t.requests.filter(r=>new URL(r.url).pathname==='/Bluefin/Home/Searches').length,0);
});

test('search publishes the playable library result while the guide is still loading',async()=>{
 const movie={Id:'66666666666646668666666666666666',Type:'Movie',Name:'Ocean Crossing'};
 let releaseGuide;
 const t=setup({apiResponse:async u=>{
  if(u.searchParams.get('SearchTerm')!=='ocean')return;
  if(u.pathname==='/LiveTv/Programs')await new Promise(resolve=>releaseGuide=resolve);
  return jsonReply({Items:u.pathname.endsWith('/Items')?[movie]:[],TotalRecordCount:u.pathname.endsWith('/Items')?1:0});
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const search=t.ui.startSearch('ocean');
 try{
  const playResult=()=>findElement(t.root,e=>e.tagName==='BUTTON'&&e.attributes['aria-label']==='Play Ocean Crossing');
  await until(()=>releaseGuide&&playResult());
  assert.equal(t.ui.state.route.loading,true);
  assert.equal(findElement(t.root,e=>e.id==='bluefin-scope-all').attributes['aria-pressed'],'true');
  await playResult().listeners.click();
  const open=t.requests.find(r=>new URL(r.url).pathname==='/Bluefin/Web/v1/leases'&&r.options.method==='POST');
  assert.equal(JSON.parse(open.options.body).itemId,movie.Id);
  assert.equal(t.video.playCalls,1);
  assert.equal(t.ui.state.route.kind,'search');
 }finally{releaseGuide?.();await search;}
});

test('sports search opens the scheduled programme without tuning to a different broadcast',async()=>{
 const programme={Id:'77777777777747778777777777777777',Type:'LiveTvProgram',Name:'Cup Final',
  StartDate:'2099-01-01T20:00:00Z',EndDate:'2099-01-01T22:00:00Z',
  ChannelId:'66666666666646668666666666666666',ChannelName:'Arena TV'};
 const t=setup({apiResponse:u=>u.pathname==='/LiveTv/Programs'&&u.searchParams.get('IsSports')==='true'
  ?jsonReply({Items:[programme],TotalRecordCount:1}):undefined});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.openSearch();
 const input=findElement(t.root,e=>e.id==='bluefin-search');
 input.value='cup';input.oninput();
 await findElement(t.root,e=>e.id==='bluefin-scope-sports').listeners.click();
 const requests=t.requests.filter(r=>new URL(r.url).searchParams.get('SearchTerm')==='cup');
 assert.equal(requests.length,1);
 assert.equal(new URL(requests[0].url).searchParams.get('IsSports'),'true');
 const result=findElement(t.root,e=>e.dataset.focusKey==='guide-result|'+programme.Id);
 await result.listeners.click();
 assert.equal(t.ui.state.route.kind,'programme');
 assert.match(t.root.textContent,/Cup Final.+Arena TV/);
 assert(!t.root.textContent.includes('Watch live channel'));
 assert(!t.requests.some(r=>new URL(r.url).pathname.endsWith('/Items/'+programme.ChannelId)));
 assert(!t.requests.some(r=>new URL(r.url).pathname.endsWith('/leases')));
});

test('default public search discovers TV, movies, music, credited people, and dated sports without account credentials',async()=>{
 const statement=value=>({mainsnak:{datavalue:{value}}});
 const entity=(label,kind,extra={})=>({labels:{en:{value:label}},claims:{P31:[statement({id:kind})],...extra}});
 const t=setup({apiResponse:url=>{
  if(url.href.startsWith('https://api.tvmaze.com/search/shows'))return jsonReply([
   {show:{id:47,name:'Screen Fixture',url:'https://unrelated.test/title',premiered:'2025-04-01',
    image:{medium:'https://static.tvmaze.com/uploads/images/medium_portrait/2/47.png'},
    officialSite:'https://www.netflix.com/title/80057281'}},
   {show:{id:48,name:'No Poster',image:{medium:'https://unrelated.test/image.jpg'}}},
  ]);
  if(url.href.startsWith('https://api.tvmaze.com/search/people'))return jsonReply([{person:{id:73,name:'TV Crew Fixture'}}]);
  if(url.hostname==='www.wikidata.org'){
   if(url.searchParams.get('action')==='wbsearchentities')return jsonReply({search:[...['Q1','Q2','Q3','Q4','Q5'].map(id=>({id})),{id:'https://unrelated.test/'}]});
   assert.equal(url.searchParams.get('ids'),'Q1|Q2|Q3|Q4|Q5');
   return jsonReply({entities:{
    Q1:entity('Film Fixture','Q20650540'),Q2:entity('Album Fixture','Q482994'),
    Q3:entity('Person Fixture','Q5',{P106:['Q33999','Q2526255','Q28389'].map(id=>statement({id}))}),
    Q4:entity('Upcoming Cup Fixture','Q16510064',{P585:[statement({time:'+2030-04-02T00:00:00Z'})]}),
    Q5:entity('Not media','Q42'),
   }});
  }
 }});
 await findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Search media')).listeners.click();
 await t.ui.startSearch('fixture');
 assert.equal(t.requests.length,4);
 assert.equal(t.ui.state.session,null);
 assert.match(t.root.textContent,/Film Fixture.*Album Fixture.*Person Fixture.*Actor, Film director, Screenwriter.*Upcoming Cup Fixture.*2030-04-02/);
 assert.match(t.root.textContent,/TV Crew Fixture/);
 assert(!t.root.textContent.includes('Not media'));
 assert(findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Screen Fixture')));
 assert(findElement(t.root,e=>e.src==='https://static.tvmaze.com/uploads/images/medium_portrait/2/47.png'));
 assert.equal(findElement(t.root,e=>e.tagName==='A'&&e.textContent==='Open on Netflix').href,'https://www.netflix.com/title/80057281');
 assert(!findElement(t.root,e=>String(e.href).includes('/shows/47')));
 assert(!findElement(t.root,e=>String(e.src).includes('unrelated.test')));
 assert(t.requests.every(r=>r.options.headers===undefined&&r.options.credentials==='omit'&&r.options.referrerPolicy==='no-referrer'));
});

test('public film search uses a bounded close-match lookup when the exact catalog misses',async()=>{
 const t=setup({apiResponse:url=>{
  if(url.hostname==='api.tvmaze.com')return jsonReply([]);
  if(url.hostname!=='www.wikidata.org')return;
  const action=url.searchParams.get('action');
  if(action==='wbsearchentities')return jsonReply({search:[]});
  if(action==='query'){
   if(url.searchParams.get('srsearch').endsWith(' haswbstatement:P31=Q11424'))return jsonReply({query:{search:[]}});
   assert.equal(url.searchParams.get('srsearch'),'spirted~1 away~1');
   assert.equal(url.searchParams.get('srlimit'),'16');
   return jsonReply({query:{search:[{title:'Q155653'},{title:'https://unrelated.test/'}]}});
  }
  assert.equal(url.searchParams.get('ids'),'Q155653');
  return jsonReply({entities:{Q155653:{labels:{en:{value:'Spirited Away'}},claims:{P31:[
   {mainsnak:{datavalue:{value:{id:'Q20650540'}}}}
  ]}}}});
 }});
 await findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Search media')).listeners.click();
 await t.ui.startSearch('Spirted Away',{scope:'movies'});
 assert.match(t.root.textContent,/Spirited Away.*Anime film/);
 assert.equal(t.requests.length,4);
 assert(t.requests.every(r=>r.options.credentials==='omit'));
});

test('broad account search includes connected catalog and a close upcoming sports match, with a private library scope',async()=>{
 const t=setup({apiResponse:url=>{
  if(url.hostname==='api.tvmaze.com')return jsonReply([]);
  if(url.hostname==='www.wikidata.org')return jsonReply(url.searchParams.get('action')==='query'?{query:{search:[]}}:{search:[]});
  if(url.pathname==='/Bluefin/Web/Discovery/Search')return jsonReply({results:[url.searchParams.get('scope')==='music'
   ?{title:'Track from connected catalog',kind:'track'}:{title:'Film from connected catalog',kind:'movie'}]});
  if(url.pathname==='/sports/api/v1/home')return jsonReply({schema:1,presentation_only:true,local_broadcasts:[],
   games:[{event:{name:'Boston Celtics at Toronto',date:'2030-04-02T19:30:00Z',
    competitors:[{name:'Boston Celtics'},{name:'Toronto Raptors'}]}}]});
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.startSearch('celitcs');
 assert.match(t.root.textContent,/Boston Celtics at Toronto/);
 assert.match(t.root.textContent,/Film from connected catalog/);
 assert.match(t.root.textContent,/Track from connected catalog/);
 assert(t.requests.some(r=>new URL(r.url).pathname==='/Persons'&&new URL(r.url).searchParams.get('PersonTypes')==='Actor,Director,Writer,GuestStar'));
 const before=t.requests.length;
 await t.ui.startSearch('private title',{scope:'library'});
 assert(t.requests.slice(before).every(r=>new URL(r.url).hostname==='bluefin.test'&&!new URL(r.url).pathname.includes('/Discovery/Search')));
});

const claim=(value,rank='normal')=>({rank,mainsnak:{snaktype:'value',datavalue:{value}}});
test('a film result displays a licensed Commons poster and exact watch links before opening details',async()=>{
 const t=setup({apiResponse:u=>{
  if(u.hostname==='commons.wikimedia.org'){
   assert.equal(u.searchParams.get('titles'),'File:Film poster.png');
   return jsonReply({query:{pages:[{title:'File:Film poster.png',imageinfo:[{
    thumburl:'https://thumb.wikimedia.org/wikipedia/commons/thumb/0/00/Film_poster.png/250px-Film_poster.png?utm_source=commons.wikimedia.org',
    descriptionurl:'https://commons.wikimedia.org/wiki/File:Film_poster.png',
    extmetadata:{Artist:{value:'<a>Poster Artist</a>'},LicenseShortName:{value:'CC BY-SA 4.0'},
     LicenseUrl:{value:'https://creativecommons.org/licenses/by-sa/4.0/'}}
   }]}]}});
  }
  if(u.hostname!=='www.wikidata.org')return;
  if(u.searchParams.get('action')==='query')return jsonReply({query:{search:[{title:'Q184843'}]}});
  return jsonReply({entities:{Q184843:{labels:{en:{value:'Film Fixture'}},claims:{
   P3383:[claim('Film poster.png')],P18:[claim('Secondary artwork.jpg')],
   P1874:[claim('80112233'),claim('80112234')],P856:[claim('https://www.paramountplus.com/movies/fixture/'),
    claim('https://www.netflix.com/title/80112233/redirect'),claim('https://www.netflix.com')]
  }}}});
 }});
 await findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Search media')).listeners.click();
 await t.ui.startSearch('Film Fixture',{scope:'movies'});
 assert(findElement(t.root,e=>e.src==='https://thumb.wikimedia.org/wikipedia/commons/thumb/0/00/Film_poster.png/250px-Film_poster.png'));
 assert.equal(findElement(t.root,e=>e.tagName==='A'&&e.textContent==='Poster Artist').href,'https://commons.wikimedia.org/wiki/File:Film_poster.png');
 for(const href of ['https://www.netflix.com/watch/80112233','https://www.netflix.com/watch/80112234',
  'https://www.paramountplus.com/movies/fixture/'])assert(findElement(t.root,e=>e.href===href));
 assert(!findElement(t.root,e=>String(e.href).endsWith('/redirect')));
});
test('music scope includes solo musicians and keeps album, track and artist destinations distinct',async()=>{
 const ids=['Q100','Q200','Q300'];
 const t=setup({apiResponse:u=>{
  if(u.hostname!=='www.wikidata.org')return;
  if(u.searchParams.get('action')==='wbsearchentities')return jsonReply({search:ids.map(id=>({id}))});
  const make=(name,kind,extra)=>({labels:{en:{value:name}},claims:{P31:[claim({id:kind})],...extra}});
  return jsonReply({entities:{
   Q100:make('Solo Fixture','Q5',{P106:[claim({id:'Q177220'})],P1902:[claim('A'.repeat(22))]}),
   Q200:make('Album Fixture','Q482994',{P2205:[claim('B'.repeat(22))]}),
   Q300:make('Song Fixture','Q7366',{P2207:[claim('C'.repeat(22))]}),
  }});
 }});
 await t.ui.openSearch();await t.ui.startSearch('audio',{scope:'music'});
 for(const [id,name] of [['Q100','Solo Fixture'],['Q200','Album Fixture'],['Q300','Song Fixture']]){
  const result=findElement(t.root,e=>e.dataset.focusKey==='public-title|music|'+id);
  assert(result?.textContent.includes(name));
 }
 for(const [kind,id] of [['artist','A'],['album','B'],['track','C']])assert(findElement(t.root,e=>e.href==='https://open.spotify.com/'+kind+'/'+id.repeat(22)));
});
test('film selection stays in Bluefin and offers a direct player or exact provider title',async()=>{
 const t=setup({apiResponse:u=>{
  if(u.hostname!=='www.wikidata.org')return undefined;
  if(u.searchParams.get('action')==='wbsearchentities')return jsonReply({search:[]});
  if(u.searchParams.get('action')==='query')return jsonReply({query:{search:[
   {title:'Q184843'},{title:'Q184843'},{title:'File:Unrelated'}]}});
  return jsonReply({entities:{Q184843:{labels:{en:{value:'Blade Runner'}},
   descriptions:{en:{value:'1982 film directed by Ridley Scott'}},claims:{
   P1874:[claim('80234304')],
   P6466:[claim('e47078f3-1c0e-49a8-9da9-c571a7a20fec')],
   P13902:[claim('entity-f6174ebf-cb92-453c-a52b-62bb3576e402')],
   P11815:[claim('movies/psych-3-this-is-gus'),claim('tv/unrelated')],
   P13147:[claim('WwH_xKtpmHlHq2_4qrlBjfFDKIsgeI0e','deprecated')],
   P8298:[claim('movie/../../unrelated.test')],
   P9586:[claim('unused')]
  }}}});
 }});
 await findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Search media')).listeners.click();
 await t.ui.startSearch('Blade Runner',{scope:'movies'});
 assert.equal(t.ui.state.session,null);
 const selection=findElement(t.root,e=>e.dataset.focusKey==='public-title|movie|Q184843');
 assert(selection);
 await selection.listeners.click();
 const lookups=t.requests.filter(r=>new URL(r.url).hostname==='www.wikidata.org');
 assert.equal(t.ui.state.route.kind,'public-detail');
 assert.match(t.root.textContent,/1982 film directed by Ridley Scott.*Where to watch/);
 assert(lookups.some(r=>new URL(r.url).searchParams.get('props')==='labels|descriptions|claims'));
 assert(lookups.every(r=>r.options.credentials==='omit'&&r.options.referrerPolicy==='no-referrer'));
 const link=label=>findElement(t.root,e=>e.tagName==='A'&&e.textContent===label)?.href;
 assert.equal(link('Watch on Netflix'),'https://www.netflix.com/watch/80234304');
 assert.equal(link('Open on Disney+'),'https://www.disneyplus.com/browse/entity-f6174ebf-cb92-453c-a52b-62bb3576e402');
 assert.equal(link('Open on Hulu'),'https://www.hulu.com/movie/e47078f3-1c0e-49a8-9da9-c571a7a20fec');
 assert.equal(link('Open on Peacock'),'https://www.peacocktv.com/stream-movies/psych-3-this-is-gus');
 assert(!findElement(t.root,e=>String(e.href).includes('unrelated.test')||String(e.href).includes('apple.com')));
});

test('series lookup resolves an exact TVmaze identity and goes straight to the provider title',async()=>{
 const t=setup({apiResponse:u=>{
  if(u.hostname==='api.tvmaze.com'){
   if(u.pathname==='/shows/47')return jsonReply({id:47,summary:'<p>The series premise</p>',_embedded:{cast:[
    {person:{id:19,name:'Sample Actor'},character:{name:'Lead Character'}}]}});
   if(u.pathname.startsWith('/people/19/'))return jsonReply([{type:'Director',_embedded:{show:{id:47,name:'Example series'}}}]);
   return jsonReply([{show:{id:47,name:'Example series'}}]);
  }
  if(u.hostname==='www.wikidata.org'){
   if(u.searchParams.get('action')==='wbsearchentities')return jsonReply({search:[]});
   if(u.searchParams.get('action')==='query'){
    if(u.searchParams.get('srsearch')!=='haswbstatement:P8600=47')return jsonReply({query:{search:[]}});
    return jsonReply({query:{search:[{title:'Q1234'}]}});
   }
   return jsonReply({entities:{Q1234:{claims:{P1874:[claim('80057281')],P6467:[claim('e47078f3-1c0e-49a8-9da9-c571a7a20fec')]}}}});
  }
 }});
 await findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Search media')).listeners.click();
 await t.ui.startSearch('example',{scope:'tvShows'});
 await findElement(t.root,e=>e.dataset.focusKey==='public-title|series|47').listeners.click();
 assert.equal(t.ui.state.route.kind,'public-detail');
 assert.match(t.root.textContent,/The series premise.*Where to watch.*Credits and related media.*Sample Actor.*Lead Character/);
 assert.equal(findElement(t.root,e=>e.tagName==='A'&&e.textContent==='Open on Netflix').href,'https://www.netflix.com/title/80057281');
 assert.equal(findElement(t.root,e=>e.tagName==='A'&&e.textContent==='Open on Hulu').href,
  'https://www.hulu.com/series/e47078f3-1c0e-49a8-9da9-c571a7a20fec');
 assert(!findElement(t.root,e=>String(e.href).includes('/watch/')));
 await findElement(t.root,e=>e.tagName==='BUTTON'&&e.textContent.includes('Sample Actor')).listeners.click();
 assert.equal(t.ui.state.route.title,'Sample Actor');
 assert.match(t.root.textContent,/Credits.*Example series.*Director/);
 await findElement(t.root,e=>e.dataset.focusKey==='back|Back').listeners.click();
 assert.equal(t.ui.state.route.title,'Example series');
});

test('public film offers in-app playback only for the exact authorized library title',async()=>{
 const t=setup({apiResponse:u=>{
  if(u.hostname==='www.wikidata.org'){
   if(u.searchParams.get('action')==='wbsearchentities')return jsonReply({search:[]});
  if(u.searchParams.get('action')==='query')return jsonReply({query:{search:[{title:'Q184843'}]}});
   return jsonReply({entities:{Q184843:{labels:{en:{value:'Blade Runner'}},claims:{P345:[claim('tt0083658')]}}}});
  }
  if(u.pathname.startsWith('/Users/')&&u.pathname.endsWith('/Items')&&u.searchParams.has('SearchTerm')){
   return jsonReply({Items:[
    {Id:'66666666666646668666666666666666',Name:'Blade Runner',Type:'Movie',ProviderIds:{Imdb:'tt0083658'}},
    {Id:'77777777777747778777777777777777',Name:'Blade Runner',Type:'Movie',ProviderIds:{Imdb:'tt1856101'}}
   ],TotalRecordCount:2});
  }
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.startSearch('Blade Runner',{scope:'movies'});
 await findElement(t.root,e=>e.dataset.focusKey==='public-title|movie|Q184843').listeners.click();
 assert(findElement(t.root,e=>e.dataset.focusKey==='film-play|66666666666646668666666666666666'));
 assert(!findElement(t.root,e=>e.dataset.focusKey==='film-play|77777777777747778777777777777777'));
 const search=t.requests.find(r=>new URL(r.url).searchParams.has('SearchTerm'));
 assert(new URL(search.url).searchParams.get('Fields').includes('ProviderIds'));
});

test('moonlit explicit search history uses the current account revision and survives a stale write rejection',async()=>{
 let rejectOld;
 const t=setup({apiResponse:async(u,o)=>{
  if(u.pathname==='/Bluefin/Home/Recommendations')return jsonReply({recommendations:[],searchHistorySupported:true,searchHistoryGeneration:3});
  if(u.pathname==='/Bluefin/Home/Searches'){
   if(o.method==='DELETE')return jsonReply({generation:4});
   await new Promise(resolve=>rejectOld=resolve);return jsonReply({},409);
  }
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const pending=t.ui.recordHomeSearch('ocean');await until(()=>rejectOld);
 const write=t.requests.find(r=>new URL(r.url).pathname==='/Bluefin/Home/Searches'&&r.options.method==='POST');
 const payload=JSON.parse(write.options.body);
 assert.equal(payload.query,'ocean');assert.equal(payload.generation,3);assert(!('userId' in payload));assert(payload.eventId);
 await t.ui.clearHomeSearchHistory();rejectOld();await pending;
 assert.equal(t.ui.state.homeHistory.generation,4);
});

test('artist results browse credited media and preserve the credit filter when paging',async()=>{
 const artist='77777777777747778777777777777777',t=setup({apiResponse:u=>u.searchParams.get('ArtistIds')===artist?jsonReply({Items:[{Id:'66666666666646668666666666666666',Type:'MusicAlbum',Name:'Credited album'}],TotalRecordCount:150}):undefined});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.openItem({Id:artist,Name:'Account artist',Type:'MusicArtist'});
 await t.ui.state.route.window.request(100);
 const requests=t.requests.map(request=>new URL(request.url)).filter(url=>url.searchParams.get('ArtistIds')===artist);
 assert.equal(requests.length,2);
 assert.equal(requests[0].searchParams.get('StartIndex'),'0');assert.equal(requests[1].searchParams.get('StartIndex'),'100');
 for(const url of requests){assert.equal(url.searchParams.get('IncludeItemTypes'),'MusicAlbum,Audio,MusicVideo');assert.equal(url.searchParams.has('ParentId'),false);}
});

test('artwork cache rotates offscreen entries instead of stopping permanently at 64 images',async()=>{
 const bytes=Uint8Array.from([255,216,255,217]),wanted='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
 const t=setup({apiResponse:u=>u.pathname.includes('/Images/')?new Response(bytes,{headers:{'Content-Type':'image/jpeg'}}):undefined});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 for(let i=1;i<=64;i++)t.ui.state.images.set('p:'+i.toString(16).padStart(32,'0'),URL.createObjectURL(new Blob([bytes])));
 const target=new Element('div');target.dataset.artKey='p:'+wanted;target.dataset.artId=wanted;
 t.context.document.querySelectorAll=selector=>selector==='[data-art-key]'?[target]:[];
 t.ui.pumpArtwork();await until(()=>t.ui.state.images.has('p:'+wanted));
 assert.equal(t.ui.state.images.size,64);assert.equal(target.children[0].tagName,'IMG');
 const request=t.requests.find(request=>new URL(request.url).pathname.includes('/Images/'));
 assert(request.options.headers.Authorization.includes('synthetic-private-bearer'));assert(!request.url.includes('synthetic-private-bearer'));
 t.context.document.querySelectorAll=()=>[];t.ui.clearImages();
});
test('clearing account artwork aborts its HTTP request and rejects the late response',async()=>{
 let finish,signal;const t=setup({apiResponse:async(u,o)=>{if(u.pathname.includes('/Images/')){signal=o.signal;await new Promise(resolve=>finish=resolve);return new Response(Uint8Array.from([255,216,255,217]),{headers:{'Content-Type':'image/jpeg'}});}}});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const target=new Element('div');target.dataset.artKey='p:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';target.dataset.artId='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
 t.context.document.querySelectorAll=selector=>selector==='[data-art-key]'?[target]:[];
 t.ui.pumpArtwork();await until(()=>finish);t.context.document.querySelectorAll=()=>[];t.ui.clearImages();assert.equal(signal.aborted,true);
 finish();await new Promise(resolve=>setTimeout(resolve,0));assert.equal(t.ui.state.images.size,0);assert.equal(target.children.length,0);
});

test('refinements stay lazy and publish studios while tags are still loading',async()=>{
 const studio='44444444444444448444444444444444';let finishTags;
 const t=setup({apiResponse:async u=>{
  if(u.pathname==='/Studios')return jsonReply({Items:[{Id:studio,Name:'Account studio',Type:'Studio'}],TotalRecordCount:1});
  if(u.pathname==='/Items/Filters2'){await new Promise(resolve=>finishTags=resolve);return jsonReply({Tags:['Service: Account']});}
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);await t.ui.openSearch();
 const isFacet=request=>{const url=new URL(request.url);return ['/Studios','/Items/Filters2'].includes(url.pathname)||url.searchParams.get('IncludeItemTypes')==='BoxSet';};
 assert.equal(t.requests.filter(isFacet).length,0);
 t.ui.state.route.refine=true;const loading=t.ui.loadSearchFacets();
 try{
  await until(()=>finishTags);
  assert.equal(t.requests.filter(isFacet).length,3);
  await until(()=>t.ui.state.facets.studios.length===1);
  assert.equal(t.ui.state.facets.loading,true);
  const studioMenu=findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-studio');
  const tagMenu=findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-tag');
  assert(studioMenu.children.some(option=>option.textContent==='Account studio'));
  assert.equal(studioMenu.disabled,false);assert.equal(tagMenu.disabled,true);
 }finally{finishTags?.();await loading;}
 assert.equal(t.ui.state.facets.loading,false);
 assert.equal(findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-tag').disabled,false);
 await t.ui.loadSearchFacets();assert.equal(t.requests.filter(isFacet).length,3);
});

test('pending refinements survive a search route change and preserve selected filters',async()=>{
 const studio='44444444444444448444444444444444',releases=[],signals=[];
 const t=setup({apiResponse:async(u,options)=>{
  if(u.pathname==='/Studios')return jsonReply({Items:[{Id:studio,Name:'Account studio',Type:'Studio'}],TotalRecordCount:1});
  if(u.pathname==='/Items/Filters2'){signals.push(options.signal);await new Promise(resolve=>releases.push(resolve));return jsonReply({Tags:['Service: Account']});}
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);await t.ui.openSearch();
 t.ui.state.route.refine=true;const loading=t.ui.loadSearchFacets();
 try{
  await until(()=>releases.length===1);
  await t.ui.startSearch('ocean',{scope:'movies',studioID:studio,tag:'Service: Account'});
  assert.equal(signals[0].aborted,false);
  assert.equal(t.requests.filter(request=>new URL(request.url).pathname==='/Items/Filters2').length,1);
  const studioMenu=findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-studio');
  const tagMenu=findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-tag');
  assert.equal(studioMenu.value,studio);assert.equal(tagMenu.value,'Service: Account');
  assert(tagMenu.children.some(option=>option.value==='Service: Account'));
  assert.equal(t.ui.state.route.filters.studioID,studio);assert.equal(t.ui.state.route.filters.tag,'Service: Account');
 }finally{for(const release of releases)release();await loading;await new Promise(setImmediate);}
 assert.equal(findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-studio').value,studio);
 assert.equal(findElement(t.root,e=>e.tagName==='SELECT'&&e.id==='search-tag').value,'Service: Account');
 assert.equal(t.ui.state.facets.tags[0],'Service: Account');
});

test('signout aborts account refinement requests and late tags cannot enter the next account',async()=>{
 let finishOld,oldSignal,logins=0,tagCalls=0;
 const t=setup({apiResponse:async(u,options)=>{
  if(u.pathname==='/Users/AuthenticateByName'){logins++;return jsonReply({AccessToken:'synthetic-account-'+logins,User:{Id:logins===1?'11111111111141118111111111111111':'88888888888848888888888888888888',Name:'Listener'}});}
  if(u.pathname==='/Studios')return jsonReply({Items:[],TotalRecordCount:0});
  if(u.pathname==='/Items/Filters2'){
   if(++tagCalls===1){oldSignal=options.signal;await new Promise(resolve=>finishOld=resolve);return jsonReply({Tags:['Old account private tag']});}
   return jsonReply({Tags:['New account tag']});
  }
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);await t.ui.openSearch();
 t.ui.state.route.refine=true;const loading=t.ui.loadSearchFacets();
 try{
  await until(()=>finishOld);await t.ui.signOut();assert.equal(oldSignal.aborted,true);assert.equal(t.ui.state.facets,null);
  await t.ui.signIn('Listener','example-password',false);await t.ui.openSearch();t.ui.state.route.refine=true;await t.ui.loadSearchFacets();
 }finally{finishOld?.();await loading;}
 t.ui.render();
 assert.deepEqual(Array.from(t.ui.state.facets.tags),['New account tag']);
 assert(!t.root.textContent.includes('Old account private tag'));
 const latest=t.requests.filter(request=>new URL(request.url).pathname==='/Items/Filters2').at(-1);
 assert.equal(new URL(latest.url).searchParams.get('UserId'),'88888888888848888888888888888888');
});


function lyricLine(root,index){return findElement(root,e=>e.dataset.lyricLine===String(index));}
function cueWords(line){return (line?.children||[]).filter(e=>e.dataset.lyricWord!==undefined);}
function lyricsResponse(lines,metadata={}){return new Response(JSON.stringify({Metadata:metadata,Lyrics:lines}),{headers:{'Content-Type':'application/json'}});}
async function openLyrics(t){
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);await until(()=>t.ui.state.player?.lyrics?.status!=='loading'&&!!t.ui.state.player?.lyrics);
 t.ui.state.expanded=true;t.ui.state.nowPanel='lyrics';t.ui.render();
}
test('server lyric ticks and real UTF-16 word cues follow the playing clock and offset',async()=>{
 const lines=[{Text:'Rise 🎵 together',Start:10000000,Cues:[
  {Position:0,EndPosition:4,Start:10000000,End:20000000},
  {Position:5,EndPosition:7,Start:20000000,End:30000000},
  {Position:8,EndPosition:16,Start:30000000,End:40000000}]},
  {Text:'The harbor wakes',Start:40000000}];
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Lyrics')?lyricsResponse(lines,{Offset:10000000,IsSynced:true}):undefined});
 await openLyrics(t);
 const request=t.requests.find(r=>r.url.endsWith('/Lyrics'));
 assert(request.options.headers.Authorization.includes('synthetic-private-bearer'));
 assert(!request.url.includes('synthetic-private-bearer'));
 t.video.currentTime=1.9;t.video.listeners.timeupdate();
 assert.equal(lyricLine(t.root,0).getAttribute('aria-current'),null);
 t.video.currentTime=3.5;t.video.listeners.timeupdate();
 const first=lyricLine(t.root,0),words=cueWords(first);
 assert.equal(first.getAttribute('aria-current'),'true');
 assert.equal(words[1].textContent,'🎵');
 assert.equal(words[0].style.getPropertyValue('--word-progress'),'100%');
 assert.equal(words[1].style.getPropertyValue('--word-progress'),'50%');
 assert.equal(words[2].style.getPropertyValue('--word-progress'),'0%');
 t.video.currentTime=5;t.video.listeners.timeupdate();
 assert.equal(first.getAttribute('aria-current'),null);
 assert.equal(lyricLine(t.root,1).getAttribute('aria-current'),'true');
 t.video.paused=true;t.video.listeners.pause();
 assert.equal(lyricLine(t.root,1).getAttribute('aria-current'),'true');
 await t.ui.signOut();
});
test('clicking a timed line seeks the real player and resumes following the song',async()=>{
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Lyrics')?lyricsResponse([
  {Text:'Along the quiet water',Start:10000000},{Text:'We find our way',Start:40000000}],{Offset:10000000}):undefined});
 await openLyrics(t);
 const scrolling=findElement(t.root,e=>e.className==='lyrics-scroll');
 scrolling.listeners.wheel();assert.equal(t.ui.state.player.lyrics.follow,false);
 const follow=findElement(t.root,e=>e.textContent==='Follow song'&&e.tagName==='BUTTON');assert.equal(follow.hidden,false);
 lyricLine(t.root,1).listeners.click();await until(()=>!t.ui.state.busy);
 assert.equal(t.video.currentTime,5);assert.equal(t.ui.state.player.lyrics.follow,true);
 assert.equal(lyricLine(t.root,1).getAttribute('aria-current'),'true');
 await t.ui.signOut();
});
test('untimed lyrics remain readable without invented synchronization or seek controls',async()=>{
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Lyrics')?lyricsResponse([
  {Text:'An original unsynchronized line'},{Text:'A second original line'}],{IsSynced:true}):undefined});
 await openLyrics(t);
 assert.match(t.root.textContent,/Timing is not available/);
 assert.equal(lyricLine(t.root,0).tagName,'P');
 assert.equal(lyricLine(t.root,0).getAttribute('aria-current'),null);
 t.video.currentTime=35;t.video.listeners.timeupdate();
 assert.equal(lyricLine(t.root,0).getAttribute('aria-current'),null);
 await t.ui.signOut();
});
test('missing lyrics never interrupt playback or advertise karaoke',async()=>{
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Lyrics')?new Response('{}',{status:404}):undefined});
 await openLyrics(t);
 assert.equal(t.ui.state.player.lyrics.status,'missing');assert.equal(t.video.paused,false);
 assert.match(t.root.textContent,/Lyrics aren’t available for this song/);
 assert(!/microphone|Sing ready|karaoke ready/i.test(t.root.textContent));
 await t.ui.signOut();
});
test('changing songs aborts and fences late lyrics while keeping Now Playing open',async()=>{
 let release,firstSignal;
 const t=setup({apiResponse:async(u,options)=>{
  if(!u.pathname.endsWith('/Lyrics'))return undefined;
  if(u.pathname.includes('22222222')){firstSignal=options.signal;await new Promise(resolve=>release=resolve);return lyricsResponse([{Text:'Old song must stay private',Start:0}]);}
  return lyricsResponse([{Text:'A new song in the harbor',Start:0}]);
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.play(t.item);await until(()=>release);
 t.ui.state.expanded=true;t.ui.state.nowPanel='lyrics';t.ui.render();
 const next={...t.item,Id:'44444444444444448444444444444444',Name:'Another original song'};
 await t.ui.play(next,[t.item,next],1);await until(()=>t.ui.state.player.lyrics?.status==='ready');
 assert.equal(firstSignal.aborted,true);assert.equal(t.ui.state.expanded,true);assert.equal(t.ui.state.nowPanel,'lyrics');
 release();await new Promise(setImmediate);t.ui.render();
 assert.match(t.root.textContent,/A new song in the harbor/);assert(!t.root.textContent.includes('Old song must stay private'));
 await t.ui.signOut();assert.equal(t.ui.state.player,null);
});
test('sign-out erases lyrics and aborts their optional request even if the response arrives late',async()=>{
 let release,signal;
 const t=setup({apiResponse:async(u,options)=>{
  if(!u.pathname.endsWith('/Lyrics'))return undefined;
  signal=options.signal;await new Promise(resolve=>release=resolve);return lyricsResponse([{Text:'Private words',Start:0}]);
 }});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);await t.ui.play(t.item);await until(()=>release);
 const retired=t.ui.state.player;await t.ui.signOut();assert.equal(signal.aborted,true);
 const count=t.requests.length;await t.ui.loadLyrics(retired,true);assert.equal(t.requests.length,count);
 release();await new Promise(setImmediate);t.ui.render();
 assert.equal(t.ui.state.player,null);assert(!t.root.textContent.includes('Private words'));
});
test('malformed word ranges cannot split a surrogate pair or create fake timed words',async()=>{
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Lyrics')?lyricsResponse([
  {Text:'A 🎵 rises',Start:0,Cues:[{Position:2,EndPosition:3,Start:0,End:10000000}]}]):undefined});
 await openLyrics(t);
 const line=lyricLine(t.root,0);assert.equal(line.textContent,'A 🎵 rises');assert.equal(cueWords(line).length,0);
 await t.ui.signOut();
});

test('tapping an earlier lyric renews a cropped audio lease at the requested line',async()=>{
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Lyrics')?lyricsResponse([
  {Text:'Back to the opening shore',Start:100000000},{Text:'Farther along the tide',Start:18050000000}]):undefined});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 const resumed={...t.item,RunTimeTicks:36000000000,UserData:{PlaybackPositionTicks:18000000000}};
 await t.ui.play(resumed,[resumed]);await until(()=>t.ui.state.player.lyrics?.status==='ready');
 t.ui.state.expanded=true;t.ui.state.nowPanel='lyrics';t.ui.render();
 lyricLine(t.root,0).listeners.click();await until(()=>!t.ui.state.busy);
 const leases=t.requests.filter(r=>r.url.endsWith('/Bluefin/Web/v1/leases')&&r.options.method==='POST');
 assert.equal(JSON.parse(leases.at(-1).options.body).playback.startTicks,100000000);
 assert.equal(t.ui.state.player.offset,10);assert.equal(t.ui.state.expanded,true);assert.equal(t.ui.state.nowPanel,'lyrics');
 await t.ui.signOut();
});
test('lyrics use the authenticated mesh request and its owned audio clock without HTTP fallback',async()=>{
 const calls=[];let missing=false;
 const engine={webMeshRequest:async(...args)=>{calls.push(args);if(missing)throw 'The peer returned HTTP 404.';return JSON.stringify({Lyrics:[{Text:'At the start',Start:100000000},{Text:'At the next shore',Start:120000000},{Text:'Much later',Start:990000000}]});},webMeshStop:async()=>{},webMeshLogout:async()=>{}};
 const t=setup({engine:async()=>engine});t.context.BluefinRuntime.ready=()=>engine;
 Object.assign(t.ui.state,{phase:'library',mode:'mesh',session:{handle:'owned-preview-handle',user:'11111111111141118111111111111111'}});
 const player={mode:'mesh',handle:t.ui.state.session.handle,item:t.item,session:t.ui.state.session,offset:10,snapshot:{plan:{},position:99,duration:120}};
 t.ui.state.player=player;t.video.currentTime=2;t.video.__bluefinOutputOwner={revision:1};
 await t.ui.loadLyrics(player);t.ui.state.expanded=true;t.ui.state.nowPanel='lyrics';t.ui.render();
 assert.equal(calls[0][0],'owned-preview-handle');assert.equal(calls[0][1],'GET');assert.equal(calls[0][2],'Audio/'+t.item.Id+'/Lyrics');assert.equal(t.requests.length,0);
 assert.equal(lyricLine(t.root,1).getAttribute('aria-current'),'true');assert.equal(lyricLine(t.root,2).getAttribute('aria-current'),null);
 missing=true;await t.ui.loadLyrics(player,true);assert.equal(player.lyrics.status,'missing');assert.equal(t.requests.length,0);
 await t.ui.signOut();
});

test('library alternate sort and watch filters reach every server range',async()=>{
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Items')&&u.searchParams.has('SearchTerm')?jsonReply({Items:[{Id:'66666666666646668666666666666666',Type:'Movie',Name:'A movie'}],TotalRecordCount:120}):undefined});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);
 await t.ui.startSearch('ocean',{scope:'library',sort:'newest',played:'false',favorite:true,year:'2026'});
 await t.ui.loadMoreSearch('library');
 for(const r of t.requests.filter(r=>new URL(r.url).searchParams.has('SearchTerm'))){const q=new URL(r.url).searchParams;assert.equal(q.get('SearchOrder'),'SortBy');assert.equal(q.get('SortBy'),'PremiereDate');assert.equal(q.get('SortOrder'),'Descending');assert.equal(q.get('IsPlayed'),'false');assert.equal(q.get('IsFavorite'),'true');assert.equal(q.get('Years'),'2026');}
 await t.ui.startSearch('ocean',{scope:'library'});
 const q=new URL(t.requests.at(-1).url).searchParams;assert.equal(q.has('SortBy'),false);assert.equal(q.has('SearchOrder'),false);
});
test('home cards show series titles and series art without changing episode playback',async()=>{
 const episode={Id:'66666666666646668666666666666666',Type:'Episode',Name:'The Big Quiet',SeriesName:'The Adventures of Pete & Pete',SeriesId:'77777777777747778777777777777777',ParentIndexNumber:2,IndexNumber:4};
 const t=setup({apiResponse:u=>u.pathname.endsWith('/Resume')?jsonReply({Items:[episode]}):undefined});
 await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);t.ui.render();
 const heading=findElement(t.root,e=>e.tagName==='H2'&&e.textContent==='The Adventures of Pete & Pete');assert(heading);
 assert.match(t.root.textContent,/S2 E4 · The Big Quiet/);
 const artwork=findElement(t.root,e=>e.dataset.artSources?.includes(episode.SeriesId));assert(artwork);assert(!artwork.dataset.artSources.includes(episode.Id));
 assert.equal(t.ui.state.featured.Id,episode.Id);
});
test('collections expose a bounded scroll region without page buttons',async()=>{
 const t=setup();await t.ui.connect('https://bluefin.test');await t.ui.signIn('Listener','example-password',false);await t.ui.navigate('Movies');t.ui.render();
 assert(findElement(t.root,e=>e.className.includes('virtual-list')));assert(!t.root.textContent.includes('Next page'));assert(!t.root.textContent.includes('Previous page'));
});

test('mesh browsing serializes each sort/filter/range value for the Rust String-pair contract',async()=>{
 const requests=[],movie={Id:'66666666666646668666666666666666',Type:'Movie',Name:'Mesh movie'};
 const engine=async()=>({webMeshRequest:async(handle,method,path,serialized)=>{
  const query=JSON.parse(serialized);assert(query.every(pair=>pair.length===2&&pair.every(value=>typeof value==='string')));requests.push(new Map(query));
  return JSON.stringify({Items:[movie],TotalRecordCount:120});
 }});
 const t=setup({engine});t.ui.state.mode='mesh';t.ui.state.phase='library';t.ui.state.session={user:'11111111111141118111111111111111',handle:'bluefin-mesh-v1:synthetic'};
 await t.ui.startSearch('ocean',{scope:'library',sort:'titleDesc',favorite:true,played:'false',year:'2026'});await t.ui.loadMoreSearch('library');
 assert.equal(requests.length,2);for(const q of requests){assert.equal(q.get('SearchOrder'),'SortBy');assert.equal(q.get('SortBy'),'SortName');assert.equal(q.get('SortOrder'),'Descending');assert.equal(q.get('IsFavorite'),'true');assert.equal(q.get('IsPlayed'),'false');assert.equal(q.get('Years'),'2026');}
 assert.equal(requests[0].get('StartIndex'),'0');assert.equal(requests[1].get('StartIndex'),'1');
});
