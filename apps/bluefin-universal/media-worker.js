// Ephemeral media authority is bound to the sending WindowClient. Account
// credentials stay in worker memory and leave only in Authorization headers.
const sessions = new Map();
const operations = new Map(), pending = new Map();
const idleMs = 10000, manifestLimit = 1024 * 1024, resourceLimit = 2048;
const credential = key => /^(api_?key|token|access_token|authorization|x-emby-token|x-emby-authorization)$/i.test(key);
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
function pinned(session, raw, base) {
  if (typeof raw !== 'string' || raw.length > 16384 || /[\x00-\x1f\x7f\\]/.test(raw)) throw Error('Invalid media URI');
  const url = new URL(raw, base);
  if (url.origin !== session.origin || !url.pathname.toLowerCase().startsWith(session.prefix.toLowerCase()) || url.username || url.password || url.hash || url.pathname.includes('%')) throw Error('Media URI escaped item scope');
  for (const key of [...url.searchParams.keys()]) if (credential(key)) url.searchParams.delete(key);
  return url.href;
}
function resource(session, raw, base) {
  const upstream = pinned(session, raw, base);
  for (const [key, value] of session.resources) if (value === upstream) {if(key!==session.root){session.resources.delete(key);session.resources.set(key,value);}return key;}
  if (session.resources.size >= resourceLimit) {const old=[...session.resources.keys()].find(key=>key!==session.root);session.resources.delete(old);}
  const route = `${session.route}/${session.next++}`;
  session.resources.set(route, upstream);
  return route;
}
function rewrite(session, text, base) {
  if (!text.startsWith('#EXTM3U')) throw Error('Invalid HLS playlist');
  return text.split(/\r?\n/).map(line => line.startsWith('#') ? line.replace(/URI="([^"]*)"/g, (_, uri) => `URI="${resource(session, uri, base)}"`) : line.trim() ? resource(session, line.trim(), base) : '').join('\n');
}
async function retire(id) {
  const session = sessions.get(id);
  sessions.delete(id);
  if (!session) return;
  clearTimeout(session.expiry);
  const closing = [];
  for (const [controller, active] of session.active) {
    controller.abort();
    if (active.reader) closing.push(active.reader.cancel().catch(() => {}));
  }
  await Promise.all(closing);
  session.active.clear(); session.resources.clear(); session.auth = '';
}
function lease(id, session) {
  clearTimeout(session.expiry);
  session.expiry = setTimeout(() => { retire(id); }, 60000);
}
self.addEventListener('message', event => {
  const clientId=event.source?.id;
  if(!clientId)return;
  if((pending.get(clientId)||0)>=8){event.ports?.[0]?.postMessage({ok:false,error:'Media operation limit'});return;}
  pending.set(clientId,(pending.get(clientId)||0)+1);
  const task=(operations.get(clientId)||Promise.resolve()).catch(()=>{}).then(async () => {
    const id = event.source?.id, port = event.ports?.[0];
    if (!id || !port) return;
    try {
      const client = await self.clients.get(id);
      if (!client || new URL(client.url).origin !== self.location.origin) throw Error('Invalid client');
      const message = event.data;
      if (message?.op === 'clear') { await retire(id); port.postMessage({ok:true, cleared:true}); return; }
      if (message?.op === 'renew') {
        const session = sessions.get(id);
        if (!session || session.generation !== message.generation) throw Error('Retired session');
        lease(id, session); port.postMessage({ok:true}); return;
      }
      await retire(id);
      if (message?.op !== 'start' || typeof message.authorization !== 'string' || message.authorization.length > 8192 || /[\x00-\x1f\x7f]/.test(message.authorization) || !message.authorization.startsWith('MediaBrowser Client=')) throw Error('Invalid authorization');
      const base = new URL(message.base), plan = message.plan;
      const localFixture = message.fixture === true && base.origin === self.location.origin && ['localhost','127.0.0.1','[::1]'].includes(base.hostname) && base.pathname.endsWith('/fixture/');
      if ((base.protocol !== 'https:' && !localFixture) || base.username || base.password || base.search || base.hash || typeof plan.prefix !== 'string' || !plan.prefix.startsWith(base.pathname) || !/^\/[A-Za-z0-9_./-]+\/$/.test(plan.prefix)) throw Error('Invalid server scope');
      const generation = crypto.randomUUID().replaceAll('-', '');
      const scope = new URL('./bluefin-media/', self.location.href).pathname;
      const session = {origin:base.origin, prefix:plan.prefix, auth:message.authorization, resources:new Map(), active:new Map(), next:0, generation, route:`${scope}${generation}`};
      const route = resource(session, plan.url, base);
      session.root=route;
      if(sessions.size>=64)await retire(sessions.keys().next().value);
      sessions.set(id,session); lease(id,session);
      port.postMessage({ok:true, generation, route});
    } catch (_) { port.postMessage({ok:false, error:'Media bridge rejected or could not clear the session'}); }
  });
  operations.set(clientId,task);
  event.waitUntil(task.finally(()=>{
    const count=(pending.get(clientId)||1)-1;if(count)pending.set(clientId,count);else pending.delete(clientId);
    if(operations.get(clientId)===task)operations.delete(clientId);
  }));
});
async function boundedRead(reader, controller) {
  let timer;
  try { return await Promise.race([reader.read(), new Promise((_,reject) => { timer=setTimeout(() => {controller.abort(); reject(Error('Media read idle'));},idleMs); })]); }
  finally {clearTimeout(timer);}
}
async function media(event, session, upstream) {
  const controller = new AbortController(), active = {reader:null};
  session.active.set(controller, active);
  const abort = () => controller.abort();
  event.request.signal.addEventListener('abort',abort,{once:true});
  const finish = () => { session.active.delete(controller);event.request.signal.removeEventListener('abort',abort); };
  try {
    const headers = new Headers({Authorization:session.auth});
    const range=event.request.headers.get('Range');if(range){if(range.length>128)throw Error('Invalid range');headers.set('Range',range);}
    const deadline=setTimeout(abort,idleMs);
    let response;
    try {response=await fetch(upstream,{method:event.request.method,headers,redirect:'error',credentials:'omit',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});}
    finally {clearTimeout(deadline);}
    const safe=new Headers({'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'});
    for(const key of ['Content-Type','Content-Length','Content-Range','Accept-Ranges'])if(response.headers.has(key))safe.set(key,response.headers.get(key));
    if(!response.ok || !response.body || event.request.method==='HEAD'){finish();return new Response(null,{status:response.status,headers:safe});}
    active.reader=response.body.getReader();
    if(new URL(upstream).pathname.endsWith('.m3u8') || /mpegurl/i.test(safe.get('Content-Type')||'')) {
      let total=0;const chunks=[];
      for(;;){const {done,value}=await boundedRead(active.reader,controller);if(done)break;total+=value.length;if(total>manifestLimit)throw Error('Playlist limit');chunks.push(value);}
      const bytes=new Uint8Array(total);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}
      const text=rewrite(session,new TextDecoder('utf-8',{fatal:true}).decode(bytes),upstream);
      finish();safe.delete('Content-Length');safe.set('Content-Type','application/vnd.apple.mpegurl');return new Response(text,{status:response.status,headers:safe});
    }
    // One fetch-reader chunk is pulled at a time. Browser/media backpressure
    // bounds memory; no full movie Blob or arrayBuffer is created.
    const body=new ReadableStream({
      async pull(output){try{const {done,value}=await boundedRead(active.reader,controller);if(done){finish();output.close();}else{output.enqueue(value);}}catch(_){finish();controller.abort();output.error(Error('Media stream interrupted or idle'));}},
      async cancel(){controller.abort();await active.reader.cancel().catch(()=>{});finish();}
    },{highWaterMark:0});
    return new Response(body,{status:response.status,headers:safe});
  } catch (_) {controller.abort();await active.reader?.cancel().catch(()=>{});finish();return new Response(null,{status:502,headers:{'Cache-Control':'no-store'}});}
}
self.addEventListener('fetch', event => {
  const url=new URL(event.request.url),scope=new URL('./bluefin-media/',self.location.href).pathname;
  if(url.origin!==self.location.origin || !url.pathname.startsWith(scope))return;
  event.respondWith((async()=>{
    const session=sessions.get(event.clientId),upstream=session?.resources.get(url.pathname);
    if(!session || !upstream || url.search || !['GET','HEAD'].includes(event.request.method) || session.active.size>=4)return new Response(null,{status:403,headers:{'Cache-Control':'no-store'}});
    return media(event,session,upstream);
  })());
});
