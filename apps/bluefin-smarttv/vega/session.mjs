// Native-shell network ownership. Tokens are private process memory; never URLs.
const grant = '[a-f0-9]{64}';
export const categories=['Home','Sports','TV Shows','Movies','Live TV','Music'];
export const searchScopes=[['all','All Media',''],['tvShows','TV Shows','Series,Season,Episode'],['movies','Movies','Movie'],['music','Music','Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist'],['liveTv','Live TV','TvChannel,LiveTvProgram']];
export function searchLanes(filters={}){if(filters.studioID||filters.tag||filters.collectionID)return ['library'];if(filters.scope==='liveTv')return ['programmes','channels'];return ['library',...(!filters.scope||['all','music'].includes(filters.scope)?['artists']:[]),...(!filters.scope||filters.scope==='all'?['programmes','channels']:[])];}
const query=(path,fields)=>path+'?'+Object.entries(fields).map(([key,value])=>encodeURIComponent(key)+'='+encodeURIComponent(String(value))).join('&');
const rows=value=>{const list=Array.isArray(value)?value:value?.Items;if(!Array.isArray(list)||list.length>250)throw Error('Invalid library response.');return list.filter(item=>item&&/^[a-f0-9-]{32,36}$/i.test(item.Id||''));};
export function mergeSearchPage(page,result){
  const found=rows(result),seen=new Set(page.items.map(item=>item.Id.toLowerCase().replace(/-/g,'')));
  const next=page.next+(Array.isArray(result)?result:result.Items).length;
  const items=[...page.items];for(const item of found){const key=item.Id.toLowerCase().replace(/-/g,'');if(!seen.has(key)){seen.add(key);items.push(item);}}
  return {...page,items,next,total:found.length?Math.max(next,Number(result.TotalRecordCount)||next):next};
}
export class Session {
  #token = ''; #epoch = 0; #accountGeneration = 0; #user = ''; #lease = null; #controllers = new Set();
  constructor(base, device, transport = fetch) {
    if (!/^https:\/\/[a-z0-9.-]+(?::[0-9]+)?(?:\/[a-z0-9._~-]+)*\/?$/i.test(base) || !/^[a-z0-9_-]{16,128}$/i.test(device)) throw Error('Use HTTPS and a valid process device identifier.');
    this.base = base.replace(/\/$/, ''); this.device = device; this.transport = transport;
  }
  async request(path, method = 'GET', body, anonymous = false, signal) {
    if(signal?.aborted)throw Error('Request cancelled.');const accountToken=this.#token;
    const controller = new AbortController(); this.#controllers.add(controller);
    const cancel=()=>controller.abort();signal?.addEventListener('abort',cancel,{once:true});
    const timer = setTimeout(() => controller.abort(), 15000);
    try {
      const r = await this.transport(this.base + path, {method, credentials: 'omit', redirect: 'error', cache: 'no-store', referrerPolicy: 'no-referrer', signal: controller.signal,
        headers: {'Content-Type': 'application/json', Authorization: `MediaBrowser Client="Bluefin Vega", Device="Vega", DeviceId="${this.device}", Version="0.1.0"${!anonymous && this.#token ? `, Token="${this.#token}"` : ''}`}, body: body === undefined ? undefined : JSON.stringify(body)});
      if (r.status === 401 && accountToken===this.#token) { this.#token = ''; this.#user=''; this.#epoch++; this.#accountGeneration++; }
      if (!r.ok) throw Object.assign(Error('Bluefin refused the request ('+r.status+').'),{status:r.status});
      if (r.status === 204) return null;
      if (Number(r.headers.get('content-length')) > 2097152) throw Error('Response limit.');
      const text = await r.text(); if (text.length > 2097152) throw Error('Response limit.');
      return JSON.parse(text);
    } finally { signal?.removeEventListener('abort',cancel);clearTimeout(timer); this.#controllers.delete(controller); }
  }
  async connect() {
    const info = await this.request('/System/Info/Public', 'GET', undefined, true);
    if (!info?.Id && !info?.ServerName) throw Error('No media server at this address.');
    let options;
    try { options = await this.request('/Bluefin/Accounts/Options', 'GET', undefined, true); }
    catch { options = {quickConnect: await this.request('/QuickConnect/Enabled', 'GET', undefined, true).catch(() => false) === true}; }
    return {name: info.ServerName || 'Sign in', options};
  }
  async signIn(username, password, create = false) {
    return this.finishSignIn(async () => {
      if (create) await this.request('/Bluefin/Accounts/Create', 'POST', {username, password}, true);
      return this.request('/Users/AuthenticateByName', 'POST', {Username: username, Pw: password}, true);
    });
  }
  beginDeviceSignIn(passkey) {
    return this.request(passkey ? '/Bluefin/Accounts/Passkey/Initiate' : '/QuickConnect/Initiate', 'POST', {}, true);
  }
  async finishDeviceSignIn(request, passkey) {
    return this.finishSignIn(async () => {
      if (passkey) return this.request('/Bluefin/Accounts/Passkey/Exchange', 'POST', request.proof, true);
      const status = await this.request('/QuickConnect/Connect?secret=' + encodeURIComponent(request.Secret), 'GET', undefined, true);
      return status.Authenticated
        ? this.request('/Users/AuthenticateWithQuickConnect', 'POST', {Secret: request.Secret}, true)
        : {pending:true};
    });
  }
  cancelDeviceSignIn(request) {
    return request?.proof ? this.request('/Bluefin/Accounts/Passkey/Cancel', 'POST', request.proof, true) : Promise.resolve();
  }
  async finishSignIn(request) {
    const stopped = this.stop(); this.#token = ''; this.#user='';this.#accountGeneration++;const epoch = this.#epoch;
    await stopped; if (epoch !== this.#epoch) throw Error('Sign-in cancelled.');
    const result = await request();
    if (epoch !== this.#epoch) throw Error('Sign-in cancelled.');
    if (result.pending === true) return null;
    if (!/^[a-z0-9_-]{1,512}$/i.test(result.AccessToken || '') || !result.User?.Id) throw Error('Invalid session.');
    if(!/^[a-f0-9-]{32,36}$/i.test(result.User.Id))throw Error('Invalid account.');
    this.#token = result.AccessToken; this.#user=result.User.Id;
    try { const c = await this.request('/Bluefin/TV/v1'); return c.version === 1 && c.mediaLease === true; }
    catch (e) { this.#token = ''; throw e; }
  }
  async catalogRequest(path,signal) {
    const generation=this.#accountGeneration;
    if(!this.#token||!this.#user)throw Error('Sign in to browse this account.');
    const value=await this.request(path,'GET',undefined,false,signal);
    if(generation!==this.#accountGeneration||signal?.aborted)throw Error('The account or search changed.');
    return value;
  }
  library(kind = 'Movie,Series,Episode', term = '', start=0, signal) {
    return this.catalogRequest(query('/Items',{UserId:this.#user,Recursive:true,Limit:40,StartIndex:start,Fields:'Overview,RunTimeTicks',IncludeItemTypes:kind,SearchTerm:term.slice(0,128)}),signal);
  }
  search(term='',filters={},start=0,lane='library',signal) {
    const scope=searchScopes.find(scope=>scope[0]===filters.scope)||searchScopes[0];
    const fields={UserId:this.#user,Limit:40,StartIndex:start};
    if(term.trim())fields.SearchTerm=term.trim().slice(0,128);
    let path;
    if(lane==='library'){
      path='/Items';fields.Recursive=true;fields.Fields='Overview,RunTimeTicks,PrimaryImageAspectRatio';
      if(scope[2])fields.IncludeItemTypes=scope[2];
      for(const [name,value]of [['StudioIds',filters.studioID],['ParentId',filters.collectionID]])if(value){
        if(!/^[a-f0-9-]{32,36}$/i.test(value))throw Error('Invalid refinement.');fields[name]=value;
      }
      if(filters.tag)fields.Tags=String(filters.tag).slice(0,512);
    }else{
      if(filters.studioID||filters.tag||filters.collectionID)throw Error('Guide and artist results do not use library metadata refinements.');
      if(lane==='artists'){path='/Artists';fields.Recursive=true;}
      else if(lane==='programmes'){path='/LiveTv/Programs';fields.MinEndDate=filters.after||new Date().toISOString();}
      else if(lane==='channels')path='/LiveTv/Channels';
      else throw Error('Invalid search source.');
    }
    return this.catalogRequest(query(path,fields),signal);
  }
  async searchFacets(signal) {
    const generation=this.#accountGeneration;
    const results=await Promise.allSettled([
      this.catalogRequest(query('/Studios',{UserId:this.#user,Limit:200}),signal),
      this.catalogRequest(query('/Items/Filters2',{UserId:this.#user}),signal),
      this.catalogRequest(query('/Items',{UserId:this.#user,Recursive:true,IncludeItemTypes:'BoxSet',Limit:200}),signal)]);
    if(generation!==this.#accountGeneration||signal?.aborted)throw Error('The account or refinements changed.');
    const [studios,tags,collections]=results.map(result=>result.status==='fulfilled'?result.value:null);
    return {studios:studios?rows(studios).map(item=>({id:item.Id,name:item.Name})):[],
      tags:Array.isArray(tags?.Tags)?tags.Tags.filter(tag=>typeof tag==='string').slice(0,512):[],
      collections:collections?rows(collections).map(item=>({id:item.Id,name:item.Name})):[],
      truncated:Number(studios?.TotalRecordCount)>200||Number(collections?.TotalRecordCount)>200||(tags?.Tags?.length||0)>512,
      partial:results.some(result=>result.status==='rejected')};
  }
  async recommendations(signal) {
    try{
      const value=await this.catalogRequest('/Bluefin/Home/Recommendations',signal);
      if(!Array.isArray(value?.recommendations)||value.recommendations.length>12)throw Error('Invalid Home response.');
      return value.recommendations.map(shelf=>({title:String(shelf.title||'Recommended for you'),items:rows(shelf.items).slice(0,18)}));
    }catch(error){if(error.status!==404)throw error;}
    const value=await this.catalogRequest(query('/Movies/Recommendations',{UserId:this.#user,CategoryLimit:4,ItemLimit:12,Fields:'Overview,RunTimeTicks'}),signal);
    if(!Array.isArray(value)||value.length>12)throw Error('Invalid recommendations.');
    const labels={SimilarToRecentlyPlayed:'Because you watched',SimilarToLikedItem:'Because you like',HasDirectorFromRecentlyPlayed:'More from',HasActorFromRecentlyPlayed:'More with',HasLikedDirector:'From a director you like:',HasLikedActor:'With an actor you like:'};
    return value.filter(row=>labels[row.RecommendationType]).map(row=>({title:labels[row.RecommendationType]+' '+String(row.BaselineItemName||'a favorite').slice(0,128),items:rows(row.Items).slice(0,18)}));
  }
  async home(signal) {
    const generation=this.#accountGeneration,account=this.#user;
    const result=await Promise.allSettled([
      this.catalogRequest(query('/Users/'+account+'/Items/Resume',{Limit:12,Fields:'Overview,RunTimeTicks'}),signal).then(rows),
      this.catalogRequest(query('/Shows/NextUp',{UserId:account,Limit:12,Fields:'Overview,RunTimeTicks'}),signal).then(rows),
      this.catalogRequest(query('/Users/'+account+'/Items/Latest',{Limit:18,Fields:'Overview,RunTimeTicks'}),signal).then(rows),
      this.recommendations(signal)]);
    if(generation!==this.#accountGeneration||signal?.aborted)throw Error('The account or Home changed.');
    const [resume,next,latest,recommendations]=result.map(value=>value.status==='fulfilled'?value.value:[]);
    return {shelves:[{title:'Continue watching',items:resume},{title:'Up next',items:next},...recommendations,{title:'Recently added',items:latest}].filter(shelf=>shelf.items.length),
      featured:resume[0]||recommendations.find(shelf=>shelf.items.length)?.items[0]||next[0]||latest[0]||null,
      partial:result.some(value=>value.status==='rejected')};
  }
  async sports(signal) {
    try{
      const value=await this.catalogRequest('/sports/api/v1/home',signal);
      if(value?.schema!==1||value.presentation_only!==true||!Array.isArray(value.games)||!Array.isArray(value.local_broadcasts))throw Error('Invalid sports feed.');
      return {games:[...value.games,...value.local_broadcasts].slice(0,200),items:[]};
    }catch(error){if(error.status!==404)throw error;}
    const page=await this.catalogRequest(query('/LiveTv/Programs',{UserId:this.#user,IsSports:true,Limit:100,MinEndDate:new Date().toISOString()}),signal);
    return {games:[],items:rows(page)};
  }
  collections(signal){return this.catalogRequest('/Users/'+this.#user+'/Views',signal);}
  children(id,signal) { if (!/^[a-f0-9]{32}$/i.test(id)) throw Error('Invalid item.'); return this.catalogRequest('/Items?UserId='+this.#user+'&ParentId='+id+'&Limit=40',signal); }
  item(id,signal) { if (!/^[a-f0-9]{32}$/i.test(id)) throw Error('Invalid item.'); return this.catalogRequest('/Items/'+id+'?UserId='+this.#user,signal); }
  guide(signal) { const now = encodeURIComponent(new Date().toISOString()); return this.catalogRequest('/LiveTv/Programs?UserId='+this.#user+'&Limit=200&IsAiring=true&MaxStartDate='+now+'&MinEndDate='+now,signal); }
  artworkSource(item,backdrop=false){
    if(!this.#token||!/^[a-f0-9-]{32,36}$/i.test(item?.Id||''))return undefined;
    const kind=backdrop&&item.BackdropImageTags?.length?'Backdrop':item.ImageTags?.Primary?'Primary':null;
    if(!kind)return undefined;
    return {uri:this.base+'/Items/'+item.Id+'/Images/'+kind+'?maxWidth='+(backdrop?1200:400)+'&quality=85',
      headers:{Authorization:'MediaBrowser Client="Bluefin Vega", Device="Vega", DeviceId="'+this.device+'", Version="0.1.0", Token="'+this.#token+'"'}};
  }
  async loadQueue() {
    const result = await this.request('/Bluefin/Music/Queue');
    if (!Number.isSafeInteger(result.revision) || !Array.isArray(result.queue?.itemIds) || result.queue.itemIds.length > 200 || !result.queue.itemIds.every(id => typeof id === 'string' && /^[a-f0-9]{32}$/i.test(id.replace(/-/g,'')))) throw Error('Invalid queue');
    return result;
  }
  saveQueue(ids,revision) {
    if (!Number.isSafeInteger(revision) || revision < 0 || !Array.isArray(ids) || ids.length > 200 || !ids.every(id => /^[a-f0-9]{32}$/i.test(id))) throw Error('Invalid queue');
    return this.request('/Bluefin/Music/Queue','PUT',{protocolVersion:1,expectedRevision:revision,queue:{itemIds:ids,cursor:ids.length ? 0 : null,shuffle:false,repeat:'off'}});
  }
  route(path, release) {
    if (typeof path !== 'string' || !new RegExp(release ? `^/Bluefin/TV/v1/leases/${grant}$` : `^/Bluefin/TV/v1/media/${grant}/[0-9]+$`).test(path)) throw Error('Invalid grant route.');
    return this.base + path;
  }
  async acquire(id, kind, startTicks = 0, tracks = {}) {
    if (!this.#token || !/^[a-f0-9]{32}$/i.test(id) || !['audio','video'].includes(kind) || !Number.isSafeInteger(startTicks) || startTicks < 0 || startTicks > 6048000000000) throw Error('Invalid playback request.');
    const stopped = this.stop(), epoch = this.#epoch;
    await stopped; if (epoch !== this.#epoch) throw Error('Playback cancelled.');
    for (const index of [tracks.audio,tracks.subtitle]) if (index != null && (!Number.isSafeInteger(index) || index < 0 || index > 1024)) throw Error('Invalid track');
    const lease = await this.request('/Bluefin/TV/v1/leases', 'POST', {version:1,itemId:id,playback:{kind,maxBitrate:8000000,startTicks,audioStreamIndex:tracks.audio ?? null,subtitleStreamIndex:tracks.subtitle ?? null}});
    // Retain a valid release route even when another response field is invalid.
    this.route(lease.releaseUrl, true);
    try {
      const url = this.route(lease.mediaUrl, false);
      if (epoch !== this.#epoch || lease.version !== 1 || typeof lease.playSessionId !== 'string' || !lease.playSessionId) throw Error('Playback cancelled or invalid.');
      this.#lease = {...lease, itemId:id}; return url;
    } catch (e) { await this.request(lease.releaseUrl, 'DELETE').catch(() => {}); throw e; }
  }
  async report(position, paused, stopped = false) {
    const l = this.#lease; if (!l || !Number.isFinite(position)) return;
    await this.request('/Sessions/Playing' + (stopped ? '/Stopped' : '/Progress'), 'POST', {ItemId:l.itemId,PlaySessionId:l.playSessionId,PositionTicks:Math.round(Math.max(0,position)*10000000),IsPaused:!!paused,PlayMethod:'Transcode'}).catch(() => {});
  }
  async stop() {
    ++this.#epoch; const lease = this.#lease; this.#lease = null;
    if (lease) await this.request(lease.releaseUrl, 'DELETE').catch(() => {});
  }
  async suspend() { ++this.#epoch; for (const c of this.#controllers) c.abort(); await this.stop(); }
  async logout() { ++this.#accountGeneration;this.#user='';await this.suspend(); try { await this.request('/Sessions/Logout','POST'); } finally { this.#token = ''; } }
}
