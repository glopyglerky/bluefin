// Platform binding only. PlaybackInfo, reporting and queue policy live in Rust.
let current = null;
let revision = 0, transition = Promise.resolve();
function sequential(operation) {
  const next=transition.then(operation,operation);
  transition=next.then(()=>{},()=>{});
  return next;
}
const media = () => document.querySelector('#media');
const status = text => {document.querySelector('#media-status').textContent=text;};
async function mediaWorker() {
  const container = navigator.serviceWorker;
  if (!container) throw Error('Playback requires a secure connection with service worker support');
  const expected = new URL('./media-worker.js', document.baseURI).href;
  const currentWorker = () => {
    const worker = container.controller;
    return worker?.scriptURL === expected ? worker : null;
  };
  const active = currentWorker();
  if (active) return active;
  return new Promise((resolve, reject) => {
    const onChange = () => {
      const worker = currentWorker();
      if (!worker) return;
      clearTimeout(timer);
      container.removeEventListener('controllerchange', onChange);
      resolve(worker);
    };
    const timer = setTimeout(() => {
      container.removeEventListener('controllerchange', onChange);
      reject(Error('The media worker did not activate. Reload the page and try again.'));
    }, 5000);
    container.addEventListener('controllerchange', onChange);
    onChange();
  });
}
async function workerMessage(message) {
  const worker = await mediaWorker();
  return new Promise((resolve,reject)=>{
    const channel=new MessageChannel(),timer=setTimeout(()=>{channel.port1.close();reject(Error('Media cleanup/registration acknowledgement timed out'));},12000);
    channel.port1.onmessage=event=>{clearTimeout(timer);channel.port1.close();event.data.ok?resolve(event.data):reject(Error(event.data.error));};
    try {
      worker.postMessage(message,[channel.port2]);
    } catch (error) {
      clearTimeout(timer);
      channel.port1.close();
      reject(error);
    }
  });
}
function stopLocal() {
  const old=current;current=null;
  if(old){clearInterval(old.renew);old.hls?.destroy();}
  const video=media();video.pause();video.removeAttribute('src');video.load();
  for(const track of [...video.querySelectorAll('track')])track.remove();
}
export function stopMedia() {
  const mine=++revision;
  stopLocal();
  return sequential(async()=>{
    await workerMessage({op:'clear'});
    if(mine===revision)status('Playback stopped; media-worker cleanup acknowledged');
  });
}
export function startMedia(base,authorization,serialized,fixture,meshHandle) {
  const mine=++revision;
  stopLocal();
  return sequential(async()=>{
  const active=()=>{if(mine!==revision)throw Error('Media preparation was canceled');};
  active();
  await workerMessage({op:'clear'});
  active();
  const plan=JSON.parse(serialized),ack=await workerMessage({op:'start',base,authorization,plan,fixture,meshHandle});
  if(mine!==revision){await workerMessage({op:'clear'});active();}
  const state={meshHandle,generation:ack.generation,loaded:false,eof:false,failed:false,error:null,hls:null,renew:null};current=state;
  const video=media();
  video.onloadedmetadata=()=>{if(current!==state)return;state.loaded=true;const start=Math.max(0,plan.selection.position-plan.offset);if(start && Number.isFinite(video.duration))video.currentTime=start;};
  video.onended=()=>{if(current===state)state.eof=true;};
  video.onerror=()=>{if(current===state){state.failed=true;state.error='Media decoder failed or account access was denied';status(state.error);}};
  state.renew=setInterval(()=>{workerMessage({op:'renew',generation:state.generation}).catch(()=>{if(current!==state)return;state.failed=true;state.error='Media bridge lease expired';video.pause();});},15000);
  if(plan.hls && !video.canPlayType('application/vnd.apple.mpegurl')) {
    if(!globalThis.Hls?.isSupported()) {stopLocal();await workerMessage({op:'clear'});throw Error('This browser needs native HLS or MediaSource support');}
    state.hls=new Hls({enableWorker:false,maxBufferLength:20,maxMaxBufferLength:40,backBufferLength:10,maxBufferSize:32*1024*1024,liveSyncDurationCount:3,liveMaxLatencyDurationCount:8,manifestLoadingTimeOut:10000,fragLoadingTimeOut:10000,fragLoadingMaxRetry:1,manifestLoadingMaxRetry:1});
    state.hls.on(Hls.Events.ERROR,(_,data)=>{if(data.fatal&&current===state){state.failed=true;state.error='HLS playback failed';status(state.error);}});
    state.hls.loadSource(ack.route);state.hls.attachMedia(video);
  }else{video.src=ack.route;video.load();}
  status('Prepared. Press Play if your browser blocks autoplay.');
  video.play().catch(()=>{if(current===state)status('Prepared. Press Play to begin.');});
  });
}
export function sampleMedia() {
  const video=media(),s=current;
  return JSON.stringify({position:video.currentTime||0,duration:Number.isFinite(video.duration)?video.duration:0,paused:video.paused,loaded:s?.loaded||false,eof:s?.eof||false,failed:s?.failed||false,error:s?.error||null});
}
export function controlMedia(kind,value) {
  const video=media();
  if(kind==='pause'){if(value)video.pause();else video.play().catch(()=>status('Press Play to resume'));}
  if(kind==='seek') {if(!Number.isFinite(value)||value<0)throw Error('Invalid seek');video.currentTime=value;}
  if(kind==='volume')video.volume=Math.max(0,Math.min(1,value/100));
}
export function delay(ms){return new Promise(resolve=>setTimeout(resolve,ms));}
addEventListener('pagehide',()=>{stopMedia().catch(()=>{});});

export function interruptMedia() {
  const retirement=stopMedia(),mine=revision;
  retirement.catch(()=>{if(mine===revision)status('Immediate media cancellation was not acknowledged; retirement is still pending');});
  // Local output stops synchronously. The lifecycle's awaited stopMedia owns
  // the cleanup acknowledgement and account revocation result.
}

addEventListener('bluefin-mesh-retired',event=>{if(current?.meshHandle===event.detail)stopMedia().catch(()=>status('Device sign-out cleanup is still pending'));});
