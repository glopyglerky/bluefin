// Security contract tests. Actual decoder acceptance uses generated media.
const {test}=require('node:test');const assert=require('node:assert/strict');const vm=require('node:vm');const fs=require('node:fs');const {webcrypto}=require('node:crypto');
function setup(){
 const events={},requests=[];const scope={location:new URL('https://client.example/media-worker.js'),clients:{claim:async()=>{},get:async id=>({id,url:'https://client.example/'})},skipWaiting:()=>{},addEventListener:(kind,fn)=>events[kind]=fn};
 const context=vm.createContext({self:scope,URL,Headers,Response,ReadableStream,TextDecoder,Uint8Array,AbortController,crypto:webcrypto,setTimeout,clearTimeout,fetch:async(url,options)=>{requests.push({url,options});return new Response('original synthetic bytes',{headers:{'Content-Type':'video/mp4'}});}});
 vm.runInContext(fs.readFileSync(require('node:path').join(__dirname,'../media-worker.js'),'utf8'),context);
 const message=async(id,data)=>{let promise,result;events.message({source:{id},data,ports:[{postMessage:r=>result=r}],waitUntil:p=>promise=p});await promise;return result;};
 const get=async(id,path)=>{let result;events.fetch({clientId:id,request:new Request('https://client.example'+path),respondWith:p=>result=p});return result;};
 return {message,get,context,requests};
}
const plan={prefix:'/root/Videos/abc/',url:'https://server.example/root/Videos/abc/stream?ApiKey=discard'};
test('client binding, standard header, query stripping and acknowledged retirement',async()=>{
 const s=setup();const ack=await s.message('a',{op:'start',base:'https://server.example/root/',authorization:'MediaBrowser Client="Synthetic", Device="Browser", DeviceId="original", Version="1", Token="synthetic"',plan});assert.equal(ack.ok,true);
 assert.equal((await s.get('b',ack.route)).status,403);
 const response=await s.get('a',ack.route);assert.equal(await response.text(),'original synthetic bytes');
 assert.equal(s.requests.length,1);const {url,options}=s.requests[0];assert.equal(url,'https://server.example/root/Videos/abc/stream');assert(options.headers.has('Authorization'));assert(!options.headers.has('X-Emby-Token'));assert.equal(options.credentials,'omit');assert.equal(options.redirect,'error');
 const clear=await s.message('a',{op:'clear'});assert.equal(clear.cleared,true);assert.equal((await s.get('a',ack.route)).status,403);assert.equal(vm.runInContext('sessions.size',s.context),0);
});
test('replacement retires old capability and rejects origin/path/credential injection',async()=>{
 const s=setup(),start={op:'start',base:'https://server.example/root/',authorization:'MediaBrowser Client="Synthetic", Token="synthetic"',plan};
 const old=await s.message('a',start);const next=await s.message('a',start);assert.equal((await s.get('a',old.route)).status,403);assert.equal((await s.get('a',next.route+'?api_key=evil')).status,403);
 for(const url of ['https://evil.example/root/Videos/abc/stream','https://server.example/root/Users/private','https://server.example/root/Videos/abc/%2fprivate']){const result=await s.message('a',{...start,plan:{...plan,url}});assert.equal(result.ok,false);}
 const invalid=await s.message('a',{...start,authorization:'MediaBrowser Client="x"\r\nCookie: bad'});assert.equal(invalid.ok,false);await s.message('a',{op:'clear'});
});
test('HLS nested resources, keys and variants stay pinned and opaque',async()=>{
 const s=setup(),ack=await s.message('a',{op:'start',base:'https://server.example/root/',authorization:'MediaBrowser Client="Synthetic"',plan});assert(ack.ok);
 const text=vm.runInContext('rewrite(sessions.get("a"),"#EXTM3U\\n#EXT-X-KEY:METHOD=AES-128,URI=\\\"key.bin?api_key=discard\\\"\\nsegment.ts\\n","https://server.example/root/Videos/abc/master.m3u8")',s.context);
 assert(!text.includes('server.example'));assert(!text.includes('api_key'));assert(text.includes('URI="/bluefin-media/'));
 assert.throws(()=>vm.runInContext('rewrite(sessions.get("a"),"#EXTM3U\\nhttps://evil.example/a.ts","https://server.example/root/Videos/abc/master.m3u8")',s.context));await s.message('a',{op:'clear'});
});
test('sliding live resources stay bounded without a total-session resource deadline',async()=>{
 const s=setup(),ack=await s.message('a',{op:'start',base:'https://server.example/root/',authorization:'MediaBrowser Client="Synthetic"',plan});assert(ack.ok);
 const count=vm.runInContext('(()=>{const s=sessions.get("a");for(let n=0;n<5000;n++)resource(s,`segment${n}.ts`,"https://server.example/root/Videos/abc/master.m3u8");return s.resources.size;})()',s.context);
 assert.equal(count,2048);assert.equal((await s.get('a',ack.route)).status,200);await s.message('a',{op:'clear'});
});
