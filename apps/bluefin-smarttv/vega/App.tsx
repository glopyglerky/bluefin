import React, {useEffect, useRef, useState} from 'react';
import {AppState, BackHandler, Pressable, Image, ScrollView, Text, TextInput, View} from 'react-native';
import {VideoPlayer, KeplerVideoSurfaceView} from '@amazon-devices/react-native-w3cmedia';
import {useComponentInstance} from '@amazon-devices/react-native-kepler';
import {Session} from './session.mjs';
import {createAdaptive as hlsPlayer} from './HlsPlayer';

// Application lifecycle interface around upstream Apache-2.0 hls.js.
export interface AdaptivePlayer {
  load(url: string): Promise<void>;
  destroy(): Promise<void>;
}
type Props = {deviceId: string};
type Item = {Id: string; Name: string; Type: string; IsFolder?: boolean; Overview?: string; MediaStreams?: {Type:string;Index:number;DisplayTitle?:string}[]};
async function bounded<T>(operation: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try { return await Promise.race([operation,new Promise<never>((_,reject) => {timer = setTimeout(() => reject(Error('Native player timeout.')),15000);})]); }
  finally { if (timer) clearTimeout(timer); }
}

export function App({deviceId}: Props) {
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
  async function list(kind = 'Movie,Series,Episode', search = '') {
    const e = epoch.current, result = await session.current!.library(kind,search);
    if (e === epoch.current && active.current) { setItems(result.Items || []); setSelected(null); }
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
    setRequest(null); setSigned(true); await list(); setStatus('');
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
      if (!active.current) { void session.current?.suspend(); void stop(); setPassword(''); }
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
    return () => { lifecycle.remove(); back.remove(); clearInterval(progress); void session.current?.suspend(); void stop(); };
  },[]);
  function control(label: string, action: () => Promise<unknown>, first = false) {
    return <Pressable key={label} accessible accessibilityRole="button" accessibilityLabel={label} hasTVPreferredFocus={first}
      onFocus={() => setFocused(label)} onPress={() => run(action)}
      style={{padding:16,margin:6,borderWidth:3,borderColor:focused === label ? '#65e2d7' : '#35444e'}}>
      <Text style={{color:'white',fontSize:22}}>{label}</Text></Pressable>;
  }
  function seek(delta: number) { const p = video.current; if (p && Number.isFinite(p.duration)) p.currentTime = Math.max(0,Math.min(p.duration,p.currentTime+delta)); return Promise.resolve(); }
  const renderedPlayer = video.current;
  return <View style={{flex:1,backgroundColor:'#0a171f',padding:36}}>
    <Text style={{color:'#65e2d7',fontSize:36}}>BLUEFIN TV · Vega candidate</Text>
    <Text accessibilityLiveRegion="polite" style={{color:'white',fontSize:22}}>{status}</Text>
    {!signed ? <View>
      {!connected ? <>
        <TextInput accessibilityLabel="HTTPS server URL" value={base} onChangeText={setBase} autoCapitalize="none" style={{color:'white',padding:16}} />
        {control('Continue',connectServer,true)}
      </> : <>
        <Text style={{color:'white',fontSize:26}}>{connected.name}</Text>
        {!mode ? <>
          {control('Sign in',async () => {setMode('password');},true)}
          {connected.options.passkeys && control('Sign in with a passkey',() => beginDevice(true))}
          {connected.options.quickConnect && control('Quick Connect',() => beginDevice(false))}
          {connected.options.allowSignups && control('Create account',async () => {setMode('create');})}
          {control('Change server',async () => {setConnected(null); setUsername('');})}
        </> : request ? <>
          <Text style={{color:'white',fontSize:32}}>{request.code ?? request.Code}</Text>
          {request.qr_url && <Image source={{uri:request.qr_url}} style={{width:208,height:208}} accessibilityLabel="Passkey QR code" />}
          {control('Finish sign-in',finishDevice)}
          {control('Cancel',cancelAccount)}
        </> : <>
          <TextInput accessibilityLabel="Username" value={username} onChangeText={setUsername} autoCapitalize="none" style={{color:'white',padding:16}} />
          <TextInput accessibilityLabel="Password" value={password} onChangeText={setPassword} secureTextEntry style={{color:'white',padding:16}} />
          {mode === 'create' && <Text style={{color:'white'}}>Use at least 15 characters. Your administrator can grant library access.</Text>}
          {control(mode === 'create' ? 'Create account' : 'Sign in',signIn)}
          {control('Back to sign-in options',async () => {setMode('');setPassword('');})}
        </>}
      </>}
    </View> : <ScrollView>
      {control('Library',() => list(),true)}{control('Music',() => list('Audio,MusicAlbum'))}
      {control('On now',async () => {const own=epoch.current,r = await session.current!.guide();if(own !== epoch.current || !active.current)return;setGuide((r.Items || []).slice(0,200).map((p: any) => `${p.ChannelName || 'Channel'} · ${p.Name || 'Untitled'}`));setStatus('Guide only. Live sources require separate server admission.');})}
      {guide.map((entry,index) => <Text key={index} style={{color:'white'}}>{entry}</Text>)}
      {control('Load account queue',async () => {const own=epoch.current,r = await session.current!.loadQueue();const songs: Item[]=[];for(const id of r.queue.itemIds) {if(own !== epoch.current || !active.current)return;songs.push(await session.current!.item(id.replace(/-/g,'')));}if(own === epoch.current && active.current){queueRevision.current=r.revision;setQueue(songs);}})}
      {control('Save account queue',async () => {if(queueRevision.current === null) throw Error('Load queue first');const r=await session.current!.saveQueue(queue.map(i => i.Id.replace(/-/g,'')),queueRevision.current);queueRevision.current=r.revision;setStatus('Account queue saved.');})}
      {control('Clear TV queue',async () => {setQueue([]);})}
      {queue.map((song,index) => control(`Queue ${index+1}: ${song.Name}`,() => play(song)))}
      <TextInput accessibilityLabel="Search library" value={term} onChangeText={setTerm} maxLength={128} style={{color:'white',padding:16}} />
      {control('Search',() => list('Movie,Series,Episode,Audio,MusicAlbum',term))}
      {control('Sign out',async () => { await stop(); await session.current!.logout(); setSigned(false); setConnected(null); setMode(''); setItems([]); setSelected(null);setQueue([]);setGuide([]);queueRevision.current=null; })}
      {selected ? <View><Text style={{color:'white',fontSize:28}}>{selected.Name}</Text><Text style={{color:'white'}}>{selected.Overview}</Text>
        {control(selected.IsFolder ? 'Browse' : 'Play',async () => { if (selected.IsFolder) { const r = await session.current!.children(selected.Id.replace(/-/g,'')); setItems(r.Items || []); setSelected(null); } else await play(selected); })}
        {(selected.MediaStreams || []).filter(t => t.Type === 'Audio' || t.Type === 'Subtitle').slice(0,32).map(t => control(`Source ${t.Type}: ${t.DisplayTitle || t.Index}`,() => play(selected,Math.round((video.current?.currentTime || 0)*10000000),t.Type === 'Audio' ? {audio:t.Index} : {subtitle:t.Index})))}
        {control('Back to library',async () => {setSelected(null);})}</View> : items.slice(0,40).map(item => control(item.Name,async () => {const own=epoch.current,detail=await session.current!.item(item.Id.replace(/-/g,''));if(own === epoch.current && active.current)setSelected(detail);}))}
      {selected?.Type === 'Audio' ? control('Add to TV queue',async () => {setQueue(previous => previous.length < 200 ? [...previous,selected] : previous);}) : null}
    </ScrollView>}
    {playing && renderedPlayer ? <View style={{height:360}}><KeplerVideoSurfaceView style={{flex:1}} onSurfaceViewCreated={handle => renderedPlayer.setSurfaceHandle(handle)} onSurfaceViewDestroyed={handle => renderedPlayer.clearSurfaceHandle(handle)} />
      {control('Play / pause',async () => { const p = video.current; if (p) { if (p.paused) await p.play(); else p.pause(); } })}
      {control('Back 10 seconds',() => seek(-10))}{control('Forward 30 seconds',() => seek(30))}{control('Stop',stop)}
    </View> : null}
  </View>;
}
