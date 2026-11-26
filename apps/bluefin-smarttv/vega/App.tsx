import React, {useEffect, useRef, useState} from 'react';
import {AppState, BackHandler, Pressable, Image, ScrollView, Text, TextInput, useColorScheme, View} from 'react-native';
import {VideoPlayer, KeplerVideoSurfaceView} from '@amazon-devices/react-native-w3cmedia';
import {useComponentInstance} from '@amazon-devices/react-native-kepler';
import {Session, categories, searchScopes, searchLanes, mergeSearchPage} from './session.mjs';
import {createAdaptive as hlsPlayer} from './HlsPlayer';

// Application lifecycle interface around upstream Apache-2.0 hls.js.
export interface AdaptivePlayer {
  load(url: string): Promise<void>;
  destroy(): Promise<void>;
}
type Props = {deviceId: string};
type Item = {Id: string; Name: string; Type: string; ChannelId?:string; RunTimeTicks?:number; UserData?:{PlaybackPositionTicks?:number}; ImageTags?:Record<string,string>; BackdropImageTags?:string[]; IsFolder?: boolean; Overview?: string; MediaStreams?: {Type:string;Index:number;DisplayTitle?:string}[]};
type Filters = {scope:string;studioID?:string;tag?:string;collectionID?:string;after?:string};
type SearchPage = {lane:string;items:Item[];next:number;total:number};
async function bounded<T>(operation: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try { return await Promise.race([operation,new Promise<never>((_,reject) => {timer = setTimeout(() => reject(Error('Native player timeout.')),15000);})]); }
  finally { if (timer) clearTimeout(timer); }
}

const moonlitLight = {background:'#A3ACB1',panel:'#CCD0D3',hover:'#BDC5C9',text:'#061727',muted:'#2B4050',accent:'#333B5A',line:'#465966'};
const moonlitDark = {background:'#061727',panel:'#102435',hover:'#182D3D',text:'#CCD0D3',muted:'#A3ACB1',accent:'#ADB5D1',line:'#637889'};

