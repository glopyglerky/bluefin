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
  featured:null,homeHistory:null,facets:null,search:null,sportsPreferences:null,
  queue:[],unshuffledQueue:[],queueIndex:0,shuffle:false,repeat:'off',query:'',local:false,localQuery:'',localSnapshot:null,images:new Map(),imageEpoch:0};
let modalWasOpen=false,focusReturn=null,pageRetired=false,authTask=false,playbackGeneration=0,leaseRenew=null,progressTimer=null,meshPoll=null,hlsPromise,meshRetirement=null;
let meshBeginStarted=false;
let localLibrary=null,routeController=null,lyricsView=null,lyricsFrame=null;
const categories=[['Home','home'],['Sports','sports'],['TV Shows','shows'],['Movies','movies'],['Live TV','live'],['Music','music']];
const searchScopes=[['all','All',null],['movies','Movies','Movie'],['tvShows','TV Shows','Series,Season,Episode'],['music','Music','Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist'],['sports','Sports',null],['liveTv','Live TV','TvChannel,LiveTvProgram'],['publicTv','Public TV catalog (TVmaze)',null]];
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
 if(p?.sing?.active)return {position:singPosition(p.sing),duration:p.sing.duration,minimum:0};
 if(p?.mode==='mesh'&&p.snapshot)return {position:p.snapshot.position||0,duration:p.snapshot.duration||0,minimum:offset};
 const position=offset+(Number.isFinite(media.currentTime)?media.currentTime:0);
 const duration=Number.isFinite(media.duration)&&media.duration>0?offset+media.duration:0;
 return {position,duration,minimum:offset};
}
function seekPlayback(position){
 const p=state.player;if(!p||!Number.isFinite(position)||p.item.Type==='TvChannel')return;
 if(p.sing?.active){runSing(p,position,p.sing.paused).catch(error=>{state.error=String(error.message||error);render();});return;}
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

const artworkPending=new Map(),artworkMissing=new Map(),artworkRetries=new Map(),imageControllers=new Set(),requestControllers=new Set();let artworkActive=0,artworkScrollTimer,artworkRetryTimer;
function assertLive(epoch=state.epoch,session){if(pageRetired||epoch!==state.epoch||(session!==undefined&&session!==state.session))throw Error('This page or account changed. Resume the current connection.');}
function clearSearchFacets(){
 const facets=state.facets;facets?.controller?.abort();
 for(const controller of Object.values(facets?.lookupControllers||{}))controller.abort();
 for(const timer of Object.values(facets?.lookupTimers||{}))clearTimeout(timer);
 state.facets=null;
}
function clearPrivateView(){clearTimeout(sportsRefreshTimer);state.sportsPreferences=null;routeController?.abort();routeController=null;state.featured=null;state.homeHistory=null;clearSearchFacets();state.search=null;state.rows=[];state.shelves=[];state.back=[];state.query='';state.route={kind:'home'};state.queue=[];state.unshuffledQueue=[];state.queueIndex=0;state.shuffle=false;state.local=false;state.localQuery='';state.localSnapshot=null;state.repeat='off';state.nowPanel='artwork';state.capabilities=null;state.capability=null;state.code='';state.pairing=null;state.device=null;state.invitation=null;}
const paths={
 artist:'M8 11a4 4 0 1 0 8 0 4 4 0 1 0-8 0M4 22v-2a8 8 0 0 1 16 0v2M9 2l3 2 3-2',album:'M4 8h16v13H4zM6 5h12M8 2h8',star:'m12 3 3 6 7 1-5 5 1 7-6-3-6 3 1-7-5-5 7-1z',genres:'M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z',mic:'M9 4a3 3 0 0 1 6 0v8a3 3 0 0 1-6 0V4M6 10v2a6 6 0 0 0 12 0v-2M12 18v4M8 22h8',close:'m5 5 14 14M19 5 5 19',filter:'M3 6h18M6 12h12M9 18h6',
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
 try{await task();}catch(e){if(!pageRetired&&!state.capability&&e.name!=='AbortError')state.error=String(e.message||e);}
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
 if(scope==='sports'||scope==='publicTv')return {scope,studioID:'',tag:'',collectionID:''};
 return {scope,studioID:uuid(value.studioID)?id(value.studioID):'',tag:typeof value.tag==='string'?value.tag.slice(0,512):'',
  collectionID:uuid(value.collectionID)?id(value.collectionID):''};
}
function searchLanes(filters){
 if(filters.scope==='publicTv')return [];
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
  path='Users/'+state.session.user+'/Items';query.push(['Recursive',true],['Fields','Overview,PrimaryImageAspectRatio']);
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
async function publicTVSearch(term,signal){
 const url=new URL('https://api.tvmaze.com/search/shows');
 url.searchParams.set('q',term);
 const limit=1024*1024,controller=new AbortController(),timeout=setTimeout(()=>controller.abort(),6000);
 const stop=()=>controller.abort();
 signal.addEventListener('abort',stop,{once:true});
 try{
  if(signal.aborted)throw aborted();
  const response=await fetch(url,{credentials:'omit',redirect:'error',referrerPolicy:'no-referrer',signal:controller.signal});
  if(!response.ok)throw Error('Catalog unavailable');
  const reader=response.body.getReader(),bytes=new Uint8Array(limit);
  let length=0;
  for(;;){
   const {done,value}=await reader.read();if(done)break;
   if(length+value.length>limit){await reader.cancel();throw Error('Catalog response too large');}
   bytes.set(value,length);length+=value.length;
  }
  const rows=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(0,length)));
  if(!Array.isArray(rows))throw Error('Invalid catalog response');
  const titles=[],seen=new Set();
  for(const {show} of rows.filter(row=>row&&typeof row==='object')){
   if(!Number.isSafeInteger(show?.id)||show.id<=0||seen.has(show.id)||typeof show.name!=='string'||!show.name.trim()||show.name.length>256)continue;
   seen.add(show.id);
   const raw=show.image?.medium;
   const art=typeof raw==='string'&&raw.length<=1024&&/^https:\/\/static\.tvmaze\.com\/uploads\/images\/medium_portrait\/[0-9]+\/[0-9]+\.jpg$/.test(raw)?raw:null;
   const year=typeof show.premiered==='string'&&/^\d{4}-/.test(show.premiered)?show.premiered.slice(0,4):'';
   titles.push({name:show.name,art,year,url:'https://www.tvmaze.com/shows/'+show.id});
   if(titles.length===20)break;
  }
  return titles;
 }finally{clearTimeout(timeout);signal.removeEventListener('abort',stop);}
}
async function loadSearch(route,get,current,signal){
 const search=state.search={route,pages:[],problems:[],publicTV:null};
 if(!route.query&&route.filters.scope==='all'&&!hasSearchFacets(route.filters)){
  if(state.session){search.browseMusic=state.tab==='Music';try{const value=await get(search.browseMusic?'MusicGenres':'Genres',[['UserId',state.session.user],['Recursive',true],['Limit',24]]);current();search.browse=items(value);render();}catch(error){current();}}
  return;
 }
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
 if(publicTerm.length>=2&&route.filters.scope==='publicTv'){
  search.publicTV={loading:true,items:[],unavailable:false};
  lookups.push(publicTVSearch(publicTerm,signal).then(titles=>{
   current();search.publicTV.items=titles;
  },()=>{
   current();search.publicTV.unavailable=true;
  }).then(()=>{search.publicTV.loading=false;render();}));
 }
 await Promise.all(lookups);
}
async function startSearch(query,filters={},record=false){
 const previous=state.route,route={kind:'search',title:'Search',query:String(query).trim().slice(0,256),
  filters:cleanSearchFilters(state.session?filters:{scope:'publicTv'}),refine:previous.kind==='search'&&previous.refine,liveAfter:new Date().toISOString()};
 state.query=route.query;state.route=route;state.error='';
 const search=loadRoute();render();
 if(route.refine&&!['sports','publicTv'].includes(route.filters.scope)&&(!state.facets||state.facets.loading))loadSearchFacets().catch(()=>{});
 if(record&&route.filters.scope!=='publicTv')recordHomeSearch(route.query).catch(()=>{});
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
 const facets=state.facets={epoch,session,controller,studios:[],tags:[],collections:[],pending:new Set(['studios','tags','collections']),loading:true,problems:[],truncated:false,queries:{},lookupControllers:{},lookupTimers:{},lookupBusy:new Set(),lookupProblems:{},lookupTruncated:{}};
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
    values=[...new Set(value.Tags.filter(tag=>typeof tag==='string'&&tag.length<=512))].slice(0,8192);truncated=value.Tags.length>8192;
   }else{
    values=items(value).map(item=>({id:id(item.Id),name:item.Name}));truncated=Number(value.TotalRecordCount)>values.length;
   }
   if(!facets.queries[kind])facets[kind]=values;facets.truncated||=truncated;
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
 clearTimeout(sportsRefreshTimer);routeController?.abort();const controller=routeController=new AbortController();
 const current=()=>{if(pageRetired||epoch!==state.epoch||session!==state.session||route!==state.route||controller!==routeController||controller.signal.aborted)throw aborted();};
 const get=(path,query=[])=>api(path,'GET',undefined,query,controller.signal);
 if(route.kind==='local_music'||route.kind==='local_search'){state.rows=route.kind==='local_search'?localOwner().search(state.localQuery).map(localItem):localItems();state.shelves=[];return;}
 if(!user&&route.kind!=='search')return;
 state.rows=[];state.shelves=[];state.featured=null;state.error='';route.loading=true;
 if(route.kind!=='search')state.search=null;
 const publish=()=>{current();render();};
 try{
  if(route.kind==='home'){
   const values=[null,null,null,null];
   const show=()=>{
    current();const resume=values[0]||[],next=values[1]||[],latest=values[2]||[],recommended=values[3]?.shelves||[];
    state.homeHistory=values[3]?.history||null;
    state.shelves=[{title:'Continue Watching',items:resume},{title:'Up Next',items:next},...recommended,{title:'Recently Added',items:latest}].filter(shelf=>shelf.items.length);
    // Publish useful rows immediately, then preserve the account's recommendation priority.
    state.featured = resume[0]||recommended.find(shelf=>shelf.items.length)?.items[0]||next[0]||latest[0]||null;
    publish();
   };
   const requests=[
    get('Users/'+user+'/Items/Resume',[['Limit',12],['Fields','Overview']]).then(items),
    get('Shows/NextUp',[['UserId',user],['Limit',12],['Fields','Overview']]).then(items),
    get('Users/'+user+'/Items/Latest',[['Limit',18],['Fields','Overview']]).then(items),
    homeRecommendations(get,user)];
   const results=await Promise.allSettled(requests.map((request,index)=>request.then(value=>{current();values[index]=value;show();})));current();
   if(results.every(result=>result.status==='rejected'))throw Error('Your library could not refresh. Open Collections or try again.');
   if(results.some(result=>result.status==='rejected'))state.notice='Some Home sections could not refresh. Try again to load the remaining sections.';
  }else if(route.kind==='library'){state.rows=items(await get('Users/'+user+'/Views'));current();}
  else if(route.kind==='music'){
   const q=[['Limit',18],['Recursive',true],['IncludeItemTypes','MusicAlbum'],['Fields','Overview,PrimaryImageAspectRatio'],['SortBy','DateCreated'],['SortOrder','Descending']];
   const values=[[],[]],labels=['Recently Added','Favorite Albums'];
   const results=await Promise.allSettled([q,[...q,['IsFavorite',true]]].map((query,index)=>get('Users/'+user+'/Items',query).then(value=>{
    current();values[index]=items(value);state.shelves=values.map((entries,index)=>({title:labels[index],items:entries})).filter(shelf=>shelf.items.length);publish();
   })));current();if(results.every(result=>result.status==='rejected'))throw Error('Your music library could not load. Try again.');
  }else if(route.kind==='catalog'){
   const base=[['Recursive',true],['IncludeItemTypes',route.type],['Fields','Overview,PrimaryImageAspectRatio']];
   const tasks=[
    get('Users/'+user+'/Items',[...base,['Limit',16],['SortBy','DateCreated'],['SortOrder','Descending']]).then(value=>{current();route.recent=items(value);state.featured ||= route.recent[0]||null;publish();}),
    get('Users/'+user+'/Items',[...base,['Limit',16],['IsFavorite',true]]).then(value=>{current();route.favorites=items(value);publish();}),
    get('Users/'+user+'/Items',[...base,['Limit',100],['StartIndex',route.start||0],['SortBy','SortName']]).then(value=>{current();state.rows=items(value);route.total=Number(value.TotalRecordCount)||state.rows.length;publish();})
   ];
   const results=await Promise.allSettled(tasks);current();
   if(results[2].status==='rejected')throw Error('This library could not load. Try again.');
  }else if(route.kind==='genres'){state.rows=items(await get('MusicGenres',[['UserId',user],['Recursive',true],['Limit',200]]));current();}
  else if(route.kind==='live'){state.rows=items(await get('LiveTv/Channels',[['UserId',user],['Limit',100]]));current();}
  else if(route.kind==='sports'){await loadSports(route,get,current);
  }else if(route.kind==='sports_detail'){await loadSportsDetail(route,get,current);
  }else if(route.kind==='search'){route.filters=cleanSearchFilters(route.filters);route.query=route.query??state.query;route.liveAfter??=new Date().toISOString();await loadSearch(route,get,current,controller.signal);}
  else if(route.kind==='collection'){
   const q=[['Limit',100],['StartIndex',route.start||0],['Recursive',true],['Fields','Overview,PrimaryImageAspectRatio'],['SortBy',route.type==='Audio'&&route.parent?'ParentIndexNumber,IndexNumber,SortName':'SortName']];
   if(route.type)q.push(['IncludeItemTypes',route.type]);if(route.parent)q.push(['ParentId',route.parent]);if(route.artist)q.push(['ArtistIds',route.artist]);
   if(route.favorite)q.push(['IsFavorite',true]);if(route.genre)q.push(['GenreIds',route.genre]);
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
 if(tab==='Movies'||tab==='TV Shows')state.route={kind:'catalog',title:tab,type:tab==='Movies'?'Movie':'Series',start:0,recent:[],favorites:[]};
 else state.route={kind:({Home:'home',Sports:'sports',Music:'music','Live TV':'live'})[tab]||'home'};
 await loadRoute();
}
async function openCollection(title,type,parent,artist,options={}){state.back.push({...state.route});state.route={kind:'collection',title,type,parent,artist,start:0,...options};await loadRoute();}
async function openItem(item,recordSelection=true){
 if(recordSelection&&state.route.kind==='search')recordHomeSearch(state.route.query,item).catch(()=>{});
 if(['Program','LiveTvProgram'].includes(item.Type)){
  if(!uuid(item.ChannelId))throw Error('This programme has no available channel.');
  const channel=await api('Users/'+state.session.user+'/Items/'+id(item.ChannelId));
  if(!uuid(channel?.Id)||!['TvChannel','LiveTvChannel'].includes(channel.Type))throw Error('This channel is no longer available.');
  return openItem({...channel,Type:'TvChannel'},false);
 }
 if(item.Type==='LiveTvChannel')item={...item,Type:'TvChannel'};
 if(item.Type==='TvChannel'&&state.mode!=='mesh'){state.back.push({...state.route});state.route={kind:'detail',item};return;}
 if(isLocalItem(item)){const queue=localItems();return playLocal(item,queue,queue.findIndex(track=>track.Id===item.Id));}
 if(item.Type==='Audio')return play(item,[item],0);
 if(item.Type==='MusicArtist')return openCollection(item.Name,'MusicAlbum,Audio,MusicVideo',null,id(item.Id),{headerItem:item});
 if(['MusicAlbum','Playlist','Series','Season','CollectionFolder','Folder','BoxSet'].includes(item.Type)||item.IsFolder){
  const type=['Series','Season'].includes(item.Type)?'Episode':['MusicAlbum','Playlist'].includes(item.Type)?'Audio':null;
  return openCollection(item.Name,type,id(item.Id),null,{headerItem:item});
 }
 state.back.push({...state.route});state.route={kind:'detail',item};await loadRoute();
}
function artworkSource(item,backdrop=false){
 const tag=value=>typeof value==='string'&&value.length<=256?value:'';
 const own=id(item.Id);
 if(backdrop){
  if(Array.isArray(item.BackdropImageTags)&&item.BackdropImageTags.length)return {item:own,backdrop:true,tag:tag(item.BackdropImageTags[0])};
  if(uuid(item.ParentBackdropItemId)&&Array.isArray(item.ParentBackdropImageTags)&&item.ParentBackdropImageTags.length)return {item:id(item.ParentBackdropItemId),backdrop:true,tag:tag(item.ParentBackdropImageTags[0])};
 }
 if(item.ImageTags?.Primary)return {item:own,backdrop:false,tag:tag(item.ImageTags.Primary)};
 if(uuid(item.AlbumId)&&item.AlbumPrimaryImageTag)return {item:id(item.AlbumId),backdrop:false,tag:tag(item.AlbumPrimaryImageTag)};
 if(uuid(item.ParentPrimaryImageItemId)&&item.ParentPrimaryImageTag)return {item:id(item.ParentPrimaryImageItemId),backdrop:false,tag:tag(item.ParentPrimaryImageTag)};
 // Some older servers omit image tags. Keep their authenticated image lookup.
 return {item:own,backdrop,tag:''};
}
function artKey(item,backdrop=false){const source=artworkSource(item,backdrop);return (backdrop?'b:':'p:')+source.item+(source.tag?':'+source.tag:'');}
function applyArtwork(element,url,backdrop=false){
 if(backdrop){
  const classes=element.className.split(/\s+/),overlay=classes.includes('hero')?'linear-gradient(180deg,rgba(6,23,39,0),rgba(6,23,39,.15) 35%,rgba(6,23,39,.94)),':classes.includes('browse-tile')?'linear-gradient(transparent,rgba(6,23,39,.65)),':'';
  element.style.backgroundImage=overlay+'url("'+url+'")';element.style.backgroundSize='cover';element.style.backgroundPosition='center';element.classList.add('has-artwork');return;
 }
 const progress=element.querySelector?.('.poster-progress'),image=node('img');image.alt='';image.src=url;image.loading='eager';image.decoding='async';
 image.onerror=()=>{element.replaceChildren(icon(element.dataset.artSymbol||'library'));if(progress)element.append(progress);};
 element.replaceChildren(image);if(progress)element.append(progress);
}
function bindArtwork(element,item,backdrop=false){
 if(isLocalItem(item))return;const source=artworkSource(item,backdrop),key=artKey(item,backdrop);
 element.dataset.artKey=key;element.dataset.artId=source.item;element.dataset.artBackdrop=String(source.backdrop);
 element.dataset.artSymbol=['Audio','MusicAlbum','MusicArtist','Playlist'].includes(item.Type)?'music':'library';
 if(backdrop)element.dataset.backdrop='true';const cached=state.images.get(key);if(cached)applyArtwork(element,cached,backdrop);
}
function cover(item,square=false){const d=node('div',undefined,'artwork'+(square?' square':''));d.append(icon(item.Type==='Audio'||item.Type==='MusicAlbum'?'music':'library'));bindArtwork(d,item);return d;}
async function imageBytes(item,backdrop,session,mode){
 if(mode==='mesh')return (await runtime().engine()).webMeshArtwork(session.handle,item,backdrop);
 const url=endpoint('Items/'+item+'/Images/'+(backdrop?'Backdrop':'Primary'),[['maxWidth',backdrop?1200:400],['quality',85]]);
 const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),8000);imageControllers.add(controller);
 try{const response=await fetch(url,{headers:{Authorization:authorization(session.token)},credentials:'omit',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});
 if(!response.ok)throw Object.assign(Error('Artwork unavailable'),{status:response.status});
 if(!['image/jpeg','image/png','image/webp'].includes((response.headers.get('Content-Type')||'').split(';')[0].trim()))throw Error('Artwork unavailable');
 const reader=response.body.getReader(),parts=[];let total=0;try{for(;;){const {done,value}=await reader.read();if(done)break;total+=value.length;if(total>2*1024*1024)throw Error('Artwork limit');parts.push(value);}}catch(e){await reader.cancel().catch(()=>{});throw e;}
 const bytes=new Uint8Array(total);let at=0;for(const part of parts){bytes.set(part,at);at+=part.length;}return bytes;
 }finally{clearTimeout(timer);imageControllers.delete(controller);}
}
async function authenticatedBytes(path,query,session,signal,limit,expected,timeout=20000){
 const controller=new AbortController(),cancel=()=>controller.abort(),timer=setTimeout(cancel,timeout);requestControllers.add(controller);signal?.addEventListener('abort',cancel,{once:true});if(signal?.aborted)controller.abort();
 try{
  const response=await fetch(endpoint(path,query),{headers:{Authorization:authorization(session.token)},credentials:'omit',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});
  if(!response.ok)throw Object.assign(Error('The server returned HTTP '+response.status+'.'),{status:response.status});
  if(!response.body)throw Error('The media response was empty.');
  const reader=response.body.getReader(),parts=expected?null:[],bytes=expected?new Uint8Array(expected):null;let total=0;
  try{for(;;){const {done,value}=await reader.read();if(done)break;if(total+value.length>limit||(expected&&total+value.length>expected))throw Error('The media response exceeded its size limit.');if(bytes)bytes.set(value,total);else parts.push(value);total+=value.length;}}
  catch(error){await reader.cancel().catch(()=>{});throw error;}
  if(expected&&total!==expected)throw Error('The media response was incomplete.');
  if(bytes)return bytes;const result=new Uint8Array(total);let offset=0;for(const part of parts){result.set(part,offset);offset+=part.length;}return result;
 }finally{clearTimeout(timer);requestControllers.delete(controller);signal?.removeEventListener('abort',cancel);}
}
async function sportsLogoBytes(asset,session,mode){
 if(!/^[a-f0-9]{64}$/.test(asset))throw Error('Invalid team artwork.');
 if(mode==='mesh'){const engine=await runtime().engine();if(typeof engine.webMeshSportsLogo!=='function')throw Error('Reload Bluefin to load team logos.');return engine.webMeshSportsLogo(session.handle,asset);}
 return authenticatedBytes('sports/api/v1/logos/'+asset,[],session,undefined,2*1024*1024,undefined,8000);
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
  if(artworkActive>=(mode==='mesh'?2:4))break;const key=element.dataset.artKey;
  if(state.images.has(key)||artworkPending.has(key)||(artworkMissing.get(key)||0)>Date.now())continue;
  artworkMissing.delete(key);
  while(state.images.size+artworkPending.size>=64){
   const evict=Array.from(state.images.keys()).find(candidate=>!visible.has(candidate));
   if(evict===undefined)break;URL.revokeObjectURL(state.images.get(evict));state.images.delete(evict);
  }
  if(state.images.size+artworkPending.size>=64)break;artworkPending.set(key,epoch);artworkActive++;
  const item=element.dataset.artId,backdrop=element.dataset.artBackdrop===undefined?element.dataset.backdrop==='true':element.dataset.artBackdrop==='true';
  (element.dataset.sportsLogo?sportsLogoBytes(element.dataset.sportsLogo,session,mode):imageBytes(item,backdrop,session,mode)).then(bytes=>{
   if(epoch!==state.imageEpoch||session!==state.session)return;const url=URL.createObjectURL(new Blob([bytes],{type:imageType(bytes)}));state.images.set(key,url);artworkRetries.delete(key);
   for(const target of document.querySelectorAll('[data-art-key]'))if(target.dataset.artKey===key)applyArtwork(target,url,target.dataset.backdrop==='true');
  }).catch(error=>{if(epoch===state.imageEpoch&&session===state.session){
   if(artworkMissing.size>=256){const oldest=artworkMissing.keys().next().value;artworkMissing.delete(oldest);artworkRetries.delete(oldest);}
   const attempts=(artworkRetries.get(key)||0)+1;artworkRetries.set(key,error.status===404?3:attempts);
   artworkMissing.set(key,Date.now()+(error.status===404||attempts>=3?300000:5000*attempts));
  }})
   .finally(()=>{artworkActive--;if(artworkPending.get(key)===epoch)artworkPending.delete(key);pumpArtwork();});
 }
 clearTimeout(artworkRetryTimer);
 const retry=elements.map(element=>element.dataset.artKey).filter(key=>(artworkRetries.get(key)||0)<3).map(key=>artworkMissing.get(key)).filter(value=>value>Date.now());
 if(retry.length)artworkRetryTimer=setTimeout(pumpArtwork,Math.max(50,Math.min(...retry)-Date.now()+20));
}
addEventListener('scroll',()=>{clearTimeout(artworkScrollTimer);artworkScrollTimer=setTimeout(pumpArtwork,60);},{passive:true,capture:true});
addEventListener('resize',pumpArtwork,{passive:true});
addEventListener('online',()=>{artworkMissing.clear();artworkRetries.clear();pumpArtwork();});

