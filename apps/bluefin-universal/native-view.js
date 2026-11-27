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
  code:launched,remember:false,pairing:null,pairingView:0,device:null,invitation:null,inviteTimer:null,
  busy:false,error:'',notice:'',epoch:0,capabilities:null,capability:null,player:null,expanded:false,nowPanel:'artwork',
  featured:null,homeHistory:null,facets:null,search:null,
  queue:[],unshuffledQueue:[],queueIndex:0,shuffle:false,repeat:'off',query:'',local:false,localQuery:'',localSnapshot:null,images:new Map(),imageEpoch:0};
let modalWasOpen=false,focusReturn=null,pageRetired=false,authTask=false,playbackGeneration=0,leaseRenew=null,progressTimer=null,meshPoll=null,hlsPromise,meshRetirement=null;
let meshBeginStarted=false;
let localLibrary=null,routeController=null,lyricsView=null,lyricsFrame=null;
const categories=[['Home','home'],['Sports','sports'],['TV Shows','shows'],['Movies','movies'],['Live TV','live'],['Music','music']];
const searchScopes=[['all','All',null],['movies','Movies','Movie'],['tvShows','TV Shows','Series,Season,Episode'],['music','Music','Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist'],['sports','Sports',null],['liveTv','Live TV','TvChannel,LiveTvProgram']];
function aborted(){return Object.assign(Error('The search or library view changed.'),{name:'AbortError'});}
function notFound(error){return error?.status===404||/\bHTTP 404\b/.test(String(error?.message||error||''));}

function isLocalItem(item){return item?.LocalSource===true&&globalThis.BluefinLocalMusic?.isLocalId(item.Id)===true;}
function localItem(record){return {Id:record.id,Name:record.title,Type:'Audio',LocalSource:true,Source:record.source,
 Artists:['On this device'],Album:record.format.toUpperCase(),RunTimeTicks:record.durationSeconds?Math.floor(record.durationSeconds*10000000):0};}
function localItems(){return (state.localSnapshot?.items||[]).map(localItem);}
function localOwner(){
 assertLive();
 if(!localLibrary){
  if(!globalThis.BluefinLocalMusic)throw Error('The local music controls did not load. Reload this page.');
  localLibrary=globalThis.BluefinLocalMusic.create({onChange:snapshot=>{
   if(pageRetired)return;state.localSnapshot=snapshot;
   if(state.player?.mode==='local'){
    const records=new Map(snapshot.items.map(record=>[record.id,localItem(record)]));
    state.queue=snapshot.queue.items.map(id=>records.get(id)).filter(Boolean);state.queueIndex=snapshot.queue.currentIndex||0;
    state.unshuffledQueue=state.unshuffledQueue.filter(item=>records.has(item.Id));
    if(records.has(state.player.item.Id))state.player.item=records.get(state.player.item.Id);
   }
   if(state.route.kind==='local_music')state.rows=snapshot.items.map(localItem);
   else if(state.route.kind==='local_search')state.rows=snapshot.items.filter(item=>item.title.toLowerCase().includes(state.localQuery.trim().toLowerCase())).map(localItem);
   render();
  }});
 }
 return localLibrary;
}
function openLocalMusic(){
 assertLive();localOwner();state.local=true;state.phase='library';state.tab='Music';
 state.route={kind:'local_music'};state.back=[];state.sheet=null;state.rows=localItems();state.shelves=[];
 state.notice='Files stay in this tab. They are never uploaded.';render();
}
async function addLocalFiles(files){
 const epoch=state.epoch,owner=localOwner();assertLive(epoch);
 await owner.addFiles(files);assertLive(epoch);
 state.rows=state.route.kind==='local_search'?owner.search(state.localQuery).map(localItem):localItems();
 state.notice='Music opened in this tab. Reloading or closing the page releases it.';
}
function localPicker(){
 const label=node('label','Import music','file-action'),input=node('input');
 input.type='file';input.accept='.mp3,.aac,.m4a,.flac,.wav,.aif,.aiff,audio/mpeg,audio/aac,audio/mp4,audio/flac,audio/wav,audio/aiff';
 input.multiple=true;input.disabled=state.busy;input.className='visually-hidden';input.setAttribute('aria-label','Import music');
 input.onchange=()=>{const selected=Array.from(input.files||[]);input.value='';if(selected.length)perform(()=>addLocalFiles(selected));};
 label.append(input);return label;
}
async function openAccountConnection(){await stopPlayback();assertLive();state.local=false;state.phase='connect';state.route={kind:'home'};state.rows=[];state.shelves=[];state.back=[];state.query='';state.sheet=null;render();}
function localMusicView(main){
 main.append(node('h2','Music on this device'),node('p','Temporary files · never uploaded','secondary'));
 if(state.route.kind==='local_music'||state.route.kind==='local_search'){
  const form=node('form',undefined,'search-form'),input=node('input');input.type='search';input.value=state.localQuery;
  input.placeholder='Search music on this device';input.setAttribute('aria-label','Search music on this device');
  const submit=node('button','Search','primary');form.append(input,submit);
  form.onsubmit=event=>{event.preventDefault();perform(async()=>{
   state.localQuery=input.value;state.rows=localOwner().search(state.localQuery).map(localItem);state.route={kind:'local_search'};
  });};main.append(form);
 }
 const actions=node('div',undefined,'actions');actions.append(localPicker());
 if(state.rows.length)actions.append(action('Play',()=>playLocal(state.rows[0],state.rows,0),'primary','play'),
  action('Shuffle',()=>playShuffled(state.rows),'','shuffle'));
 main.append(actions);
 const list=node('div',undefined,'group');
 for(const item of state.rows){
  const entry=node('div',undefined,'local-track');entry.append(row(item));
  const remove=action('Remove',()=>localOwner().remove(item.Id),'plain');remove.setAttribute('aria-label','Remove '+item.Name+' from this tab');
  entry.append(remove);list.append(entry);
 }
 main.append(list);
 if(!state.rows.length)main.append(empty(state.route.kind==='local_search'?'No matching music':'Open your music',
  'Choose up to 20 MP3, AAC, M4A, FLAC, WAV, or AIFF files. Files remain on your device; this tab keeps permission only while it stays open.'));
 main.append(node('p','Selected files are separate from server libraries, streaming subscriptions, storage donation and content-sharing grants.','secondary'));
}
function installQueue(queue,index,original,preserve){
 state.queue=queue.slice(0,512);state.queueIndex=index;
 if(original){state.unshuffledQueue=original.slice(0,512);state.shuffle=true;}
 else if(!preserve){state.unshuffledQueue=state.queue.slice();state.shuffle=false;}
}
async function playShuffled(items){
 const original=items.slice(0,512);if(!original.length)return;
 const order=shuffled(original);await play(order[0],order,0,original);
}
async function importInvitationImage(file){
 const epoch=state.epoch,view=state.pairingView;
 const current=()=>{assertLive(epoch);if(view!==state.pairingView||state.sheet!=='invite')throw Error('The connection screen was closed.');};
 current();if(file.size<=0||file.size>8*1024*1024)throw Error('Choose a PNG or JPEG smaller than 8 MiB.');
 const engine=await meshEngine(()=>importInvitationImage(file),canReloadBeforeMeshBegin(),false,view);current();
 const bytes=new Uint8Array(await file.arrayBuffer());current();
 const code=engine.webMeshImage(bytes);current();state.code=code;state.notice='Invitation read. Continue to connect.';
}

function syncLocalQueue(){
 if(state.player?.mode==='local')localOwner().setQueue({items:state.queue.map(item=>item.Id),
  currentIndex:state.queue.length?state.queueIndex:null,repeatMode:state.repeat,shuffled:state.shuffle});
}
async function playLocal(item,queue=[item],index=0,original=null){
 const epoch=state.epoch;assertLive(epoch);const owner=localOwner(),preserve=queue===state.queue&&state.player?.mode==='local',presentation={expanded:state.expanded,panel:state.nowPanel};
 if(!isLocalItem(item)||!queue.length||queue.length>512||!queue.every(isLocalItem)||!Number.isInteger(index)||index<0||index>=queue.length)throw Error('Choose a track from this tab’s local music.');
 await stopPlayback();assertLive(epoch);const generation=playbackGeneration;
 const current=()=>{assertLive(epoch);if(generation!==playbackGeneration)throw Error('Playback was canceled.');};
 const source=owner.acquireSource(item.Id);
 try{
  current();if(!media.canPlayType(source.mimeType))throw Error('This browser cannot play this audio format. Choose another file or use the installed app.');
  installQueue(queue,index,original,preserve);
  state.player={mode:'local',item,source,offset:0,generation,domOwner:Object.freeze({generation,kind:'local'})};
  media.__bluefinOutputOwner=state.player.domOwner;
  source.signal.addEventListener('abort',()=>{
   if(state.player?.source!==source)return;
   stopPlayback(false).catch(()=>{}).finally(()=>{if(!pageRetired)render();});
  },{once:true});
  syncLocalQueue();current();media.src=source.url;media.load();
  media.play().catch(()=>{if(!pageRetired&&state.player?.source===source)document.getElementById('media-status').textContent='Press Play to begin.';});
  state.expanded=presentation.expanded;state.nowPanel=presentation.panel;loadLyrics(state.player);
   document.body.classList.add('audio-playing','with-player');document.body.classList.remove('video-playing');render();
 }catch(error){
  if(state.player?.source===source){const previous=state.player;state.player=null;clearOutput(previous);}
  else source.release();
  throw error;
 }
}
function playbackTimeline(){
 const p=state.player,offset=Number.isFinite(p?.offset)?p.offset:0;
 if(p?.mode==='mesh'&&p.snapshot)return {position:p.snapshot.position||0,duration:p.snapshot.duration||0,minimum:offset};
 const position=offset+(Number.isFinite(media.currentTime)?media.currentTime:0);
 const duration=Number.isFinite(media.duration)&&media.duration>0?offset+media.duration:0;
 return {position,duration,minimum:offset};
}
function seekPlayback(position){
 const p=state.player;if(!p||!Number.isFinite(position)||p.item.Type==='TvChannel')return;
 if(p.mode==='mesh')runtime()?.ready()?.webMeshControl(p.handle,'seek',position);
 else media.currentTime=Math.max(0,position-(p.offset||0));
 updatePlayback();
}
function toggleShuffle(){
 const current=state.queue[state.queueIndex];
 if(!state.shuffle){state.unshuffledQueue=state.queue.slice();state.queue=current?[current,...shuffled(state.queue.filter((_,index)=>index!==state.queueIndex))]:[];state.queueIndex=0;state.shuffle=true;}
 else{state.queue=state.unshuffledQueue.slice();state.queueIndex=Math.max(0,state.queue.findIndex(item=>item.Id===current?.Id));state.shuffle=false;}
 syncLocalQueue();render();
}

const artworkPending=new Map(),artworkMissing=new Set(),imageControllers=new Set(),requestControllers=new Set();let artworkActive=0,artworkScrollTimer;
function assertLive(epoch=state.epoch,session){if(pageRetired||epoch!==state.epoch||(session!==undefined&&session!==state.session))throw Error('This page or account changed. Resume the current connection.');}
function clearSearchFacets(){state.facets?.controller?.abort();state.facets=null;}
function clearPrivateView(){routeController?.abort();routeController=null;state.featured=null;state.homeHistory=null;clearSearchFacets();state.search=null;state.rows=[];state.shelves=[];state.back=[];state.query='';state.route={kind:'home'};state.queue=[];state.unshuffledQueue=[];state.queueIndex=0;state.shuffle=false;state.local=false;state.localQuery='';state.localSnapshot=null;state.repeat='off';state.nowPanel='artwork';state.capabilities=null;state.capability=null;state.code='';state.pairing=null;state.device=null;state.invitation=null;}
const paths={
 waves:'M3 6c3-3 6 3 9 0s6 3 9 0M3 12c3-3 6 3 9 0s6 3 9 0M3 18c3-3 6 3 9 0s6 3 9 0',
 sports:'M4 4h16v16H4zM4 12h16M12 4v16M9 12a3 3 0 1 0 6 0 3 3 0 1 0-6 0',
 shows:'M3 6h18v14H3zM8 2l4 4 4-4',movies:'M3 4h18v16H3zM7 4v16M17 4v16M3 9h4m-4 6h4M17 9h4m-4 6h4',
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
 lyrics:'M4 4h16v13H9l-5 4V4M8 8h8M8 12h6',airplay:'M3 16V4h18v12M12 14l6 7H6z',
 queue:'M4 5h16M4 12h16M4 19h10',download:'M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4',
 qr:'M3 3h6v6H3zM15 3h6v6h-6zM3 15h6v6H3zM15 15h3v3h3v3h-6z',info:'M12 11v6M12 7h.01M21 12a9 9 0 1 1-18 0 9 9 0 1 1 18 0'
};
function node(tag,text,cls){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;if(cls)e.className=cls;return e;}
function icon(name){const e=document.createElementNS('http://www.w3.org/2000/svg','svg');e.setAttribute('viewBox',name==='waves'?'200 210 610 640':'0 0 24 24');e.setAttribute('aria-hidden','true');const p=document.createElementNS(e.namespaceURI,'path');p.setAttribute('d',name==='waves'?"M225 604C388 534 410 344 620 239C580 399 670 554 790 604C560 554 420 687 225 604Z":paths[name]||paths.music);e.append(p);if(name==='waves'){e.setAttribute('class','brand-mark');p.setAttribute('class','brand-fin');const w=document.createElementNS(e.namespaceURI,'path');w.setAttribute('class','brand-waves');w.setAttribute('d',"M235 724C430 789 575 644 790 712M235 784C430 849 575 704 790 772");e.append(w);}return e;}
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
async function http(path,method='GET',body,query=[],session=state.session,keepalive=false,captureReply=false,base=state.base,signal){
 if(!keepalive)assertLive();if(signal?.aborted)throw aborted();
 const url=endpoint(path,query,base),controller=new AbortController(),timer=setTimeout(()=>controller.abort(),18000);
 const cancel=()=>controller.abort();signal?.addEventListener('abort',cancel,{once:true});
 if(!keepalive)requestControllers.add(controller);
 const headers={Authorization:authorization(session?.token||'')};if(body!==undefined)headers['Content-Type']='application/json';
 try{
  const response=await fetch(url,{method,headers,body:body===undefined?undefined:JSON.stringify(body),
   credentials:'same-origin',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal,keepalive});
  if(!response.ok){const error=Error(response.status===401||response.status===403?'Account access was denied. Sign in again.':'The server returned HTTP '+response.status+'.');error.status=response.status;throw error;}
  const value=await readJson(response);
  if(!keepalive&&!captureReply)assertLive();
  return value;
 }catch(e){if(e.name==='AbortError'&&!signal?.aborted)throw Error('The server did not respond in time or the page was closed.');throw e;}
 finally{signal?.removeEventListener('abort',cancel);requestControllers.delete(controller);clearTimeout(timer);}
}
async function retireLateAccount(session,base){
 await http('Sessions/Logout','POST',undefined,[],session,true,true,base).catch(()=>{});
}
async function api(path,method='GET',body,query=[],signal){
 const epoch=state.epoch,session=state.session,mode=state.mode,base=state.base;assertLive(epoch,session);if(signal?.aborted)throw aborted();
 let value;
 if(mode==='mesh'){
  if(!session?.handle)throw Error('Resume the encrypted device connection.');
  const engine=await runtime().engine();assertLive(epoch,session);
  value=JSON.parse(await engine.webMeshRequest(session.handle,method,path,JSON.stringify(query),body===undefined?undefined:JSON.stringify(body)));
 }else value=await http(path,method,body,query,session,false,false,base,signal);
 assertLive(epoch,session);if(signal?.aborted)throw aborted();return value;
}
function items(value){const list=Array.isArray(value)?value:value?.Items;if(!Array.isArray(list)||list.length>200)throw Error('The server returned an invalid library list.');return list.filter(item=>uuid(item.Id)).map(item=>({...item,Name:String(item.Name||'Untitled').slice(0,4096),Overview:String(item.Overview||'').slice(0,65536)}));}
function subtitle(item){return [item.AlbumArtist||item.Artists?.join(', ')||item.ProductionYear,item.Album].filter(Boolean).join(' · ');}
async function perform(task){
 if(pageRetired||state.busy)return;state.busy=true;state.error='';state.notice='';render();
 try{await task();}catch(e){if(!pageRetired&&!state.capability)state.error=String(e.message||e);}
 finally{if(!pageRetired){state.busy=false;render();}}
}
async function connect(raw){
 const epoch=state.epoch,base=validateBase(raw);assertLive(epoch);state.base=base;
 let info;try{info=await http('System/Info/Public','GET',undefined,[],null,false,false,base);}
 catch(error){if(error.status===404)throw Error('This address hosts the Bluefin app. Open local music here, use a device invitation when available, or open your server’s trusted Bluefin address to sign in.');throw error;}
 assertLive(epoch);
 if(typeof info?.ServerName!=='string')throw Error('No Bluefin server was found at this address.');
 const options=await http('Bluefin/Accounts/Options','GET',undefined,[],null,false,false,base).catch(()=>({}));
 assertLive(epoch);state.server=info.ServerName;state.options=options;state.phase='options';
}
function account(value){const token=value?.AccessToken,user=value?.User?.Id;if(!uuid(user)||!token)throw Error('The server returned an invalid account.');authorization(token);return {token,user:id(user),name:String(value.User.Name||'Your account')};}
async function accept(value,mode='https',epoch=state.epoch){
 assertLive(epoch);state.epoch++;const accepted=state.epoch;
 state.featured=null;state.homeHistory=null;clearSearchFacets();state.search=null;state.session=value;state.mode=mode;state.phase='library';state.sheet=null;state.capability=null;state.device=null;
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
 state.code=code;
 const engine=await meshEngine(()=>beginMesh(code,name,remember),canReloadBeforeMeshBegin(),false,view);assertLive(epoch);if(view!==state.pairingView)throw Error('The connection screen was closed.');
 state.remember=remember;
 meshBeginStarted=true;
 const result=await engine.webMeshBegin(code,name,remember);
 await meshProgress(result,epoch,view,engine);
}
async function resumeMesh(remembered){
 const epoch=state.epoch,view=state.pairingView;assertLive(epoch);
 const engine=await meshEngine(()=>resumeMesh(remembered),false,false,view);assertLive(epoch);if(view!==state.pairingView)throw Error('The connection screen was closed.');
 if(remembered)state.remember=true;
 await meshProgress(await engine.webMeshResume(remembered),epoch,view,engine);
}
async function continueMesh(){
 const epoch=state.epoch,view=state.pairingView;assertLive(epoch);
 const engine=await meshEngine(continueMesh,false,false,view);assertLive(epoch);if(view!==state.pairingView)throw Error('The connection screen was closed.');
 await meshProgress(await engine.webMeshProceed(),epoch,view,engine);
}
function closeSheet(){if(state.sheet==='invite')state.pairingView++;state.capability=null;state.sheet=null;state.expanded=false;state.invitation=null;clearTimeout(state.inviteTimer);render();}