export function App({deviceId}: Props) {
  const palette = useColorScheme() === 'light' ? moonlitLight : moonlitDark;
  const component = useComponentInstance();
  const session = useRef<Session | null>(null), video = useRef<VideoPlayer | null>(null);
  const adaptive = useRef<AdaptivePlayer | null>(null), epoch = useRef(0), busy = useRef(false);
  const active = useRef(AppState.currentState === 'active');
  const [base,setBase] = useState(''), [username,setUsername] = useState(''), [password,setPassword] = useState('');
  const [signed,setSigned] = useState(false), [items,setItems] = useState<Item[]>([]), [term,setTerm] = useState('');
  const [status,setStatus] = useState('Connect to your server.'), [playing,setPlaying] = useState(false);
  const [connected,setConnected] = useState<{name:string;options:{allowSignups?:boolean; passkeys?:boolean; quickConnect?:boolean}} | null>(null);
  const [mode,setMode] = useState('');
  const [request,setRequest] = useState<any>(null);
  const [focused,setFocused] = useState(''), [selected,setSelected] = useState<Item | null>(null);
  const [queue,setQueue] = useState<Item[]>([]), [guide,setGuide] = useState<string[]>([]);
  const [category,setCategory]=useState('Home'),[utility,setUtility]=useState<string|null>(null);
  const [shelves,setShelves]=useState<{title:string;items:Item[]}[]>([]),[featured,setFeatured]=useState<Item|null>(null);
  const [sports,setSports]=useState<any[]>([]),[filters,setFilters]=useState<Filters>({scope:'all'});
  const [pages,setPages]=useState<SearchPage[]>([]),[facets,setFacets]=useState<any>(null),[filterPanel,setFilterPanel]=useState<string|null>(null);
  const [categoryPage,setCategoryPage]=useState<{type:string;next:number;total:number;parent?:string}|null>(null);
  const browseEpoch=useRef(0),browseController=useRef<AbortController|null>(null);
  const searchRequest=useRef<{term:string;filters:Filters}|null>(null);
  const queueRevision = useRef<number | null>(null);
  const selection = useRef<Item | null>(null); selection.current = selected;
  const accountScreen = useRef({mode, request, connected, signed});
  accountScreen.current = {mode, request, connected, signed};
  const running = useRef(false);
  async function stop() {
    ++epoch.current; running.current = false;
    const s = session.current, p = video.current; video.current = null;
    const a = adaptive.current; adaptive.current = null;
    // Start capability revocation before potentially slow native destruction.
    const release = s?.stop();
    setPlaying(false);
    await Promise.allSettled([release,(async () => {
      try { p?.pause(); await bounded(Promise.resolve(a?.destroy())); }
      finally { await bounded(Promise.resolve(p?.deinitialize())); }
    })()]);
  }
  function run(action: () => Promise<unknown>) {
    if (busy.current) return;
    busy.current = true;
    action().catch(async () => { await stop(); setStatus('This action could not complete. Check sign-in or the negotiated media format.'); }).finally(() => { busy.current = false; });
  }
  function browseRequest() {
    browseController.current?.abort();const controller=new AbortController();browseController.current=controller;
    const serial=++browseEpoch.current,client=session.current!;
    return {client,signal:controller.signal,current:()=>serial===browseEpoch.current&&session.current===client&&active.current&&!controller.signal.aborted};
  }
  function browseAction(action:()=>Promise<unknown>) {
    void action().catch((error:unknown)=>{if(active.current)setStatus(error instanceof Error?error.message:'This library request could not complete.');});
  }
  async function chooseCategory(next:string) {
    const request=browseRequest();setCategory(next);setUtility(null);setSelected(null);setItems([]);setPages([]);
    setShelves([]);setFeatured(null);setSports([]);setGuide([]);setCategoryPage(null);setFilterPanel(null);setStatus('Loading…');
    if(next==='Home'){
      const result=await request.client.home(request.signal);if(!request.current())return;
      setShelves(result.shelves);setFeatured(result.featured);setStatus(result.partial?'Some Home sections could not refresh.':'');
    }else if(next==='Sports'){
      const result=await request.client.sports(request.signal);if(!request.current())return;setSports(result.games);setItems(result.items);setStatus('');
    }else if(next==='Live TV'){
      const result=await request.client.guide(request.signal);if(!request.current())return;setItems(result.Items||[]);setStatus('Guide information from your server. Live playback uses the server’s existing admission rules.');
    }else{
      const type=next==='Movies'?'Movie':next==='TV Shows'?'Series':'Audio,MusicAlbum,MusicArtist,Playlist';
      const result=await request.client.library(type,'',0,request.signal);if(!request.current())return;
      const values:Item[]=result.Items||[];setItems(values);setCategoryPage({type,next:values.length,total:Number(result.TotalRecordCount)||values.length});setStatus('');
    }
  }
  async function moreCategory() {
    if(!categoryPage)return;const page=categoryPage,request=browseRequest();
    const result=page.parent?await request.client.children(page.parent,request.signal,page.type,page.next):await request.client.library(page.type,'',page.next,request.signal);if(!request.current())return;
    const found:Item[]=result.Items||[];setItems(previous=>mergeSearchPage({items:previous,next:page.next},result).items);
    const next=page.next+found.length;setCategoryPage({...page,next,total:found.length?Number(result.TotalRecordCount)||next:next});
  }
  async function openItem(item:Item) {
    const request=browseRequest();let key=item.Id;
    if(['Program','LiveTvProgram'].includes(item.Type)){if(!item.ChannelId)throw Error('This programme has no available channel.');key=item.ChannelId;}
    const detail=await request.client.item(key.replace(/-/g,''),request.signal);if(request.current())setSelected(detail);
  }
  async function openCollections(){
    const request=browseRequest();setUtility('collections');setSelected(null);setFilterPanel(null);setCategoryPage(null);setItems([]);setStatus('Loading collections…');
    const result=await request.client.collections(request.signal);if(request.current()){setItems(result.Items||[]);setStatus('');}
  }
  async function openSearch(){
    setFilterPanel(null);await runSearch({scope:'all'});
  }
  async function runSearch(nextFilters:Filters=filters) {
    const request=browseRequest(),query=term.trim();const selected={...nextFilters,after:new Date().toISOString()};
    searchRequest.current={term:query,filters:selected};setFilters(selected);setUtility('search');setSelected(null);setPages([]);setItems([]);setStatus('Searching…');
    if(!query&&selected.scope==='all'&&!selected.studioID&&!selected.tag&&!selected.collectionID){setStatus('Search your movies, shows, music, channels, and programmes.');return;}
    const lanes=searchLanes(selected),results=await Promise.allSettled(lanes.map((lane:string)=>request.client.search(query,selected,0,lane,request.signal)));
    if(!request.current())return;
    const next:SearchPage[]=[];results.forEach((result,index)=>{if(result.status==='fulfilled'){next.push(mergeSearchPage({lane:lanes[index],items:[],next:0},result.value));}});
    setPages(next);setStatus(results.some(result=>result.status==='rejected')?'Some search sources could not refresh.':'');
  }
  async function moreSearch(page:SearchPage){
    const query=searchRequest.current;if(!query)return;const request=browseRequest();
    const result=await request.client.search(query.term,query.filters,page.next,page.lane,request.signal);if(!request.current())return;
    setPages(previous=>previous.map(value=>value===page?mergeSearchPage(page,result):value));
  }
  async function openFilter(key:string){
    setFilterPanel(key);if(facets)return;const client=session.current!,result=await client.searchFacets();
    if(client===session.current&&active.current)setFacets(result);
  }
  async function connectServer() {
    const address = base.trim().includes('://') ? base.trim() : 'https://' + base.trim();
    const client = new Session(address, deviceId);
    const result = await client.connect();
    if (!active.current) return;
    session.current = client;
    setConnected(result); setMode(''); setStatus('');
  }
  async function signIn() {
    const client = session.current!;
    const secret = password; setPassword('');
    const create = mode === 'create';
    try {
      if (!await client.signIn(username.trim(), secret, create)) throw Error('TV lease support required.');
    } catch (error) {
      if (create) { setMode(''); setConnected(await client.connect()); }
      throw error;
    }
    await accountReady(client);
  }
  async function accountReady(client: Session) {
    if (!active.current) { await client.logout(); return; }
    setRequest(null); setSigned(true);setFacets(null);await chooseCategory('Home');
  }
  async function beginDevice(passkey: boolean) {
    const client = session.current!;
    const next = await client.beginDeviceSignIn(passkey);
    if (passkey) {
      const prefix = client.base + '/Bluefin/Accounts/Passkey/QR/';
      if (!next.qr_url?.startsWith(prefix) || !/^[a-f0-9]{32}$/.test(next.qr_url.slice(prefix.length))) {
        await client.cancelDeviceSignIn(next);
        throw Error('Invalid passkey scan link.');
      }
    }
    if (!active.current) { await client.cancelDeviceSignIn(next); return; }
    setRequest(next); setMode(passkey ? 'passkey' : 'code');
    setStatus(passkey ? 'Scan with your phone and confirm the code.' : 'Enter this code in a signed-in client.');
  }
  async function finishDevice() {
    const client = session.current!;
    const done = await client.finishDeviceSignIn(request, mode === 'passkey');
    if (done === null) { setStatus('Finish approving the request first.'); return; }
    if (!done) throw Error('TV lease support required.');
    await accountReady(client);
  }
  async function cancelAccount() {
    await session.current?.cancelDeviceSignIn(request);
    setRequest(null); setMode(''); setStatus('');
  }
  async function play(item: Item, startTicks = 0, tracks: {audio?:number;subtitle?:number} = {}) {
    await stop();
    if (!active.current) return;
    const kind = item.Type === 'Audio' ? 'audio' : 'video';
    const own = epoch.current, p = new VideoPlayer(); video.current = p;
    p.addEventListener('error', () => { if (video.current === p) { void stop(); setStatus('Native player rejected this negotiated stream.'); } });
    p.addEventListener('ended', () => { if (video.current === p) void stop(); });
    try {
      await bounded(p.initialize());
      await bounded(p.setMediaControlFocus(component));
      if (own !== epoch.current) { await p.deinitialize(); return; }
      const url = await session.current!.acquire(item.Id.replace(/-/g,''),kind,startTicks,tracks);
      if (own !== epoch.current || !active.current) { await session.current!.stop(); return; }
      setPlaying(true); running.current = true;
      if (kind === 'audio') { p.src = url; await bounded(p.play()); }
      else { const a = hlsPlayer(p,() => {void stop();setStatus('HLS playback failed.');}); adaptive.current = a; await bounded(a.load(url)); await bounded(p.play()); }
      if (own !== epoch.current) return;
      setStatus(item.Name);
    } catch (e) { await stop(); throw e; }
  }
  useEffect(() => {
    const lifecycle = AppState.addEventListener('change', state => {
      active.current = state === 'active';
      if (!active.current) { browseController.current?.abort();++browseEpoch.current;void session.current?.suspend(); void stop(); setPassword(''); }
    });
    const back = BackHandler.addEventListener('hardwareBackPress', () => {
      if (running.current) { void stop(); return true; }
      if (selection.current) { ++epoch.current; setSelected(null); return true; }
      const account = accountScreen.current;
      if (!account.signed && account.mode) {
        if (account.request) void session.current?.cancelDeviceSignIn(account.request).catch(() => {
          setStatus('Could not cancel sign-in. Try again when connected.');
        });
        setRequest(null); setPassword(''); setMode('');
        return true;
      }
      if (!account.signed && account.connected) {
        session.current = null; setConnected(null);
        return true;
      }
      return false;
    });
    const progress = setInterval(() => { const p = video.current; if (p && running.current) void session.current?.report(p.currentTime,p.paused); },10000);
    return () => { browseController.current?.abort();++browseEpoch.current;lifecycle.remove(); back.remove(); clearInterval(progress); void session.current?.suspend(); void stop(); };
  },[]);
  function control(label: string, action: () => Promise<unknown>, first = false, catalogue=false) {
    return <Pressable key={label} accessible accessibilityRole="button" accessibilityLabel={label} hasTVPreferredFocus={first}
      onFocus={() => setFocused(label)} onPress={() => catalogue?browseAction(action):run(action)}
      style={{padding:16,margin:6,borderWidth:3,borderColor:focused === label ? palette.accent : palette.line,backgroundColor:focused === label ? palette.hover : palette.panel}}>
      <Text style={{color:palette.text,fontSize:22}}>{label}</Text></Pressable>;
  }
  function card(item:Item) {
    const source=session.current?.artworkSource(item),progress=item.RunTimeTicks?Math.min(1,(item.UserData?.PlaybackPositionTicks||0)/item.RunTimeTicks):0;
    return <Pressable key={item.Id} accessible accessibilityRole="button" accessibilityLabel={item.Name}
      onFocus={()=>setFocused(item.Id)} onPress={()=>browseAction(()=>openItem(item))}
      style={{width:190,height:item.Type==='MusicAlbum'?190:285,margin:10,backgroundColor:palette.panel,borderRadius:14,overflow:'hidden',
       borderWidth:3,borderColor:focused===item.Id?palette.accent:palette.line}}>
      {source&&<Image source={source} style={{position:'absolute',width:'100%',height:'100%'}} resizeMode="cover"/>}
      <View style={{position:'absolute',bottom:0,left:0,right:0,padding:12,backgroundColor:'#061727dd'}}>
        <Text numberOfLines={3} style={{color:'#CCD0D3',fontSize:20,fontWeight:'600'}}>{item.Name}</Text>
        {progress>0&&<View style={{height:4,backgroundColor:'#637889',marginTop:8}}><View style={{height:4,width:(progress*100+'%') as any,backgroundColor:'#ADB5D1'}}/></View>}
      </View>
    </Pressable>;
  }
  function cards(values:Item[]){return <View style={{flexDirection:'row',flexWrap:'wrap'}}>{values.map(card)}</View>;}
  function filterChoices(){
    if(!filterPanel)return null;const key=filterPanel;
    const values:{id:string;name:string}[]=key==='tag'?(facets?.tags||[]).map((tag:string)=>({id:tag,name:tag})):(key==='studioID'?facets?.studios:facets?.collections)||[];
    const title=key==='tag'?'Tag':key==='studioID'?'Studio':'Collection';
    return <View><Text style={{color:palette.text,fontSize:28}}>{title}</Text>
      {control('All '+title.toLowerCase()+'s',async()=>{setFilterPanel(null);await runSearch({...filters,[key]:''});},true,true)}
      {values.map(value=>control(value.name,async()=>{setFilterPanel(null);await runSearch({...filters,[key]:value.id});},false,true))}
      {!facets&&<Text style={{color:palette.muted}}>Loading your server’s refinements…</Text>}
      {facets?.partial&&<Text style={{color:palette.muted}}>Some refinements could not refresh.</Text>}
      {control('Back to search',async()=>{setFilterPanel(null);},false,true)}
    </View>;
  }
  function seek(delta: number) { const p = video.current; if (p && Number.isFinite(p.duration)) p.currentTime = Math.max(0,Math.min(p.duration,p.currentTime+delta)); return Promise.resolve(); }
  const renderedPlayer = video.current;
  return <View style={{flex:1,backgroundColor:palette.background,padding:36}}>
    <Text style={{color:palette.accent,fontSize:36}}>Bluefin</Text>
    <Text accessibilityLiveRegion="polite" style={{color:palette.text,fontSize:22}}>{status}</Text>
    {!signed ? <View>
      {!connected ? <>
        <TextInput accessibilityLabel="HTTPS server URL" value={base} onChangeText={setBase} autoCapitalize="none" selectionColor={palette.accent} placeholderTextColor={palette.muted} style={{color:palette.text,backgroundColor:palette.panel,padding:16,borderWidth:1,borderColor:palette.line}} />
        {control('Continue',connectServer,true)}
      </> : <>
        <Text style={{color:palette.text,fontSize:26}}>{connected.name}</Text>
        {!mode ? <>
          {control('Sign in',async () => {setMode('password');},true)}
          {connected.options.passkeys && control('Sign in with a passkey',() => beginDevice(true))}
          {connected.options.quickConnect && control('Quick Connect',() => beginDevice(false))}
          {connected.options.allowSignups && control('Create account',async () => {setMode('create');})}
          {control('Change server',async () => {setConnected(null); setUsername('');})}
        </> : request ? <>
          <Text style={{color:palette.text,fontSize:32}}>{request.code ?? request.Code}</Text>
          {request.qr_url && <Image source={{uri:request.qr_url}} style={{width:208,height:208}} accessibilityLabel="Passkey QR code" />}
          {control('Finish sign-in',finishDevice)}
          {control('Cancel',cancelAccount)}
        </> : <>
          <TextInput accessibilityLabel="Username" value={username} onChangeText={setUsername} autoCapitalize="none" selectionColor={palette.accent} placeholderTextColor={palette.muted} style={{color:palette.text,backgroundColor:palette.panel,padding:16,borderWidth:1,borderColor:palette.line}} />
          <TextInput accessibilityLabel="Password" value={password} onChangeText={setPassword} secureTextEntry selectionColor={palette.accent} placeholderTextColor={palette.muted} style={{color:palette.text,backgroundColor:palette.panel,padding:16,borderWidth:1,borderColor:palette.line}} />
          {mode === 'create' && <Text style={{color:palette.text}}>Use at least 15 characters. Your administrator can grant library access.</Text>}
          {control(mode === 'create' ? 'Create account' : 'Sign in',signIn)}
          {control('Back to sign-in options',async () => {setMode('');setPassword('');})}
        </>}
      </>}
    </View> : <ScrollView>
      <View style={{flexDirection:'row',flexWrap:'wrap'}}>
        {categories.map((name:string)=>control(name,()=>chooseCategory(name),name===category,true))}
      </View>
      <View style={{flexDirection:'row',flexWrap:'wrap'}}>
        {control('Search',openSearch,false,true)}{control('Collections',openCollections,false,true)}
        {control('Account',async()=>{browseController.current?.abort();++browseEpoch.current;setUtility('account');setSelected(null);setFilterPanel(null);},false,true)}
      </View>
      {selected ? <View><Text style={{color:palette.text,fontSize:28}}>{selected.Name}</Text><Text style={{color:palette.text,fontSize:22}}>{selected.Overview}</Text>
        {['TvChannel','LiveTvChannel','Program','LiveTvProgram'].includes(selected.Type)?
          <Text style={{color:palette.muted}}>Live guide information. Use the server’s admitted live playback client to watch this channel.</Text>:
          control(selected.IsFolder||['Series','Season','MusicAlbum','MusicArtist','Playlist','BoxSet','CollectionFolder'].includes(selected.Type)?'Browse':'Play',async()=>{
           if(selected.IsFolder||['Series','Season','MusicAlbum','MusicArtist','Playlist','BoxSet','CollectionFolder'].includes(selected.Type)){
            const parent=selected.Id.replace(/-/g,''),kind=selected.Type,request=browseRequest(),result=await request.client.children(parent,request.signal,kind);
            if(request.current()){const values:Item[]=result.Items||[];setItems(values);setCategoryPage({type:kind,parent,next:values.length,total:Number(result.TotalRecordCount)||values.length});setSelected(null);setUtility('collections');}
           }else await play(selected);
          })}
        {(selected.MediaStreams||[]).filter(track=>track.Type==='Audio'||track.Type==='Subtitle').slice(0,32).map(track=>control('Source '+track.Type+': '+(track.DisplayTitle||track.Index),()=>play(selected,Math.round((video.current?.currentTime||0)*10000000),track.Type==='Audio'?{audio:track.Index}:{subtitle:track.Index})))}
        {selected.Type==='Audio'&&control('Add to TV queue',async()=>{setQueue(previous=>previous.length<200?[...previous,selected]:previous);})}
        {control('Back',async()=>{setSelected(null);},false,true)}
      </View>:utility==='account'?<View>
        <Text style={{color:palette.text,fontSize:28}}>Account</Text>
        {control('Load account queue',async()=>{const request=browseRequest(),result=await request.client.loadQueue();const songs:Item[]=[];
          for(const key of result.queue.itemIds){if(!request.current())return;songs.push(await request.client.item(key.replace(/-/g,''),request.signal));}
          if(request.current()){queueRevision.current=result.revision;setQueue(songs);}})}
        {control('Save account queue',async()=>{if(queueRevision.current===null)throw Error('Load queue first');const result=await session.current!.saveQueue(queue.map(item=>item.Id.replace(/-/g,'')),queueRevision.current);queueRevision.current=result.revision;setStatus('Account queue saved.');})}
        {control('Clear TV queue',async()=>{setQueue([]);})}
        {queue.map((song,index)=>control('Queue '+(index+1)+': '+song.Name,()=>play(song)))}
        {control('Sign out',async()=>{browseController.current?.abort();++browseEpoch.current;await stop();await session.current!.logout();
          setSigned(false);setConnected(null);setMode('');setItems([]);setSelected(null);setQueue([]);setGuide([]);setShelves([]);setFeatured(null);setSports([]);setPages([]);setFacets(null);setUtility(null);queueRevision.current=null;})}
      </View>:utility==='search'?<View>
        {filterPanel?filterChoices():<>
          <TextInput accessibilityLabel="Search all media" value={term} onChangeText={setTerm} maxLength={128}
            onSubmitEditing={()=>browseAction(()=>runSearch())} selectionColor={palette.accent} placeholderTextColor={palette.muted}
            style={{color:palette.text,backgroundColor:palette.panel,padding:16,borderWidth:1,borderColor:palette.line}}/>
          {control('Search all media',()=>runSearch(),false,true)}
          <View style={{flexDirection:'row',flexWrap:'wrap'}}>{searchScopes.map(([scope,title]:string[])=>control(title,()=>runSearch({...filters,scope}),false,true))}</View>
          <View style={{flexDirection:'row',flexWrap:'wrap'}}>
            {control('Studio'+(filters.studioID?' selected':''),()=>openFilter('studioID'),false,true)}
            {control('Tag'+(filters.tag?': '+filters.tag:''),()=>openFilter('tag'),false,true)}
            {control('Collection'+(filters.collectionID?' selected':''),()=>openFilter('collectionID'),false,true)}
            {(filters.studioID||filters.tag||filters.collectionID)&&control('Clear refinements',()=>runSearch({scope:filters.scope}),false,true)}
          </View>
          <Text style={{color:palette.muted,fontSize:20}}>Studios, tags, and collections come from your server. Its tags can identify services or custom groups. Refinements search your library’s metadata.</Text>
          {facets?.truncated&&<Text style={{color:palette.muted}}>Open Collections to browse groups beyond this menu.</Text>}
          {pages.map(page=><View key={page.lane}><Text style={{color:palette.text,fontSize:28}}>{{library:'Your library',artists:'Artists',programmes:'On now and upcoming',channels:'Channels'}[page.lane as 'library']}</Text>
            {cards(page.items)}{page.next<page.total&&control('More '+page.lane+' results',()=>moreSearch(page),false,true)}</View>)}
        </>}
      </View>:utility==='collections'?<View><Text style={{color:palette.text,fontSize:28}}>Collections</Text>{cards(items)}{categoryPage&&categoryPage.next<categoryPage.total&&control('More titles',moreCategory,false,true)}</View>:<>
        {category==='Home'?<View>
          {featured&&<View style={{height:380,borderRadius:20,overflow:'hidden',backgroundColor:palette.panel,marginVertical:20}}>
            {session.current?.artworkSource(featured,true)&&<Image source={session.current.artworkSource(featured,true)} resizeMode="cover" style={{position:'absolute',width:'100%',height:'100%'}}/>}
            <View style={{position:'absolute',bottom:0,left:0,right:0,padding:24,backgroundColor:'#061727cc'}}>
              <Text style={{color:'#CCD0D3',fontSize:18}}>FEATURED</Text><Text style={{color:'#CCD0D3',fontSize:40}}>{featured.Name}</Text>
              <View style={{flexDirection:'row',flexWrap:'wrap'}}>
                {['Movie','Episode','Audio'].includes(featured.Type)&&control(featured.UserData?.PlaybackPositionTicks?'Resume':'Play',()=>play(featured,featured.UserData?.PlaybackPositionTicks||0))}
                {control('Details',()=>openItem(featured),false,true)}
              </View>
            </View>
          </View>}
          {shelves.map((shelf,index)=><View key={shelf.title+index}><Text style={{color:palette.text,fontSize:28,marginTop:20}}>{shelf.title}</Text><ScrollView horizontal>{shelf.items.map(card)}</ScrollView></View>)}
          {!shelves.length&&<Text style={{color:palette.muted}}>Open Collections to browse the titles available to your account.</Text>}
        </View>:<View>
          <Text style={{color:palette.text,fontSize:28}}>{category}</Text>
          {category==='Sports'&&sports.map((game,index)=><View key={index} style={{padding:20,marginVertical:10,backgroundColor:palette.panel,borderRadius:14}}>
            <Text style={{color:palette.text,fontSize:26}}>{game.event?.name||game.title||game.native_title||'Sports broadcast'}</Text>
            {(game.event?.competitors||[]).map((team:any,index:number)=><Text key={index} style={{color:palette.text,fontSize:22}}>{team.name||team.abbreviation} {team.score}</Text>)}
            <Text style={{color:palette.muted,fontSize:20}}>{game.event?.detail}</Text>
          </View>)}
          {cards(items)}{categoryPage&&categoryPage.next<categoryPage.total&&control('More titles',moreCategory,false,true)}
        </View>}
      </>}
    </ScrollView>}

    {playing && renderedPlayer ? <View style={{height:360}}><KeplerVideoSurfaceView style={{flex:1}} onSurfaceViewCreated={handle => renderedPlayer.setSurfaceHandle(handle)} onSurfaceViewDestroyed={handle => renderedPlayer.clearSurfaceHandle(handle)} />
      {control('Play / pause',async () => { const p = video.current; if (p) { if (p.paused) await p.play(); else p.pause(); } })}
      {control('Back 10 seconds',() => seek(-10))}{control('Forward 30 seconds',() => seek(30))}{control('Stop',stop)}
    </View> : null}
  </View>;
}