function poster(item,shape='poster'){
 const square=['Audio','MusicAlbum','MusicArtist','Playlist'].includes(item.Type),wide=!square&&shape==='landscape';
 const b=button('',()=>perform(()=>openItem(item)),'poster'+(square?' album':'')+(wide?' landscape':'')+(item.Type==='MusicArtist'?' artist':''));
 const art=node('div',undefined,'artwork'+(square?' square':wide?' landscape':''));
 art.append(icon(square?'music':item.Type==='Series'?'shows':'movies'));bindArtwork(art,item,wide);
 const copy=node('span',undefined,'poster-copy');copy.append(node('strong',item.Name));
 const detail=item.Type==='Episode'?[item.SeriesName,item.ParentIndexNumber&&item.IndexNumber?'S'+item.ParentIndexNumber+' · E'+item.IndexNumber:null].filter(Boolean).join(' · '):subtitle(item);
 if(detail)copy.append(node('small',detail));
 const duration=Number(item.RunTimeTicks),position=Number(item.UserData?.PlaybackPositionTicks);
 const progress=duration>0&&position>0?Math.min(100,Math.max(0,100*position/duration)):0;
 if(progress>0){const track=node('span',undefined,'poster-progress'),fill=node('span');fill.style.width=progress+'%';track.append(fill);track.setAttribute('role','progressbar');track.setAttribute('aria-label','Watched');track.setAttribute('aria-valuemin','0');track.setAttribute('aria-valuemax','100');track.setAttribute('aria-valuenow',String(Math.round(progress)));art.append(track);}
 b.setAttribute('aria-label',[item.Name,detail,progress>0?Math.round(progress)+' percent watched':''].filter(Boolean).join(', '));
 b.dataset.focusKey='media|'+id(item.Id);b.append(art,copy);return b;
}
function shelf(title,entries,shape='landscape'){
 if(!entries.length)return document.createDocumentFragment();
 const s=node('section',undefined,'shelf'),heading=node('div',undefined,'section-heading');heading.append(node('h2',title));s.append(heading);
 const track=node('div',undefined,'shelf-track');track.setAttribute('aria-label',title);
 for(const item of entries.slice(0,18))track.append(poster(item,shape));s.append(track);return s;
}
function row(item,select=()=>openItem(item),detail=subtitle(item)){
 const audio=item.Type==='Audio',b=button('',()=>perform(select),'list-row'+(audio?' song':''));
 b.dataset.focusKey='row|'+item.Id;if(audio)b.append(cover(item,true));else b.append(icon(item.CollectionType==='music'?'music':'library'));
 const text=node('span',undefined,'grow');text.append(node('strong',item.Name));if(detail)text.append(node('small',detail));b.append(text);
 if(audio&&item.RunTimeTicks>0)b.append(node('small',time(item.RunTimeTicks/10000000),'track-duration'));else b.append(icon('chevron'));return b;
}
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
 const d=node('header',undefined,'toolbar page-header');
 if(back)d.append(action('Back',async()=>{state.route=state.back.pop()||{kind:'home'};await loadRoute();},'plain back-button','back'));
 else d.append(node('h1',title));
 const utilities=node('div',undefined,'utility-actions');
 const search=button('',()=>openSearch(),'round-control','search');search.setAttribute('aria-label','Search all media');
 const collections=action('',()=>navigate('Collections'),'round-control','library');collections.setAttribute('aria-label','Collections');
 const account=button('',()=>{state.sheet='account';render();},'round-control account-control','account');account.setAttribute('aria-label','Account and settings');
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
 if(state.phase==='connect'){stack.append(button('Search TV shows',openSearch,'','search'));stack.append(button('Connect with QR and passkey',()=>{state.sheet='invite';state.error='';render();},'primary'));stack.append(action('Open local music',openLocalMusic,'','music'));const form=node('form');form.append(node('h2','Connect to your server'));const [w,i]=field('Server address','url',state.base,'server');i.autocomplete='url';i.placeholder='https://media.example.com';i.required=true;form.append(w);const submit=node('button','Continue','primary');submit.disabled=state.busy;form.append(submit);form.onsubmit=e=>{e.preventDefault();perform(()=>connect(i.value.trim()));};stack.append(form);}
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
function publicTVResults(catalog){
 const section=node('section',undefined,'search-results');
 section.append(node('h2','TV catalog'));
 if(catalog.loading)section.append(node('p','Searching TV catalog…','loading'));
 else if(catalog.unavailable)section.append(node('p','The TV catalog could not load.','error'));
 else if(!catalog.items.length)section.append(node('p','No titles found in the TV catalog.','secondary'));
 else{
  const grid=node('div',undefined,'search-grid');
  for(const item of catalog.items){
   const link=node('a',undefined,'public-title');link.href=item.url;link.rel='noopener noreferrer';link.target='_blank';
   const art=node('div',undefined,'artwork');
   if(item.art){const image=node('img');image.alt='';image.loading='lazy';image.referrerPolicy='no-referrer';image.onerror=()=>art.replaceChildren(icon('shows'));image.src=item.art;art.append(image);}
   else art.append(icon('shows'));
   link.append(art,node('strong',item.name),node('small',[item.year,'TVmaze'].filter(Boolean).join(' · ')));
   grid.append(link);
  }
  section.append(grid,node('p','TVmaze lists titles. Streaming availability for these results is unverified.','secondary'));
 }
 return section;
}
function publicSearchView(){
 const main=node('main',undefined,'app-main search-screen');main.id='content';
 main.append(button('Back',()=>{
  routeController?.abort();state.phase='connect';state.route={kind:'home'};state.back=[];state.search=null;state.query='';render();
 },'plain','back'),node('h1','Search TV shows'));
 searchView(main);
 return main;
}
function searchBrowse(main,search){
 const genres=search?.browse?.length?node('section',undefined,'browse-section'):null;
 if(genres)genres.append(node('h2',search.browseMusic?'Browse Music':'Genres'),genreGrid(search.browse,search.browseMusic));
 if(genres&&search.browseMusic)main.append(genres);
 const section=node('section',undefined,'browse-section');section.append(node('h2','Browse Categories'));
 const grid=node('div',undefined,'browse-grid'),types=[['movies','Movies','movies'],['tvShows','TV Shows','shows'],['music','Music','music'],['sports','Sports','sports'],['liveTv','Live TV','live']];
 types.forEach(([scope,label,symbol],index)=>{const tile=button(label,()=>startSearch('',{scope}),'browse-tile',symbol);tile.dataset.tone=String(index);grid.append(tile);});
 section.append(grid);main.append(section);
 if(genres&&!search.browseMusic)main.append(genres);
}
function facetLookup(kind,facets){
 const input=node('input');input.type='search';input.id='bluefin-find-'+kind;input.maxLength=128;
 const label=kind==='studios'?'Find a studio':'Find a collection';input.placeholder=label;input.setAttribute('aria-label',label);
 input.value=facets?.queries?.[kind]||'';input.className='facet-lookup';
 input.oninput=()=>{
  if(!facets||facets!==state.facets)return;
  facets.queries[kind]=input.value;clearTimeout(facets.lookupTimers[kind]);
  facets.lookupTimers[kind]=setTimeout(()=>lookupSearchFacet(kind,input.value).catch(()=>{}),250);
 };
 return input;
}
async function lookupSearchFacet(kind,query){
 if(!['studios','collections'].includes(kind))return;
 const facets=state.facets,session=state.session,epoch=state.epoch;if(!facets||!session)return;
 query=String(query).trim().slice(0,128);facets.queries[kind]=query;
 facets.lookupControllers[kind]?.abort();const controller=facets.lookupControllers[kind]=new AbortController();
 const current=()=>!pageRetired&&epoch===state.epoch&&session===state.session&&facets===state.facets&&facets.lookupControllers[kind]===controller&&!controller.signal.aborted;
 facets.lookupBusy.add(kind);delete facets.lookupProblems[kind];
 if(state.route.kind==='search'&&state.route.refine)render();
 const path=kind==='studios'?'Studios':'Users/'+session.user+'/Items',params=[['UserId',session.user],['Recursive',true],['Limit',200]];
 if(kind==='collections')params.push(['IncludeItemTypes','BoxSet']);if(query)params.push(['SearchTerm',query]);
 try{
  const value=await api(path,'GET',undefined,params,controller.signal);if(!current())return;
  facets[kind]=items(value).map(item=>({id:id(item.Id),name:item.Name}));
  facets.lookupTruncated[kind]=Number(value.TotalRecordCount)>facets[kind].length;
 }catch(error){if(current()&&error.name!=='AbortError')facets.lookupProblems[kind]=String(error.message||error);}
 finally{if(current()){facets.lookupBusy.delete(kind);if(state.route.kind==='search'&&state.route.refine)render();}}
}