function canReloadBeforeMeshBegin(){return !meshBeginStarted&&!state.pairing&&!state.session;}
function showCapability(error,retry,beforeBegin=false,mediaOnly=false) {
  if(pageRetired)return;
  state.capability={reason:String(error.message||error),retry,beforeBegin,mediaOnly};
  state.error='';
}
async function meshEngine(retry,beforeBegin=false,mediaOnly=false,view=state.pairingView) {
  const epoch=state.epoch;assertLive(epoch);let engine;
  try {engine=await runtime().engine();}
  catch(error) {
    if(!pageRetired&&epoch===state.epoch&&view===state.pairingView&&error.bluefinBrowserCapability)
      showCapability(error,retry,beforeBegin,mediaOnly);
    throw error;
  }
  assertLive(epoch);return engine;
}
function validInvitation(code=state.code) {
  return typeof code==='string'&&/^MC1-[A-Z0-9-]+$/i.test(code)&&code.length<=2048;
}
function dismissCapability() {
  if(state.busy)return;
  state.capability=null;state.error='';render();
  document.getElementById(state.sheet==='invite'?'bf-mesh-code':'content')?.focus?.();
}
function separateHttpsSignIn() {
  if(state.busy)return;
  if(!state.session)state.pairingView++;
  state.capability=null;state.error='';state.sheet=null;
  if(!state.session){state.phase='connect';state.base=hostedBase();}
  render();
}
function capabilitySheet() {
  const help=state.capability,dialog=node('dialog',undefined,'capability-dialog');
  dialog.setAttribute('role','dialog');dialog.setAttribute('aria-modal','true');
  dialog.setAttribute('aria-labelledby','capability-title');
  dialog.setAttribute('aria-describedby','capability-description');
  const mark=node('div',undefined,'capability-mark');mark.append(icon('info'));
  const title=node('h2',help.mediaOnly?'Encrypted playback is unavailable':'Enable this browser connection');
  title.id='capability-title';
  const description=node('p',help.mediaOnly
    ?'This browser could not start the encrypted media bridge. Your paired connection has not been changed.'
    :'Bluefin could not start its encrypted connection code in this browser. The rest of the page remains available.');
  description.id='capability-description';
  dialog.append(mark,title,description,node('p',
    'Lockdown Mode can limit required browser features. Bluefin cannot determine whether it is enabled.',
    'secondary'));
  if(help.beforeBegin)dialog.append(node('p',
    'This attempt has not sent your invitation. Keep the original QR code or invitation link so you can reopen it after a reload.',
    'secondary'));
  else dialog.append(node('p',
    'If pairing already began, keep this tab open. Reloading a temporary connection can erase its key; the original QR cannot restore that key. Use a new invitation if needed, or restore a device you chose to remember.',
    'secondary'));
  const actions=node('div',undefined,'capability-actions');
  const retry=action('Try again',async()=>{const retry=state.capability?.retry;state.capability=null;render();if(retry)await retry();},'primary');
  retry.autofocus=true;actions.append(retry);
  if(validInvitation()) {
    const native=node('a','Open in Bluefin','capability-link');
    native.href='bluefin://connect/'+state.code;actions.append(native,installationLink());
  }
  const leave=button(state.session?'Return to library':'Use HTTPS sign-in',separateHttpsSignIn),
    dismiss=button('Not now',dismissCapability,'plain');
  leave.disabled=dismiss.disabled=state.busy;actions.append(leave,dismiss);
  dialog.append(actions);
  if(!state.session)dialog.append(node('p',
    'HTTPS sign-in is a separate session with this website’s server. It does not redeem this invitation.',
    'secondary'));
  const guide=node('details',undefined,'capability-guide');
  guide.append(node('summary','Safari: allow only this website (optional)'),
    node('p','Only change this setting if you trust '+location.host+'. Excluding a site reduces Lockdown Mode protection for that site.'),
    node('h3','iPhone or iPad'),
    node('p','In Safari, open the Page Menu, choose Website Settings, then turn off Lockdown Mode for this website.'),
    node('h3','Mac'),
    node('p','In Safari, choose Settings for '+location.host+' from the Safari menu, then deselect Enable Lockdown Mode.'),
    node('p','These controls appear only when Lockdown Mode is on. Safari may reload the page. Before pairing begins, reopen the original QR code or invitation link after a reload. Bluefin removes invitation codes from the address bar.'),
    node('p','Return to this page and try again. Bluefin cannot change this Safari setting for you.'));
  const support=node('a','Apple’s Lockdown Mode instructions');
  support.href='https://support.apple.com/en-us/105120';support.target='_blank';support.rel='noopener noreferrer';
  guide.append(support);dialog.append(guide);
  const detail=node('details',undefined,'capability-guide');
  detail.append(node('summary','Connection details'),node('p',help.reason,'secondary'),
    node('p','The browser runs Bluefin’s connection code locally. Media and account services remain on the Bluefin devices or servers you connect to.','secondary'));
  dialog.append(detail);
  dialog.addEventListener('cancel',event=>{event.preventDefault();dismissCapability();});
  dialog.addEventListener('keydown',event=>{
    if(event.key==='Escape'){event.preventDefault();dismissCapability();return;}
    if(event.key==='Tab'&&typeof dialog.showModal!=='function') {
      const controls=Array.from(dialog.querySelectorAll('button:not(:disabled),a[href],summary'))
        .filter(control=>!control.getClientRects||control.getClientRects().length>0);
      if(!controls.length){event.preventDefault();return;}
      const first=controls[0],last=controls[controls.length-1],active=document.activeElement;
      if(event.shiftKey&&(active===first||!dialog.contains(active))){event.preventDefault();last.focus();}
      else if(!event.shiftKey&&(active===last||!dialog.contains(active))){event.preventDefault();first.focus();}
    }
  });
  return dialog;
}
async function readInvitationImage(file) {
  const epoch=state.epoch,view=state.pairingView;
  const current=()=>{assertLive(epoch);if(view!==state.pairingView||state.sheet!=='invite')throw Error('The connection screen was closed.');};
  current();if(file.size<=0||file.size>8*1024*1024)throw Error('Choose a PNG or JPEG smaller than 8 MiB.');
  const engine=await meshEngine(()=>readInvitationImage(file),!state.pairing&&!state.session,false,view);current();
  const bytes=new Uint8Array(await file.arrayBuffer());current();
  const code=engine.webMeshImage(bytes);current();state.code=code;
  state.notice='Invitation read. Continue to connect.';
}

