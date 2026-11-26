// Runs the actual platform binding with controlled worker acknowledgements.
// DOM/worker stubs observe admission and retirement, not browser decoding.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');

function setup({holdReady=false,holdStart=false,foreignWorker=false}={}){
  const pending=[],messages=[],plays=[],listeners=new Set();
  const video={src:'',paused:true,currentTime:0,duration:60,
    pause(){this.paused=true;},removeAttribute(){this.src='';},load(){},
    querySelectorAll(){return [];},canPlayType(){return 'probably';},
    play(){plays.push(this.src);this.paused=false;return Promise.resolve();}};
  const label={textContent:''};
  let generation=0;
  const baseURI='https://server.example/mount/web/bluefin/';
  const worker={scriptURL:new URL('media-worker.js',baseURI).href,postMessage(message,[port]){
    messages.push(message.op);
    const reply=()=>port.postMessage({ok:true,generation:++generation,route:`/media/${generation}`,cleared:true});
    if(message.op==='start'&&holdStart){holdStart=false;pending.push(reply);}else reply();
  }};
  class Channel{
    constructor(){this.port1={close(){}};this.port2={postMessage:data=>queueMicrotask(()=>this.port1.onmessage?.({data})),close(){}};}
  }
  const foreignMessages=[];
  const serviceWorker={
    controller:foreignWorker?{scriptURL:'https://server.example/media-worker.js',
      postMessage:message=>foreignMessages.push(message)}:holdReady?null:worker,
    addEventListener(_event,listener){listeners.add(listener);},
    removeEventListener(_event,listener){listeners.delete(listener);},
  };
  const context=vm.createContext({document:{baseURI,querySelector:id=>id==='#media'?video:label},
    navigator:{serviceWorker},MessageChannel:Channel,URL,
    setTimeout:(fn,ms)=>setTimeout(fn,ms===5000?50:ms),clearTimeout,
    setInterval:()=>1,clearInterval:()=>{},addEventListener:()=>{},queueMicrotask});
  const source=fs.readFileSync(path.join(__dirname,'../web-player.js'),'utf8');
  vm.runInContext(source.replace(/\bexport /g,'')+'\nglobalThis.binding={startMedia,stopMedia,interruptMedia};',context);
  const start=()=>context.binding.startMedia('https://server.example/', 'MediaBrowser Client="Synthetic"',
    JSON.stringify({selection:{position:0},offset:0,hls:false}),false);
  const ready=()=>{serviceWorker.controller=worker;for(const listener of listeners)listener();};
  return {binding:context.binding,start,ready,pending,messages,plays,video,listeners,foreignMessages};
}
async function until(check){for(let n=0;n<100;n++){if(check())return;await new Promise(setImmediate);}assert.fail('controlled worker operation was not reached');}
function outcome(promise){return promise.then(()=>({ok:true}),error=>({ok:false,error}));}

test('interrupt during registration never admits delayed media',async()=>{
  const s=setup({holdStart:true}),result=outcome(s.start());
  await until(()=>s.pending.length===1);
  s.binding.interruptMedia();s.pending.shift()();
  const done=await result;await s.binding.stopMedia();
  assert.equal(s.plays.length,0);assert.equal(done.ok,false);assert.equal(s.video.src,'');
});
test('stop while waiting for worker activation prevents registration',async()=>{
  const s=setup({holdReady:true}),result=outcome(s.start());
  await until(()=>s.listeners.size===1);
  const stopped=s.binding.stopMedia();s.ready();
  const done=await result;await stopped;await s.binding.stopMedia();
  assert.equal(s.plays.length,0);assert.equal(done.ok,false);assert(!s.messages.includes('start'));
});
test('a different controller never receives authorization and activation times out',async()=>{
  const s=setup({foreignWorker:true});
  await assert.rejects(s.start(),/media worker did not activate/);
  assert.equal(s.listeners.size,0);
  assert.deepEqual(s.foreignMessages,[]);
  assert.equal(s.plays.length,0);
  s.ready();
  await s.start();
  assert.equal(s.plays.length,1);
  await s.binding.stopMedia();
});
test('allowed media starts and acknowledged stop clears local output',async()=>{
  const s=setup();await s.start();
  assert.equal(s.plays.length,1);assert(s.video.src.startsWith('/media/'));assert.equal(s.video.paused,false);
  await s.binding.stopMedia();assert.equal(s.video.src,'');assert.equal(s.video.paused,true);
});
test('replacement cannot publish an old registration or clear the new player',async()=>{
  const s=setup({holdStart:true}),old=outcome(s.start());
  await until(()=>s.pending.length===1);
  const next=outcome(s.start());s.pending.shift()();
  const oldResult=await old,nextResult=await next;
  const active=s.video.src;await s.binding.stopMedia();
  assert.equal(oldResult.ok,false);assert.equal(nextResult.ok,true);
  assert.deepEqual(s.plays,[active]);assert(active.startsWith('/media/'));
});