function tagRefinement(route,filters,facets){
 const wrap=node('div'),label=node('label','Service or custom group');label.htmlFor='bluefin-tag-refinement';wrap.append(label);
 const controls=node('div',undefined,'tag-lookup'),input=node('input'),suggestions=node('datalist');input.id='bluefin-tag-refinement';input.type='text';input.maxLength=512;input.placeholder='Search your server tags';input.value=route.tagDraft??filters.tag;input.setAttribute('list','bluefin-tag-options');suggestions.id='bluefin-tag-options';
 const suggest=()=>{const query=input.value.trim().toLocaleLowerCase();suggestions.replaceChildren();for(const name of (facets?.tags||[]).filter(name=>!query||name.toLocaleLowerCase().includes(query)).slice(0,128)){const option=node('option');option.value=name;suggestions.append(option);}};
 input.oninput=()=>{route.tagDraft=input.value;suggest();};suggest();
 const apply=button('Apply',()=>startSearch(state.query,{...filters,tag:input.value.trim()}));controls.append(input,apply);wrap.append(controls,suggestions);return wrap;
}
function searchView(main){
 const route=state.route,filters=cleanSearchFilters(route.filters);
 const form=node('form',undefined,'search-form'),input=node('input');
 form.setAttribute('role','search');
 input.id='bluefin-search';input.type='search';input.placeholder=state.session&&filters.scope!=='publicTv'?'Titles, artists, and channels':'TV show title';input.value=state.query;
 input.setAttribute('aria-label',state.session&&filters.scope!=='publicTv'?'Search connected media':'Search TV shows');input.maxLength=256;
 input.oninput=()=>{state.query=input.value;};
 const submit=node('button','Search','primary');
 submit.type='submit';submit.id='bluefin-search-submit';
 form.append(icon('search'),input,submit);
 form.onsubmit=event=>{event.preventDefault();startSearch(input.value,filters,true);};
 main.append(form);
 const search=state.search;
 if(!state.session){
  if(search?.publicTV)main.append(publicTVResults(search.publicTV));
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
 if(!['sports','publicTv'].includes(filters.scope))tools.append(refine);
 main.append(tools);
 if(route.refine&&!['sports','publicTv'].includes(filters.scope)){
  const panel=node('section',undefined,'search-refinements'),facets=state.facets;
  const studio=node('div');studio.append(facetLookup('studios',facets),searchChoice('Studio','studio',filters.studioID,[['','All studios'],...(facets?.studios||[]).map(value=>[value.id,value.name])],
   studioID=>startSearch(state.query,{...filters,studioID}),facets?.pending?.has('studios')||facets?.lookupBusy?.has('studios')||false));panel.append(studio);
  panel.append(tagRefinement(route,filters,facets));
  const collection=node('div');collection.append(facetLookup('collections',facets),searchChoice('Collection','collection',filters.collectionID,[['','All collections'],...(facets?.collections||[]).map(value=>[value.id,value.name])],
   collectionID=>startSearch(state.query,{...filters,collectionID}),facets?.pending?.has('collections')||facets?.lookupBusy?.has('collections')||false));panel.append(collection);
  main.append(panel);
  if(facets?.loading)main.append(node('p','Loading '+[...facets.pending].join(', ')+'…','loading'));
  if(facets?.truncated||Object.values(facets?.lookupTruncated||{}).some(Boolean))main.append(node('p','Use a studio or collection name to narrow more matches.','secondary'));
  for(const problem of Object.values(facets?.lookupProblems||{}))main.append(node('p',problem,'error'));
  if(facets?.problems.length){main.append(node('p',facets.problems.join(' '),'error'),button('Reload refinements',()=>{clearSearchFacets();loadSearchFacets();}));}
 }
 if(hasSearchFacets(filters))main.append(node('p','Filtering your library. The live guide is searched when these refinements are cleared.','secondary'),
  button('Clear refinements',()=>startSearch(state.query,{scope:filters.scope}),'plain'));
 if(filters.scope==='publicTv'){
  if(search?.route===route&&search.publicTV)main.append(publicTVResults(search.publicTV));
  return;
 }
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
  if(!searched)searchBrowse(main,search);
  else if(!search?.problems.length&&!search?.publicTV?.items.length)main.append(empty('No matches in connected resources','Try a different title or clear your filters.'));
 }
}
const sportsLeagues=[['nhl','NHL'],['nfl','NFL'],['nba','NBA'],['mlb','MLB'],['wnba','WNBA'],['epl','Premier League'],['mls','MLS']];
let sportsRefreshTimer=null;
function sportsDay(offset=0){const date=new Date();date.setDate(date.getDate()+offset);return date.getFullYear()+'-'+String(date.getMonth()+1).padStart(2,'0')+'-'+String(date.getDate()).padStart(2,'0');}
function sportsDateLabel(value){const date=new Date(value+'T12:00:00');return Number.isFinite(date.valueOf())?date.toLocaleDateString(undefined,{weekday:'short',month:'short',day:'numeric'}):value;}
function sportsEnvelope(value,key,league){
 if(value?.schema!==1||value.provider!=='ESPN'||value.league!==league)throw Error('The server returned invalid sports information.');
 if(key==='event'){if(!value.event||!/^[0-9]{1,20}$/.test(value.event.id))throw Error('The game could not be loaded.');}
 else if(!Array.isArray(value[key])||value[key].length>400)throw Error('The server returned too much sports information.');
 return value;
}
function validSportsPreferences(value){
 if(!Number.isSafeInteger(value?.version)||value.version<0||value.preferences?.schema!==1||!['sports','leagues','conferences','teams','players'].every(key=>Array.isArray(value.preferences[key])&&value.preferences[key].length<=256&&value.preferences[key].every(id=>typeof id==='string'&&id.length<=128)))throw Error('Your Sports preferences could not be read.');
 return value;
}
async function loadSports(route,get,current){
 route.league=sportsLeagues.some(([key])=>key===route.league)?route.league:'nhl';route.date ||= sportsDay();
 route.boardLoading=true;route.boardError='';route.guideError='';route.sports=[];
 const board=get('sports/api/v1/scoreboard',[['league',route.league],['date',route.date]]).then(value=>{
  current();route.board=sportsEnvelope(value,'events',route.league);route.boardLoading=false;render();
 },error=>{current();route.boardLoading=false;route.boardError=String(error.message||error);render();});
 const preferences=get('sports/api/v1/preferences').then(value=>{current();state.sportsPreferences=validSportsPreferences(value);render();},()=>{current();});
 const guide=get('sports/api/v1/home').then(feed=>{
  current();if(feed?.schema===1&&Array.isArray(feed.local_broadcasts)&&feed.local_broadcasts.length<=400)route.sports=feed.local_broadcasts;
 },error=>{current();if(![403,404].includes(error.status))route.guideError='Your local broadcasts could not refresh.';});
 await Promise.all([board,preferences,guide]);current();
 if(route.date===sportsDay()||route.board?.events.some(event=>event.status?.state==='in'))scheduleSportsRefresh(route);
}
function scheduleSportsRefresh(route){
 clearTimeout(sportsRefreshTimer);
 sportsRefreshTimer=setTimeout(()=>{
  if(pageRetired||state.route!==route)return;
  if(document.hidden||state.busy||route.loading){scheduleSportsRefresh(route);return;}
  loadRoute().catch(error=>{if(state.route===route&&error.name!=='AbortError')state.error=String(error.message||error);}).finally(()=>{if(state.route===route)render();});
 },30000);
}
async function loadSportsDetail(route,get,current){
 const tasks=[
  get('sports/api/v1/events/'+route.league+'/'+route.event.id).then(value=>{current();route.event=sportsEnvelope(value,'event',route.league).event;route.detailError='';render();},error=>{current();route.detailError=String(error.message||error);}),
  get('sports/api/v1/standings/'+route.league).then(value=>{current();route.standings=sportsEnvelope(value,'groups',route.league);render();},error=>{current();route.standingsError=String(error.message||error);})
 ];
 await Promise.all(tasks);current();if(route.event.status?.state==='in')scheduleSportsRefresh(route);
}
async function openSportsEvent(event,league){if(!/^[0-9]{1,20}$/.test(event?.id)||!sportsLeagues.some(([key])=>key===league))throw Error('Invalid game.');state.back.push({...state.route});state.route={kind:'sports_detail',title:'Game',league,event,detailTab:'stats',keyPlays:true,playLimit:60};await loadRoute();}
function sportsTeamName(team){return String(team?.shortName||team?.name||team?.abbreviation||'Team').slice(0,128);}
function sportsTeamKey(team,league){return 'espn:'+league+':'+team.id;}
function sportsFollowed(team,league){const teams=state.sportsPreferences?.preferences?.teams||[];return teams.includes(sportsTeamKey(team,league))||teams.includes(String(team.id));}
async function saveSportsPreferences(value){
 const epoch=state.epoch,session=state.session,mode=state.mode;
 validSportsPreferences(value);const body=JSON.stringify(value.preferences);if(new TextEncoder().encode(body).length>60000)throw Error('Choose fewer teams.');
 let result;
 if(mode==='mesh'){
  const engine=await runtime().engine();assertLive(epoch,session);
  if(typeof engine.webMeshSportsPreferences!=='function')throw Error('Reload Bluefin to update your teams over this connection.');
  result=JSON.parse(await engine.webMeshSportsPreferences(session.handle,String(value.version),body));
 }else{
  const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),10000);requestControllers.add(controller);
  try{
   const response=await fetch(endpoint('sports/api/v1/preferences'),{method:'PUT',headers:{Authorization:authorization(session.token),'Content-Type':'application/json','If-Match':'"'+value.version+'"'},body,credentials:'omit',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});
   if(!response.ok)throw Object.assign(Error('The server returned HTTP '+response.status+'.'),{status:response.status});
   result=await readJson(response,65536);
  }finally{clearTimeout(timer);requestControllers.delete(controller);}
 }
 assertLive(epoch,session);return validSportsPreferences(result);
}
async function followSportsTeam(team,league){
 if(!/^[0-9]{1,20}$/.test(String(team?.id||''))||!sportsLeagues.some(([key])=>key===league))throw Error('This team cannot be followed.');
 const epoch=state.epoch,session=state.session;
 const saved=validSportsPreferences(await api('sports/api/v1/preferences'));assertLive(epoch,session);
 const preferences={...saved.preferences},teams=new Set(preferences.teams),key=sportsTeamKey(team,league);
 if(teams.has(key)||teams.has(String(team.id))){teams.delete(key);teams.delete(String(team.id));}else teams.add(key);
 preferences.teams=[...teams];preferences.all_teams=teams.size===0;
 try{state.sportsPreferences=await saveSportsPreferences({version:saved.version,preferences});}
 catch(error){
  assertLive(epoch,session);
  if(error.status===412||/HTTP 412/.test(String(error.message||error))){state.sportsPreferences=validSportsPreferences(await api('sports/api/v1/preferences'));throw Error('Your teams changed on another device. Tap Follow again to update them.');}
  throw error;
 }
}
function sportsLogo(team){
 const logo=node('span',undefined,'team-logo'),match=typeof team?.logo==='string'&&team.logo.match(/^\/sports\/api\/v1\/logos\/([a-f0-9]{64})$/);
 logo.append(node('span',String(team?.abbreviation||'').slice(0,6)));
 if(match){logo.dataset.artKey='sports:'+match[1];logo.dataset.sportsLogo=match[1];logo.dataset.artSymbol='sports';const cached=state.images.get(logo.dataset.artKey);if(cached)applyArtwork(logo,cached);}
 return logo;
}
function sportsTeamView(team,league,compact=false){
 const box=node('div',undefined,'sports-team'+(compact?' compact':''));
 box.append(sportsLogo(team));const name=node('strong',sportsTeamName(team));box.append(name);
 if(team?.record)box.append(node('small',String(team.record).slice(0,32)));
 if(sportsFollowed(team||{},league)){const favorite=node('span','★','team-followed');favorite.setAttribute('aria-label','Following');box.append(favorite);}
 return box;
}
function sportsGameCard(event,league){
 const card=button('',()=>perform(()=>openSportsEvent(event,league)),'sports-game');
 const home=event.home||{},away=event.away||{},status=event.status||{};
 const heading=node('div',undefined,'sports-game-heading');heading.append(node('small',league.toUpperCase()),node('span',status.state==='in'?'LIVE':String(status.detail||''),'game-state'+(status.state==='in'?' is-live':'')));card.append(heading);
 const match=node('div',undefined,'sports-match'),score=node('div',undefined,'sports-score');
 if(status.state==='pre'){
  const date=event.date?new Date(event.date):new Date(NaN);score.append(node('strong',Number.isFinite(date.valueOf())?date.toLocaleTimeString(undefined,{hour:'numeric',minute:'2-digit'}):'Scheduled'),node('small',Number.isFinite(date.valueOf())?date.toLocaleDateString(undefined,{month:'short',day:'numeric'}):''));
 }else score.append(node('strong',String(away.score??'–')+' – '+String(home.score??'–')),node('small',String(status.clock||status.detail||'')));
 match.append(sportsTeamView(away,league,true),score,sportsTeamView(home,league,true));card.append(match);
 if(event.broadcasts?.length)card.append(node('small',event.broadcasts.slice(0,4).join(' · '),'sports-broadcasts'));
 return card;
}
function sportsView(main){
 const route=state.route,league=route.league||'nhl';
 const controls=node('div',undefined,'sports-controls'),leagues=node('div',undefined,'sports-leagues');
 leagues.setAttribute('aria-label','League');
 for(const [key,label]of sportsLeagues){const choice=action(label,async()=>{route.league=key;route.board=null;await loadRoute();});choice.setAttribute('aria-pressed',String(key===league));leagues.append(choice);}
 controls.append(leagues);main.append(controls);
 const panel=node('section',undefined,'sports-board'),tabs=node('div',undefined,'sports-segmented');tabs.setAttribute('aria-label','Game day');
 for(const [label,offset]of[['Yesterday',-1],['Today',0],['Upcoming',1]]){const choice=action(label,async()=>{route.date=sportsDay(offset);route.board=null;await loadRoute();});choice.setAttribute('aria-pressed',String(offset===1?route.date>sportsDay():route.date===sportsDay(offset)));tabs.append(choice);}
 panel.append(tabs);
 const heading=node('div',undefined,'sports-date-heading'),previous=action('',async()=>{const d=new Date(route.date+'T12:00:00');d.setDate(d.getDate()-1);route.date=d.getFullYear()+'-'+String(d.getMonth()+1).padStart(2,'0')+'-'+String(d.getDate()).padStart(2,'0');route.board=null;await loadRoute();},'plain','back');previous.setAttribute('aria-label','Previous day');
 const next=action('',async()=>{const d=new Date(route.date+'T12:00:00');d.setDate(d.getDate()+1);route.date=d.getFullYear()+'-'+String(d.getMonth()+1).padStart(2,'0')+'-'+String(d.getDate()).padStart(2,'0');route.board=null;await loadRoute();},'plain','chevron');next.setAttribute('aria-label','Next day');
 heading.append(previous,node('h2',sportsDateLabel(route.date||sportsDay())),next);panel.append(heading);
 const events=route.board?.events||[],following=events.filter(event=>sportsFollowed(event.home||{},league)||sportsFollowed(event.away||{},league)),rest=events.filter(event=>!following.includes(event));
 for(const [title,games]of[['My Teams',following],['All Games',rest]]){
  if(!games.length)continue;const group=node('section',undefined,'sports-day-group');group.append(node('h3',title));
  for(const event of games)group.append(sportsGameCard(event,league));panel.append(group);
 }
 if(route.boardLoading&&!events.length)panel.append(node('p','Loading games…','loading'));
 else if(route.boardError)panel.append(node('p',route.boardError,'error'),action('Try again',loadRoute,'plain'));
 else if(!events.length)panel.append(empty('No games on this day','Choose another date or league.'));
 if(route.board)panel.append(node('p',(route.board.stale?'Last available scores':'Scores')+' · '+route.board.provider,'sports-attribution'));
 main.append(panel);
 if(route.sports?.length){
  const guide=node('section',undefined,'sports-local');guide.append(node('h2','From Your Live TV'));
  for(const game of route.sports.slice(0,12)){const entry=action(String(game.native_title||'Sports broadcast').slice(0,256),()=>openItem({Id:game.channel_id,Name:game.channel_name||'Live TV',Type:'TvChannel'}),'local-broadcast','live');entry.append(node('small',[game.channel_name,game.native_start?new Date(game.native_start).toLocaleString():null].filter(Boolean).join(' · ')));guide.append(entry);}main.append(guide);
 }
 if(route.guideError)main.append(node('p',route.guideError,'secondary'));
}
function sportsStats(event){
 const panel=node('section',undefined,'sports-detail-card');panel.append(node('h2','Team Stats'));
 const stats=Array.isArray(event.stats)?event.stats.slice(0,60):[];
 if(!stats.length){panel.append(node('p','Team statistics are not available for this game yet.','secondary'));return panel;}
 for(const stat of stats){
  const row=node('div',undefined,'sports-stat'),labels=node('div',undefined,'sports-stat-labels');
  labels.append(node('strong',String(stat.away??'–')),node('span',String(stat.label||stat.name||'').slice(0,128)),node('strong',String(stat.home??'–')));row.append(labels);
  const a=Number.parseFloat(stat.away),h=Number.parseFloat(stat.home);
  if(Number.isFinite(a)&&Number.isFinite(h)&&a>=0&&h>=0&&a+h>0){const bars=node('div',undefined,'sports-stat-bars'),away=node('span'),home=node('span');away.style.width=(a/(a+h)*100)+'%';home.style.width=(h/(a+h)*100)+'%';bars.append(away,home);row.append(bars);}
  panel.append(row);
 }return panel;
}
function sportsPlays(route){
 const panel=node('section',undefined,'sports-plays'),tabs=node('div',undefined,'sports-segmented');
 for(const [label,key]of[['All Plays',false],['Key Plays',true]]){const choice=button(label,()=>{route.keyPlays=key;route.playLimit=60;render();});choice.setAttribute('aria-pressed',String(route.keyPlays===key));tabs.append(choice);}
 panel.append(tabs);const plays=(Array.isArray(route.event.plays)?route.event.plays:[]).filter(play=>!route.keyPlays||play.scoring).slice().reverse();let period=null;
 if(!plays.length)panel.append(empty('No plays to show',route.keyPlays?'Scoring plays will appear here when they are available.':'Play-by-play is not available for this game yet.'));
 for(const play of plays.slice(0,route.playLimit||60)){
  if(play.period!==period){period=play.period;if(period!=null)panel.append(node('h3','Period '+period,'sports-period'));}
  const card=node('article',undefined,'sports-play-card');card.append(node('p',String(play.text||'').slice(0,1024)));const detail=node('div',undefined,'sports-play-meta');detail.append(node('span',String(play.clock||'')),node('span',String(play.awayScore??'–')+' · '+String(play.homeScore??'–')));card.append(detail);panel.append(card);
 }
 if(plays.length>(route.playLimit||60))panel.append(button('More plays',()=>{route.playLimit=(route.playLimit||60)+60;render();}));return panel;
}
function sportsStandings(route){
 const panel=node('section',undefined,'sports-standings');
 if(!route.standings){panel.append(node('p',route.standingsError||'Loading standings…',route.standingsError?'error':'loading'));return panel;}
 const columns=route.league==='nhl'?[['gamesPlayed','GP'],['wins','W'],['losses','L'],['overtimeLosses','OTL'],['points','PTS']]:['epl','mls'].includes(route.league)?[['gamesPlayed','P'],['wins','W'],['ties','D'],['losses','L'],['points','PTS']]:route.league==='nfl'?[['wins','W'],['losses','L'],['ties','T'],['winPercent','PCT']]:[['wins','W'],['losses','L'],['winPercent','PCT'],['gamesBehind','GB']];
 for(const group of route.standings.groups){
  const card=node('section',undefined,'sports-detail-card');card.append(node('h2',String(group.name||'Standings').slice(0,128)));
  const table=node('table',undefined,'standings-table'),head=node('thead'),heading=node('tr');heading.append(node('th','Team'));for(const [,label]of columns)heading.append(node('th',label));head.append(heading);table.append(head);
  const body=node('tbody');for(const entry of (group.entries||[]).slice(0,64)){
   const row=node('tr'),team=node('td'),label=node('span',sportsTeamName(entry.team));team.append(node('span',entry.rank==null?'':String(entry.rank),'standing-rank'),sportsLogo(entry.team),label);if(sportsFollowed(entry.team||{},route.league))team.append(node('span','★','standing-followed'));row.append(team);
   for(const [key]of columns)row.append(node('td',String(entry.stats?.[key]??'–')));body.append(row);
  }table.append(body);card.append(table);panel.append(card);
 }return panel;
}
function sportsDetailView(main){
 const route=state.route,event=route.event,league=route.league,home=event.home||{},away=event.away||{};
 const header=node('section',undefined,'sports-game-detail');
 if(/^[a-f0-9]{6}$/i.test(away.color||''))header.style.setProperty('--away-color','#'+away.color);
 if(/^[a-f0-9]{6}$/i.test(home.color||''))header.style.setProperty('--home-color','#'+home.color);
 header.append(node('p',league.toUpperCase(),'sports-league-label'));
 const score=node('div',undefined,'sports-detail-score');
 for(const [team,side]of[[away,'away'],[home,'home']]){
  const block=node('div',undefined,'sports-detail-team '+side);
  if(event.status?.state!=='pre')block.append(node('strong',String(team.score??'–'),'large-score'));
  block.append(sportsTeamView(team,league));
  const follow=action(sportsFollowed(team,league)?'Following':'Follow',()=>followSportsTeam(team,league),'team-follow','star');follow.setAttribute('aria-pressed',String(sportsFollowed(team,league)));block.append(follow);score.append(block);
 }
 const middle=node('div',undefined,'sports-game-state');middle.append(node('strong',String(event.status?.detail||'')));if(event.status?.state==='pre'&&event.date){const date=new Date(event.date);middle.append(node('span',date.toLocaleTimeString(undefined,{hour:'numeric',minute:'2-digit'})),node('small',date.toLocaleDateString()));}else if(event.status?.clock)middle.append(node('small',event.status.clock));score.append(middle);header.append(score);
 if(event.broadcasts?.length)header.append(node('p','Coverage: '+event.broadcasts.join(' · '),'sports-broadcasts'));
 main.append(header);
 const tabs=node('div',undefined,'sports-detail-tabs');tabs.setAttribute('aria-label','Game information');
 for(const [key,label]of[['stats','Stats'],['plays','Play-by-Play'],['standings','Standings']]){const tab=button(label,()=>{route.detailTab=key;render();});tab.setAttribute('aria-pressed',String(route.detailTab===key));tabs.append(tab);}main.append(tabs);
 if(route.detailError)main.append(node('p',route.detailError,'error'));
 main.append(route.detailTab==='plays'?sportsPlays(route):route.detailTab==='standings'?sportsStandings(route):sportsStats(event));
 const info=node('section',undefined,'sports-game-info');info.append(node('h2','Game Information'));if(event.date)info.append(node('p',new Date(event.date).toLocaleString()));if(event.venue)info.append(node('p',event.venue));if(event.broadcasts?.length)info.append(node('p',event.broadcasts.join(' · ')));info.append(node('small','Scores and game information · ESPN'));main.append(info);
}
function featureView(item,eyebrow){
 const hero=node('section',undefined,'hero featured'),copy=node('div',undefined,'hero-copy');bindArtwork(hero,item,true);
 const kind={Movie:'Movie',Series:'TV Show',Episode:'Episode',Audio:'Song',MusicAlbum:'Album'}[item.Type]||'Your Library';
 copy.append(node('p',eyebrow||kind,'hero-eyebrow'),node('h2',item.Name));
 const meta=[item.ProductionYear,item.OfficialRating,item.RunTimeTicks>0?Math.round(item.RunTimeTicks/600000000)+' min':null].filter(Boolean).join(' · ');
 if(meta)copy.append(node('p',meta,'hero-meta'));
 if(item.Overview)copy.append(node('p',item.Overview,'hero-overview'));
 const controls=node('div',undefined,'actions');
 const playable=['Movie','Episode','MusicVideo','Audio'].includes(item.Type)||(item.Type==='TvChannel'&&state.mode==='mesh');
 if(playable)controls.append(action(resumeTicks(item)>0?'Resume':'Play',()=>play(item,[item],0),'primary','play'));
 else if(item.Type!=='TvChannel')controls.append(action(item.Type==='Series'?'View Episodes':'Open',()=>openItem(item),'primary','play'));
 if(state.route.kind!=='detail'&&['Movie','Episode','MusicVideo','Audio','TvChannel'].includes(item.Type))controls.append(action('Details',()=>openItem(item),'hero-secondary','info'));
 copy.append(controls);hero.append(copy);return hero;
}
function genreGrid(entries,music=false){
 const grid=node('div',undefined,'browse-grid');
 entries.forEach((item,index)=>{
  const tile=action(item.Name,()=>openCollection(item.Name,music?'MusicAlbum,Audio':null,null,null,{genre:id(item.Id)}),'browse-tile','music');
  tile.style.setProperty('--tile-tone',String(index%6));tile.dataset.tone=String(index%6);
  if(item.ImageTags?.Primary||item.BackdropImageTags?.length)bindArtwork(tile,item,true);
  grid.append(tile);
 });return grid;
}
function musicLibraryView(main){
 const browse=node('div',undefined,'music-browse');
 const entries=[
  ['Playlists','queue',()=>openCollection('Playlists','Playlist')],
  ['Artists','artist',()=>openCollection('Artists','MusicArtist')],
  ['Albums','album',()=>openCollection('Albums','MusicAlbum')],
  ['Songs','music',()=>openCollection('Songs','Audio')],
  ['Favorites','star',()=>openCollection('Favorites','MusicAlbum,Audio',null,null,{favorite:true})],
  ['Music Videos','shows',()=>openCollection('Music Videos','MusicVideo')],
  ['Genres','genres',async()=>{state.back.push({...state.route});state.route={kind:'genres',title:'Genres'};await loadRoute();}],
  ['On This Device','download',openLocalMusic]
 ];
 for(const [name,symbol,task]of entries){
  const entry=action(name,task,'library-link',symbol),chevron=icon('chevron');chevron.setAttribute('class','chevron');entry.append(chevron);browse.append(entry);
 }
 main.append(browse);
 for(const section of state.shelves)main.append(shelf(section.title,section.items,'square'));
 if(!state.shelves.some(section=>section.items.length)&&!state.route.loading)main.append(empty('Your music, together','Albums from your connected library appear here. Open Songs to browse individual tracks.'));
}
function pagination(main,route){
 if(!route.total)return;
 const nav=node('nav',undefined,'pagination');nav.setAttribute('aria-label','Library pages');
 if(route.start>0)nav.append(action('Previous',async()=>{route.start=Math.max(0,route.start-100);await loadRoute();},'plain','back'));
 nav.append(node('span',(route.start||0)+1+'–'+Math.min((route.start||0)+state.rows.length,route.total)+' of '+route.total,'secondary'));
 if((route.start||0)+state.rows.length<route.total)nav.append(action('Next',async()=>{route.start=(route.start||0)+100;await loadRoute();},'plain','chevron'));
 main.append(nav);
}
function collectionView(main,route){
 const audio=route.type==='Audio',header=route.headerItem;
 if(header&&['MusicAlbum','Playlist','MusicArtist'].includes(header.Type)){
  const summary=node('section',undefined,'album-header'),art=cover(header,true),copy=node('div',undefined,'album-heading');
  copy.append(node('h2',header.Name),node('p',subtitle(header),'secondary'));
  if(header.Overview)copy.append(node('p',header.Overview,'album-description'));
  if(audio&&state.rows.length){const controls=node('div',undefined,'actions');controls.append(action('Play',()=>play(state.rows[0],state.rows,0),'primary','play'),action('Shuffle',()=>playShuffled(state.rows),'','shuffle'));copy.append(controls);}
  summary.append(art,copy);main.append(summary);
 }else if(audio&&state.rows.length){const controls=node('div',undefined,'actions music-actions');controls.append(action('Play',()=>play(state.rows[0],state.rows,0),'primary','play'),action('Shuffle',()=>playShuffled(state.rows),'','shuffle'));main.append(controls);}
 const list=node('div',undefined,audio?'group track-list':route.kind==='live'?'group channel-list':'catalog-grid'+(route.type==='Episode'?' episode-grid':''));
 state.rows.forEach((item,index)=>{
  if(audio){const entry=row(item,()=>play(item,state.rows,index));if(header){entry.classList.add('album-track');entry.prepend(node('span',String(item.IndexNumber||index+1),'track-number'));}list.append(entry);}
  else list.append(route.kind==='live'?row(item):poster(item,route.type==='Episode'?'landscape':'poster'));
 });
 main.append(list);
 if(!state.rows.length&&!route.loading)main.append(empty('No items to show','Try another collection or refresh your library.'));
 if(route.kind==='collection')pagination(main,route);
}
function libraryView(){
 const main=node('main',undefined,'app-main');main.id='content';
 const route=state.route,title=route.kind==='music'?'Library':route.title||(route.kind==='detail'?route.item.Name:state.tab);
 main.dataset.page=route.kind;main.append(toolbar(title,state.back.length>0));
 if(state.back.length&&!['detail','sports_detail'].includes(route.kind))main.append(node('h1',title));
 if(route.kind==='search')main.classList.add('search-screen');
 if(route.kind==='local_music'||route.kind==='local_search')localMusicView(main);
 else if(route.kind==='local_connect'){main.append(empty('Connect to your live TV','Open your server’s Bluefin address or use a device invitation to access its channels.'),action('Connect account',openAccountConnection,'primary'));}
 else if(route.kind==='home'){
  if(state.featured)main.append(featureView(state.featured,resumeTicks(state.featured)>0?'Continue Watching':'Picked for You'));
  for(const section of state.shelves)main.append(shelf(section.title,section.items));
  if(!state.featured&&!route.loading)main.append(empty('Browse your library','Open Collections to see your libraries.'));
 }else if(route.kind==='catalog'){
  if(state.featured&&(route.start||0)===0)main.append(featureView(state.featured,'Recently Added'));
  if(route.favorites?.length&&(route.start||0)===0)main.append(shelf('Your Favorites',route.favorites));
  if(route.recent?.length&&(route.start||0)===0)main.append(shelf('New to Your Library',route.recent));
  const section=node('section',undefined,'catalog-section');section.append(node('h2','All '+route.title));
  const grid=node('div',undefined,'catalog-grid');for(const item of state.rows)grid.append(poster(item));section.append(grid);main.append(section);pagination(main,route);
 }else if(route.kind==='sports')sportsView(main);
 else if(route.kind==='sports_detail')sportsDetailView(main);
 else if(route.kind==='library'){
  const collections=group();for(const item of state.rows)collections.append(row(item));main.append(collections);
  if(!state.rows.length&&!route.loading)main.append(empty('Your collections','Libraries available to your account will appear here.'));
 }else if(route.kind==='music')musicLibraryView(main);
 else if(route.kind==='genres'){main.append(genreGrid(state.rows,true));if(!state.rows.length&&!route.loading)main.append(empty('No genres yet','Genres appear when music in your library includes genre metadata.'));}
 else if(route.kind==='search')searchView(main);
 else if(route.kind==='programme')programmeView(main,route.item);
 else if(route.kind==='collection'||route.kind==='live'){
  if(route.kind==='live'&&state.mode!=='mesh')main.append(node('p','Browse your channels. Use the paired Bluefin app to play live TV.','secondary'));
  collectionView(main,route);
 }else if(route.kind==='detail'){
  const item=route.item;main.append(featureView(item));
  if(item.Type==='TvChannel'&&state.mode!=='mesh')main.append(node('p','Use the paired Bluefin app to play this live channel.','secondary'));
  const information=node('section',undefined,'detail-information');information.append(node('h2','About'),node('p',item.Overview||'No description is available for this title.'));
  if(item.Genres?.length)information.append(node('p',item.Genres.join(' · '),'secondary'));main.append(information);
 }
 if(route.loading&&!state.rows.length&&!state.shelves.length&&!state.featured){const skeleton=node('div',undefined,'loading-skeleton');skeleton.setAttribute('aria-hidden','true');for(let i=0;i<4;i++)skeleton.append(node('div'));main.append(skeleton);}
 if(state.busy||route.loading)main.append(node('p',route.kind==='search'?'Searching your account…':'Loading your library…','loading'));
 main.append(status());if(state.error)main.append(action('Try again',loadRoute));return main;
}
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
 try{await retireSing(previous);}catch(error){failure=failure||error;}
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
 if(!['Audio','Movie','Episode','MusicVideo','TvChannel'].includes(item.Type))return openItem(item);
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
   try{if(lease.kind==='cookie')await leaseRequest(lease.renewUrl,'POST',session);else if(!playbackPaused())await leaseRequest(lease.mediaUrl,'HEAD',session,false,'tv');}
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
async function reportProgress(phase){const p=state.player;if(!p||p.mode!=='https'||p.session!==state.session||!p.reportingReady)return;await api('Sessions/Playing'+(phase==='started'?'':phase==='stopped'?'/Stopped':'/Progress'),'POST',{ItemId:p.item.Id,PlaySessionId:p.lease.playSessionId,PositionTicks:Math.floor(playbackTimeline().position*10000000),IsPaused:playbackPaused(),CanSeek:p.item.Type!=='TvChannel',PlayMethod:'Transcode'});}
async function skip(forward,automatic=false){if(!state.player||!state.queue.length)return;let next=state.queueIndex+(forward?1:-1);if(automatic&&state.repeat==='one')next=state.queueIndex;if(next>=state.queue.length){if(state.repeat!=='all')return stopPlayback();next=0;}if(next<0){if(state.repeat!=='all')next=0;else next=state.queue.length-1;}await play(state.queue[next],state.queue,next);}
function toggle(){const player=state.player;if(player?.sing?.active){runSing(player,singPosition(player.sing),!player.sing.paused).catch(error=>{state.error=String(error.message||error);render();});return;}if(media.paused)media.play().catch(()=>{state.error='Press Play in the media controls.';render();});else media.pause();updatePlayback();}