function hasSearchFacets(filters){return !!(filters.studioID||filters.tag||filters.collectionID);}
function cleanSearchFilters(value={}) {
 const scope=searchScopes.some(([key])=>key===value.scope)?value.scope:'all';
 if(scope==='sports')return {scope,studioID:'',tag:'',collectionID:''};
 return {scope,studioID:uuid(value.studioID)?id(value.studioID):'',tag:typeof value.tag==='string'?value.tag.slice(0,512):'',
  collectionID:uuid(value.collectionID)?id(value.collectionID):''};
}
function searchLanes(filters){
 if(hasSearchFacets(filters))return ['library'];
 if(filters.scope==='sports')return ['programmes'];
 if(filters.scope==='liveTv')return ['programmes','channels'];
 const lanes=['library'];if(['all','music'].includes(filters.scope))lanes.push('artists');
 if(filters.scope==='all')lanes.push('programmes','channels');return lanes;
}
function searchCall(lane,route,start=0){
 const filters=cleanSearchFilters(route.filters),query=[['UserId',state.session.user],['Limit',60],['StartIndex',start]];
 if(route.query)query.push(['SearchTerm',route.query]);
 let path;
 if(lane==='library'){
  path='Users/'+state.session.user+'/Items';query.push(['Recursive',true],['Fields','Overview,PrimaryImageAspectRatio,ProviderIds']);
  const type=searchScopes.find(([key])=>key===filters.scope)?.[2];if(type)query.push(['IncludeItemTypes',type]);
  if(filters.studioID)query.push(['StudioIds',filters.studioID]);if(filters.tag)query.push(['Tags',filters.tag]);
  if(filters.collectionID)query.push(['ParentId',filters.collectionID]);
 }else if(lane==='artists'){path='Artists';query.push(['Recursive',true]);}
 else if(lane==='programmes'){
  path='LiveTv/Programs';query.push(['MinEndDate',route.liveAfter]);
  if(filters.scope==='sports')query.push(['IsSports',true]);
 }
 else path='LiveTv/Channels';
 return {path,query};
}
function appendSearchPage(page,value){
 const entries=items(value),seen=new Set(page.items.map(item=>id(item.Id)));
 for(const item of entries)if(!seen.has(id(item.Id))){seen.add(id(item.Id));page.items.push(item);}
 page.next+=(Array.isArray(value)?value:value.Items).length;
 page.total=entries.length?Math.max(page.next,Number(value.TotalRecordCount)||page.next):page.next;
}
async function publicCatalogJSON(url,signal,headers){
 const limit=1024*1024,controller=new AbortController(),timeout=setTimeout(()=>controller.abort(),6000);
 const stop=()=>controller.abort();
 signal.addEventListener('abort',stop,{once:true});
 try{
  if(signal.aborted)throw aborted();
  const response=await fetch(url,{credentials:'omit',redirect:'error',referrerPolicy:'no-referrer',signal:controller.signal,headers});
  if(!response.ok)throw Error('Catalog unavailable');
  const reader=response.body.getReader(),bytes=new Uint8Array(limit);
  let length=0;
  for(;;){
   const {done,value}=await reader.read();if(done)break;
   if(length+value.length>limit){await reader.cancel();throw Error('Catalog response too large');}
   bytes.set(value,length);length+=value.length;
  }
  return JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(0,length)));
 }finally{clearTimeout(timeout);signal.removeEventListener('abort',stop);}
}
async function publicTVSearch(term,signal){
 const url=new URL('https://api.tvmaze.com/search/shows');
 url.searchParams.set('q',term);
 const rows=await publicCatalogJSON(url,signal);
  if(!Array.isArray(rows))throw Error('Invalid catalog response');
  const titles=[],seen=new Set();
  for(const {show} of rows.filter(row=>row&&typeof row==='object')){
   if(!Number.isSafeInteger(show?.id)||show.id<=0||seen.has(show.id)||typeof show.name!=='string'||!show.name.trim()||show.name.length>256)continue;
   seen.add(show.id);
   const raw=show.image?.medium;
   const art=typeof raw==='string'&&raw.length<=1024&&/^https:\/\/static\.tvmaze\.com\/uploads\/images\/medium_portrait\/[0-9]+\/[0-9]+\.jpg$/.test(raw)?raw:null;
   const year=typeof show.premiered==='string'&&/^\d{4}-/.test(show.premiered)?show.premiered.slice(0,4):'';
   titles.push({kind:'series',id:String(show.id),name:show.name,art,year,imdbId:show.externals?.imdb});
   if(titles.length===20)break;
  }
  return titles;
}
function wikidataJSON(params,signal){
 const url=new URL('https://www.wikidata.org/w/api.php');
 for(const [key,value] of Object.entries({format:'json',formatversion:'2',origin:'*',...params}))url.searchParams.set(key,value);
 return publicCatalogJSON(url,signal,{'Api-User-Agent':'Bluefin (https://github.com/glopyglerky/bluefin)'});
}
async function publicFilmSearch(term,signal){
 // Only this adapter supplies a Wikidata predicate. User text is a literal title hint.
 const hint=term.replace(/[^\p{L}\p{N}\p{M}]+/gu,' ').trim();
 if(!hint)return [];
 const results=await wikidataJSON({action:'query',list:'search',srnamespace:'0',srlimit:'12',
  srprop:'',srsearch:hint+' haswbstatement:P31=Q11424'},signal);
 if(!Array.isArray(results?.query?.search))throw Error('Invalid film search response');
 const ids=[...new Set(results.query.search.map(item=>item?.title).filter(id=>typeof id==='string'&&/^Q[1-9][0-9]{0,14}$/.test(id)))].slice(0,12);
 if(!ids.length)return [];
 const language=(globalThis.navigator?.language||'en').toLowerCase();
 const primary=/^[a-z]{2,3}/.exec(language)?.[0]||'en';
 const details=await wikidataJSON({action:'wbgetentities',ids:ids.join('|'),props:'labels|descriptions',
  languages:primary==='en'?'en':primary+'|en'},signal);
 if(!details?.entities||details.error)throw Error('Invalid film catalog response');
 const titles=[];
 for(const id of ids){
  const entity=details.entities[id];
  const label=field=>entity?.[field]?.[primary]?.value||entity?.[field]?.en?.value;
  const name=label('labels'),description=label('descriptions');
  if(typeof name!=='string'||!name.trim()||name.length>256)continue;
  titles.push({kind:'movie',id,name,description:typeof description==='string'?description.slice(0,180):''});
 }
 return titles;
}
const publicSearchProviders=[
 {scope:'tvShows',label:'TV catalog',source:'TVmaze',icon:'shows',lookup:publicTVSearch},
 {scope:'movies',label:'Film catalog',source:'Wikidata',icon:'movies',lookup:publicFilmSearch},
];
async function loadSearch(route,get,current,signal){
 const search=state.search={route,pages:[],problems:[],catalogs:[]};
 if(!route.query&&route.filters.scope==='all'&&!hasSearchFacets(route.filters))return;
 const names={library:'Your library',artists:'Artists',programmes:route.filters.scope==='sports'?'Sports broadcasts':'On now and upcoming',channels:'Channels'};
 search.pages=state.session?searchLanes(route.filters).map(lane=>({lane,title:names[lane],items:[],next:0,total:0,loading:true})):[];
 const lookups=search.pages.map(async page=>{
  try{const call=searchCall(page.lane,route),value=await get(call.path,call.query);current();appendSearchPage(page,value);}
  catch(error){current();if(error.name!=='AbortError')search.problems.push(page.title+': '+String(error.message||error));}
  page.loading=false;
  state.rows=search.pages.flatMap(page=>page.items);
  render();
 });
 const publicTerm=Array.from(route.query).slice(0,160).join('');
 if(publicTerm.length>=2&&!hasSearchFacets(route.filters)){
  const providers=publicSearchProviders.filter(provider=>['all',provider.scope].includes(route.filters.scope));
  search.catalogs=providers.map(provider=>({provider,loading:true,items:[],unavailable:false}));
  lookups.push(...search.catalogs.map(async catalog=>{
   try{const titles=await catalog.provider.lookup(publicTerm,signal);current();catalog.items=titles;}
   catch{current();catalog.unavailable=true;}
   catalog.loading=false;render();
  }));
 }
 await Promise.all(lookups);
}
async function startSearch(query,filters={},record=false){
 const previous=state.route,route={kind:'search',title:'Search',query:String(query).trim().slice(0,256),
  filters:cleanSearchFilters(filters),refine:previous.kind==='search'&&previous.refine,liveAfter:new Date().toISOString()};
 state.query=route.query;state.route=route;state.error='';
 const search=loadRoute();render();
 if(route.refine&&route.filters.scope!=='sports'&&(!state.facets||state.facets.loading))loadSearchFacets().catch(()=>{});
 if(record)recordHomeSearch(route.query).catch(()=>{});
 try{await search;}catch(error){if(route===state.route&&error.name!=='AbortError')state.error=String(error.message||error);}
 finally{if(route===state.route)render();}
}
async function openSearch(){
 if(!state.session&&state.local)return navigate('Search');
 if(state.route.kind==='search'){
  document.getElementById('bluefin-search')?.focus?.();
  return;
 }
 if(!state.session)state.phase='public-search';
 state.back.push({...state.route});
 const pending=startSearch(state.query,{scope:'all'});
 document.getElementById('bluefin-search')?.focus?.();
 await pending;
}
async function loadMoreSearch(lane){
 const search=state.search,route=state.route,page=search?.pages.find(value=>value.lane===lane),controller=routeController;
 const epoch=state.epoch,session=state.session;
 if(!page||page.loading||page.next>=page.total||search.route!==route)return;
 page.loading=true;render();
 try{
  const call=searchCall(lane,route,page.next),value=await api(call.path,'GET',undefined,call.query,controller?.signal);
  assertLive(epoch,session);if(route!==state.route||search!==state.search)throw aborted();
  appendSearchPage(page,value);state.rows=search.pages.flatMap(value=>value.items);
 }catch(error){if(route===state.route&&epoch===state.epoch&&error.name!=='AbortError')search.problems.push(String(error.message||error));}
 finally{page.loading=false;if(route===state.route)render();}
}
async function loadSearchFacets(){
 const epoch=state.epoch,session=state.session;if(!session)return;
 if(state.facets?.epoch===epoch&&state.facets.session===session&&!state.facets.controller.signal.aborted)return state.facets.promise;
 clearSearchFacets();
 const controller=new AbortController();
 const facets=state.facets={epoch,session,controller,studios:[],tags:[],collections:[],pending:new Set(['studios','tags','collections']),loading:true,problems:[],truncated:false};
 const current=()=>!pageRetired&&epoch===state.epoch&&session===state.session&&facets===state.facets&&!controller.signal.aborted;
 const publish=()=>{if(current()&&state.route.kind==='search'&&state.route.refine)render();};
 publish();
 const calls=[
  ['studios','Studios',[['UserId',session.user],['Recursive',true],['Limit',200]]],
  ['tags','Items/Filters2',[['UserId',session.user]]],
  ['collections','Users/'+session.user+'/Items',[['Recursive',true],['IncludeItemTypes','BoxSet'],['Limit',200]]]];
 facets.promise=Promise.all(calls.map(async([kind,path,query])=>{
  try{
   const value=await api(path,'GET',undefined,query,controller.signal);if(!current())return;
   let values,truncated;
   if(kind==='tags'){
    if(!Array.isArray(value?.Tags))throw Error('The server returned invalid tags.');
    values=[...new Set(value.Tags.filter(tag=>typeof tag==='string'&&tag.length<=512))].slice(0,512);truncated=value.Tags.length>512;
   }else{
    values=items(value).map(item=>({id:id(item.Id),name:item.Name}));truncated=Number(value.TotalRecordCount)>values.length;
   }
   facets[kind]=values;facets.truncated||=truncated;
  }catch(error){if(current()&&error.name!=='AbortError')facets.problems.push(String(error.message||error));}
  finally{if(current()){facets.pending.delete(kind);facets.loading=facets.pending.size>0;publish();}}
 }));
 await facets.promise;
}
async function homeRecommendations(get,user){
 let value;
 try{
  value=await get('Bluefin/Home/Recommendations');
  if(!Array.isArray(value?.recommendations)||value.recommendations.length>12)throw Error('The server returned invalid Home recommendations.');
  return {shelves:value.recommendations.map(row=>({title:String(row.title||'Recommended for you').slice(0,256),reason:row.reason,items:items(row.items).slice(0,18)})),
   history:value.searchHistorySupported===true&&Number.isSafeInteger(value.searchHistoryGeneration)&&value.searchHistoryGeneration>=0?{generation:value.searchHistoryGeneration}:null};
 }catch(error){if(!notFound(error))throw error;}
 value=await get('Movies/Recommendations',[['UserId',user],['CategoryLimit',4],['ItemLimit',12],['Fields','Overview,PrimaryImageAspectRatio']]);
 if(!Array.isArray(value)||value.length>12)throw Error('The server returned invalid recommendations.');
 const labels={SimilarToRecentlyPlayed:'Because you watched',SimilarToLikedItem:'Because you like',
  HasDirectorFromRecentlyPlayed:'More from',HasActorFromRecentlyPlayed:'More with',HasLikedDirector:'From a director you like:',HasLikedActor:'With an actor you like:'};
 return {shelves:value.filter(row=>labels[row.RecommendationType]).map(row=>({
  title:labels[row.RecommendationType]+' '+String(row.BaselineItemName||'a favorite').slice(0,128),
  reason:row.RecommendationType.includes('Liked')?'liked':'watched',items:items(row.Items).slice(0,18)})),history:null};
}
async function recordHomeSearch(query,selectedItem){
 const epoch=state.epoch,session=state.session,history=state.homeHistory;
 const term=String(query||'').trim().replace(/\s+/g,' ');
 if(!session||!history||term.length<2||term.length>128||/[\x00-\x1f\x7f]/.test(term))return;
 const payload={eventId:crypto.randomUUID(),query:term,generation:history.generation,occurredAtUnixMs:Date.now()};
 if(selectedItem&&uuid(selectedItem.Id))payload.selectedItemId=id(selectedItem.Id);
 try{await api('Bluefin/Home/Searches','POST',payload);assertLive(epoch,session);}
 catch(error){if(epoch===state.epoch&&session===state.session&&state.homeHistory===history&&error.status===409)state.homeHistory=null;}
}
async function clearHomeSearchHistory(){
 const epoch=state.epoch,session=state.session,history=state.homeHistory;if(!history)return;
 const value=await api('Bluefin/Home/Searches','DELETE',{generation:history.generation});assertLive(epoch,session);
 if(!Number.isSafeInteger(value?.generation))throw Error('The server returned an invalid history revision.');
 state.homeHistory={generation:value.generation};state.shelves=state.shelves.filter(shelf=>shelf.reason!=='search');
 state.featured=state.shelves.find(shelf=>shelf.items.length)?.items[0]||null;
 state.notice='Search history cleared for this account.';
}
async function loadRoute(){
 const epoch=state.epoch,session=state.session,route=state.route,user=session?.user;assertLive(epoch,session);
 routeController?.abort();const controller=routeController=new AbortController();
 const current=()=>{assertLive(epoch,session);if(route!==state.route||controller!==routeController||controller.signal.aborted)throw aborted();};
 const get=(path,query=[])=>api(path,'GET',undefined,query,controller.signal);
 if(route.kind==='local_music'||route.kind==='local_search'){state.rows=route.kind==='local_search'?localOwner().search(state.localQuery).map(localItem):localItems();state.shelves=[];return;}
 if(!user&&route.kind!=='search')return;
 state.rows=[];state.shelves=[];state.featured=null;state.error='';route.loading=true;
 if(route.kind!=='search')state.search=null;
 try{
  if(route.kind==='home'){
   const requests=[
    get('Users/'+user+'/Items/Resume',[['Limit',12],['Fields','Overview']]).then(items),
    get('Shows/NextUp',[['UserId',user],['Limit',12],['Fields','Overview']]).then(items),
    get('Users/'+user+'/Items/Latest',[['Limit',18],['Fields','Overview']]).then(items),
    homeRecommendations(get,user)];
   const results=await Promise.allSettled(requests);current();
   const values=results.map(result=>result.status==='fulfilled'?result.value:null);
   const resume=values[0]||[],next=values[1]||[],latest=values[2]||[],recommended=values[3]?.shelves||[];
   state.homeHistory=values[3]?.history||null;
   state.shelves=[{title:'Continue watching',items:resume},{title:'Up next',items:next},...recommended,{title:'Recently added',items:latest}].filter(shelf=>shelf.items.length);
   state.featured=resume[0]||recommended.find(shelf=>shelf.items.length)?.items[0]||next[0]||latest[0]||null;
   if(results.every(result=>result.status==='rejected'))throw Error('Your library could not refresh. Open Collections or try again.');
   if(results.some(result=>result.status==='rejected'))state.notice='Some Home sections could not refresh. Try again to load the remaining sections.';
  }else if(route.kind==='library'){const rows=items(await get('Users/'+user+'/Views'));current();state.rows=rows;}
  else if(route.kind==='music'){
   const q=[['Limit',12],['Recursive',true],['IncludeItemTypes','MusicAlbum'],['Fields','Overview,PrimaryImageAspectRatio'],['SortBy','DateCreated'],['SortOrder','Descending']];
   const recent=items(await get('Users/'+user+'/Items',q));current();state.shelves=[{title:'Recently added',items:recent}];
   const favorites=await get('Users/'+user+'/Items',[...q,['IsFavorite',true]]).then(items).catch(()=>[]);current();
   if(favorites.length)state.shelves.push({title:'Favorite albums',items:favorites});
  }else if(route.kind==='live'){const rows=items(await get('LiveTv/Channels',[['UserId',user],['Limit',100]]));current();state.rows=rows;}
  else if(route.kind==='sports'){
   try{
    const feed=await get('sports/api/v1/home');current();
    if(feed?.schema!==1||feed.presentation_only!==true||!Array.isArray(feed.games)||!Array.isArray(feed.local_broadcasts)||feed.games.length+feed.local_broadcasts.length>400)throw Error('The server returned invalid sports information.');
    route.sports=feed.games.concat(feed.local_broadcasts);
   }catch(error){
    if(!notFound(error))throw error;
    const rows=items(await get('LiveTv/Programs',[['UserId',user],['IsSports',true],['MinEndDate',new Date().toISOString()],['Limit',100]]));current();state.rows=rows;
   }
  }else if(route.kind==='search'){route.filters=cleanSearchFilters(route.filters);route.query=route.query??state.query;route.liveAfter??=new Date().toISOString();await loadSearch(route,get,current,controller.signal);}
  else if(route.kind==='collection'){
   const q=[['Limit',100],['StartIndex',route.start||0],['Recursive',true],['Fields','Overview,PrimaryImageAspectRatio'],['SortBy',route.type==='Audio'&&route.parent?'ParentIndexNumber,IndexNumber,SortName':'SortName']];
   if(route.type)q.push(['IncludeItemTypes',route.type]);if(route.parent)q.push(['ParentId',route.parent]);if(route.artist)q.push(['ArtistIds',route.artist]);
   const value=await get('Users/'+user+'/Items',q);current();state.rows=items(value);route.total=Number(value.TotalRecordCount)||state.rows.length;
  }else if(route.kind==='detail'){
   const value=await get('Users/'+user+'/Items/'+id(route.item.Id));current();if(!uuid(value.Id))throw Error('The item is no longer available.');route.item=value;
  }
 }finally{if(controller===routeController)route.loading=false;}
}
async function navigate(tab){
 if(tab==='Search'){
  if(!state.session&&state.local){state.route={kind:'local_search',title:'Search'};state.back=[];return loadRoute();}
  return openSearch();
 }
 if(tab==='Library'||tab==='Collections'){
  state.route={kind:'library',title:'Collections'};state.back=[];return loadRoute();
 }
 state.tab=tab;state.back=[];
 if(!state.session&&state.local){state.route={kind:tab==='Music'?'local_music':'local_connect',title:tab};return loadRoute();}
 if(tab==='Movies'||tab==='TV Shows')state.route={kind:'collection',title:tab,type:tab==='Movies'?'Movie':'Series',start:0};
 else state.route={kind:({Home:'home',Sports:'sports',Music:'music','Live TV':'live'})[tab]||'home'};
 await loadRoute();
}
async function openCollection(title,type,parent,artist){state.back.push({...state.route});state.route={kind:'collection',title,type,parent,artist,start:0};await loadRoute();}
async function openItem(item,recordSelection=true){
 if(recordSelection&&state.route.kind==='search')recordHomeSearch(state.route.query,item).catch(()=>{});
 if(['Program','LiveTvProgram'].includes(item.Type)){
  if(!uuid(item.ChannelId))throw Error('This programme has no available channel.');
  const channel=await api('Users/'+state.session.user+'/Items/'+id(item.ChannelId));
  if(!uuid(channel?.Id)||!['TvChannel','LiveTvChannel'].includes(channel.Type))throw Error('This channel is no longer available.');
  return openItem({...channel,Type:'TvChannel'},false);
 }
 if(item.Type==='LiveTvChannel')item={...item,Type:'TvChannel'};
if(item.Type==='TvChannel'&&state.mode!=='mesh'){state.back.push({...state.route});state.route={kind:'detail',item};return;}if(isLocalItem(item)){const queue=localItems();return playLocal(item,queue,queue.findIndex(track=>track.Id===item.Id));}if(item.Type==='Audio')return play(item,[item],0);if(item.Type==='MusicArtist')return openCollection(item.Name,'MusicAlbum,Audio,MusicVideo',null,id(item.Id));if(['MusicAlbum','MusicArtist','Playlist','Series','Season','CollectionFolder','Folder','BoxSet'].includes(item.Type)||item.IsFolder){return openCollection(item.Name,item.Type==='Series'||item.Type==='Season'?'Episode':(item.Type==='MusicArtist'?'MusicAlbum':['MusicAlbum','Playlist'].includes(item.Type)?'Audio':null),id(item.Id));}state.back.push({...state.route});state.route={kind:'detail',item};await loadRoute();}
function artKey(item,backdrop=false){return (backdrop?'b:':'p:')+id(item.Id);}
function applyArtwork(element,url,backdrop=false){if(backdrop){element.style.backgroundImage='linear-gradient(180deg,rgba(6,23,39,.05),rgba(6,23,39,.9)),url("'+url+'")';element.style.backgroundSize='cover';element.style.backgroundPosition='center';return;}const image=node('img');image.alt='';image.src=url;image.loading='lazy';image.onerror=()=>{element.replaceChildren(icon('music'));};element.replaceChildren(image);}
function bindArtwork(element,item,backdrop=false){if(isLocalItem(item))return;const key=artKey(item,backdrop);element.dataset.artKey=key;element.dataset.artId=id(item.Id);if(backdrop)element.dataset.backdrop='true';const cached=state.images.get(key);if(cached)applyArtwork(element,cached,backdrop);}
function cover(item,square=false){const d=node('div',undefined,'artwork'+(square?' square':''));d.append(icon(item.Type==='Audio'||item.Type==='MusicAlbum'?'music':'library'));bindArtwork(d,item);return d;}
async function imageBytes(item,backdrop,session,mode){
 if(mode==='mesh')return (await runtime().engine()).webMeshArtwork(session.handle,item,backdrop);
 const url=endpoint('Items/'+item+'/Images/'+(backdrop?'Backdrop':'Primary'),[['maxWidth',backdrop?1200:400],['quality',85]]);
 const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),8000);imageControllers.add(controller);
 try{const response=await fetch(url,{headers:{Authorization:authorization(session.token)},credentials:'omit',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});
 if(!response.ok||!['image/jpeg','image/png','image/webp'].includes((response.headers.get('Content-Type')||'').split(';')[0].trim()))throw Error('Artwork unavailable');
 const reader=response.body.getReader(),parts=[];let total=0;try{for(;;){const {done,value}=await reader.read();if(done)break;total+=value.length;if(total>2*1024*1024)throw Error('Artwork limit');parts.push(value);}}catch(e){await reader.cancel().catch(()=>{});throw e;}
 const bytes=new Uint8Array(total);let at=0;for(const part of parts){bytes.set(part,at);at+=part.length;}return bytes;
 }finally{clearTimeout(timer);imageControllers.delete(controller);}
}
function imageType(bytes){if(bytes.length>2&&bytes[0]===255&&bytes[1]===216&&bytes[2]===255)return 'image/jpeg';
 if(bytes.length>8&&[137,80,78,71,13,10,26,10].every((v,i)=>bytes[i]===v))return 'image/png';
 if(bytes.length>12&&String.fromCharCode(...bytes.slice(0,4))==='RIFF'&&String.fromCharCode(...bytes.slice(8,12))==='WEBP')return 'image/webp';
 throw Error('Unsupported artwork format');}
