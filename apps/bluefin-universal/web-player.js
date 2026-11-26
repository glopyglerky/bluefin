// Platform binding only. PlaybackInfo, reporting and queue policy live in Rust.
let current = null;
let revision = 0, transition = Promise.resolve();
const retirements = new WeakMap();
function sequential(operation) {
  const next=transition.then(operation,operation);
  transition=next.then(()=>{},()=>{});
  return next;
}
const media = () => document.querySelector('#media');
const status = text => {document.querySelector('#media-status').textContent=text;};
const owns = state => !!state && media().__bluefinOutputOwner === state.owner;
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
    try { worker.postMessage(message,[channel.port2]); }
    catch (error) {clearTimeout(timer);channel.port1.close();reject(error);}
  });
}
function stopLocal() {
  const old=current;current=null;
  if(!old)return null;
  const video=media(),owned=owns(old);let failed=false;
  clearInterval(old.renew);
  if(owned){
    // Source removal cannot depend on a decoder's destructor succeeding.
    try {video.pause();} catch (_) {failed=true;}
    try {video.removeAttribute('src');} catch (_) {failed=true;}
    try {video.load();} catch (_) {failed=true;}
    try {old.hls?.destroy();} catch (_) {failed=true;}
    for(const track of [...video.querySelectorAll('track')]){try {track.remove();} catch (_) {failed=true;}}
    if(video.__bluefinOutputOwner===old.owner)delete video.__bluefinOutputOwner;
  }else{
    // An unrelated local/HTTPS source owns this element. Never detach its
    // source buffers or sample its position on behalf of the previous owner.
    try {old.hls?.stopLoad();} catch (_) {failed=true;}
  }
  if(failed&&owned)status('Local decoder cleanup reported an error; connection retirement is continuing.');
  return old;
}
export function stopMedia(owner) {
  if(owner!==undefined&&owner!==null&&current?.owner!==owner){
    return retirements.get(owner)||Promise.reject(Error('The media retirement owner is unknown.'));
  }
  const mine=++revision,old=stopLocal(),expectedOwner=media().__bluefinOutputOwner;
  const result=sequential(async()=>{
    await workerMessage({op:'clear'});
    if(mine===revision&&expectedOwner===undefined&&media().__bluefinOutputOwner===undefined)
      status('Playback stopped; media-worker cleanup acknowledged');
  });
  if(old)retirements.set(old.owner,result);
  return result;
}
export function startMedia(base,authorization,serialized,fixture,meshHandle) {
  const retirement=stopMedia(),mine=revision,expectedOwner=media().__bluefinOutputOwner;
  return sequential(async()=>{
    const active=()=>{if(mine!==revision||media().__bluefinOutputOwner!==expectedOwner)throw Error('Media preparation was canceled');};
    await retirement;
    let registration=false,state=null;
    try{
      active();
      const plan=JSON.parse(serialized);
      registration=true;
      const ack=await workerMessage({op:'start',base,authorization,plan,fixture,meshHandle});
      active();
      state={owner:Object.freeze({revision:mine}),meshHandle,generation:ack.generation,loaded:false,eof:false,failed:false,error:null,hls:null,renew:null};
      const video=media();current=state;video.__bluefinOutputOwner=state.owner;
      video.onloadedmetadata=()=>{if(current!==state||!owns(state))return;state.loaded=true;const start=Math.max(0,plan.selection.position-plan.offset);if(start&&Number.isFinite(video.duration))video.currentTime=start;};
      video.onended=()=>{if(current===state&&owns(state))state.eof=true;};
      video.onerror=()=>{if(current===state&&owns(state)){state.failed=true;state.error='Media decoder failed or account access was denied';status(state.error);}};
      state.renew=setInterval(()=>{workerMessage({op:'renew',generation:state.generation}).catch(()=>{
        if(current!==state||!owns(state))return;state.failed=true;state.error='Media bridge lease expired';video.pause();
      });},15000);
      if(plan.hls&&!video.canPlayType('application/vnd.apple.mpegurl')){
        if(!globalThis.Hls?.isSupported())throw Error('This browser needs native HLS or MediaSource support');
        state.hls=new Hls({enableWorker:false,maxBufferLength:20,maxMaxBufferLength:40,backBufferLength:10,maxBufferSize:32*1024*1024,liveSyncDurationCount:3,liveMaxLatencyDurationCount:8,manifestLoadingTimeOut:10000,fragLoadingTimeOut:10000,fragLoadingMaxRetry:1,manifestLoadingMaxRetry:1});
        state.hls.on(Hls.Events.ERROR,(_,data)=>{if(data.fatal&&current===state&&owns(state)){state.failed=true;state.error='HLS playback failed';status(state.error);}});
        state.hls.loadSource(ack.route);state.hls.attachMedia(video);
      }else{video.src=ack.route;video.load();}
      status('Prepared. Press Play if your browser blocks autoplay.');
      video.play().catch(()=>{if(current===state&&owns(state))status('Prepared. Press Play to begin.');});
      return state.owner;
    }catch(error){
      if(state&&current===state)stopLocal();
      // This operation still owns the serial lane, so clear the registration
      // before another source can be admitted. Never await a task queued behind it.
      if(registration){
        const cleared=workerMessage({op:'clear'});if(state)retirements.set(state.owner,cleared);
        try{await cleared;error.retired=true;}catch(_){/* Keep retirement unknown. */}
      }else error.retired=true; // The initial retirement already acknowledged.
      throw error;
    }
  });
}
export function sampleMedia(owner) {
  const s=current;
  if(!s||s.owner!==owner||!owns(s))return JSON.stringify({position:0,duration:0,paused:true,loaded:false,eof:false,failed:false,error:null});
  const video=media();
  return JSON.stringify({position:video.currentTime||0,duration:Number.isFinite(video.duration)?video.duration:0,paused:video.paused,loaded:s.loaded,eof:s.eof,failed:s.failed,error:s.error});
}
export function controlMedia(owner,kind,value) {
  if(current?.owner!==owner||!owns(current))throw Error('This media source no longer owns playback.');
  const video=media();
  if(kind==='pause'){if(value)video.pause();else video.play().catch(()=>{if(current?.owner===owner)status('Press Play to resume');});}
  if(kind==='seek'){if(!Number.isFinite(value)||value<0)throw Error('Invalid seek');video.currentTime=value;}
  if(kind==='volume')video.volume=Math.max(0,Math.min(1,value/100));
}
export function delay(ms){return new Promise(resolve=>setTimeout(resolve,ms));}
addEventListener('pagehide',()=>{stopMedia().catch(()=>{});});
export function interruptMedia() {
  const retirement=stopMedia(),mine=revision;
  retirement.catch(()=>{if(mine===revision&&media().__bluefinOutputOwner===undefined)status('Immediate media cancellation was not acknowledged; retirement is still pending');});
}
addEventListener('bluefin-mesh-retired',event=>{if(current?.meshHandle===event.detail)stopMedia(current.owner).catch(()=>status('Device sign-out cleanup is still pending'));});