// Lyrics use the authenticated library route and the current media clock. Word
// timing is rendered only when the server supplies real, valid UTF-16 cues.
function playbackPaused(){const player=state.player;return player?.sing?.active?player.sing.paused:player?.mode==='mesh'&&player.snapshot?player.snapshot.paused!==false:media.paused;}
function singSupported(player){return player?.mode!=='local'&&player?.item.Type==='Audio'&&(state.capabilities?.karaokePreparation?.enabled===true||!!player.sing);}
function validateSingManifest(value,item,sourceId){
 const hash=value=>typeof value==='string'&&/^[a-f0-9]{64}$/.test(value);
 if(!value||!uuid(value.itemId)||id(value.itemId)!==id(item.Id)||!hash(value.revision)||!Number.isFinite(value.durationSeconds)||value.durationSeconds<=0||value.durationSeconds>14400)throw Error('Invalid Sing package.');
 if(!uuid(sourceId)||!uuid(value.mediaSourceId)||id(value.mediaSourceId)!==id(sourceId))throw Error('Sing audio does not match the playing version.');
 for(const role of['accompaniment','leadVocal'])if(!hash(value[role]?.sha256)||!Number.isSafeInteger(value[role]?.bytes)||value[role].bytes<46||value[role].bytes>64*1024*1024)throw Error('Invalid Sing stem.');
 if(value.timingKind==='none'){if(value.timeline!=null)throw Error('Unexpected Sing timing.');}
 else if(![undefined,'word','syllable'].includes(value.timingKind)||!value.timeline)throw Error('Invalid Sing timing.');
 return value;
}
function singPcm(bytes,duration){
 if(!(bytes instanceof Uint8Array)||bytes.length<46||bytes.length>64*1024*1024)throw Error('Invalid Sing audio.');
 const view=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength),text=(start,end)=>String.fromCharCode(...bytes.slice(start,end)),rate=view.getUint32(24,true),channels=view.getUint16(22,true),size=view.getUint32(40,true);
 if(text(0,4)!=='RIFF'||text(8,16)!=='WAVEfmt '||text(36,40)!=='data'||view.getUint32(16,true)!==16||view.getUint16(20,true)!==1||![1,2].includes(channels)||rate<8000||rate>192000||view.getUint16(34,true)!==16||view.getUint16(32,true)!==channels*2||view.getUint32(28,true)!==rate*channels*2||size+44!==bytes.length||view.getUint32(4,true)+8!==bytes.length||size%(channels*2)!==0)throw Error('Sing audio is not supported PCM.');
 const frames=size/(channels*2);if(Math.abs(frames/rate-duration)>.05)throw Error('Sing audio does not match the song duration.');return {rate,channels,frames};
}
function singLyrics(manifest){
 if(manifest.timingKind==='none')return null;
 const timeline=manifest.timeline;
 if(!Number.isFinite(timeline?.offsetSeconds)||!Array.isArray(timeline.lines)||!timeline.lines.length||timeline.lines.length>4096)throw Error('Invalid Sing lyrics.');
 let lastStart=-1,totalTokens=0;const ids=new Set();
 const lines=timeline.lines.map(line=>{
  if(typeof line.id!=='string'||!line.id||line.id.length>128||ids.has(line.id)||!Array.isArray(line.tokens)||!line.tokens.length||line.tokens.length>1000||(totalTokens+=line.tokens.length)>16384)throw Error('Invalid Sing lyric line.');
  ids.add(line.id);let text='',end=0;const cues=[];
  for(const token of line.tokens){
   if(typeof token.text!=='string'||!token.text||token.text.length>4096||!Number.isFinite(token.start)||!Number.isFinite(token.end)||token.start<end||token.end<=token.start||token.end+timeline.offsetSeconds>manifest.durationSeconds+.05)throw Error('Invalid Sing lyric token.');
   const from=text.length;text+=token.text;cues.push({Position:from,EndPosition:text.length,Start:Math.round(token.start*10000000),End:Math.round(token.end*10000000)});end=token.end;
  }
  if(line.tokens[0].start<lastStart)throw Error('Sing lyric lines are out of order.');lastStart=line.tokens[0].start;
  return {Text:text,Start:cues[0].Start,Cues:cues};
 });
 return {...normalizeLyrics({Metadata:{Offset:Math.round(timeline.offsetSeconds*10000000)},Lyrics:lines}),status:'ready',follow:true,controller:new AbortController()};
}
async function singStem(player,sing,manifest,role){
 const stem=manifest[role==='lead-vocal'?'leadVocal':'accompaniment'],query=manifest.mediaSourceId?[['MediaSourceId',manifest.mediaSourceId]]:[];
 let bytes;
 if(player.mode==='mesh'){
  const engine=await runtime().engine();sing.current();
  if(typeof engine.webMeshKaraokeStem!=='function')throw Error('Reload Bluefin to use Sing over this connection.');
  bytes=await engine.webMeshKaraokeStem(player.handle,id(player.item.Id),manifest.mediaSourceId||undefined,manifest.revision,role,stem.bytes);
 }else bytes=await authenticatedBytes('Bluefin/Karaoke/'+id(player.item.Id)+'/'+manifest.revision+'/Stems/'+role,query,player.session,sing.controller.signal,64*1024*1024,stem.bytes,60000);
 sing.current();if(!(bytes instanceof Uint8Array))bytes=new Uint8Array(bytes);
 if(bytes.byteLength!==stem.bytes)throw Error('The Sing download was incomplete.');
 const digest=new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),actual=Array.from(digest,value=>value.toString(16).padStart(2,'0')).join('');sing.current();
 if(actual!==stem.sha256)throw Error('The Sing download did not pass verification. Try again.');return bytes;
}
function singDelay(ms,signal){return new Promise((resolve,reject)=>{const cancel=()=>{clearTimeout(timer);reject(aborted());};const timer=setTimeout(()=>{signal.removeEventListener('abort',cancel);resolve();},ms);signal.addEventListener('abort',cancel,{once:true});if(signal.aborted)cancel();});}
async function prepareSing(player){
 if(state.player!==player||!singSupported(player)||['loading','queued','preparing','ready'].includes(player.sing?.status))return;
 const selection=player.mode==='mesh'?player.snapshot?.mediaSourceId:player.lease?.mediaSourceId;
 if(!uuid(selection)){state.error='This server must identify the playing version before Sing can start. Refresh Bluefin and try again.';render();return;}
 if(player.sing)await retireSing(player);if(state.player!==player)return;
 const Context=globalThis.AudioContext||globalThis.webkitAudioContext;
 if(!Context||!crypto.subtle){state.error='This browser cannot start the Sing audio engine.';render();return;}
 const epoch=state.epoch,session=state.session,generation=playbackGeneration;
 const sing=player.sing={status:'loading',active:false,paused:true,position:0,vocalLevel:.15,controller:new AbortController(),context:new Context({latencyHint:'interactive'}),base:state.base,requestId:null,transportRevision:0,buffers:null};
 sing.current=()=>{if(pageRetired||state.player!==player||player.sing!==sing||epoch!==state.epoch||session!==state.session||generation!==playbackGeneration||sing.controller.signal.aborted)throw aborted();};
 render();
 try{
  await sing.context.resume();sing.current();await sing.context.suspend();sing.current();
  let manifest;const query=[['MediaSourceId',id(selection)]];
  try{manifest=await api('Bluefin/Karaoke/'+id(player.item.Id),'GET',undefined,query,sing.controller.signal);}
  catch(error){
   sing.current();if(!notFound(error))throw error;
   if(state.capabilities?.karaokePreparation?.enabled!==true)throw Error('Sing is not available for this song.');
   sing.requestId=crypto.randomUUID();let status=await api('Bluefin/Karaoke/'+id(player.item.Id)+'/Preparation','POST',{requestId:sing.requestId,...(selection?{mediaSourceId:selection}:{})},[],sing.controller.signal);
   const deadline=Date.now()+15*60*1000;
   for(;;){
    sing.current();
    if(!uuid(status?.itemId)||id(status.itemId)!==id(player.item.Id)||!uuid(status?.jobId)||id(status.jobId)!==id(sing.requestId)||!['queued','preparing','ready','unavailable','failed','cancelling','cancelled'].includes(status.state))throw Error('Invalid Sing preparation response.');
    if(status.state==='ready'){
     sing.preparationFinished=true;
     if(!uuid(status.mediaSourceId)||id(status.mediaSourceId)!==id(selection))throw Error('Sing preparation selected a different version of this song.');
     break;
    }
    if(!['queued','preparing'].includes(status.state))throw Error(({durationUnsupported:'This song is too long for automatic Sing preparation.',unsupportedSource:'Sing is unavailable for this version of the song.',busy:'The server is preparing another song. Try again shortly.',timingRequired:'This server needs supplied Sing timing for this song.'})[status.reason]||'Sing preparation could not finish. Your song can keep playing.');
    sing.status=status.state;render();if(Date.now()>deadline)throw Error('Sing preparation is taking too long. Try again.');
    await singDelay(Math.min(10000,Math.max(1000,Number(status.retryAfterMilliseconds)||2000)),sing.controller.signal);sing.current();
    status=await api('Bluefin/Karaoke/'+id(player.item.Id)+'/Preparation/'+sing.requestId,'GET',undefined,[],sing.controller.signal);
   }
   sing.current();
   manifest=await api('Bluefin/Karaoke/'+id(player.item.Id),'GET',undefined,query,sing.controller.signal);
  }
  sing.current();sing.manifest=validateSingManifest(manifest,player.item,selection);sing.preparedLyrics=singLyrics(manifest);sing.status='loading';render();
  const files=await Promise.all(['accompaniment','lead-vocal'].map(role=>singStem(player,sing,manifest,role)));sing.current();
  const formats=files.map(bytes=>singPcm(bytes,manifest.durationSeconds));
  if(formats[0].rate!==formats[1].rate||formats[0].channels!==formats[1].channels||formats[0].frames!==formats[1].frames)throw Error('The Sing tracks are not synchronized.');
  if(manifest.durationSeconds*sing.context.sampleRate*formats[0].channels*4*2>256*1024*1024)throw Error('This Sing package is too large for browser playback. Open it in the Bluefin app.');
  const buffers=await Promise.all(files.map(bytes=>sing.context.decodeAudioData(bytes.buffer)));sing.current();
  if(buffers.some(buffer=>buffer.numberOfChannels!==formats[0].channels||Math.abs(buffer.duration-manifest.durationSeconds)>.05)||buffers[0].length!==buffers[1].length||buffers[0].sampleRate!==buffers[1].sampleRate)throw Error('The Sing audio clocks do not match.');
  sing.buffers=buffers;sing.duration=buffers[0].duration;sing.status='ready';render();
 }catch(error){
  if(player.sing===sing&&state.player===player&&!sing.controller.signal.aborted){sing.status='error';sing.message=String(error.message||error);await disposeSingAudio(sing);render();await cancelSingPreparation(player,sing);}
 }
}
function singPosition(sing){return Math.min(sing.duration||0,Math.max(0,sing.position+(sing.paused?0:Math.max(0,sing.context.currentTime-sing.startedAt))));}
function stopSingNodes(sing){sing.transportRevision++;for(const source of sing.nodes||[]){source.onended=null;try{source.stop();}catch(_){}source.disconnect();}sing.nodes=[];}
async function runSing(player,position,paused=false){
 const sing=player.sing;if(!sing?.active||!sing.buffers)return;
 sing.current();sing.position=Math.max(0,Math.min(sing.duration,position));sing.paused=true;stopSingNodes(sing);
 const revision=sing.transportRevision;
 if(paused){await sing.context.suspend();updatePlayback();return;}
 await sing.context.resume();sing.current();if(revision!==sing.transportRevision)return;
 const when=sing.context.currentTime+.035;
 const vocal=sing.vocalGain=sing.context.createGain(),backing=sing.context.createGain();vocal.gain.value=sing.vocalLevel;backing.gain.value=1;vocal.connect(sing.context.destination);backing.connect(sing.context.destination);
 sing.gains?.forEach(gain=>gain.disconnect());sing.gains=[backing,vocal];
 sing.nodes=sing.buffers.map((buffer,index)=>{const source=sing.context.createBufferSource();source.buffer=buffer;source.connect(index?vocal:backing);return source;});
 sing.nodes[0].onended=()=>{if(state.player!==player||player.sing!==sing||revision!==sing.transportRevision||sing.paused)return;sing.position=sing.duration;sing.paused=true;updatePlayback();skip(true,true).catch(error=>{state.error=String(error.message||error);render();});};
 sing.startedAt=when;sing.paused=false;for(const source of sing.nodes)source.start(when,sing.position);updatePlayback();
}
async function activateSing(player){
 const sing=player.sing;if(state.player!==player||sing?.status!=='ready')return;
 const position=playbackTimeline().position;sing.current();
 if(player.mode==='mesh')runtime().ready().webMeshControl(player.handle,'pause',1);else media.pause();
 sing.active=true;sing.originalLyrics=player.lyrics;if(sing.preparedLyrics)player.lyrics=sing.preparedLyrics;
 state.nowPanel='lyrics';state.expanded=true;await runSing(player,position);sing.current();
 sing.timer=setInterval(()=>{if(state.player===player&&player.sing===sing&&!document.hidden)updatePlayback();},250);render();
}
async function disposeSingAudio(sing){stopSingNodes(sing);clearInterval(sing.timer);sing.gains?.forEach(gain=>gain.disconnect());sing.gains=[];sing.buffers=null;try{await sing.context.close();}catch(_){}}
async function cancelSingPreparation(player,sing){
 if(!sing.requestId||sing.preparationFinished||sing.preparationCancelled)return;
 sing.preparationCancelled=true;
 const path='Bluefin/Karaoke/'+id(player.item.Id)+'/Preparation/'+sing.requestId;
 try{if(player.mode==='mesh'){const engine=await runtime().engine();await engine.webMeshRequest(player.handle,'DELETE',path,'[]',undefined);}
 else await http(path,'DELETE',undefined,[],player.session,true,true,sing.base);}catch(_){}
}
async function retireSing(player){
 const sing=player?.sing;if(!sing)return;sing.controller.abort();sing.active=false;await disposeSingAudio(sing);
 await cancelSingPreparation(player,sing);
 if(player.sing===sing)player.sing=null;
}
async function leaveSing(player){
 const sing=player.sing;if(!sing)return;const position=sing.active?singPosition(sing):null,paused=sing.paused,lyrics=sing.originalLyrics;
 await retireSing(player);if(state.player!==player)return;if(lyrics)player.lyrics=lyrics;
 if(position!==null){
  if(player.mode==='https'&&position<(player.offset||0)){
   const item={...player.item,UserData:{...player.item.UserData,PlaybackPositionTicks:Math.floor(position*10000000)}};await play(item,state.queue,state.queueIndex);if(paused)media.pause();
  }else{seekPlayback(position);if(player.mode==='mesh')runtime().ready().webMeshControl(player.handle,'pause',paused?1:0);else if(!paused)await media.play();}
 }
 render();
}
function singControls(player){
 const sing=player.sing;if(!sing)return document.createDocumentFragment();
 const panel=node('section',undefined,'sing-controls');
 if(sing.active){
  const label=node('label','Lead vocal'),value=node('span',Math.round(sing.vocalLevel*100)+'%');label.htmlFor='bluefin-vocal-level';label.append(value);
  const range=node('input');range.id='bluefin-vocal-level';range.type='range';range.min='0';range.max='1';range.step='.01';range.value=String(sing.vocalLevel);range.setAttribute('aria-label','Lead vocal level');
  range.oninput=()=>{if(player.sing!==sing)return;sing.vocalLevel=Number(range.value);value.textContent=Math.round(sing.vocalLevel*100)+'%';if(sing.vocalGain)sing.vocalGain.gain.setTargetAtTime(sing.vocalLevel,sing.context.currentTime,.015);};
  panel.append(label,range);
 }else{
  const message=sing.status==='ready'?'Ready to sing':sing.status==='queued'?'Waiting to prepare this song…':sing.status==='preparing'?'Preparing vocals on your server…':sing.status==='error'?sing.message:'Loading Sing audio…';panel.append(node('p',message,'sing-status'));
  if(sing.status==='ready')panel.append(button('Start Sing',()=>activateSing(player).catch(error=>{state.error=String(error.message||error);render();}),'primary','mic'));
  else if(sing.status!=='error')panel.append(button('Cancel',()=>leaveSing(player),'plain'));
 }
 return panel;
}

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
 const p=state.player;if(p?.sing?.active)return playbackTimeline().position;
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
 const active=()=>!pageRetired&&document.visibilityState!=='hidden'&&state.expanded&&state.nowPanel==='lyrics'&&lyricsView?.player===state.player&&state.player?.lyrics?.status==='ready'&&state.player.lyrics.timed.length>0&&!playbackPaused();
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
 if(!player.sing?.active&&player.mode==='https'&&position<(player.offset||0)){
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
 const playButton=button('',toggle,'plain',playbackPaused()?'play':'pause');playButton.setAttribute('aria-label',playbackPaused()?'Play':'Pause');playButton.dataset.playToggle='';e.append(playButton);
 const next=action('',()=>skip(true),'plain','next');next.setAttribute('aria-label','Next track');next.disabled=state.busy||(state.repeat!=='all'&&state.queueIndex+1>=state.queue.length);e.append(next);return e;
}
function nowPlaying(){
 const player=state.player;if(!player){state.expanded=false;return document.createDocumentFragment();}
 const sheet=node('section',undefined,'sheet now-playing-sheet');sheet.setAttribute('role','dialog');sheet.setAttribute('aria-modal','true');sheet.setAttribute('aria-label','Now Playing');sheet.id='bluefin-dialog';sheet.tabIndex=-1;
 const ambience=node('div',undefined,'now-ambience');ambience.setAttribute('aria-hidden','true');bindArtwork(ambience,player.item);
 const inner=node('div',undefined,'now-playing'),bar=node('div',undefined,'toolbar');
 const close=button('',()=>{state.expanded=false;render();},'plain','down');close.setAttribute('aria-label','Close Now Playing');
 bar.append(close,node('span',player.sing?.active?'BLUEFIN SING':'NOW PLAYING','now-caption'),action('Stop',()=>stopPlayback(),'plain'));inner.append(bar);
 const layout=node('div',undefined,'now-layout'),primary=node('div',undefined,'now-primary'),stage=node('div',undefined,'now-stage'),art=node('div',undefined,'now-art');
 layout.dataset.panel=state.nowPanel;
 if(state.nowPanel==='artwork')art.id='now-playing-panel';
 art.append(icon('music'));bindArtwork(art,player.item);stage.append(art);primary.append(stage);
 const controls=node('div',undefined,'now-controls'),title=node('div',undefined,'now-track-title');
 title.append(node('h1',player.item.Name),node('p',player.item.AlbumArtist||player.item.Artists?.join(', ')||''));controls.append(title);
 if(player.mode==='local')controls.append(node('p','Local file'+(player.item.Album?' · '+player.item.Album:''),'now-source'));
 const timeline=playbackTimeline(),seek=node('input');seek.type='range';seek.min=String(timeline.minimum);seek.max=String(Math.max(timeline.duration,timeline.minimum+1));seek.step='.1';seek.value=String(timeline.position);seek.disabled=timeline.duration<=0;
 seek.setAttribute('aria-label','Playback position');seek.dataset.seek='';seek.oninput=()=>seekPlayback(Number(seek.value));
 const times=node('div',undefined,'time-row'),position=node('span',time(timeline.position)),remaining=node('span','−'+time(timeline.duration-timeline.position));position.dataset.position='';remaining.dataset.remaining='';times.append(position,remaining);controls.append(seek,times);
 const transport=node('div',undefined,'transport'),shuffle=button('',toggleShuffle,state.shuffle?'on':'','shuffle');shuffle.setAttribute('aria-label','Shuffle');shuffle.setAttribute('aria-pressed',String(state.shuffle));shuffle.disabled=state.queue.length<2;
 const previous=action('',()=>skip(false),'','previous');previous.setAttribute('aria-label','Previous track');
 const toggleButton=button('',toggle,'main-play',playbackPaused()?'play':'pause');toggleButton.dataset.playToggle='';toggleButton.setAttribute('aria-label',playbackPaused()?'Play':'Pause');
 const next=action('',()=>skip(true),'','next');next.setAttribute('aria-label','Next track');next.disabled=state.busy||(state.repeat!=='all'&&state.queueIndex+1>=state.queue.length);
 const repeat=button('',()=>{state.repeat=({off:'all',all:'one',one:'off'})[state.repeat];syncLocalQueue();render();},state.repeat==='off'?'':'on','repeat');repeat.setAttribute('aria-label','Repeat: '+state.repeat);repeat.setAttribute('aria-pressed',String(state.repeat!=='off'));
 transport.append(shuffle,previous,toggleButton,next,repeat);controls.append(transport);
 const options=node('div',undefined,'player-options');
 for(const [label,panel,symbol]of[['Lyrics','lyrics','lyrics'],['Queue','queue','queue']]){
  const choice=button(label,()=>{state.nowPanel=state.nowPanel===panel?'artwork':panel;render();},'',symbol);choice.setAttribute('aria-pressed',String(state.nowPanel===panel));choice.setAttribute('aria-controls','now-playing-panel');options.append(choice);
 }
 if(singSupported(player)){
  const sing=button('Sing',()=>{
   const task=player.sing?.active?leaveSing(player):player.sing?.status==='ready'?activateSing(player):prepareSing(player);
   Promise.resolve(task).catch(error=>{if(state.player===player){state.error=String(error.message||error);render();}});
  },'','mic');
  sing.setAttribute('aria-pressed',String(!!player.sing?.active));sing.disabled=['loading','queued','preparing'].includes(player.sing?.status);options.append(sing);
 }
 if(typeof media.webkitShowPlaybackTargetPicker==='function'&&!player.sing?.active){const output=button('AirPlay',()=>media.webkitShowPlaybackTargetPicker(),'','airplay');output.setAttribute('aria-label','Choose AirPlay speakers');options.append(output);}
 controls.append(options,singControls(player));primary.append(controls);layout.append(primary);
 if(state.nowPanel==='lyrics'||state.nowPanel==='queue'){const secondary=node('div',undefined,'now-secondary');secondary.append(state.nowPanel==='lyrics'?lyricPanel(player):queuePanel());layout.append(secondary);}
 inner.append(layout,status());sheet.append(ambience,inner);return sheet;
}
function updatePlayback(){
 const timeline=playbackTimeline();
 for(const b of document.querySelectorAll('[data-play-toggle]')){b.replaceChildren(icon(playbackPaused()?'play':'pause'));b.setAttribute('aria-label',playbackPaused()?'Play':'Pause');}
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
function clearImages(){state.imageEpoch++;clearTimeout(artworkScrollTimer);clearTimeout(artworkRetryTimer);for(const controller of imageControllers)controller.abort();imageControllers.clear();artworkPending.clear();artworkMissing.clear();artworkRetries.clear();for(const url of state.images.values())URL.revokeObjectURL(url);state.images.clear();}
function render(){
 if(pageRetired)return;
 lyricsView=null;
 const active=document.activeElement,activeId=active?.id,activeKey=active?.dataset?.focusKey;
 const selection=active?.tagName==='INPUT'&&['search','text'].includes(active.type)?[active.selectionStart,active.selectionEnd,active.selectionDirection]:null;
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
media.addEventListener('ended',()=>{if(pageRetired||!state.player||state.player.sing?.active||!media.ended)return;perform(async()=>{await reportProgress('stopped').catch(()=>{});await skip(true,true);});});
media.addEventListener('error',()=>{if(pageRetired||state.player?.sing?.active||!media.getAttribute('src'))return;if(state.player?.mode==='local'){stopPlayback(false).catch(()=>{});state.error='This local file could not play. Its format may be unsupported or protected. Choose another file.';}else state.error='This media could not play. Try another title or refresh your library.';render();});
window.addEventListener('pagehide',()=>{
 if(pageRetired)return;
 const p=state.player,session=state.session,mode=state.mode,base=state.base;
 pageRetired=true;state.epoch++;state.pairingView++;playbackGeneration++;
 for(const controller of requestControllers)controller.abort();requestControllers.clear();
 clearTimeout(state.inviteTimer);clearInterval(leaseRenew);clearInterval(progressTimer);clearInterval(meshPoll);
 state.player=null;retireLyrics(p);clearOutput(p);retireSing(p).catch(()=>{});
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