function visibleArtwork(){
 return Array.from(document.querySelectorAll('[data-art-key]')).filter(element=>{
  if(typeof element.getBoundingClientRect!=='function')return true;
  const rect=element.getBoundingClientRect();return rect.bottom>=-200&&rect.top<=(globalThis.innerHeight||1080)+200&&rect.right>=0&&rect.left<=(globalThis.innerWidth||1920);
 });
}
function pumpArtwork(){if(!state.session)return;const epoch=state.imageEpoch,session=state.session,mode=state.mode;
 const elements=visibleArtwork(),visible=new Set(elements.map(element=>element.dataset.artKey));
 for(const element of elements){
  if(artworkActive>=2)break;const key=element.dataset.artKey;if(state.images.has(key)||artworkPending.has(key)||artworkMissing.has(key))continue;
  while(state.images.size+artworkPending.size>=64){
   const evict=Array.from(state.images.keys()).find(candidate=>!visible.has(candidate));
   if(evict===undefined)break;URL.revokeObjectURL(state.images.get(evict));state.images.delete(evict);
  }
  if(state.images.size+artworkPending.size>=64)break;artworkPending.set(key,epoch);artworkActive++;
  const item=element.dataset.artId,backdrop=element.dataset.backdrop==='true';
  imageBytes(item,backdrop,session,mode).then(bytes=>{
   if(epoch!==state.imageEpoch||session!==state.session)return;const url=URL.createObjectURL(new Blob([bytes],{type:imageType(bytes)}));state.images.set(key,url);
   for(const target of document.querySelectorAll('[data-art-key]'))if(target.dataset.artKey===key)applyArtwork(target,url,target.dataset.backdrop==='true');
  }).catch(()=>{if(epoch===state.imageEpoch){if(artworkMissing.size>=256)artworkMissing.delete(artworkMissing.values().next().value);artworkMissing.add(key);}})
   .finally(()=>{artworkActive--;if(artworkPending.get(key)===epoch)artworkPending.delete(key);pumpArtwork();});
 }
}
addEventListener('scroll',()=>{clearTimeout(artworkScrollTimer);artworkScrollTimer=setTimeout(pumpArtwork,80);},{passive:true,capture:true});

function poster(item){
 const square=['MusicAlbum','MusicArtist'].includes(item.Type),b=button('',()=>perform(()=>openItem(item)),'poster'+(square?' album':'')+(item.Type==='MusicArtist'?' artist':''));
 const copy=node('span',undefined,'poster-copy');copy.append(node('strong',item.Name));
 if(subtitle(item))copy.append(node('small',subtitle(item)));
 const duration=Number(item.RunTimeTicks),position=Number(item.UserData?.PlaybackPositionTicks);
 const progress=duration>0&&position>0?Math.min(100,Math.max(0,100*position/duration)):0;
 if(progress>0){const track=node('span',undefined,'poster-progress'),fill=node('span');fill.style.width=progress+'%';track.append(fill);track.setAttribute('role','progressbar');track.setAttribute('aria-label','Watched');track.setAttribute('aria-valuemin','0');track.setAttribute('aria-valuemax','100');track.setAttribute('aria-valuenow',String(Math.round(progress)));copy.append(track);}
 b.setAttribute('aria-label',[item.Name,subtitle(item),progress>0?Math.round(progress)+' percent watched':''].filter(Boolean).join(', '));
 b.append(cover(item,square),copy);return b;
}
function shelf(title,entries){if(!entries.length)return document.createDocumentFragment();const s=node('section',undefined,'shelf');s.append(node('h2',title));const row=node('div',undefined,'shelf-track');for(const item of entries.slice(0,18))row.append(poster(item));s.append(row);return s;}
function row(item,select=()=>openItem(item),detail=subtitle(item)){const b=button('',()=>perform(select),'list-row'+(item.Type==='Audio'?' song':''));if(item.Type==='Audio')b.append(cover(item,true));else b.append(icon(item.CollectionType==='music'?'music':'library'));const t=node('span',undefined,'grow');t.append(node('strong',item.Name));if(detail)t.append(node('small',detail));b.append(t,icon('chevron'));return b;}
function programmeTime(item){
 const date=new Date(item.StartDate||'');
 return Number.isFinite(date.valueOf())?date.toLocaleString():'';
}
function selectGuideResult(item){
 if(!['Program','LiveTvProgram'].includes(item.Type))return openItem(item);
 recordHomeSearch(state.route.query,item).catch(()=>{});
 routeController?.abort();
 state.back.push({...state.route});
 state.route={kind:'programme',title:item.Name,item};
}
function programmeView(main,item){
 main.append(node('p',[programmeTime(item),item.ChannelName].filter(Boolean).join(' · '),'secondary'));
 if(item.Overview)main.append(node('p',item.Overview));
 const now=Date.now(),start=Date.parse(item.StartDate),end=Date.parse(item.EndDate);
 if(state.mode==='mesh'&&uuid(item.ChannelId)&&start<=now&&now<end){
  main.append(action('Watch live channel',async()=>{
   if(Date.now()>=end)throw Error('This programme has ended.');
   const channel=await api('Users/'+state.session.user+'/Items/'+id(item.ChannelId));
   if(!uuid(channel?.Id)||id(channel.Id)!==id(item.ChannelId)||!['TvChannel','LiveTvChannel'].includes(channel.Type))throw Error('This channel is no longer available.');
   return play({...channel,Type:'TvChannel'});
  },'primary','play'));
 }
}
function empty(title,message){const d=node('section',undefined,'empty-state');d.append(icon('library'),node('h2',title),node('p',message,'secondary'));return d;}
function toolbar(title,back=false){
 const d=node('div',undefined,'toolbar');
 if(back)d.append(action('Back',async()=>{state.route=state.back.pop()||{kind:'home'};await loadRoute();},'plain','back'));
 else d.append(node('h1',title));
 const utilities=node('div',undefined,'utility-actions');
 const search=button('',()=>openSearch(),'plain','search');search.setAttribute('aria-label','Search all media');
 const collections=action('',()=>navigate('Collections'),'plain','library');collections.setAttribute('aria-label','Collections');
 const account=button('',()=>{state.sheet='account';render();},'plain','account');account.setAttribute('aria-label','Account and settings');
 utilities.append(search,collections,account);d.append(utilities);return d;
}
function installationLink(){
 const link=node('a','Install Bluefin','capability-link');
 link.href=new URL('./install.html',document.baseURI).href;
 link.target='_blank';
 link.rel='noopener noreferrer';
 return link;
}
function sourceFooter(){const p=node('p',undefined,'source-links');for(const [label,file]of[['Source code','SOURCE.txt'],['License','LICENSE']]){const link=node('a',label);link.href=new URL('./'+file,document.baseURI).href;link.target='_blank';link.rel='noopener noreferrer';p.append(link);}return p;}
function welcome(){const main=node('main',undefined,'welcome');main.id='content';const mark=node('div',undefined,'wordmark');mark.append(icon('waves'),node('h1','Bluefin'));main.append(mark);const stack=node('section',undefined,'account-stack');
 if(state.phase==='connect'){stack.append(button('Search movies and shows',openSearch,'','search'));stack.append(button('Connect with QR and passkey',()=>{state.sheet='invite';state.error='';render();},'primary'));stack.append(action('Open local music',openLocalMusic,'','music'));const form=node('form');form.append(node('h2','Connect to your server'));const [w,i]=field('Server address','url',state.base,'server');i.autocomplete='url';i.placeholder='https://media.example.com';i.required=true;form.append(w);const submit=node('button','Continue','primary');submit.disabled=state.busy;form.append(submit);form.onsubmit=e=>{e.preventDefault();perform(()=>connect(i.value.trim()));};stack.append(form);}
 else if(state.phase==='options'){stack.append(node('h2',state.server||'Sign in'));stack.append(button('Sign in with password',()=>{state.phase='password';render();},'primary'));if(state.options.passkeys)stack.append(action('Sign in with a passkey',startPasskey));if(state.options.quickConnect)stack.append(action('Quick Connect',startQuick));if(state.options.allowSignups)stack.append(button('Create account',()=>{state.phase='create';render();}));stack.append(button('Change server',()=>{state.phase='connect';render();},'plain'));}
 else if(state.phase==='password'||state.phase==='create'){const create=state.phase==='create';const f=node('form');f.append(node('h2',create?'Create account':'Sign in'));const [nw,ni]=field('Username','text','', 'username'),[pw,pi]=field('Password','password','','password');ni.autocomplete='username';pi.autocomplete=create?'new-password':'current-password';ni.required=true;pi.required=true;f.append(nw,pw);let confirm;if(create){const [w,i]=field('Confirm password','password','','confirm');confirm=i;i.autocomplete='new-password';f.append(w,node('p','Use at least 15 characters.','secondary'));}const b=node('button',create?'Create account':'Sign in','primary');b.disabled=state.busy;f.append(b);f.onsubmit=e=>{e.preventDefault();const password=pi.value,name=ni.value.trim();if(create&&password!==confirm.value){state.error='The passwords do not match.';render();return;}pi.value='';if(confirm)confirm.value='';perform(()=>signIn(name,password,create));};stack.append(f,button('Back to sign-in options',()=>{state.phase='options';render();},'plain'));}
 else if(state.phase==='device'){const d=state.device;stack.append(node('h2',d?.kind==='passkey'?'Sign in with a passkey':'Quick Connect'),node('p',d?.code||'','confirmation-code'));if(d?.url){const link=node('a','Open passkey page');link.href=d.url;link.target='_blank';link.rel='noopener noreferrer';stack.append(link,node('p','After verifying, return here to finish.','secondary'));}else stack.append(node('p','Enter this code in another client signed in to your server.'));stack.append(action('Finish sign-in',finishDevice,'primary'),action('Cancel',cancelDevice));}
 if(state.phase==='connect')stack.append(installationLink());
 if(state.busy)stack.append(node('p','Connecting…','loading'));stack.append(status());main.append(stack,sourceFooter());return main;}
