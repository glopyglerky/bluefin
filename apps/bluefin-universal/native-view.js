// Native-style progressive presentation. Server/account/media authority stays
// in Rust. Optional Wasm supplies the strict PQ device protocol; this module
// never converts a paired connection into an HTTP bearer session.
(() => {
'use strict';
const app=document.getElementById('app'), media=document.getElementById('media');
const runtime=()=>window.BluefinRuntime;
const launched=globalThis.takeBluefinLaunch?.()||'';
const state={phase:'connect',base:hostedBase(),server:null,options:{},session:null,mode:'https',
  tab:'Home',route:{kind:'home'},rows:[],shelves:[],back:[],sheet:launched?'invite':null,
  code:launched,meshName:'',remember:false,pairing:null,pairingView:0,device:null,invitation:null,inviteTimer:null,
  busy:false,error:'',notice:'',epoch:0,capabilities:null,player:null,expanded:false,
  queue:[],queueIndex:0,shuffle:false,repeat:'off',query:'',images:new Map(),imageEpoch:0};
let modalWasOpen=false,focusReturn=null,pageRetired=false,authTask=false,playbackGeneration=0,leaseRenew=null,progressTimer=null,meshPoll=null,hlsPromise;
const artworkPending=new Set(),artworkMissing=new Set(),requestControllers=new Set();let artworkActive=0;
function assertLive(epoch=state.epoch,session){if(pageRetired||epoch!==state.epoch||(session!==undefined&&session!==state.session))throw Error('This page or account changed. Resume the current connection.');}
function clearPrivateView(){state.rows=[];state.shelves=[];state.back=[];state.query='';state.route={kind:'home'};state.queue=[];state.queueIndex=0;state.shuffle=false;state.repeat='off';state.capabilities=null;state.code='';state.meshName='';state.pairing=null;state.device=null;state.invitation=null;}
const paths={
 waves:'M3 6c3-3 6 3 9 0s6 3 9 0M3 12c3-3 6 3 9 0s6 3 9 0M3 18c3-3 6 3 9 0s6 3 9 0',
 home:'M3 10 12 3l9 7M5 9v12h5v-7h4v7h5V9',
 library:'M4 7h14v14H4zM7 3h14v14',
 music:'M9 18V5l11-2v13M9 9l11-2M9 18c0 3-6 4-6 1s6-4 6-1M20 16c0 3-6 4-6 1s6-4 6-1',
 search:'M16 16l5 5M18 10a8 8 0 1 1-16 0 8 8 0 1 1 16 0',
 live:'M5 4a12 12 0 0 0 0 16M19 4a12 12 0 0 1 0 16M8 7a7 7 0 0 0 0 10M16 7a7 7 0 0 1 0 10M12 12h.01',
 account:'M20 12a8 8 0 1 1-16 0 8 8 0 1 1 16 0M15 9a3 3 0 1 1-6 0 3 3 0 1 1 6 0M6 17c2-4 10-4 12 0',
 play:'M7 4v16l13-8z',pause:'M8 4v16M16 4v16',
 next:'M4 5v14l11-7zM18 5v14',previous:'M20 5v14L9 12zM6 5v14',
 back:'m15 4-8 8 8 8',down:'m4 8 8 8 8-8',chevron:'m9 5 7 7-7 7',
 shuffle:'M3 6h3c5 0 7 12 12 12h3M3 18h3c5 0 7-12 12-12h3M18 3l3 3-3 3M18 15l3 3-3 3',
 repeat:'M4 8a4 4 0 0 1 4-4h12l-3-3M20 16a4 4 0 0 1-4 4H4l3 3M20 4l-3 3M4 20l3-3',
 queue:'M4 5h16M4 12h16M4 19h10',download:'M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4',
 qr:'M3 3h6v6H3zM15 3h6v6h-6zM3 15h6v6H3zM15 15h3v3h3v3h-6z',info:'M12 11v6M12 7h.01M21 12a9 9 0 1 1-18 0 9 9 0 1 1 18 0'
};
function node(tag,text,cls){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;if(cls)e.className=cls;return e;}
function icon(name){const e=document.createElementNS('http://www.w3.org/2000/svg','svg');e.setAttribute('viewBox','0 0 24 24');e.setAttribute('aria-hidden','true');const p=document.createElementNS(e.namespaceURI,'path');p.setAttribute('d',paths[name]||paths.music);e.append(p);return e;}
function button(label,action,cls='',symbol){const b=node('button',undefined,cls);b.type='button';b.dataset.focusKey=(symbol||'')+'|'+label;if(symbol)b.append(icon(symbol));if(label)b.append(node('span',label));b.addEventListener('click',action);return b;}
function action(label,task,cls='',symbol){const b=button(label,()=>perform(task),cls,symbol);b.disabled=state.busy;return b;}
function field(label,type,value='',name=''){const wrap=node('div'),id='bf-'+name,lab=node('label',label),input=node('input');lab.htmlFor=id;input.id=id;input.name=name;input.type=type;input.value=value;input.autocomplete=type==='password'?'current-password':'off';wrap.append(lab,input);return [wrap,input];}
function group(title){const g=node('section',undefined,'group');if(title)g.append(node('h2',title));return g;}
function status(){const e=node('p',state.error||state.notice,'page-status'+(state.error?' error':''));e.setAttribute('role',state.error?'alert':'status');return e;}
function hostedBase(){const u=new URL(location.href);u.search='';u.hash='';const match=u.pathname.match(/^(.*)\/web\/(?:bluefin\/)?(?:index\.html)?$/);u.pathname=match?(match[1]||'')+'/':'/';return u.href;}
function validateBase(raw){const url=new URL(raw.includes('://')?raw:'https://'+raw);if(url.protocol!=='https:'||url.username||url.password||url.search||url.hash||url.origin!==location.origin)throw Error('Open that server’s trusted HTTPS Bluefin page to sign in. This page connects to '+location.host+'.');if(!url.pathname.endsWith('/'))url.pathname+='/';return url.href;}
function uuid(value){return typeof value==='string'&&/^(?:[a-f0-9]{32}|[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})$/i.test(value)&&!/^0{32}$/.test(value.replaceAll('-',''));}
function id(value){if(!uuid(value))throw Error('The server returned an invalid item.');return value.replaceAll('-','').toLowerCase();}
const deviceId=(()=>{try{return crypto.randomUUID();}catch(_){return Array.from(crypto.getRandomValues(new Uint8Array(16)),v=>v.toString(16).padStart(2,'0')).join('');}})();
function authorization(token=''){if(typeof token!=='string'||token.length>4096||/["\\\x00-\x1f\x7f]/.test(token)||token.startsWith('bluefin-mesh-'))throw Error('Invalid HTTPS account credential.');return 'MediaBrowser Client="Bluefin Web", Device="Browser", DeviceId="'+deviceId+'", Version="1.0"'+(token?', Token="'+token+'"':'');}
function endpoint(path,query=[],base=state.base){if(typeof path!=='string'||path.startsWith('/')||path.includes('..')||/[\\#%\x00-\x20]/.test(path))throw Error('Invalid API route.');const u=new URL(path,base);if(u.origin!==location.origin)throw Error('The HTTPS account must stay on this origin.');for(const [key,value]of query)u.searchParams.set(key,String(value));return u;}
async function readJson(response,limit=4*1024*1024){if(!response.body)return null;const reader=response.body.getReader(),chunks=[];let size=0;try{for(;;){const {done,value}=await reader.read();if(done)break;size+=value.length;if(size>limit)throw Error('The server response exceeded the browser limit.');chunks.push(value);}}catch(e){await reader.cancel().catch(()=>{});throw e;}if(!size)return null;const bytes=new Uint8Array(size);let at=0;for(const part of chunks){bytes.set(part,at);at+=part.length;}return JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));}
async function http(path,method='GET',body,query=[],session=state.session,keepalive=false,captureReply=false,base=state.base){
 if(!keepalive)assertLive();
 const url=endpoint(path,query,base),controller=new AbortController(),timer=setTimeout(()=>controller.abort(),18000);
 if(!keepalive)requestControllers.add(controller);
 const headers={Authorization:authorization(session?.token||'')};if(body!==undefined)headers['Content-Type']='application/json';
 try{
  const response=await fetch(url,{method,headers,body:body===undefined?undefined:JSON.stringify(body),
   credentials:'same-origin',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal,keepalive});
  if(!response.ok){if(response.status===401||response.status===403)throw Error('Account access was denied. Sign in again.');throw Error('The server returned HTTP '+response.status+'.');}
  const value=await readJson(response);
  if(!keepalive&&!captureReply)assertLive();
  return value;
 }catch(e){if(e.name==='AbortError')throw Error('The server did not respond in time or the page was closed.');throw e;}
 finally{requestControllers.delete(controller);clearTimeout(timer);}
}
async function retireLateAccount(session,base){
 await http('Sessions/Logout','POST',undefined,[],session,true,true,base).catch(()=>{});
}
async function api(path,method='GET',body,query=[]){
 const epoch=state.epoch,session=state.session,mode=state.mode,base=state.base;assertLive(epoch,session);
 let value;
 if(mode==='mesh'){
  if(!session?.handle)throw Error('Resume the encrypted device connection.');
  const engine=await runtime().engine();assertLive(epoch,session);
  value=JSON.parse(await engine.webMeshRequest(session.handle,method,path,JSON.stringify(query),body===undefined?undefined:JSON.stringify(body)));
 }else value=await http(path,method,body,query,session,false,false,base);
 assertLive(epoch,session);return value;
}
function items(value){const list=Array.isArray(value)?value:value?.Items;if(!Array.isArray(list)||list.length>200)throw Error('The server returned an invalid library list.');return list.filter(item=>uuid(item.Id)).map(item=>({...item,Name:String(item.Name||'Untitled').slice(0,4096),Overview:String(item.Overview||'').slice(0,65536)}));}
function subtitle(item){return [item.AlbumArtist||item.Artists?.join(', ')||item.ProductionYear,item.Album].filter(Boolean).join(' · ');}
async function perform(task){
 if(pageRetired||state.busy)return;state.busy=true;state.error='';state.notice='';render();
 try{await task();}catch(e){if(!pageRetired)state.error=String(e.message||e);}
 finally{if(!pageRetired){state.busy=false;render();}}
}
async function connect(raw){
 const epoch=state.epoch,base=validateBase(raw);assertLive(epoch);state.base=base;
 const info=await http('System/Info/Public','GET',undefined,[],null,false,false,base);assertLive(epoch);
 if(typeof info?.ServerName!=='string')throw Error('No Bluefin server was found at this address.');
 const options=await http('Bluefin/Accounts/Options','GET',undefined,[],null,false,false,base).catch(()=>({}));
 assertLive(epoch);state.server=info.ServerName;state.options=options;state.phase='options';
}
function account(value){const token=value?.AccessToken,user=value?.User?.Id;if(!uuid(user)||!token)throw Error('The server returned an invalid account.');authorization(token);return {token,user:id(user),name:String(value.User.Name||'Your account')};}
async function accept(value,mode='https',epoch=state.epoch){
 assertLive(epoch);state.epoch++;const accepted=state.epoch;
 state.session=value;state.mode=mode;state.phase='library';state.sheet=null;state.device=null;
 state.pairing=null;state.invitation=null;state.code='';state.tab='Home';state.back=[];state.route={kind:'home'};
 const capabilities=await api('Bluefin/Capabilities').catch(()=>null);assertLive(accepted,value);
 state.capabilities=capabilities;await loadRoute();assertLive(accepted,value);
}
async function signIn(name,password,create){
 const epoch=state.epoch,base=state.base;assertLive(epoch);authTask=true;
 try{
  if(create){if(!state.options.allowSignups||password.length<15)throw Error('Use a password with at least 15 characters.');
   await http('Bluefin/Accounts/Create','POST',{username:name,password},[],null,false,false,base);assertLive(epoch);}
  const next=account(await http('Users/AuthenticateByName','POST',{Username:name,Pw:password},[],null,false,true,base));
  if(pageRetired||epoch!==state.epoch){await retireLateAccount(next,base);assertLive(epoch);}
  await accept(next,'https',epoch);
 }finally{authTask=false;password='';}
}
async function startPasskey(){
 const epoch=state.epoch,base=state.base;assertLive(epoch);
 const reply=await http('Bluefin/Accounts/Passkey/Initiate','POST',{},[],null,false,true,base);
 if(pageRetired||epoch!==state.epoch){if(reply?.proof)await http('Bluefin/Accounts/Passkey/Cancel','POST',reply.proof,[],null,true,true,base).catch(()=>{});assertLive(epoch);}
 const target=new URL(reply.url),expected=endpoint('Bluefin/Accounts/Passkey',[],base),fragment=target.hash;target.hash='';
 if(target.href!==expected.href||!/^#[a-f0-9]{32}$/.test(fragment)){
  if(reply.proof)await http('Bluefin/Accounts/Passkey/Cancel','POST',reply.proof,[],null,false,false,base).catch(()=>{});
  throw Error('The server returned an invalid passkey page.');
 }
 target.hash=fragment;state.device={kind:'passkey',proof:reply.proof,url:target.href,code:String(reply.code||'')};state.phase='device';
}
async function startQuick(){
 const epoch=state.epoch;assertLive(epoch);const reply=await http('QuickConnect/Initiate','POST',{},[],null);assertLive(epoch);
 if(typeof reply.Secret!=='string'||reply.Secret.length>4096||typeof reply.Code!=='string')throw Error('Invalid Quick Connect response.');
 state.device={kind:'quick',proof:reply.Secret,code:reply.Code};state.phase='device';
}
async function finishDevice(){
 const d=state.device,epoch=state.epoch,base=state.base;if(!d)return;assertLive(epoch);let value;
 if(d.kind==='passkey')value=await http('Bluefin/Accounts/Passkey/Exchange','POST',d.proof,[],null,false,true,base);
 else{const check=await http('QuickConnect/Connect','GET',undefined,[['Secret',d.proof]],null,false,false,base);assertLive(epoch);
  if(check.Authenticated!==true){state.notice='Finish approving the request first.';return;}
  value=await http('Users/AuthenticateWithQuickConnect','POST',{Secret:d.proof},[],null,false,true,base);}
 if(value?.pending){assertLive(epoch);state.notice='Finish approving the request first.';return;}
 const next=account(value);if(pageRetired||epoch!==state.epoch){await retireLateAccount(next,base);assertLive(epoch);}
 await accept(next,'https',epoch);
}
async function cancelDevice(){const d=state.device;if(d?.kind==='passkey')await http('Bluefin/Accounts/Passkey/Cancel','POST',d.proof,[],null);state.device=null;state.phase='options';}
async function meshProgress(serialized,epoch,view,engine){
 const value=JSON.parse(serialized);
 if(value.kind==='connected'){
  if(!/^bluefin-mesh-v1:[a-f0-9]{64}$/.test(value.handle)||!uuid(value.user))throw Error('Invalid device connection.');
  if(pageRetired||epoch!==state.epoch||view!==state.pairingView){
   if(pageRetired)engine.webMeshPageExit();
   await engine.webMeshSuspend(value.handle).catch(()=>{});
   if(!pageRetired){state.pairing=null;state.notice='The connection is saved in this tab. Open Connect Bluefin and resume it.';}
   return;
  }
  state.base=value.origin;await accept({handle:value.handle,user:id(value.user),name:'Your account'},'mesh',epoch);
 }else{
  assertLive(epoch);if(value.kind==='pairing'){state.pairing=value;state.code='';}
  else{state.pairing=null;state.notice='No saved connection is available.';}
 }
}
async function beginMesh(code,name,remember){
 const epoch=state.epoch,view=state.pairingView;assertLive(epoch);
 const engine=await runtime().engine();assertLive(epoch);if(view!==state.pairingView)throw Error('The connection screen was closed.');
 state.remember=remember;const result=await engine.webMeshBegin(code,name,remember);
 await meshProgress(result,epoch,view,engine);
}
async function resumeMesh(remembered){
 const epoch=state.epoch,view=state.pairingView;assertLive(epoch);
 const engine=await runtime().engine();assertLive(epoch);if(view!==state.pairingView)throw Error('The connection screen was closed.');
 if(remembered)state.remember=true;
 await meshProgress(await engine.webMeshResume(remembered),epoch,view,engine);
}
async function continueMesh(){
 const epoch=state.epoch,view=state.pairingView;assertLive(epoch);
 const engine=await runtime().engine();assertLive(epoch);if(view!==state.pairingView)throw Error('The connection screen was closed.');
 await meshProgress(await engine.webMeshProceed(),epoch,view,engine);
}
function closeSheet(){if(state.sheet==='invite')state.pairingView++;state.sheet=null;state.expanded=false;state.invitation=null;clearTimeout(state.inviteTimer);render();}

async function loadRoute(){const epoch=state.epoch,session=state.session,route=state.route,user=session?.user;assertLive(epoch,session);if(!user)return;const current=()=>{assertLive(epoch,session);if(route!==state.route)throw Error('The library view changed.');};state.rows=[];state.shelves=[];state.error='';
 if(route.kind==='home'){const requests=[
 ['Continue watching','Users/'+user+'/Items/Resume',[['Limit',12],['Fields','Overview']]],
 ['Up next','Shows/NextUp',[['UserId',user],['Limit',12],['Fields','Overview']]],
 ['Recently added','Users/'+user+'/Items/Latest',[['Limit',18],['Fields','Overview']]]];
 const results=await Promise.allSettled(requests.map(async([title,path,q])=>({title,items:items(await api(path,'GET',undefined,q))})));
 current();state.shelves=results.filter(r=>r.status==='fulfilled').map(r=>r.value).filter(s=>s.items.length);
 if(!state.shelves.length&&results.every(r=>r.status==='rejected'))throw Error('Your library could not refresh. Open Library or try again.');
 }else if(route.kind==='library'){const rows=items(await api('Users/'+user+'/Views'));current();state.rows=rows;}
 else if(route.kind==='music'){const q=[['Limit',12],['Recursive',true],['IncludeItemTypes','MusicAlbum'],['Fields','Overview,PrimaryImageAspectRatio'],['SortBy','DateCreated'],['SortOrder','Descending']];
 const recent=items(await api('Users/'+user+'/Items','GET',undefined,q));current();state.shelves=[{title:'Recently added',items:recent}];
 const favorites=await api('Users/'+user+'/Items','GET',undefined,[...q,['IsFavorite',true]]).then(items).catch(()=>[]);current();if(favorites.length)state.shelves.push({title:'Favorite albums',items:favorites});
 }else if(route.kind==='live'){const rows=items(await api('LiveTv/Channels','GET',undefined,[['UserId',user],['Limit',100]]));current();state.rows=rows;}
 else if(route.kind==='search'){if(!state.query.trim())return;const rows=items(await api('Users/'+user+'/Items','GET',undefined,[['Limit',100],['Recursive',true],['SearchTerm',state.query],['Fields','Overview,PrimaryImageAspectRatio']]));current();state.rows=rows;}
 else if(route.kind==='collection'){const q=[['Limit',100],['StartIndex',route.start||0],['Recursive',true],['Fields','Overview,PrimaryImageAspectRatio'],['SortBy',route.type==='Audio'&&route.parent?'ParentIndexNumber,IndexNumber,SortName':'SortName']];if(route.type)q.push(['IncludeItemTypes',route.type]);if(route.parent)q.push(['ParentId',route.parent]);const value=await api('Users/'+user+'/Items','GET',undefined,q);current();state.rows=items(value);route.total=Number(value.TotalRecordCount)||state.rows.length;}
 else if(route.kind==='detail'){const value=await api('Users/'+user+'/Items/'+id(route.item.Id));current();if(!uuid(value.Id))throw Error('The item is no longer available.');route.item=value;}
}
async function navigate(tab){state.tab=tab;state.route={kind:({Home:'home',Library:'library',Music:'music',Search:'search','Live TV':'live'})[tab]};state.back=[];await loadRoute();}
async function openCollection(title,type,parent){state.back.push({...state.route});state.route={kind:'collection',title,type,parent,start:0};await loadRoute();}
async function openItem(item){if(item.Type==='Audio')return play(item,[item],0);if(['MusicAlbum','MusicArtist','Playlist','Series','Season','CollectionFolder','Folder','BoxSet'].includes(item.Type)||item.IsFolder){return openCollection(item.Name,item.Type==='Series'||item.Type==='Season'?'Episode':(item.Type==='MusicArtist'?'MusicAlbum':['MusicAlbum','Playlist'].includes(item.Type)?'Audio':null),id(item.Id));}state.back.push({...state.route});state.route={kind:'detail',item};await loadRoute();}
function artKey(item,backdrop=false){return (backdrop?'b:':'p:')+id(item.Id);}
function applyArtwork(element,url,backdrop=false){if(backdrop){element.style.backgroundImage='linear-gradient(transparent,#111b),url("'+url+'")';element.style.backgroundSize='cover';element.style.backgroundPosition='center';return;}const image=node('img');image.alt='';image.src=url;image.loading='lazy';image.onerror=()=>{element.replaceChildren(icon('music'));};element.replaceChildren(image);}
function bindArtwork(element,item,backdrop=false){const key=artKey(item,backdrop);element.dataset.artKey=key;element.dataset.artId=id(item.Id);if(backdrop)element.dataset.backdrop='true';const cached=state.images.get(key);if(cached)applyArtwork(element,cached,backdrop);}
function cover(item,square=false){const d=node('div',undefined,'artwork'+(square?' square':''));d.append(icon(item.Type==='Audio'||item.Type==='MusicAlbum'?'music':'library'));bindArtwork(d,item);return d;}
async function imageBytes(item,backdrop,session,mode){
 if(mode==='mesh')return (await runtime().engine()).webMeshArtwork(session.handle,item,backdrop);
 const url=endpoint('Items/'+item+'/Images/'+(backdrop?'Backdrop':'Primary'),[['maxWidth',backdrop?1200:400],['quality',85]]);
 const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),8000);
 try{const response=await fetch(url,{headers:{Authorization:authorization(session.token)},credentials:'omit',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});
 if(!response.ok||!['image/jpeg','image/png','image/webp'].includes((response.headers.get('Content-Type')||'').split(';')[0].trim()))throw Error('Artwork unavailable');
 const reader=response.body.getReader(),parts=[];let total=0;try{for(;;){const {done,value}=await reader.read();if(done)break;total+=value.length;if(total>2*1024*1024)throw Error('Artwork limit');parts.push(value);}}catch(e){await reader.cancel().catch(()=>{});throw e;}
 const bytes=new Uint8Array(total);let at=0;for(const part of parts){bytes.set(part,at);at+=part.length;}return bytes;
 }finally{clearTimeout(timer);}
}
function imageType(bytes){if(bytes.length>2&&bytes[0]===255&&bytes[1]===216&&bytes[2]===255)return 'image/jpeg';
 if(bytes.length>8&&[137,80,78,71,13,10,26,10].every((v,i)=>bytes[i]===v))return 'image/png';
 if(bytes.length>12&&String.fromCharCode(...bytes.slice(0,4))==='RIFF'&&String.fromCharCode(...bytes.slice(8,12))==='WEBP')return 'image/webp';
 throw Error('Unsupported artwork format');}
function pumpArtwork(){if(!state.session)return;const epoch=state.imageEpoch,session=state.session,mode=state.mode;
 for(const element of document.querySelectorAll('[data-art-key]')){
  if(artworkActive>=3)break;const key=element.dataset.artKey;if(state.images.has(key)||artworkPending.has(key)||artworkMissing.has(key))continue;
  if(state.images.size>=64)break;artworkPending.add(key);artworkActive++;
  const item=element.dataset.artId,backdrop=element.dataset.backdrop==='true';
  imageBytes(item,backdrop,session,mode).then(bytes=>{
   if(epoch!==state.imageEpoch||session!==state.session)return;const url=URL.createObjectURL(new Blob([bytes],{type:imageType(bytes)}));state.images.set(key,url);
   for(const target of document.querySelectorAll('[data-art-key]'))if(target.dataset.artKey===key)applyArtwork(target,url,target.dataset.backdrop==='true');
  }).catch(()=>{if(epoch===state.imageEpoch)artworkMissing.add(key);}).finally(()=>{artworkActive--;artworkPending.delete(key);if(epoch===state.imageEpoch)pumpArtwork();});
 }
}
function poster(item){const b=button('',()=>perform(()=>openItem(item)),'poster'+(item.Type==='MusicAlbum'?' album':''));b.append(cover(item,item.Type==='MusicAlbum'),node('strong',item.Name));if(subtitle(item))b.append(node('small',subtitle(item)));return b;}
function shelf(title,entries){if(!entries.length)return document.createDocumentFragment();const s=node('section',undefined,'shelf');s.append(node('h2',title));const row=node('div',undefined,'shelf-track');for(const item of entries.slice(0,18))row.append(poster(item));s.append(row);return s;}
function row(item){const b=button('',()=>perform(()=>openItem(item)),'list-row'+(item.Type==='Audio'?' song':''));if(item.Type==='Audio')b.append(cover(item,true));else b.append(icon(item.CollectionType==='music'?'music':'library'));const t=node('span',undefined,'grow');t.append(node('strong',item.Name));if(subtitle(item))t.append(node('small',subtitle(item)));b.append(t,icon('chevron'));return b;}
function empty(title,message){const d=node('section',undefined,'empty-state');d.append(icon('library'),node('h2',title),node('p',message,'secondary'));return d;}
function toolbar(title,back=false){const d=node('div',undefined,'toolbar');if(back)d.append(action('Back',async()=>{state.route=state.back.pop()||{kind:'home'};await loadRoute();},'plain','back'));else d.append(node('h1',title));const b=button('',()=>{state.sheet='account';render();},'plain','account');b.setAttribute('aria-label','Account and settings');d.append(b);return d;}
function welcome(){const main=node('main',undefined,'welcome');main.id='content';const mark=node('div',undefined,'wordmark');mark.append(icon('waves'),node('h1','Bluefin'));main.append(mark);const stack=node('section',undefined,'account-stack');
 if(state.phase==='connect'){stack.append(button('Connect with QR or invitation',()=>{state.sheet='invite';state.error='';render();}));const form=node('form');form.append(node('h2','Connect to your server'));const [w,i]=field('Server address','url',state.base,'server');i.autocomplete='url';i.placeholder='https://media.example.com';i.required=true;form.append(w);const submit=node('button','Continue','primary');submit.disabled=state.busy;form.append(submit);form.onsubmit=e=>{e.preventDefault();perform(()=>connect(i.value.trim()));};stack.append(form);}
 else if(state.phase==='options'){stack.append(node('h2',state.server||'Sign in'));stack.append(button('Sign in with password',()=>{state.phase='password';render();},'primary'));if(state.options.passkeys)stack.append(action('Sign in with a passkey',startPasskey));if(state.options.quickConnect)stack.append(action('Quick Connect',startQuick));if(state.options.allowSignups)stack.append(button('Create account',()=>{state.phase='create';render();}));stack.append(button('Change server',()=>{state.phase='connect';render();},'plain'));}
 else if(state.phase==='password'||state.phase==='create'){const create=state.phase==='create';const f=node('form');f.append(node('h2',create?'Create account':'Sign in'));const [nw,ni]=field('Username','text','', 'username'),[pw,pi]=field('Password','password','','password');ni.autocomplete='username';pi.autocomplete=create?'new-password':'current-password';ni.required=true;pi.required=true;f.append(nw,pw);let confirm;if(create){const [w,i]=field('Confirm password','password','','confirm');confirm=i;i.autocomplete='new-password';f.append(w,node('p','Use at least 15 characters.','secondary'));}const b=node('button',create?'Create account':'Sign in','primary');b.disabled=state.busy;f.append(b);f.onsubmit=e=>{e.preventDefault();const password=pi.value,name=ni.value.trim();if(create&&password!==confirm.value){state.error='The passwords do not match.';render();return;}pi.value='';if(confirm)confirm.value='';perform(()=>signIn(name,password,create));};stack.append(f,button('Back to sign-in options',()=>{state.phase='options';render();},'plain'));}
 else if(state.phase==='device'){const d=state.device;stack.append(node('h2',d?.kind==='passkey'?'Sign in with a passkey':'Quick Connect'),node('p',d?.code||'','confirmation-code'));if(d?.url){const link=node('a','Open passkey page');link.href=d.url;link.target='_blank';link.rel='noopener noreferrer';stack.append(link,node('p','After verifying, return here to finish.','secondary'));}else stack.append(node('p','Enter this code in another client signed in to your server.'));stack.append(action('Finish sign-in',finishDevice,'primary'),action('Cancel',cancelDevice));}
 if(state.busy)stack.append(node('p','Connecting…','loading'));stack.append(status());main.append(stack);return main;}
function libraryView(){const main=node('main',undefined,'app-main');main.id='content';const route=state.route,title=route.title||(route.kind==='detail'?route.item.Name:state.tab);main.append(toolbar(title,state.back.length>0));if(state.back.length)main.append(node('h1',title));
 if(route.kind==='home'){const feature=state.shelves.find(s=>s.items.length)?.items[0];if(feature){const box=node('section',undefined,'featured'),hero=node('div',undefined,'hero');bindArtwork(hero,feature,true);hero.append(node('small','IN YOUR LIBRARY'),node('h2',feature.Name));if(subtitle(feature))hero.append(node('p',subtitle(feature)));box.append(hero);const a=node('div',undefined,'actions');if(['Movie','Episode','Audio','TvChannel'].includes(feature.Type))a.append(action(resumeTicks(feature)>0?'Resume':'Play',()=>play(feature,[feature],0),'primary','play'));a.append(action('Details',()=>openItem(feature),'','info'));box.append(a);main.append(box);}for(const s of state.shelves)main.append(shelf(s.title,s.items));if(!feature&&!state.busy)main.append(empty('Browse your library','Open Library to see your collections.'));}
 else if(route.kind==='library'){const collections=group('Collections');for(const item of state.rows)collections.append(row(item));if(!state.rows.length&&!state.busy)collections.append(node('p','No libraries are available to this account.','secondary'));main.append(collections);}
 else if(route.kind==='music'){const browse=node('div',undefined,'music-browse');for(const [name,type,symbol]of[['Songs','Audio','music'],['Albums','MusicAlbum','library'],['Artists','MusicArtist','account'],['Playlists','Playlist','queue']])browse.append(action(name,()=>openCollection(name,type,null),'',symbol));main.append(browse);for(const s of state.shelves)main.append(shelf(s.title,s.items));if(!state.shelves.some(s=>s.items.length)&&!state.busy)main.append(empty('Your music, in one place','Albums from your connected library will appear here. Browse Songs to find individual tracks.'));}
 else if(route.kind==='search'){const f=node('form',undefined,'search-form'),i=node('input');i.type='search';i.placeholder='Search your library';i.value=state.query;i.setAttribute('aria-label','Search your library');const b=node('button','Search');f.append(i,b);f.onsubmit=e=>{e.preventDefault();state.query=i.value.trim().slice(0,256);perform(loadRoute);};main.append(f);const results=node('div',undefined,'catalog-grid');for(const item of state.rows)results.append(poster(item));main.append(results);if(!state.rows.length&&!state.busy)main.append(empty('Search your library',state.query?'No results. Try another title, artist or album.':'Find movies, shows, artists, albums and songs.'));}
 else if(route.kind==='collection'||route.kind==='live'){const audio=route.type==='Audio';if(audio&&state.rows.length){const a=node('div',undefined,'actions');a.append(action('Play',()=>play(state.rows[0],state.rows,0),'','play'),action('Shuffle',async()=>{state.shuffle=true;const order=shuffled(state.rows);await play(order[0],order,0);},'','shuffle'));main.append(a);}const list=node('div',undefined,audio||route.kind==='live'?'group':'catalog-grid');for(const item of state.rows)list.append(audio||route.kind==='live'?row(item):poster(item));main.append(list);if(!state.rows.length&&!state.busy)main.append(empty('No items to show','Try another collection or refresh your library.'));if(route.kind==='collection'){const a=node('div',undefined,'actions');if(route.start>0)a.append(action('Previous page',async()=>{route.start=Math.max(0,route.start-100);await loadRoute();}));if(route.start+state.rows.length<route.total)a.append(action('Next page',async()=>{route.start+=100;await loadRoute();}));main.append(a);}}
 else if(route.kind==='detail'){const item=route.item;main.append(cover(item,item.Type==='Audio'));main.lastChild.style.maxWidth='360px';main.append(node('h2',item.Name),node('p',subtitle(item),'secondary'));if(['Movie','Episode','Audio','TvChannel'].includes(item.Type))main.append(action(resumeTicks(item)>0?'Resume':'Play',()=>play(item,[item],0),'primary','play'));main.append(node('p',item.Overview||''));}
 if(state.busy)main.append(node('p','Loading your library…','loading'));main.append(status());if(state.error)main.append(action('Try again',loadRoute));return main;}
function tabbar(){const nav=node('nav',undefined,'tabbar');nav.setAttribute('aria-label','Library');for(const [title,symbol]of[['Home','home'],['Library','library'],['Music','music'],['Search','search'],['Live TV','live']]){const b=action(title,()=>navigate(title),'',symbol);b.setAttribute('aria-current',state.tab===title?'page':'false');nav.append(b);}return nav;}
function sheet(){if(state.expanded)return nowPlaying();const outer=node('section',undefined,'sheet');outer.setAttribute('role','dialog');outer.setAttribute('aria-modal','true');outer.id='bluefin-dialog';outer.tabIndex=-1;outer.setAttribute('aria-label',state.sheet==='invite'?'Connect Bluefin':state.sheet==='connections'?'Connections':'Account and settings');const inner=node('div',undefined,'sheet-inner'),bar=node('div',undefined,'toolbar');bar.append(node('h2',state.sheet==='invite'?'Connect Bluefin':state.sheet==='connections'?'Connections':'Account and settings'),button('Done',closeSheet,'plain'));inner.append(bar);
 if(state.sheet==='invite'){const g=group(state.pairing?'Saved connection':'Connect with an invitation');if(state.pairing){g.append(node('p',state.remember?'This request is saved on this device.':'This request stays in this tab. Keep it open until your connection is complete.'));if(state.pairing.awaitingApproval)g.append(node('p','Your request is waiting for administrator approval.'));if(state.pairing.externalUrl){const u=new URL(state.pairing.externalUrl);if(u.protocol==='https:'&&!u.username&&!u.password){const a=node('a','Open passkey page');a.href=u.href;a.target='_blank';a.rel='noopener noreferrer';g.append(a,node('p','After verifying, return here to finish.','secondary'));}}g.append(action(state.pairing.awaitingApproval?'Check approval':state.pairing.needsBeginRetry?'Resume connection':'Verify and finish',continueMesh,'primary'));}
 else{g.append(node('p','Scan a Bluefin QR code, open an invitation image, or paste the full connection code.'));const f=node('form'),[cw,ci]=field('Connection code','password',state.code,'mesh-code'),[nw,ni]=field('Your account name for a signup request','text',state.meshName,'mesh-name');ni.oninput=()=>{state.meshName=ni.value;};ci.autocomplete='off';ci.spellcheck=false;ci.maxLength=3072;ci.oninput=()=>{state.code=ci.value;};f.append(cw,nw);const remember=node('label',undefined,'check'),c=node('input');c.type='checkbox';c.checked=state.remember;c.onchange=()=>{state.remember=c.checked;};remember.append(c,node('span','Remember this device in protected browser storage'));f.append(remember,node('p','Temporary by default. Closing this tab removes its connection key. Use Sign out to request immediate remote revocation.','secondary'));const b=node('button','Connect securely','primary');b.disabled=state.busy;f.append(b);f.onsubmit=e=>{e.preventDefault();const code=ci.value.trim(),name=ni.value.trim(),keep=c.checked;ci.value='';perform(()=>beginMesh(code,name,keep));};g.append(f);for(const [label,capture]of[['Open QR image or screenshot',false],['Take a photo of a QR code',true]]){const l=node('label',label),input=node('input');input.type='file';input.accept='image/png,image/jpeg';if(capture)input.setAttribute('capture','environment');input.onchange=()=>{const file=input.files?.[0];input.value='';if(!file)return;perform(async()=>{if(file.size<=0||file.size>8*1024*1024)throw Error('Choose a PNG or JPEG smaller than 8 MiB.');const engine=await runtime().engine();state.code=engine.webMeshImage(new Uint8Array(await file.arrayBuffer()));state.notice='Invitation read. Continue to connect.';});};l.append(input);g.append(l);}if(/^MC1-[A-Z0-9-]+$/i.test(state.code)&&state.code.length<=2048){const a=node('a','Open in the installed Bluefin app');a.href='bluefin://connect/'+state.code;g.append(a);}}
 g.append(action('Resume request in this tab',()=>resumeMesh(false)),action('Restore a remembered device',()=>resumeMesh(true)));inner.append(g,node('p','The QR verifies a device connection. Your browser still uses its normal trusted HTTPS certificate.','secondary'));
 }else if(state.sheet==='account'){const g=group(state.session?.name||'Your account');g.append(node('p',state.mode==='mesh'?'Connected through the encrypted Bluefin mesh':'Signed in through the server address'));g.append(button('Connections',()=>{state.sheet='connections';render();},'','qr'),action('Sign out',signOut,'danger'));inner.append(g);}
 else if(state.sheet==='connections'){const device=group('This device');device.append(node('p',state.mode==='mesh'?'Connected through the encrypted Bluefin mesh':'Signed in through the server address'));if(state.mode==='mesh')device.append(node('p',state.remember?'Your connection key is saved in protected storage on this browser.':'Your connection key exists only in this tab.','secondary'));inner.append(device);const g=group('Connect another device to your account');g.append(node('p','Use this invitation on a device you own. Your account passkey is still required.'),action('Create one-use invitation',issueInvite));if(state.invitation){const invite=state.invitation,image=node('img',undefined,'qr');image.src=invite.image;image.alt='One-use Bluefin device invitation';g.append(image,node('p','Every copy is the same invitation. Once claimed, no copy can connect another device.','secondary'));g.append(button('Copy invitation',()=>copy(invite.webLink)),button('Copy connection code',()=>copy(invite.code)));const save=node('a','Save QR image');save.href=invite.image;save.download='bluefin-invitation.png';g.append(save);const d=node('details'),summary=node('summary','Manual connection code');d.append(summary,node('p',invite.code,'code'));g.append(d);}g.append(node('p','Sharing a library with another person requires a separate grant to that person’s own account. A device invitation does not authorize shared storage or onward sharing.','secondary'));inner.append(g);}
 if(state.busy)inner.append(node('p','Connecting…','loading'));inner.append(status());outer.append(inner);return outer;}
async function copy(value){try{await navigator.clipboard.writeText(value);state.notice='Copied.';}catch(_){state.error='Copy was not available. Open the manual code and select it.';}render();}
async function issueInvite(){state.invitation=null;clearTimeout(state.inviteTimer);const response=await api('Bluefin/Accounts/Mesh/Invite','POST',{intent:'login',lifetimeSeconds:600});let result;if(state.mode==='mesh')result=JSON.parse((await runtime().engine()).webMeshInvitation(JSON.stringify(response)));else{const link=new URL(response.webLink||response.link),code=response.code,png=response.pngBase64;if(response.singleUse!==true||response.intent!=='login'||!/^MC1-[A-Z0-9-]+$/i.test(code||'')||code.length>2048||link.origin!==location.origin||link.protocol!=='https:'||!link.hash.startsWith('#bluefin-connect=')||typeof png!=='string'||png.length>1400000||!/^[A-Za-z0-9+/]*={0,2}$/.test(png)||!/^[a-f0-9]{64}$/.test(response.handle||''))throw Error('The server returned an invalid invitation.');result={webLink:link.href,code,image:'data:image/png;base64,'+png,handle:response.handle};}state.invitation=result;const epoch=state.epoch;const poll=async()=>{if(epoch!==state.epoch||state.sheet!=='connections'||state.invitation!==result)return;try{const status=await api('Bluefin/Accounts/Mesh/Invites/'+result.handle);if(status.state!=='issued'){state.invitation=null;state.notice='This invitation is no longer available. Create a new invitation for another device.';render();return;}}catch(_){state.invitation=null;state.notice='The invitation status could not be confirmed.';render();return;}state.inviteTimer=setTimeout(poll,3000);};state.inviteTimer=setTimeout(poll,3000);}
function shuffled(items){const values=items.slice();for(let i=values.length-1;i>0;i--){const draw=new Uint32Array(1);crypto.getRandomValues(draw);const j=draw[0]%(i+1);[values[i],values[j]]=[values[j],values[i]];}return values;}
function time(value){if(!Number.isFinite(value)||value<0)value=0;return Math.floor(value/60)+':'+String(Math.floor(value%60)).padStart(2,'0');}
function nativeLeasePath(raw,kind,idValue){if(typeof raw!=='string'||raw.length>256||!raw.startsWith('/Bluefin/Web/v1/')||raw.includes('%')||raw.includes('?')||raw.includes('#'))throw Error('Invalid playback lease route.');const u=new URL(raw,location.origin);if(u.origin!==location.origin||u.username||u.password||u.hash||u.search)throw Error('Invalid playback lease origin.');const pattern=kind==='media'?/^\/Bluefin\/Web\/v1\/media\/([a-f0-9]{64})\/[0-9]+$/:/^\/Bluefin\/Web\/v1\/leases\/([a-f0-9]{64})$/;const match=u.pathname.match(pattern);if(!match||(idValue&&match[1]!==idValue))throw Error('Invalid playback lease scope.');return {url:u.href,id:match[1]};}
async function leaseRequest(raw,method,session=state.session,keepalive=false){
 if(!keepalive)assertLive();const path=nativeLeasePath(raw,'lease');
 const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),10000);
 if(!keepalive)requestControllers.add(controller);
 try{const response=await fetch(path.url,{method,headers:{Authorization:authorization(session?.token||''),'Content-Type':'application/json'},
  body:method==='POST'?'{}':undefined,credentials:'same-origin',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal,keepalive});
  if(!response.ok)throw Error('Playback cleanup or renewal was not acknowledged.');
  return await readJson(response,8192);
 }finally{requestControllers.delete(controller);clearTimeout(timer);}
}
async function hlsSupport(){
 if(media.canPlayType('application/vnd.apple.mpegurl'))return null;
 if(!hlsPromise)hlsPromise=runtime().bounded(new Promise((resolve,reject)=>{
  const script=node('script');script.src=new URL('./vendor/hls.light.min.js',document.baseURI).href;
  script.onload=()=>globalThis.Hls?.isSupported()?resolve(globalThis.Hls):reject(Error('This browser cannot decode the available HLS profile.'));
  script.onerror=()=>reject(Error('The installed HLS decoder could not load.'));document.head.append(script);
 }),6000,'The installed HLS decoder did not load in time.').catch(error=>{hlsPromise=null;throw error;});
 return hlsPromise;
}
function clearOutput(previous){
 let failure;
 // Pause and remove the source before asking any decoder to dispose itself.
 try{media.pause();}catch(error){failure=error;}
 try{media.removeAttribute('src');}catch(error){failure=failure||error;}
 try{media.load();}catch(error){failure=failure||error;}
 try{previous?.hls?.destroy();}catch(error){failure=failure||error;}
 document.body.classList.remove('audio-playing','video-playing','with-player');
 return failure;
}
async function stopPlayback(remote=true){
 const previous=state.player;state.player=null;state.expanded=false;
 clearInterval(leaseRenew);clearInterval(progressTimer);clearInterval(meshPoll);
 leaseRenew=progressTimer=meshPoll=null;playbackGeneration++;
 let failure=clearOutput(previous);
 try{
  if(previous?.mode==='mesh'){const engine=runtime()?.ready();if(engine)engine.webMeshControl(previous.handle,'stop',0);}
  else if(previous?.lease&&remote)await leaseRequest(previous.lease.releaseUrl,'DELETE',previous.session);
 }catch(error){failure=failure||error;}
 document.getElementById('media-status').textContent='';
 if(failure)throw failure;
}
function resumeTicks(item){
 const value=item.UserData?.PlaybackPositionTicks,duration=item.RunTimeTicks;
 return Number.isSafeInteger(value)&&value>0&&Number.isSafeInteger(duration)&&value<duration?value:0;
}
async function play(item,queue=[item],index=0){
 const epoch=state.epoch,session=state.session,mode=state.mode,base=state.base;assertLive(epoch,session);
 if(!session)throw Error('Sign in to play your library.');
 if(!['Audio','Movie','Episode','TvChannel'].includes(item.Type))return openItem(item);
 await stopPlayback();assertLive(epoch,session);
 const generation=playbackGeneration;
 const current=()=>{assertLive(epoch,session);if(generation!==playbackGeneration)throw Error('Playback was canceled.');};
 state.queue=queue.slice(0,512);state.queueIndex=index;
 if(mode==='mesh'){
  await hlsSupport();current();
  await runtime().mediaWorker();current();
  const engine=await runtime().engine();current();
  engine.webMeshPlay(session.handle,JSON.stringify(item));current();
  state.player={mode:'mesh',handle:session.handle,item,session};
  meshPoll=setInterval(()=>{
   if(pageRetired||generation!==playbackGeneration||session!==state.session)return;
   try{const sample=JSON.parse(engine.webMeshPlayback(session.handle));if(sample?.message)document.getElementById('media-status').textContent=sample.message;updatePlayback();}
   catch(_){if(generation===playbackGeneration)stopPlayback(false).catch(()=>{});}
  },500);
 }else{
  const Hls=await hlsSupport();current();
  const available=await http('Bluefin/Web/v1','GET',undefined,[],session,false,false,base);current();
  if(available?.version!==1||available.cookieMediaLease!==true)throw Error('This installation does not yet provide browser playback without a media worker. Your library remains available.');
  const lease=await http('Bluefin/Web/v1/leases','POST',{version:1,itemId:id(item.Id),
   playback:{kind:item.Type==='Audio'?'audio':'video',maxBitrate:8000000,startTicks:resumeTicks(item)}},[],session,false,true,base);
  let route;
  try{
   route=nativeLeasePath(lease.mediaUrl,'media');nativeLeasePath(lease.releaseUrl,'lease',route.id);nativeLeasePath(lease.renewUrl,'lease',route.id);
   if(lease.version!==1||lease.expiresInSeconds!==300)throw Error('Invalid playback lease.');
  }catch(error){if(typeof lease?.releaseUrl==='string')await leaseRequest(lease.releaseUrl,'DELETE',session,true).catch(()=>{});throw error;}
  if(pageRetired||epoch!==state.epoch||session!==state.session||generation!==playbackGeneration){
   await leaseRequest(lease.releaseUrl,'DELETE',session,true).catch(()=>{});current();
  }
  state.player={mode:'https',item,lease,session,offset:resumeTicks(item)/10000000};
  if(Hls){
   const hls=new Hls({enableWorker:false,maxBufferLength:20,maxMaxBufferLength:40,backBufferLength:10,maxBufferSize:32*1024*1024,
    manifestLoadingTimeOut:10000,fragLoadingTimeOut:10000,fragLoadingMaxRetry:1,manifestLoadingMaxRetry:1,
    xhrSetup(xhr,url){const target=new URL(url,location.origin);if(target.origin!==location.origin||target.search||target.hash||target.username||target.password)throw Error('Media left its lease origin.');nativeLeasePath(target.pathname,'media',route.id);}});
   state.player.hls=hls;hls.on(Hls.Events.ERROR,(_event,data)=>{
    if(data.fatal&&!pageRetired&&generation===playbackGeneration){stopPlayback().catch(()=>{});state.error='The media decoder could not play this title.';render();}
   });hls.loadSource(route.url);hls.attachMedia(media);
  }else{media.src=route.url;media.load();}
  leaseRenew=setInterval(async()=>{
   if(pageRetired||generation!==playbackGeneration||session!==state.session)return;
   try{await leaseRequest(lease.renewUrl,'POST',session);}
   catch(_){if(pageRetired||generation!==playbackGeneration||session!==state.session)return;
    await stopPlayback(false).catch(()=>{});state.error='Playback stopped because its access lease could not be renewed.';render();}
  },60000);
  progressTimer=setInterval(()=>{if(!pageRetired&&generation===playbackGeneration)reportProgress('progress').catch(()=>{});},10000);
  reportProgress('started').catch(()=>{});current();
  media.play().catch(()=>{if(!pageRetired&&generation===playbackGeneration)document.getElementById('media-status').textContent='Press Play to begin.';});
 }
 current();document.body.classList.toggle('audio-playing',item.Type==='Audio');
 document.body.classList.toggle('video-playing',item.Type!=='Audio');document.body.classList.add('with-player');render();
}
async function reportProgress(phase){const p=state.player;if(!p||p.mode!=='https'||p.session!==state.session)return;await api('Sessions/Playing'+(phase==='started'?'':phase==='stopped'?'/Stopped':'/Progress'),'POST',{ItemId:p.item.Id,PlaySessionId:p.lease.playSessionId,PositionTicks:Math.floor((media.currentTime||0)*10000000),IsPaused:media.paused,CanSeek:true,PlayMethod:'Transcode'});}
async function skip(forward,automatic=false){if(!state.player||!state.queue.length)return;let next=state.queueIndex+(forward?1:-1);if(automatic&&state.repeat==='one')next=state.queueIndex;if(next>=state.queue.length){if(state.repeat!=='all')return stopPlayback();next=0;}if(next<0){if(state.repeat!=='all')next=0;else next=state.queue.length-1;}await play(state.queue[next],state.queue,next);}
function toggle(){if(media.paused)media.play().catch(()=>{state.error='Press Play in the media controls.';render();});else media.pause();updatePlayback();}
function miniPlayer(){const p=state.player;if(!p||p.item.Type!=='Audio')return document.createDocumentFragment();const e=node('section',undefined,'mini-player');e.setAttribute('aria-label','Now playing');e.append(cover(p.item,true));const title=button('',()=>{state.expanded=true;render();},'grow');title.append(node('strong',p.item.Name),node('small',subtitle(p.item)));e.append(title);const playButton=button('',toggle,'plain',media.paused?'play':'pause');playButton.setAttribute('aria-label',media.paused?'Play':'Pause');playButton.dataset.playToggle='';e.append(playButton);const next=action('',()=>skip(true),'plain','next');next.setAttribute('aria-label','Next track');next.disabled=state.busy||(state.repeat!=='all'&&state.queueIndex+1>=state.queue.length);e.append(next);return e;}
function nowPlaying(){const p=state.player;if(!p){state.expanded=false;return document.createDocumentFragment();}const e=node('section',undefined,'sheet');e.setAttribute('role','dialog');e.setAttribute('aria-modal','true');e.setAttribute('aria-label','Now Playing');e.id='bluefin-dialog';e.tabIndex=-1;const inner=node('div',undefined,'now-playing'),bar=node('div',undefined,'toolbar');const close=button('',()=>{state.expanded=false;render();},'plain','down');close.setAttribute('aria-label','Close Now Playing');bar.append(close,node('span','NOW PLAYING','now-caption'),action('Stop',()=>stopPlayback(),'plain'));inner.append(bar);const layout=node('div',undefined,'now-layout'),art=node('div',undefined,'now-art');art.append(icon('music'));bindArtwork(art,p.item);layout.append(art);const controls=node('div',undefined,'now-controls');controls.append(node('h1',p.item.Name),node('p',subtitle(p.item)));const seek=node('input');seek.type='range';seek.min='0';seek.max=String(Number.isFinite(media.duration)?media.duration:1);seek.step='.1';seek.value=String(media.currentTime||0);seek.setAttribute('aria-label','Playback position');seek.dataset.seek='';seek.oninput=()=>{if(Number.isFinite(Number(seek.value)))media.currentTime=Number(seek.value);};const times=node('div',undefined,'time-row'),position=node('span',time(media.currentTime)),remaining=node('span','−'+time(media.duration-media.currentTime));position.dataset.position='';remaining.dataset.remaining='';times.append(position,remaining);controls.append(seek,times);const transport=node('div',undefined,'transport');const shuffle=button('',()=>{state.shuffle=!state.shuffle;if(state.shuffle){const current=state.queue[state.queueIndex];state.queue=[current,...shuffled(state.queue.filter((_,i)=>i!==state.queueIndex))];state.queueIndex=0;}render();},state.shuffle?'on':'','shuffle');shuffle.setAttribute('aria-label','Shuffle');shuffle.setAttribute('aria-pressed',String(state.shuffle));const previous=action('',()=>skip(false),'','previous');previous.setAttribute('aria-label','Previous track');const toggleButton=button('',toggle,'main-play',media.paused?'play':'pause');toggleButton.dataset.playToggle='';toggleButton.setAttribute('aria-label',media.paused?'Play':'Pause');const next=action('',()=>skip(true),'','next');next.setAttribute('aria-label','Next track');const repeat=button('',()=>{state.repeat=({off:'all',all:'one',one:'off'})[state.repeat];render();},state.repeat==='off'?'':'on','repeat');repeat.setAttribute('aria-label','Repeat: '+state.repeat);transport.append(shuffle,previous,toggleButton,next,repeat);controls.append(transport);const queue=node('details'),summary=node('summary','Playing next');queue.append(summary);for(const track of state.queue.slice(state.queueIndex+1))queue.append(node('p',track.Name));controls.append(queue);layout.append(controls);inner.append(layout,status());e.append(inner);return e;}
function updatePlayback(){for(const b of document.querySelectorAll('[data-play-toggle]')){b.replaceChildren(icon(media.paused?'play':'pause'));b.setAttribute('aria-label',media.paused?'Play':'Pause');}for(const e of document.querySelectorAll('[data-position]'))e.textContent=time(media.currentTime);for(const e of document.querySelectorAll('[data-remaining]'))e.textContent='−'+time(media.duration-media.currentTime);for(const e of document.querySelectorAll('[data-seek]')){e.max=String(Number.isFinite(media.duration)?media.duration:1);if(document.activeElement!==e)e.value=String(media.currentTime||0);}}
async function signOut(){
 const session=state.session,mode=state.mode,base=state.base;state.epoch++;state.pairingView++;
 clearTimeout(state.inviteTimer);state.sheet=null;state.phase='connect';state.session=null;clearPrivateView();clearImages();
 let cleanup;
 try{await stopPlayback();}catch(error){cleanup=error;}
 try{if(session){if(mode==='mesh')await(await runtime().engine()).webMeshLogout(session.handle);
  else await http('Sessions/Logout','POST',undefined,[],session,false,true,base);}}
 catch(error){cleanup=error;}
 if(pageRetired)return;
 state.mode='https';state.base=hostedBase();state.server=null;state.options={};state.remember=false;
 if(cleanup)throw Error('Signed out in this tab. Remote cleanup is not confirmed; '+String(cleanup.message||cleanup));
}
function clearImages(){state.imageEpoch++;artworkMissing.clear();for(const url of state.images.values())URL.revokeObjectURL(url);state.images.clear();}
function render(){
 if(pageRetired)return;
 const active=document.activeElement,activeId=active?.id,activeKey=active?.dataset?.focusKey;
 const videoModal=!!state.player&&state.player.item.Type!=='Audio'&&!state.sheet&&!state.expanded;
 const modal=!!state.sheet||state.expanded||videoModal;
 if(modal&&!modalWasOpen)focusReturn={id:activeId,key:activeKey};
 document.body.classList.toggle('signed-in',!!state.session);
 const fragment=document.createDocumentFragment(),main=state.session?libraryView():welcome();
 if(modal){main.inert=true;main.setAttribute('aria-hidden','true');}fragment.append(main);
 if(state.session){const nav=tabbar(),mini=miniPlayer();if(modal){nav.inert=true;mini.inert=true;nav.setAttribute('aria-hidden','true');mini.setAttribute('aria-hidden','true');}fragment.append(nav,mini);}
 if(state.sheet||state.expanded)fragment.append(sheet());app.replaceChildren(fragment);
 const host=document.getElementById('media-host');host.inert=!videoModal;host.setAttribute('aria-hidden',String(!videoModal));
 if(videoModal){host.setAttribute('role','dialog');host.setAttribute('aria-modal','true');host.setAttribute('aria-label',state.player.item.Name);host.tabIndex=-1;}
 else{host.removeAttribute('role');host.removeAttribute('aria-modal');}
 const findFocus=hint=>hint?.id?document.querySelector?.('#'+CSS.escape(hint.id)):
   [...document.querySelectorAll('[data-focus-key]')].find(e=>e.dataset.focusKey===hint?.key);
 let focus;
 if(modal&&!modalWasOpen)focus=videoModal?host:document.querySelector?.('#bluefin-dialog');
 else if(!modal&&modalWasOpen){focus=findFocus(focusReturn);focusReturn=null;}
 else focus=findFocus({id:activeId,key:activeKey});
 modalWasOpen=modal;
 if(focus&&!focus.inert)focus.focus?.({preventScroll:true});
 pumpArtwork();
}
document.addEventListener('keydown',event=>{
 const dialog=document.querySelector?.('#bluefin-dialog')||
  (state.player?.item.Type!=='Audio'&&state.player?document.getElementById('media-host'):null);
 if(!dialog)return;
 if(event.key==='Escape'){event.preventDefault();if(state.sheet||state.expanded)closeSheet();else perform(()=>stopPlayback());return;}
 if(event.key!=='Tab')return;
 const controls=[...dialog.querySelectorAll('button:not([disabled]),a[href],input:not([disabled]),select:not([disabled]),summary,video[controls]')]
  .filter(e=>!e.hidden&&e.getAttribute('aria-hidden')!=='true'&&e.getClientRects().length);
 if(!controls.length){event.preventDefault();dialog.focus();return;}
 const first=controls[0],last=controls[controls.length-1];
 if(!dialog.contains(document.activeElement)||document.activeElement===dialog){event.preventDefault();(event.shiftKey?last:first).focus();}
 else if(event.shiftKey&&document.activeElement===first){event.preventDefault();last.focus();}
 else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus();}
});

media.addEventListener('play',updatePlayback);media.addEventListener('pause',updatePlayback);media.addEventListener('timeupdate',updatePlayback);
media.addEventListener('ended',()=>{if(pageRetired||!state.player||!media.ended)return;perform(async()=>{await reportProgress('stopped').catch(()=>{});await skip(true,true);});});
media.addEventListener('error',()=>{if(pageRetired||!media.getAttribute('src'))return;state.error='This media could not play. Try another title or refresh your library.';render();});
window.addEventListener('pagehide',()=>{
 if(pageRetired)return;
 const p=state.player,session=state.session,mode=state.mode,base=state.base;
 pageRetired=true;state.epoch++;state.pairingView++;playbackGeneration++;
 for(const controller of requestControllers)controller.abort();requestControllers.clear();
 clearTimeout(state.inviteTimer);clearInterval(leaseRenew);clearInterval(progressTimer);clearInterval(meshPoll);
 state.player=null;clearOutput(p);
 try{runtime()?.ready()?.webMeshPageExit();}catch(_){}
 if(mode!=='mesh'){
  if(p?.lease)leaseRequest(p.lease.releaseUrl,'DELETE',p.session,true).catch(()=>{});
  if(session)http('Sessions/Logout','POST',undefined,[],session,true,true,base).catch(()=>{});
 }
 state.session=null;clearPrivateView();clearImages();
 app.replaceChildren(node('p','This browser page is closed. Reopen Bluefin to connect.','notice'));
});
window.addEventListener('pageshow',event=>{if(event.persisted)location.reload();});
const videoBar=node('div',undefined,'video-toolbar');
videoBar.append(button('Done',()=>perform(()=>stopPlayback()),'plain'),
 button('Full screen',()=>{if(media.requestFullscreen)media.requestFullscreen().catch(()=>{});else if(media.webkitEnterFullscreen)media.webkitEnterFullscreen();},'plain'));
document.getElementById('media-host').append(videoBar);
document.getElementById('startup-status')?.remove();
render();
})();