function searchChoice(label,key,value,choices,changed,loading=false){
 const wrap=node('div'),name='search-'+key,lab=node('label',label),select=node('select');
 lab.htmlFor=name;select.id=name;select.setAttribute('aria-label',label);
 for(const [id,title]of choices){const option=node('option',title);option.value=id;select.append(option);}
 if(value&&!choices.some(([id])=>id===value)){const selected=node('option',label==='Tag'?value:'Selected '+label.toLowerCase());selected.value=value;select.append(selected);}
 select.disabled=loading;select.setAttribute('aria-busy',String(loading));
 select.value=value||'';select.onchange=()=>changed(select.value);wrap.append(lab,select);return wrap;
}
function searchPoster(item){
 const card=node('article',undefined,'search-poster');
 const details=poster(item);
 details.dataset.focusKey='search-details|'+id(item.Id);
 card.append(details);
 if(!['Movie','Episode','Audio'].includes(item.Type))return card;
 const verb=resumeTicks(item)>0?'Resume':'Play';
 const playResult=action(verb,()=>{
  recordHomeSearch(state.route.query,item).catch(()=>{});
  return play(item);
 },'search-play','play');
 playResult.setAttribute('aria-label',verb+' '+item.Name);
 playResult.dataset.focusKey='search-play|'+id(item.Id);
 card.append(playResult);
 return card;
}
const filmId=/^tt[0-9]{7,12}$/;
const entityId=/^Q[1-9][0-9]{0,14}$/;
const uuidText=/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i;
function statedIdentifiers(claims,property,valid){
 const rows=claims?.[property];
 if(!Array.isArray(rows))return [];
 return [...new Set(rows.filter(row=>row?.rank!=='deprecated'&&row?.mainsnak?.snaktype==='value')
  .map(row=>row.mainsnak.datavalue?.value).filter(id=>typeof id==='string'&&valid.test(id)))].slice(0,6);
}
function directProviderLinks(claims,kind){
 const movie=kind==='movie';
 const rules=[
  ['Netflix','P1874',/^[0-9]{6,8}$/,movie?'https://www.netflix.com/watch/':'https://www.netflix.com/title/',movie],
  ['Disney+','P13902',/^(?:entity|page)-[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i,'https://www.disneyplus.com/browse/'],
  ['Disney+',movie?'P7595':'P7596',/^[0-9A-Za-z]{12}$/,movie?'https://www.disneyplus.com/movies/wd/':'https://www.disneyplus.com/series/wp/'],
  ['Hulu',movie?'P6466':'P6467',uuidText,movie?'https://www.hulu.com/movie/':'https://www.hulu.com/series/'],
  ['Paramount+','P13147',/^[A-Za-z0-9_-]{8,80}$/,'https://www.paramountplus.com/shows/video/',true],
  ['Peacock','P11815',/^(?:movies|tv)\/[a-z0-9]+(?:-[a-z0-9]+)*$/,'https://www.peacocktv.com/stream-'],
  ['Prime Video','P8055',/^B[0-9A-Z]{9}$/,'https://www.amazon.com/gp/video/detail/'],
  ['HBO Max','P8298',/^(?:movie|show)\/[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i,'https://www.hbomax.com/'],
  ['Tubi',movie?'P7760':'P7761',/^[1-9][0-9]{0,11}$/,movie?'https://tubitv.com/movies/':'https://tubitv.com/series/'],
 ];
 const sources=[],seen=new Set();
 for(const [provider,property,valid,prefix,player] of rules){
  const id=statedIdentifiers(claims,property,valid).find(value=>
   property==='P11815'?value.startsWith(movie?'movies/':'tv/'):
   property==='P8298'?value.startsWith(movie?'movie/':'show/'):true);
  if(!id||seen.has(provider))continue;
  seen.add(provider);
  const suffix=provider==='Paramount+'?'/':'';
  sources.push({provider,url:prefix+id+suffix,kind:player?'player':'title'});
 }
 return sources;
}
async function directCatalogIdentity(item,signal){
 if(item.kind==='movie')return item.id;
 const find=async(property,value)=>{
  const data=await wikidataJSON({action:'query',list:'search',srnamespace:'0',srlimit:'3',srprop:'',
   srsearch:'haswbstatement:'+property+'='+value},signal);
  if(!Array.isArray(data?.query?.search))throw Error('Catalog identity unavailable');
  const ids=[...new Set(data.query.search.map(row=>row?.title).filter(id=>typeof id==='string'&&entityId.test(id)))];
  return ids.length===1?ids[0]:null;
 };
 const exact=await find('P8600',item.id);
 if(exact||typeof item.imdbId!=='string'||!filmId.test(item.imdbId))return exact;
 return find('P345',item.imdbId);
}
async function findTitleSources(item){
 const signal=routeController?.signal;
 if(!signal||signal.aborted||item.sources?.loading)return;
 const request=item.sources={loading:true};render();
 try{
  const key=await directCatalogIdentity(item,signal);
  if(signal.aborted||item.sources!==request)return;
  if(!key){item.sources={links:[],imdbIds:[]};render();return;}
  const response=await wikidataJSON({action:'wbgetentities',ids:key,props:'claims'},signal);
  if(signal.aborted||item.sources!==request)return;
  const claims=response?.entities?.[key]?.claims;
  if(!claims||response?.error)throw Error('Provider identities unavailable');
  item.sources={links:directProviderLinks(claims,item.kind),imdbIds:statedIdentifiers(claims,'P345',filmId)};
 }catch{
  if(signal.aborted||item.sources!==request)return;
  item.sources={error:true};
 }
 render();
}
function publicTitleActions(item){
 const actions=node('div',undefined,'provider-actions'),status=item.sources;
 const library=state.session&&state.search?.pages.find(page=>page.lane==='library');
 const ids=status?.imdbIds||[];
 const owned=library?.items.filter(value=>value.Type==='Movie'&&item.kind==='movie'&&
  ids.includes(value.ProviderIds?.Imdb||value.ProviderIds?.IMDb));
 for(const media of owned||[]){
  const caption=resumeTicks(media)>0?'Resume in Bluefin':'Play in Bluefin';
  const start=action(caption,()=>play(media),'primary','play');
  start.dataset.focusKey='film-play|'+id(media.Id);
  actions.append(start);
 }
 if(status?.loading)actions.append(node('p','Looking up providers…','loading'));
 else if(status?.error){
  actions.append(node('p','Provider lookup is temporarily unavailable.','secondary'),
   button('Retry',()=>findTitleSources(item),'plain'));
 }else if(status){
  for(const source of status.links){
   const link=node('a',source.kind==='player'?'Watch on '+source.provider:'Open on '+source.provider);
   link.href=source.url;link.rel='noopener noreferrer';link.className='provider-link';actions.append(link);
  }
  if(!status.links.length)actions.append(node('p','No direct provider destinations found.','secondary'));
  else actions.append(node('p','The provider confirms streaming availability for your region and account.','secondary'));
 }
 return actions;
}
function publicCatalogResults(catalog){
 const {label,source,icon:category}=catalog.provider;
 const section=node('section',undefined,'search-results');
 section.append(node('h2',label));
 if(catalog.loading)section.append(node('p','Searching '+label.toLowerCase()+'…','loading'));
 else if(catalog.unavailable)section.append(node('p','Could not load '+source+' results.','error'));
 else if(!catalog.items.length)section.append(node('p','No matches from '+source+'.','secondary'));
 else{
  const grid=node('div',undefined,'search-grid');
  for(const item of catalog.items){
   const card=node('div',undefined,'public-title');
   const art=node('div',undefined,'artwork');
   if(item.art){const image=node('img');image.alt='';image.loading='lazy';image.referrerPolicy='no-referrer';image.onerror=()=>art.replaceChildren(icon(category));image.src=item.art;art.append(image);}
   else art.append(icon(category));
   const open=button(item.name,()=>findTitleSources(item),'plain');
   open.dataset.focusKey='public-title|'+item.kind+'|'+item.id;
   open.setAttribute('aria-label','Where to watch '+item.name);
   card.append(art,open,node('small',[item.description||item.year,source].filter(Boolean).join(' · ')),publicTitleActions(item));
   grid.append(card);
  }
  section.append(grid);
 }
 return section;
}
function publicSearchView(){
 const main=node('main',undefined,'app-main search-screen');main.id='content';
 main.append(button('Back',()=>{
  routeController?.abort();state.phase='connect';state.route={kind:'home'};state.back=[];state.search=null;state.query='';render();
 },'plain','back'),node('h1','Search movies and shows'));
 searchView(main);
 return main;
}
function searchView(main){
 const route=state.route,filters=cleanSearchFilters(route.filters);
 const form=node('form',undefined,'search-form'),input=node('input');
 form.setAttribute('role','search');
 input.id='bluefin-search';input.type='search';input.placeholder=state.session?'Titles, artists, and channels':'Movie or TV show title';input.value=state.query;
 input.setAttribute('aria-label',state.session?'Search all media':'Search movies and shows');input.maxLength=256;
 input.oninput=()=>{state.query=input.value;};
 const submit=node('button','Search','primary');
 submit.type='submit';submit.id='bluefin-search-submit';
 form.append(icon('search'),input,submit);
 form.onsubmit=event=>{event.preventDefault();startSearch(input.value,filters,true);};
 main.append(form);
 const search=state.search;
 if(!state.session){
  for(const catalog of search?.catalogs||[])main.append(publicCatalogResults(catalog));
  return;
 }
 const tools=node('div',undefined,'search-tools'),scopes=node('div',undefined,'search-scopes');
 scopes.setAttribute('role','group');scopes.setAttribute('aria-label','Media type');
 for(const [scope,label] of searchScopes){
  const select=button(label,()=>startSearch(state.query,{...filters,scope}));
  select.id='bluefin-scope-'+scope;
  select.setAttribute('aria-pressed',String(scope===filters.scope));
  scopes.append(select);
 }
 tools.append(scopes);
 const refine=button('Filters',()=>{
  route.refine=!route.refine;render();if(route.refine)loadSearchFacets().catch(error=>{state.error=String(error.message||error);render();});
 },'search-filter');
 refine.setAttribute('aria-expanded',String(!!route.refine));
 if(filters.scope!=='sports')tools.append(refine);
 main.append(tools);
 if(route.refine&&filters.scope!=='sports'){
  const panel=node('section',undefined,'search-refinements'),facets=state.facets;
  panel.append(searchChoice('Studio','studio',filters.studioID,[['','All studios'],...(facets?.studios||[]).map(value=>[value.id,value.name])],
   studioID=>startSearch(state.query,{...filters,studioID}),facets?.pending?.has('studios')||false));
  panel.append(searchChoice('Tag','tag',filters.tag,[['','All tags'],...(facets?.tags||[]).map(value=>[value,value])],
   tag=>startSearch(state.query,{...filters,tag}),facets?.pending?.has('tags')||false));
  panel.append(searchChoice('Collection','collection',filters.collectionID,[['','All collections'],...(facets?.collections||[]).map(value=>[value.id,value.name])],
   collectionID=>startSearch(state.query,{...filters,collectionID}),facets?.pending?.has('collections')||false));
  main.append(panel);
  if(facets?.loading)main.append(node('p','Loading '+[...facets.pending].join(', ')+'…','loading'));
  if(facets?.truncated)main.append(node('p','Your server has more refinements than this menu shows. Open Collections to browse all groups.','secondary'));
  if(facets?.problems.length){main.append(node('p',facets.problems.join(' '),'error'),button('Reload refinements',()=>{clearSearchFacets();loadSearchFacets();}));}
 }
 if(hasSearchFacets(filters))main.append(node('p','Filtering your library. The live guide is searched when these refinements are cleared.','secondary'),
  button('Clear refinements',()=>startSearch(state.query,{scope:filters.scope}),'plain'));
 if(search?.route===route)for(const catalog of search.catalogs)main.append(publicCatalogResults(catalog));
 if(search?.route===route)for(const page of search.pages){
  if(!page.items.length)continue;
  const section=node('section',undefined,'search-results');section.append(node('h2',page.title));
  const guide=['channels','programmes'].includes(page.lane);
  const list=node('div',undefined,guide?'group':'search-grid');
  for(const item of page.items){
   if(!guide){list.append(searchPoster(item));continue;}
   const result=row(item,()=>selectGuideResult(item),[programmeTime(item),item.ChannelName].filter(Boolean).join(' · '));
   result.dataset.focusKey='guide-result|'+id(item.Id);
   list.append(result);
  }
  section.append(list);
  if(page.next<page.total){const more=button(page.loading?'Loading…':'More results',()=>loadMoreSearch(page.lane));more.disabled=page.loading;section.append(more);}
  main.append(section);
 }
 for(const problem of search?.problems||[])main.append(node('p',problem,'error'));
 const searched=route.query||filters.scope!=='all'||hasSearchFacets(filters);
 if(!route.loading&&!search?.pages.some(page=>page.items.length)){
  if(!searched)main.append(empty('Search titles, music, and live TV','Enter a title, artist, or channel, or choose a category to browse.'));
  else if(!search?.problems.length&&!search?.catalogs.some(catalog=>catalog.items.length))main.append(empty('No matches in connected resources','Try a different title or clear your filters.'));
 }
}
function sportsView(main){
 const games=state.route.sports||[];
 if(games.length){
  const grid=node('div',undefined,'sports-grid');
  for(const game of games){
   const card=node('section',undefined,'sports-card'),event=game.event||{};
   card.append(node('small',String(event.league_name||game.kind||'Sports').slice(0,128)),
    node('h2',String(event.name||game.title||game.native_title||'Sports broadcast').slice(0,256)));
   for(const team of (Array.isArray(event.competitors)?event.competitors:[]).slice(0,4)){
    const score=node('div',undefined,'score-row');score.append(node('span',String(team.short_name||team.name||team.abbreviation||'').slice(0,128)),
     node('strong',String(team.score||'').slice(0,24)));card.append(score);
   }
   const start=new Date(game.native_start||game.start||event.date),time=Number.isNaN(start.valueOf())?'':start.toLocaleString();
   if(event.detail||time)card.append(node('p',String(event.detail||time).slice(0,256),'secondary'));
   if(uuid(game.channel_id))card.append(action(game.channel_name?'Open '+String(game.channel_name).slice(0,128):'Open channel',
    ()=>openItem({Id:game.channel_id,Name:game.channel_name||'Live TV',Type:'TvChannel'}),'','live'));
   grid.append(card);
  }
  main.append(grid);
 }
 if(state.rows.length){const list=node('div',undefined,'group');for(const item of state.rows)list.append(row(item));main.append(list);}
 if(!games.length&&!state.rows.length&&!state.route.loading)main.append(empty('Sports for your account','Your server’s selected games and available sports broadcasts will appear here.'));
}
function libraryView(){const main=node('main',undefined,'app-main');main.id='content';const route=state.route,title=route.title||(route.kind==='detail'?route.item.Name:state.tab);main.append(toolbar(title,state.back.length>0));if(state.back.length)main.append(node('h1',title));
 if(route.kind==='search')main.classList.add('search-screen');
 if(route.kind==='local_music'||route.kind==='local_search')localMusicView(main);
 else if(route.kind==='local_connect'){main.append(empty('Connect to your live TV','Open your server’s Bluefin address or use a device invitation to access its channels.'),action('Connect account',openAccountConnection,'primary'));
 }
 else if(route.kind==='home'){
  const feature=state.featured;
  if(feature){
   const hero=node('section',undefined,'hero featured'),copy=node('div',undefined,'hero-copy');bindArtwork(hero,feature,true);
   copy.append(node('small','FEATURED'),node('h2',feature.Name));if(subtitle(feature))copy.append(node('p',subtitle(feature)));
   const actions=node('div',undefined,'actions');
   if(['Movie','Episode','Audio','TvChannel'].includes(feature.Type))actions.append(action(resumeTicks(feature)>0?'Resume':'Play',()=>play(feature,[feature],0),'primary','play'));
   actions.append(action('Details',()=>openItem(feature),'hero-secondary','info'));copy.append(actions);hero.append(copy);main.append(hero);
  }
  for(const shelfValue of state.shelves)main.append(shelf(shelfValue.title,shelfValue.items));
  if(!feature&&!route.loading)main.append(empty('Browse your library','Open Collections to see your libraries.'));
 }
 else if(route.kind==='sports')sportsView(main);
 else if(route.kind==='library'){const collections=group('Collections');for(const item of state.rows)collections.append(row(item));if(!state.rows.length&&!state.busy)collections.append(node('p','No libraries are available to this account.','secondary'));main.append(collections);}
 else if(route.kind==='music'){
  main.append(node('h2','Your library','music-section-title'));
  const browse=node('div',undefined,'music-browse');
  for(const [name,type,symbol]of[['Playlists','Playlist','queue'],['Artists','MusicArtist','account'],['Albums','MusicAlbum','library'],['Songs','Audio','music']]){
   const entry=action(name,()=>openCollection(name,type,null),'',symbol),chevron=icon('chevron');chevron.setAttribute('class','chevron');entry.append(chevron);browse.append(entry);
  }
  const local=action('Music on this device',openLocalMusic,'','download'),chevron=icon('chevron');chevron.setAttribute('class','chevron');local.append(chevron);browse.append(local);main.append(browse);for(const s of state.shelves)main.append(shelf(s.title,s.items));if(!state.shelves.some(s=>s.items.length)&&!state.busy)main.append(empty('Your music, in one place','Albums from your connected library will appear here. Browse Songs to find individual tracks.'));}
 else if(route.kind==='search')searchView(main);
 else if(route.kind==='programme')programmeView(main,route.item);
 else if(route.kind==='collection'||route.kind==='live'){if(route.kind==='live'&&state.mode!=='mesh')main.append(node('p','You can browse channels here. Live TV playback is not yet available on this server’s web connection.','secondary'));const audio=route.type==='Audio';if(audio&&state.rows.length){const a=node('div',undefined,'actions');a.append(action('Play',()=>play(state.rows[0],state.rows,0),'','play'),action('Shuffle',()=>playShuffled(state.rows),'','shuffle'));main.append(a);}const list=node('div',undefined,audio||route.kind==='live'?'group':'catalog-grid');for(const item of state.rows)list.append(audio||route.kind==='live'?row(item):poster(item));main.append(list);if(!state.rows.length&&!state.busy)main.append(empty('No items to show','Try another collection or refresh your library.'));if(route.kind==='collection'){const a=node('div',undefined,'actions');if(route.start>0)a.append(action('Previous page',async()=>{route.start=Math.max(0,route.start-100);await loadRoute();}));if(route.start+state.rows.length<route.total)a.append(action('Next page',async()=>{route.start+=100;await loadRoute();}));main.append(a);}}
 else if(route.kind==='detail'){const item=route.item;main.append(cover(item,item.Type==='Audio'));main.lastChild.style.maxWidth='360px';main.append(node('h2',item.Name),node('p',subtitle(item),'secondary'));if(item.Type==='TvChannel'&&state.mode!=='mesh')main.append(node('p','Live TV playback is not yet available on this server’s web connection.','secondary'));else if(['Movie','Episode','Audio','TvChannel'].includes(item.Type))main.append(action(resumeTicks(item)>0?'Resume':'Play',()=>play(item,[item],0),'primary','play'));main.append(node('p',item.Overview||''));}
 if(state.busy||route.loading)main.append(node('p',route.kind==='search'?'Searching your account…':'Loading your library…','loading'));main.append(status());if(state.error)main.append(action('Try again',loadRoute));return main;}
function tabbar(){
 const nav=node('nav',undefined,'tabbar');nav.setAttribute('aria-label','Categories');
 const brand=node('div',undefined,'nav-brand');brand.append(icon('waves'),node('strong','Bluefin'));nav.append(brand);
 const searching=['search','local_search'].includes(state.route.kind);
 for(const [title,symbol]of categories){
  const entry=action(title,()=>navigate(title),'',symbol);
  entry.setAttribute('aria-current',!searching&&state.tab===title?'page':'false');
  nav.append(entry);
  if(title==='Home'){
   const group=node('div',undefined,'rail-search'),search=button('Search',()=>openSearch(),'','search');
   search.setAttribute('aria-current',searching?'page':'false');
   group.append(search);nav.append(group);
  }
 }
 return nav;
}
function sheet(){if(state.expanded)return nowPlaying();const outer=node('section',undefined,'sheet');outer.setAttribute('role','dialog');outer.setAttribute('aria-modal','true');outer.id='bluefin-dialog';outer.tabIndex=-1;outer.setAttribute('aria-label',state.sheet==='invite'?'Connect Bluefin':state.sheet==='connections'?'Connections':'Account and settings');const inner=node('div',undefined,'sheet-inner'),bar=node('div',undefined,'toolbar');bar.append(node('h2',state.sheet==='invite'?'Connect Bluefin':state.sheet==='connections'?'Connections':'Account and settings'),button('Done',closeSheet,'plain'));inner.append(bar);
 if(state.sheet==='invite'){const g=group(state.pairing?'Saved connection':'Connect with an invitation');if(state.pairing){g.append(node('p',state.remember?'This request is saved on this device.':'This request stays in this tab. Keep it open until your connection is complete.'));if(state.pairing.awaitingApproval)g.append(node('p','Your request is waiting for administrator approval.'));if(state.pairing.externalUrl){const u=new URL(state.pairing.externalUrl);if(u.protocol==='https:'&&!u.username&&!u.password){const a=node('a','Open passkey page');a.href=u.href;a.target='_blank';a.rel='noopener noreferrer';g.append(a,node('p','After verifying, return here to finish.','secondary'));}}g.append(action(state.pairing.awaitingApproval?'Check approval':state.pairing.needsBeginRetry?'Resume connection':'Verify and finish',continueMesh,'primary'));}
 else {
  g.append(node('p','Scan a Bluefin QR code, open an invitation image, or paste the full connection code. You will use a passkey to connect.'));
  const form=node('form');
  const [wrap,input]=field('Connection code','password',state.code,'mesh-code');
  input.autocomplete='off';input.spellcheck=false;input.maxLength=3072;
  input.oninput=()=>{state.code=input.value;};
  const remember=node('label',undefined,'check'),checkbox=node('input');
  checkbox.type='checkbox';checkbox.checked=state.remember;
  checkbox.onchange=()=>{state.remember=checkbox.checked;};
  remember.append(checkbox,node('span','Remember this device in protected browser storage'));
  const connect=node('button','Connect securely','primary');
  connect.disabled=state.busy;
  form.append(wrap,remember,node('p','Temporary by default. Closing this tab removes its connection key. Use Sign out to request immediate remote revocation.','secondary'),connect);
  form.onsubmit=event=>{
   event.preventDefault();
   const code=input.value.trim(),keep=checkbox.checked;
   input.value='';
   perform(()=>beginMesh(code,'',keep));
  };
  g.append(form);
  for(const [label,capture] of [['Open QR image or screenshot',false],['Take a photo of a QR code',true]]){
   const prompt=node('label',label),fileInput=node('input');
   fileInput.type='file';fileInput.accept='image/png,image/jpeg';
   if(capture)fileInput.setAttribute('capture','environment');
   fileInput.onchange=()=>{
    const file=fileInput.files?.[0];
    fileInput.value='';
    if(file)perform(()=>readInvitationImage(file));
   };
   prompt.append(fileInput);g.append(prompt);
  }
  if(validInvitation()){
   const native=node('a','Open in the installed Bluefin app');
   native.href='bluefin://connect/'+state.code;
   g.append(native);
  }
 }
 if(!state.pairing)g.append(installationLink());
 g.append(action('Resume request in this tab',()=>resumeMesh(false)),action('Restore a remembered device',()=>resumeMesh(true)));inner.append(g,node('p','The QR verifies a device connection. Your browser still uses its normal trusted HTTPS certificate.','secondary'));
 }else if(state.sheet==='account'){const g=group(state.session?.name||'Music on this device');g.append(node('p',!state.session?'Temporary local music · no account connected':state.mode==='mesh'?'Connected through the encrypted Bluefin mesh':'Signed in through the server address'));if(state.session)g.append(button('Connections',()=>{state.sheet='connections';render();},'','qr'));else g.append(action('Connect account',openAccountConnection,'','qr'));g.append(action(state.session?'Sign out':'Close local library',signOut,'danger'));inner.append(g);}
 else if(state.sheet==='connections'){const device=group('This device');device.append(node('p',state.mode==='mesh'?'Connected through the encrypted Bluefin mesh':'Signed in through the server address'));if(state.mode==='mesh')device.append(node('p',state.remember?'Your connection key is saved in protected storage on this browser.':'Your connection key exists only in this tab.','secondary'));inner.append(device);const g=group('Connect another device to your account');g.append(node('p','Use this invitation on a device you own. Your account passkey is still required.'),action('Create one-use invitation',issueInvite));if(state.invitation){const invite=state.invitation,image=node('img',undefined,'qr');image.src=invite.image;image.alt='One-use Bluefin device invitation';g.append(image,node('p','Every copy is the same invitation. Once claimed, no copy can connect another device.','secondary'));g.append(button('Copy invitation',()=>copy(invite.webLink)),button('Copy connection code',()=>copy(invite.code)));const save=node('a','Save QR image');save.href=invite.image;save.download='bluefin-invitation.png';g.append(save);const d=node('details'),summary=node('summary','Manual connection code');d.append(summary,node('p',invite.code,'code'));g.append(d);}g.append(node('p','Sharing a library with another person requires a separate grant to that person’s own account. A device invitation does not authorize shared storage or onward sharing.','secondary'));inner.append(g);}
 if(state.sheet==='account'&&state.session){const home=group('Home personalization');home.append(node('p','Your viewing progress and favorites shape Home on your account’s server.','secondary'));if(state.homeHistory)home.append(node('p','Submitted searches and selected results also help. History stays with this account for up to 90 days, with at most 64 recent events.','secondary'),action('Clear search history',clearHomeSearchHistory,'danger'));inner.append(home);}if(state.sheet==='account')inner.append(sourceFooter());if(state.busy)inner.append(node('p','Connecting…','loading'));inner.append(status());outer.append(inner);return outer;}
async function copy(value){try{await navigator.clipboard.writeText(value);state.notice='Copied.';}catch(_){state.error='Copy was not available. Open the manual code and select it.';}render();}
async function issueInvite(){
 const epoch=state.epoch,session=state.session,mode=state.mode;
 const current=()=>{assertLive(epoch,session);if(state.sheet!=='connections')throw Error('The connection screen was closed.');};
 current();state.invitation=null;clearTimeout(state.inviteTimer);
 const response=await api('Bluefin/Accounts/Mesh/Invite','POST',{intent:'login',lifetimeSeconds:600});current();
 let result;
 if(mode==='mesh'){const engine=await runtime().engine();current();result=JSON.parse(engine.webMeshInvitation(JSON.stringify(response)));}
 else{
  const link=new URL(response.webLink||response.link),code=response.code,png=response.pngBase64;
  if(response.singleUse!==true||response.intent!=='login'||!/^MC1-[A-Z0-9-]+$/i.test(code||'')||code.length>2048||
   link.origin!==location.origin||link.protocol!=='https:'||link.username||link.password||link.search||link.hash!=='#bluefin-connect='+code||
   typeof png!=='string'||png.length>1400000||!/^[A-Za-z0-9+/]*={0,2}$/.test(png)||!/^[a-f0-9]{64}$/.test(response.handle||''))throw Error('The server returned an invalid invitation.');
  result={webLink:link.href,code,image:'data:image/png;base64,'+png,handle:response.handle};
 }
 current();state.invitation=result;
 const poll=async()=>{
  if(pageRetired||epoch!==state.epoch||state.sheet!=='connections'||state.invitation!==result)return;
  try{const value=await api('Bluefin/Accounts/Mesh/Invites/'+result.handle);current();if(state.invitation!==result)return;
   if(value.state!=='issued'){state.invitation=null;state.notice='This invitation is no longer available. Create a new invitation for another device.';render();return;}}
  catch(_){if(pageRetired||epoch!==state.epoch||state.invitation!==result)return;state.invitation=null;state.notice='The invitation status could not be confirmed.';render();return;}
  state.inviteTimer=setTimeout(poll,3000);
 };
 state.inviteTimer=setTimeout(poll,3000);
}
function shuffled(items){const values=items.slice();for(let i=values.length-1;i>0;i--){const draw=new Uint32Array(1);crypto.getRandomValues(draw);const j=draw[0]%(i+1);[values[i],values[j]]=[values[j],values[i]];}return values;}
function time(value){if(!Number.isFinite(value)||value<0)value=0;return Math.floor(value/60)+':'+String(Math.floor(value%60)).padStart(2,'0');}
function nativeLeasePath(raw,kind,idValue){if(typeof raw!=='string'||raw.length>256||!raw.startsWith('/Bluefin/Web/v1/')||raw.includes('%')||raw.includes('?')||raw.includes('#')||raw.includes('\\')||raw.includes('..'))throw Error('Invalid playback lease route.');const u=new URL(raw,location.origin);if(u.origin!==location.origin||u.username||u.password||u.hash||u.search)throw Error('Invalid playback lease origin.');const pattern=kind==='media'?/^\/Bluefin\/Web\/v1\/media\/([a-f0-9]{64})\/[0-9]+$/:/^\/Bluefin\/Web\/v1\/leases\/([a-f0-9]{64})$/;const match=u.pathname.match(pattern);if(!match||(idValue&&match[1]!==idValue))throw Error('Invalid playback lease scope.');return {url:u.href,id:match[1]};}
// The cookie adapter exposes a public route ID. The old TV adapter exposes a
// secret per-item capability. Keep their validation and lifetimes distinct.
function tvLeasePath(raw,resource,idValue){
 if(typeof raw!=='string'||raw.length>256||!raw.startsWith('/Bluefin/TV/v1/')||/[?#%\\]/.test(raw)||raw.includes('..'))throw Error('Invalid TV playback capability.');
 const u=new URL(raw,location.origin);
 const pattern=resource==='media'?/^\/Bluefin\/TV\/v1\/media\/([a-f0-9]{64})\/[0-9]+$/:/^\/Bluefin\/TV\/v1\/leases\/([a-f0-9]{64})$/;
 const match=u.pathname.match(pattern);
 if(u.origin!==location.origin||u.username||u.password||u.hash||u.search||!match||(idValue&&match[1]!==idValue))throw Error('Invalid TV playback scope.');
 return {url:u.href,id:match[1]};
}
function mediaLeasePath(kind,raw,resource,idValue){
 if(kind==='cookie')return nativeLeasePath(raw,resource,idValue);
 if(kind==='tv')return tvLeasePath(raw,resource,idValue);
 throw Error('Unknown playback lease type.');
}
async function httpsMediaPolicy(session,base,epoch){
 assertLive(epoch,session);
 try{
  const value=await http('Bluefin/Web/v1','GET',undefined,[],session,false,false,base);assertLive(epoch,session);
  if(value?.version!==1||value.cookieMediaLease!==true||value.expiresInSeconds!==300||value.maxSessionSeconds!==7200||value.idleSeconds!==90)throw Error('The server returned an unsupported browser playback capability.');
  return {kind:'cookie',create:'Bluefin/Web/v1/leases'};
 }catch(error){
  // Absence is the only compatibility selector. Auth failures, malformed
  // advertisements, timeouts and failed creations never choose another path.
  if(![404,405,501].includes(error.status))throw error;
 }
 assertLive(epoch,session);
 const value=await http('Bluefin/TV/v1','GET',undefined,[],session,false,false,base);assertLive(epoch,session);
 if(value?.version!==1||value.mediaLease!==true||value.absoluteSeconds!==7200||value.idleSeconds!==90)throw Error('This server does not advertise a supported browser media capability.');
 return {kind:'tv',create:'Bluefin/TV/v1/leases'};
}
function releaseLease(lease,session,keepalive=false){return leaseRequest(lease.releaseUrl,'DELETE',session,keepalive,lease.kind);}
async function leaseRequest(raw,method,session=state.session,keepalive=false,kind='cookie'){
 if(!keepalive)assertLive();
 if(!['DELETE','POST','HEAD'].includes(method)||(kind==='tv'&&method==='POST')||(kind==='cookie'&&method==='HEAD'))throw Error('Invalid playback lease operation.');
 const path=mediaLeasePath(kind,raw,method==='HEAD'?'media':'lease');
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
 try{previous?.source?.release();}catch(error){failure=failure||error;}
 if(previous?.domOwner&&media.__bluefinOutputOwner===previous.domOwner)delete media.__bluefinOutputOwner;
 document.body.classList.remove('audio-playing','video-playing','with-player');
 return failure;
}
async function stopPlayback(remote=true){
 const previous=state.player;state.player=null;state.expanded=false;state.nowPanel='artwork';retireLyrics(previous);
 clearInterval(leaseRenew);clearInterval(progressTimer);clearInterval(meshPoll);
 leaseRenew=progressTimer=meshPoll=null;playbackGeneration++;
 let failure=clearOutput(previous);
 try{
  if(previous?.mode==='mesh'){
   const engine=runtime()?.ready();
   const pending=Promise.resolve().then(()=>{if(typeof engine?.webMeshStop!=='function')throw Error('The playback retirement module is unavailable. Reload before starting another source.');return engine.webMeshStop(previous.handle);});
   meshRetirement=pending;await pending;if(meshRetirement===pending)meshRetirement=null;
  }else{
   if(meshRetirement)await meshRetirement;
   if(previous?.lease&&remote)await releaseLease(previous.lease,previous.session);
  }
 }catch(error){failure=failure||error;}
 document.getElementById('media-status').textContent='';
 if(failure)throw failure;
}
function resumeTicks(item){
 const value=item.UserData?.PlaybackPositionTicks,duration=item.RunTimeTicks;
 return Number.isSafeInteger(value)&&value>0&&Number.isSafeInteger(duration)&&value<duration?value:0;
}
async function play(item,queue=[item],index=0,original=null){
 if(isLocalItem(item))return playLocal(item,queue,index,original);
 const epoch=state.epoch,session=state.session,mode=state.mode,base=state.base,preserve=queue===state.queue&&state.player?.mode===state.mode,presentation={expanded:state.expanded,panel:state.nowPanel};assertLive(epoch,session);
 if(!session)throw Error('Sign in to play your library.');
 if(mode!=='mesh'&&item.Type==='TvChannel')throw Error('Live TV playback is not yet available on this server’s web connection.');
 if(!['Audio','Movie','Episode','TvChannel'].includes(item.Type))return openItem(item);
 await stopPlayback();assertLive(epoch,session);
 const generation=playbackGeneration;
 const current=()=>{assertLive(epoch,session);if(generation!==playbackGeneration)throw Error('Playback was canceled.');};
 installQueue(queue,index,original,preserve);
 if(mode==='mesh'){
  await hlsSupport();current();
  try{await runtime().mediaWorker();}catch(error){
   current();
   if(error.bluefinBrowserCapability)showCapability(error,()=>play(item,queue,index),false,true);
   throw error;
  }current();
  const engine=await meshEngine(()=>play(item,queue,index),false,true);current();
  engine.webMeshPlay(session.handle,JSON.stringify(item));current();
  state.player={mode:'mesh',handle:session.handle,item,session,offset:0,snapshot:null};
  meshPoll=setInterval(()=>{
   if(pageRetired||generation!==playbackGeneration||session!==state.session)return;
   try{const sample=JSON.parse(engine.webMeshPlayback(session.handle));if(state.player?.handle===session.handle){state.player.snapshot=sample;state.player.offset=Number.isFinite(sample?.plan?.offset)?sample.plan.offset:0;}if(sample?.message)document.getElementById('media-status').textContent=sample.message;updatePlayback();}
   catch(_){if(generation===playbackGeneration)stopPlayback(false).catch(()=>{});}
  },500);
 }else{
  if(item.Type==='Audio'&&!media.canPlayType('audio/mpeg'))throw Error('This browser cannot play the server’s MP3 audio profile.');
  const Hls=item.Type==='Audio'?null:await hlsSupport();current();
  const policy=await httpsMediaPolicy(session,base,epoch);current();
  const wire=await http(policy.create,'POST',{version:1,itemId:id(item.Id),
   playback:{kind:item.Type==='Audio'?'audio':'video',maxBitrate:8000000,startTicks:resumeTicks(item)}},[],session,false,true,base);
  const lease={...wire,kind:policy.kind};
  let route;
  try{
   route=mediaLeasePath(lease.kind,lease.mediaUrl,'media');mediaLeasePath(lease.kind,lease.releaseUrl,'lease',route.id);
   if(lease.version!==1||!uuid(lease.playSessionId)||lease.idleSeconds!==90)throw Error('Invalid playback lease.');
   if(lease.kind==='cookie'){nativeLeasePath(lease.renewUrl,'lease',route.id);if(lease.expiresInSeconds!==300)throw Error('Invalid browser lease lifetime.');}
   else if(lease.expiresInSeconds!==7200||lease.renewUrl!==undefined)throw Error('Invalid TV capability lifetime.');
  }catch(error){if(typeof lease?.releaseUrl==='string')await releaseLease(lease,session,true).catch(()=>{});throw error;}
  if(pageRetired||epoch!==state.epoch||session!==state.session||generation!==playbackGeneration){
   await releaseLease(lease,session,true).catch(()=>{});current();
  }
  state.player={mode:'https',item,lease,session,offset:item.Type==='Audio'?resumeTicks(item)/10000000:0,
   resumePosition:resumeTicks(item)/10000000,resumeApplied:item.Type==='Audio',reportingReady:item.Type==='Audio'||resumeTicks(item)===0,domOwner:Object.freeze({generation,kind:'https'})};
  media.__bluefinOutputOwner=state.player.domOwner;
  if(Hls){
   const hls=new Hls({enableWorker:false,startPosition:state.player.resumePosition,maxBufferLength:20,maxMaxBufferLength:40,backBufferLength:10,maxBufferSize:32*1024*1024,
    manifestLoadingTimeOut:10000,fragLoadingTimeOut:10000,fragLoadingMaxRetry:1,manifestLoadingMaxRetry:1,
    xhrSetup(xhr,url){const target=new URL(url,location.origin);if(target.origin!==location.origin||target.search||target.hash||target.username||target.password)throw Error('Media left its lease origin.');mediaLeasePath(lease.kind,target.pathname,'media',route.id);}});
   state.player.hls=hls;hls.on(Hls.Events.ERROR,(_event,data)=>{
    if(data.fatal&&!pageRetired&&generation===playbackGeneration){stopPlayback().catch(()=>{});state.error='The media decoder could not play this title.';render();}
   });hls.loadSource(route.url);hls.attachMedia(media);
  }else{media.src=route.url;media.load();}
  leaseRenew=setInterval(async()=>{
   if(pageRetired||generation!==playbackGeneration||session!==state.session)return;
   try{if(lease.kind==='cookie')await leaseRequest(lease.renewUrl,'POST',session);else if(!media.paused)await leaseRequest(lease.mediaUrl,'HEAD',session,false,'tv');}
   catch(_){if(pageRetired||generation!==playbackGeneration||session!==state.session)return;
    await stopPlayback(false).catch(()=>{});state.error='Playback stopped because its access lease could not be renewed.';render();}
  },60000);
  progressTimer=setInterval(()=>{if(!pageRetired&&generation===playbackGeneration)reportProgress('progress').catch(()=>{});},10000);
  reportProgress('started').catch(()=>{});current();
  media.play().catch(()=>{if(!pageRetired&&generation===playbackGeneration)document.getElementById('media-status').textContent='Press Play to begin.';});
 }
 current();state.expanded=item.Type==='Audio'&&presentation.expanded;state.nowPanel=presentation.panel;loadLyrics(state.player);
  document.body.classList.toggle('audio-playing',item.Type==='Audio');
 document.body.classList.toggle('video-playing',item.Type!=='Audio');document.body.classList.add('with-player');render();
}
async function reportProgress(phase){const p=state.player;if(!p||p.mode!=='https'||p.session!==state.session||!p.reportingReady)return;await api('Sessions/Playing'+(phase==='started'?'':phase==='stopped'?'/Stopped':'/Progress'),'POST',{ItemId:p.item.Id,PlaySessionId:p.lease.playSessionId,PositionTicks:Math.floor(playbackTimeline().position*10000000),IsPaused:media.paused,CanSeek:p.item.Type!=='TvChannel',PlayMethod:'Transcode'});}
async function skip(forward,automatic=false){if(!state.player||!state.queue.length)return;let next=state.queueIndex+(forward?1:-1);if(automatic&&state.repeat==='one')next=state.queueIndex;if(next>=state.queue.length){if(state.repeat!=='all')return stopPlayback();next=0;}if(next<0){if(state.repeat!=='all')next=0;else next=state.queue.length-1;}await play(state.queue[next],state.queue,next);}
function toggle(){if(media.paused)media.play().catch(()=>{state.error='Press Play in the media controls.';render();});else media.pause();updatePlayback();}

// Lyrics use the authenticated library route and the current media clock. Word
// timing is rendered only when the server supplies real, valid UTF-16 cues.
function normalizeLyrics(value){
 if(!value||!Array.isArray(value.Lyrics)||value.Lyrics.length>4096)throw Error('Invalid lyrics.');
 const offset=value.Metadata?.Offset??0;if(!Number.isSafeInteger(offset))throw Error('Invalid lyric offset.');
 let characters=0,cueCount=0;
 const tick=value=>Number.isSafeInteger(value)?value/10000000:null;
 const boundary=(text,index)=>index===0||index===text.length||!(text.charCodeAt(index-1)>=0xd800&&text.charCodeAt(index-1)<=0xdbff&&text.charCodeAt(index)>=0xdc00&&text.charCodeAt(index)<=0xdfff);
 const lines=value.Lyrics.map(raw=>{
  if(typeof raw?.Text!=='string'||raw.Text.length>8192||(characters+=raw.Text.length)>262144)throw Error('Invalid lyric text.');
  const line={text:raw.Text,start:tick(raw.Start),cues:[]};
  if(Array.isArray(raw.Cues)&&raw.Cues.length&&line.start!==null){
   if((cueCount+=raw.Cues.length)>16384)throw Error('Too many lyric cues.');
   let endPosition=0,lastStart=-Infinity,valid=true;
   for(const cue of raw.Cues){
    const start=tick(cue?.Start),end=cue?.End==null?null:tick(cue.End),from=cue?.Position,to=cue?.EndPosition;
    if(!Number.isInteger(from)||!Number.isInteger(to)||from<endPosition||to<=from||to>line.text.length||!boundary(line.text,from)||!boundary(line.text,to)||start===null||start<lastStart||(cue?.End!=null&&(end===null||end<start))){valid=false;break;}
    line.cues.push({from,to,start,end});endPosition=to;lastStart=start;
   }
   if(!valid)line.cues=[];
  }
  return line;
 });
 const timed=lines.map((line,index)=>({index,start:line.start})).filter(line=>line.start!==null).sort((a,b)=>a.start-b.start||a.index-b.index);
 return {lines,timed,offset:offset/10000000};
}
function retireLyrics(player){
 if(player?.lyrics){player.lyrics.controller?.abort();player.lyrics.lines=[];player.lyrics.timed=[];player.lyrics.status='retired';player.lyrics=null;}
 lyricsView=null;
 if(lyricsFrame!==null){globalThis.cancelAnimationFrame?.(lyricsFrame);lyricsFrame=null;}
}
async function loadLyrics(player,force=false){
 if(pageRetired||state.player!==player||!player||player.item.Type!=='Audio'||(!force&&player.lyrics))return;
 player.lyrics?.controller?.abort();
 const epoch=state.epoch,generation=playbackGeneration,session=state.session;
 const lyrics={status:player.mode==='local'?'local':'loading',lines:[],timed:[],offset:0,follow:true,controller:new AbortController()};
 player.lyrics=lyrics;
 if(player.mode==='local')return;
 const current=()=>!pageRetired&&state.player===player&&player.lyrics===lyrics&&state.epoch===epoch&&state.session===session&&generation===playbackGeneration&&!lyrics.controller.signal.aborted;
 try{
  const reply=await api('Audio/'+id(player.item.Id)+'/Lyrics','GET',undefined,[],lyrics.controller.signal);
  if(!current())return;
  Object.assign(lyrics,normalizeLyrics(reply));lyrics.status=lyrics.lines.some(line=>line.text.trim())?'ready':'missing';
 }catch(error){
  if(!current())return;
  lyrics.status=notFound(error)?'missing':'error';
 }
 if(current()&&state.expanded&&state.nowPanel==='lyrics')render();
}
function lyricsPosition(){
 const p=state.player;
 // The mesh decoder owns this same media element. Read its exact clock once
 // the current plan is ready instead of animating between periodic snapshots.
 if(p?.mode==='mesh'&&p.snapshot?.plan&&media.__bluefinOutputOwner&&Number.isFinite(media.currentTime))return (p.offset||0)+media.currentTime;
 return playbackTimeline().position;
}
function activeLyric(lyrics,position){
 const time=position-lyrics.offset;let lo=0,hi=lyrics.timed.length;
 while(lo<hi){const mid=(lo+hi)>>1;if(lyrics.timed[mid].start<=time)lo=mid+1;else hi=mid;}
 return lo?lyrics.timed[lo-1].index:-1;
}
function updateLyrics(force=false){
 const view=lyricsView,p=state.player,lyrics=p?.lyrics;
 if(!view||view.player!==p||!state.expanded||state.nowPanel!=='lyrics'||lyrics?.status!=='ready'||!lyrics.timed.length)return;
 const position=lyricsPosition(),index=activeLyric(lyrics,position),changed=index!==view.active;
 if(changed){
  if(view.active>=0)view.lines[view.active]?.element.removeAttribute('aria-current');
  view.active=index;
  if(index>=0)view.lines[index]?.element.setAttribute('aria-current','true');
 }
 if(index>=0){
  const line=view.lines[index],time=position-lyrics.offset;
  for(const word of line.words){
   const progress=time<word.cue.start?0:word.cue.end!==null&&word.cue.end>word.cue.start?Math.round(Math.min(1,Math.max(0,(time-word.cue.start)/(word.cue.end-word.cue.start)))*1000)/10:100;
   if(word.progress!==progress){word.element.style.setProperty('--word-progress',progress+'%');word.progress=progress;}
  }
  if((changed||force)&&lyrics.follow&&Number.isFinite(line.element.offsetTop)){
   const top=Math.max(0,line.element.offsetTop-view.scroll.clientHeight*.4+line.element.clientHeight*.5);
   const reduced=globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
   view.scroll.scrollTo?.({top,behavior:force||reduced?'auto':'smooth'});
  }
 }
 view.follow.hidden=lyrics.follow;
}
function syncLyricsAnimation(){
 const active=()=>!pageRetired&&document.visibilityState!=='hidden'&&state.expanded&&state.nowPanel==='lyrics'&&lyricsView?.player===state.player&&state.player?.lyrics?.status==='ready'&&state.player.lyrics.timed.length>0&&!media.paused;
 if(!active()){if(lyricsFrame!==null)globalThis.cancelAnimationFrame?.(lyricsFrame);lyricsFrame=null;return;}
 if(lyricsFrame!==null||typeof globalThis.requestAnimationFrame!=='function')return;
 const frame=()=>{lyricsFrame=null;if(!active())return;updateLyrics();lyricsFrame=globalThis.requestAnimationFrame(frame);};
 lyricsFrame=globalThis.requestAnimationFrame(frame);
}
async function seekLyric(player,index){
 if(state.player!==player)return;
 const lyrics=player.lyrics,line=lyrics?.lines[index];if(line?.start==null)return;
 const position=Math.max(0,line.start+lyrics.offset);lyrics.follow=true;
 // A resumed audio lease starts at an offset. Earlier lyrics require a fresh
 // scoped lease so tapping a line always seeks to that line, not the old start.
 if(player.mode==='https'&&position<(player.offset||0)){
  const item={...player.item,UserData:{...player.item.UserData,PlaybackPositionTicks:Math.floor(position*10000000)}};
  await play(item,state.queue,state.queueIndex);
 }else seekPlayback(position);
 updateLyrics(true);
}
function lyricPanel(player){
 const panel=node('section',undefined,'now-panel');panel.id='now-playing-panel';panel.setAttribute('aria-label','Lyrics');
 const lyrics=player.lyrics,heading=node('div',undefined,'now-panel-heading');heading.append(node('h2','Lyrics'));
 const follow=button('Follow song',()=>{if(state.player===player&&player.lyrics){player.lyrics.follow=true;updateLyrics(true);}},'plain');follow.hidden=lyrics?.follow!==false;heading.append(follow);panel.append(heading);
 if(!lyrics||lyrics.status!=='ready'){
  const message=node('div',undefined,'lyrics-message');
  if(!lyrics||lyrics.status==='loading'){message.append(node('p','Loading lyrics…'));message.setAttribute('role','status');}
  else if(lyrics.status==='local')message.append(node('h2','Lyrics aren’t available for this file'),node('p','Open a song from your server library to use its lyrics.'));
  else if(lyrics.status==='missing')message.append(node('h2','Lyrics aren’t available for this song'),node('p','You can keep listening or choose another song.'));
  else{message.append(node('h2','Lyrics couldn’t be loaded'),node('p','Your song can keep playing.'),button('Try again',()=>{loadLyrics(player,true);render();}));}
  panel.append(message);return panel;
 }
 if(!lyrics.timed.length)panel.append(node('p','Timing is not available for these lyrics.','secondary'));
 const scroll=node('div',undefined,'lyrics-scroll');scroll.tabIndex=0;scroll.setAttribute('aria-label','Song lyrics');
 const view={player,scroll,follow,lines:[],active:-1};lyricsView=view;
 const browsing=()=>{if(state.player!==player)return;lyrics.follow=false;follow.hidden=false;};
 scroll.addEventListener('wheel',browsing,{passive:true});scroll.addEventListener('touchstart',browsing,{passive:true});
 scroll.addEventListener('pointerdown',browsing,{passive:true});
 scroll.addEventListener('keydown',event=>{if(['ArrowUp','ArrowDown','PageUp','PageDown','Home','End'].includes(event.key))browsing();});
 lyrics.lines.forEach((line,index)=>{
  const element=line.start===null?node('p',line.text,'lyric-line untimed'):button('',()=>perform(()=>seekLyric(player,index)),'lyric-line');
  element.dataset.lyricLine=String(index);const words=[];
  if(line.start!==null){
   element.setAttribute('aria-label','Seek to '+time(Math.max(0,line.start+lyrics.offset))+': '+line.text);
   if(line.cues.length){
    let at=0;
    for(const cue of line.cues){
     if(cue.from>at)element.append(node('span',line.text.slice(at,cue.from)));
     const word=node('span',line.text.slice(cue.from,cue.to),'lyric-word');word.dataset.lyricWord='';element.append(word);words.push({element:word,cue,progress:null});at=cue.to;
    }
    if(at<line.text.length)element.append(node('span',line.text.slice(at)));
   }else element.textContent=line.text;
  }
  view.lines.push({element,words});scroll.append(element);
 });
 panel.append(scroll);return panel;
}
function queuePanel(){
 const panel=node('section',undefined,'now-panel');panel.id='now-playing-panel';panel.setAttribute('aria-label','Playing next');
 const heading=node('div',undefined,'now-panel-heading');heading.append(node('h2','Playing next'));panel.append(heading);
 const scroll=node('div',undefined,'queue-scroll'),next=state.queue.slice(state.queueIndex+1);
 if(!next.length)scroll.append(node('p',state.repeat==='all'?'Your queue will repeat.':'You’re at the end of the queue.','secondary'));
 next.forEach((track,index)=>{
  const entry=action('',()=>play(track,state.queue,state.queueIndex+index+1),'queue-track');
  const label=node('span',undefined,'grow');label.append(node('strong',track.Name),node('small',subtitle(track)));entry.append(cover(track,true),label);scroll.append(entry);
 });
 panel.append(scroll);return panel;
}
function miniPlayer(){
 const p=state.player;if(!p||p.item.Type!=='Audio')return document.createDocumentFragment();
 const e=node('section',undefined,'mini-player');e.setAttribute('aria-label','Now playing');e.append(cover(p.item,true));
 const title=button('',()=>{state.expanded=true;render();},'grow');title.setAttribute('aria-label','Open Now Playing for '+p.item.Name);title.append(node('strong',p.item.Name),node('small',p.item.AlbumArtist||p.item.Artists?.join(', ')||subtitle(p.item)));e.append(title);
 const playButton=button('',toggle,'plain',media.paused?'play':'pause');playButton.setAttribute('aria-label',media.paused?'Play':'Pause');playButton.dataset.playToggle='';e.append(playButton);
 const next=action('',()=>skip(true),'plain','next');next.setAttribute('aria-label','Next track');next.disabled=state.busy||(state.repeat!=='all'&&state.queueIndex+1>=state.queue.length);e.append(next);return e;
}
function nowPlaying(){
 const p=state.player;if(!p){state.expanded=false;return document.createDocumentFragment();}
 const e=node('section',undefined,'sheet now-playing-sheet');e.setAttribute('role','dialog');e.setAttribute('aria-modal','true');e.setAttribute('aria-label','Now Playing');e.id='bluefin-dialog';e.tabIndex=-1;
 const inner=node('div',undefined,'now-playing'),bar=node('div',undefined,'toolbar');
 const close=button('',()=>{state.expanded=false;render();},'plain','down');close.setAttribute('aria-label','Close Now Playing');
 bar.append(close,node('span','NOW PLAYING','now-caption'),action('Stop',()=>stopPlayback(),'plain'));inner.append(bar);
 const layout=node('div',undefined,'now-layout'),stage=node('div',undefined,'now-stage');
 if(state.nowPanel==='lyrics')stage.append(lyricPanel(p));
 else if(state.nowPanel==='queue')stage.append(queuePanel());
 else{const art=node('div',undefined,'now-art');art.id='now-playing-panel';art.append(icon('music'));bindArtwork(art,p.item);stage.append(art);}
 layout.append(stage);
 const controls=node('div',undefined,'now-controls'),title=node('div',undefined,'now-track-title');
 title.append(node('h1',p.item.Name),node('p',p.item.AlbumArtist||p.item.Artists?.join(', ')||''));
 controls.append(title);if(p.item.Album||p.mode==='local')controls.append(node('p',p.mode==='local'?'Local file · '+p.item.Album:p.item.Album,'now-source'));
 const timeline=playbackTimeline(),seek=node('input');seek.type='range';seek.min=String(timeline.minimum);seek.max=String(Math.max(timeline.duration,timeline.minimum+1));seek.step='.1';
 seek.value=String(timeline.position);seek.disabled=p.item.Type==='TvChannel'||timeline.duration<=0;seek.setAttribute('aria-label','Playback position');seek.dataset.seek='';seek.oninput=()=>seekPlayback(Number(seek.value));
 const times=node('div',undefined,'time-row'),position=node('span',time(timeline.position)),remaining=node('span','−'+time(timeline.duration-timeline.position));
 position.dataset.position='';remaining.dataset.remaining='';times.append(position,remaining);controls.append(seek,times);
 const transport=node('div',undefined,'transport'),shuffle=button('',toggleShuffle,state.shuffle?'on':'','shuffle');shuffle.setAttribute('aria-label','Shuffle');shuffle.setAttribute('aria-pressed',String(state.shuffle));shuffle.disabled=state.queue.length<2;
 const previous=action('',()=>skip(false),'','previous');previous.setAttribute('aria-label','Previous track');
 const toggleButton=button('',toggle,'main-play',media.paused?'play':'pause');toggleButton.dataset.playToggle='';toggleButton.setAttribute('aria-label',media.paused?'Play':'Pause');
 const next=action('',()=>skip(true),'','next');next.setAttribute('aria-label','Next track');next.disabled=state.busy||(state.repeat!=='all'&&state.queueIndex+1>=state.queue.length);
 const repeat=button('',()=>{state.repeat=({off:'all',all:'one',one:'off'})[state.repeat];syncLocalQueue();render();},state.repeat==='off'?'':'on','repeat');repeat.setAttribute('aria-label','Repeat: '+state.repeat);repeat.setAttribute('aria-pressed',String(state.repeat!=='off'));
 transport.append(shuffle,previous,toggleButton,next,repeat);controls.append(transport);
 const options=node('div',undefined,'player-options');
 for(const [label,panel,symbol]of[['Lyrics','lyrics','lyrics'],['Queue','queue','queue']]){
  const choice=button(label,()=>{state.nowPanel=state.nowPanel===panel?'artwork':panel;render();},'',symbol);choice.setAttribute('aria-pressed',String(state.nowPanel===panel));choice.setAttribute('aria-controls','now-playing-panel');options.append(choice);
 }
 if(typeof media.webkitShowPlaybackTargetPicker==='function'){const output=button('AirPlay',()=>media.webkitShowPlaybackTargetPicker(),'','airplay');output.setAttribute('aria-label','Choose AirPlay speakers');options.append(output);}
 controls.append(options);layout.append(controls);inner.append(layout,status());e.append(inner);return e;
}
function updatePlayback(){
 const timeline=playbackTimeline();
 for(const b of document.querySelectorAll('[data-play-toggle]')){b.replaceChildren(icon(media.paused?'play':'pause'));b.setAttribute('aria-label',media.paused?'Play':'Pause');}
 for(const e of document.querySelectorAll('[data-position]'))e.textContent=time(timeline.position);
 for(const e of document.querySelectorAll('[data-remaining]'))e.textContent='−'+time(timeline.duration-timeline.position);
 for(const e of document.querySelectorAll('[data-seek]')){e.min=String(timeline.minimum);e.max=String(Math.max(timeline.duration,timeline.minimum+1));e.disabled=!timeline.duration;if(document.activeElement!==e)e.value=String(timeline.position);}
 updateLyrics();syncLyricsAnimation();
}
async function signOut(){
 const session=state.session,mode=state.mode,base=state.base;state.epoch++;state.pairingView++;
 clearTimeout(state.inviteTimer);state.sheet=null;state.phase='connect';state.session=null;clearPrivateView();clearImages();
 let cleanup;
 try{await stopPlayback();}catch(error){cleanup=error;}
 try{localLibrary?.clear();}catch(error){cleanup=cleanup||error;}
 state.localSnapshot=null;
 try{if(session){if(mode==='mesh'){await(await runtime().engine()).webMeshLogout(session.handle);meshRetirement=null;}
  else await http('Sessions/Logout','POST',undefined,[],session,false,true,base);}}
 catch(error){cleanup=error;}
 if(pageRetired)return;
 state.mode='https';state.base=hostedBase();state.server=null;state.options={};state.remember=false;
 if(cleanup)throw Error('Signed out in this tab. Remote cleanup is not confirmed; '+String(cleanup.message||cleanup));
}
function clearImages(){state.imageEpoch++;clearTimeout(artworkScrollTimer);for(const controller of imageControllers)controller.abort();imageControllers.clear();artworkPending.clear();artworkMissing.clear();for(const url of state.images.values())URL.revokeObjectURL(url);state.images.clear();}
function render(){
 if(pageRetired)return;
 lyricsView=null;
 const active=document.activeElement,activeId=active?.id,activeKey=active?.dataset?.focusKey;
 const selection=activeId==='bluefin-search'?[active.selectionStart,active.selectionEnd,active.selectionDirection]:null;
 const videoModal=!state.capability&&!!state.player&&state.player.item.Type!=='Audio'&&!state.sheet&&!state.expanded;
 const modal=!!state.capability||!!state.sheet||state.expanded||videoModal;
 if(modal&&!modalWasOpen)focusReturn={id:activeId,key:activeKey};
 document.body.classList.toggle('signed-in',!!state.session||state.local);
 const fragment=document.createDocumentFragment();
 const main=state.session||state.local?libraryView():state.phase==='public-search'?publicSearchView():welcome();
 if(modal){main.inert=true;main.setAttribute('aria-hidden','true');}fragment.append(main);
 if(state.session||state.local){const nav=tabbar(),mini=miniPlayer();if(modal){nav.inert=true;mini.inert=true;nav.setAttribute('aria-hidden','true');mini.setAttribute('aria-hidden','true');}fragment.append(nav,mini);}
 if(state.sheet||state.expanded){
  const sheetRoot=sheet();
  if(state.capability){sheetRoot.inert=true;sheetRoot.setAttribute('aria-hidden','true');}
  fragment.append(sheetRoot);
 }
 const help=state.capability?capabilitySheet():null;
 if(help)fragment.append(help);
 app.replaceChildren(fragment);
 if(help){
  if(typeof help.showModal==='function')help.showModal();
  else{help.setAttribute('open','');help.querySelector?.('button')?.focus?.();}
 }
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
 if(!help&&focus&&!focus.inert){
  focus.focus?.({preventScroll:true});
  if(selection&&focus.id===activeId&&selection[0]!==null)focus.setSelectionRange?.(...selection);
 }
 pumpArtwork();updateLyrics(true);syncLyricsAnimation();
}
document.addEventListener('visibilitychange',()=>{updateLyrics(true);syncLyricsAnimation();});
document.addEventListener('keydown',event=>{
 if(state.capability)return;
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

media.addEventListener('play',()=>{
 const p=state.player;
 if(!pageRetired&&p?.mode==='https'&&p.lease.kind==='tv'&&p.pausedAt&&Date.now()-p.pausedAt>70000){
  const position=Math.floor(((p.offset||0)+(media.currentTime||0))*10000000);
  const item={...p.item,UserData:{...p.item.UserData,PlaybackPositionTicks:position}};
  p.pausedAt=null;media.pause();perform(()=>play(item,state.queue,state.queueIndex));return;
 }
 if(p)p.pausedAt=null;updatePlayback();
});
media.addEventListener('pause',()=>{if(state.player)state.player.pausedAt=Date.now();updatePlayback();});
media.addEventListener('timeupdate',updatePlayback);
function applyMediaMetadata(){
 const p=state.player;if(pageRetired||!p)return;
 if(p.mode==='local'&&p.generation===playbackGeneration&&(media.currentSrc||media.src)===p.source.url)p.source.metadata(media.duration);
 else if(p.mode==='https'&&media.__bluefinOutputOwner===p.domOwner&&p.item.Type!=='Audio'&&!p.resumeApplied&&Number.isFinite(media.duration)&&media.duration>0){
  const target=p.resumePosition||0;
  if(target>0){
   const restart=target>=media.duration;
   try{media.currentTime=restart?0:target;p.resumeApplied=true;p.reportingReady=true;if(restart){p.resumePosition=0;state.notice='The saved position is beyond this version. Starting from the beginning.';document.getElementById('media-status').textContent=state.notice;render();}reportProgress('started').catch(()=>{});}catch(_){/* Retry when metadata becomes seekable. */}
  }else if(target===0){p.resumeApplied=true;p.reportingReady=true;}
 }
 updatePlayback();
}
media.addEventListener('loadedmetadata',applyMediaMetadata);
media.addEventListener('durationchange',applyMediaMetadata);
media.addEventListener('canplay',applyMediaMetadata);
media.addEventListener('encrypted',()=>{if(state.player?.mode!=='local'||pageRetired)return;stopPlayback(false).catch(()=>{});state.error='This file is protected. Play it through its licensed provider or choose an unprotected local file.';render();});
media.addEventListener('ended',()=>{if(pageRetired||!state.player||!media.ended)return;perform(async()=>{await reportProgress('stopped').catch(()=>{});await skip(true,true);});});
media.addEventListener('error',()=>{if(pageRetired||!media.getAttribute('src'))return;if(state.player?.mode==='local'){stopPlayback(false).catch(()=>{});state.error='This local file could not play. Its format may be unsupported or protected. Choose another file.';}else state.error='This media could not play. Try another title or refresh your library.';render();});
window.addEventListener('pagehide',()=>{
 if(pageRetired)return;
 const p=state.player,session=state.session,mode=state.mode,base=state.base;
 pageRetired=true;state.epoch++;state.pairingView++;playbackGeneration++;
 for(const controller of requestControllers)controller.abort();requestControllers.clear();
 clearTimeout(state.inviteTimer);clearInterval(leaseRenew);clearInterval(progressTimer);clearInterval(meshPoll);
 state.player=null;retireLyrics(p);clearOutput(p);
 try{runtime()?.ready()?.webMeshPageExit();}catch(_){}
 if(mode!=='mesh'){
  if(p?.lease)releaseLease(p.lease,p.session,true).catch(()=>{});
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
