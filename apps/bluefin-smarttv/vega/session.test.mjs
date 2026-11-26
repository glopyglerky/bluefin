import {test} from 'node:test';
import assert from 'node:assert/strict';
import {Session,mergeSearchPage} from './session.mjs';
const id = '1'.repeat(32), g = 'a'.repeat(64);
const lease = {version:1,mediaUrl:`/Bluefin/TV/v1/media/${g}/0`,releaseUrl:`/Bluefin/TV/v1/leases/${g}`,playSessionId:'synthetic-session'};
const response = (body, status=200) => new Response(status===204 ? null : JSON.stringify(body),{status});
function fixture(overrides = {}) {
  const calls = [];
  const s = new Session('https://fixture.invalid','synthetic_device_0001', async (url,options) => {
    const path = new URL(url).pathname; calls.push({path,url,...options});
    if (overrides[path]) return overrides[path](options);
    if (path === '/Users/AuthenticateByName') return response({AccessToken:'synthetic_token',User:{Id:id}});
    if (path === '/Bluefin/TV/v1') return response({version:1,mediaLease:true});
    if (path === '/Bluefin/TV/v1/leases') return response(lease,201);
    if (options.method === 'DELETE') return response(null,204);
    return response({Items:[{Id:id,Name:'Original synthetic fixture',Type:'Audio'}]});
  });
  return {s,calls};
}
test('ordinary session uses header token; player receives opaque route; stop releases exact grant',async () => {
  const {s,calls} = fixture(); assert.equal(await s.signIn('viewer','synthetic password'),true);
  await s.library(); assert.equal(await s.acquire(id,'audio'),`https://fixture.invalid${lease.mediaUrl}`);
  await s.report(12,true); await s.stop(); await s.stop();
  assert.equal(calls.filter(c => c.method==='DELETE').length,1);
  assert.match(calls[2].headers.Authorization,/Token="synthetic_token"/);
  assert.equal(calls.some(c => c.path.includes('synthetic_token')),false);
  assert.equal(JSON.parse(calls.find(c=>c.path==='/Bluefin/TV/v1/leases').body).itemId,id);
});
test('unsafe grants fail closed and valid release is cleaned',async () => {
  for (const mediaUrl of ['https://foreign.invalid/video','/Bluefin/TV/v1/media/'+g+'/0?api_key=bad','/Bluefin/TV/v1/media/%2e%2e/0']) {
    const {s,calls} = fixture({'/Bluefin/TV/v1/leases':()=>response({...lease,mediaUrl},201)});
    await s.signIn('v','p'); await assert.rejects(s.acquire(id,'video')); assert.equal(calls.filter(c=>c.method==='DELETE').length,1);
  }
});
test('background while admission is in flight releases late grant',async () => {
  let deliver, admitted; const ready = new Promise(r=>admitted=r);
  const {s,calls} = fixture({'/Bluefin/TV/v1/leases':()=>{admitted();return new Promise(r=>deliver=r);}});
  await s.signIn('v','p'); const pending = s.acquire(id,'audio'); await ready; await s.suspend(); deliver(response(lease,201));
  await assert.rejects(pending); assert.equal(calls.filter(c=>c.method==='DELETE').length,1);
});
test('logout rejects pending sign-in and does not reinstate token',async () => {
  let deliver, started; const ready = new Promise(r=>started=r);
  const {s} = fixture({'/Users/AuthenticateByName':()=>{started();return new Promise(r=>deliver=r);}});
  const pending = s.signIn('v','p'); await ready; await s.logout(); deliver(response({AccessToken:'late_token',User:{Id:id}}));
  await assert.rejects(pending); await assert.rejects(s.acquire(id,'audio'));
});
test('401 revocation and hostile header/base inputs',async () => {
  assert.throws(()=>new Session('https://user:pass@fixture.invalid','synthetic_device_0001'));
  assert.throws(()=>new Session('http://fixture.invalid','synthetic_device_0001'));
  assert.throws(()=>new Session('https://fixture.invalid','bad"device'));
  const {s} = fixture({'/Items':()=>response({},401)}); await s.signIn('v','p'); await assert.rejects(s.library()); await assert.rejects(s.acquire(id,'audio'));
});
test('account queue uses compare-and-swap and source-track admission is explicit',async () => {
  const {s,calls} = fixture({'/Bluefin/Music/Queue':() => response({revision:7,queue:{itemIds:[id],cursor:0}})});
  await s.signIn('v','p'); assert.equal((await s.loadQueue()).revision,7);
  await s.saveQueue([id],7);
  const update=JSON.parse(calls.find(c => c.path === '/Bluefin/Music/Queue' && c.method === 'PUT').body);
  assert.equal(update.expectedRevision,7); assert.deepEqual(update.queue.itemIds,[id]);
  await s.acquire(id,'video',120000000,{audio:2,subtitle:3});
  const playback=JSON.parse(calls.find(c => c.path === '/Bluefin/TV/v1/leases').body).playback;
  assert.equal(playback.audioStreamIndex,2);assert.equal(playback.subtitleStreamIndex,3);
  await s.stop(); assert.throws(() => s.saveQueue([id],null));
});
test('hostile account queues fail before item fetch',async () => {
  const {s} = fixture({'/Bluefin/Music/Queue':() => response({revision:1,queue:{itemIds:['https://foreign.invalid/']}})});
  await s.signIn('v','p');await assert.rejects(s.loadQueue());
});

test('Vega scoped metadata search preserves all refinements at the requested server cursor',async()=>{
 const {s,calls}=fixture();await s.signIn('viewer','password');
 const filters={scope:'tvShows',studioID:'2'.repeat(32),tag:'Service: Acme+',collectionID:'3'.repeat(32)};
 await s.search('ocean',filters,60);const request=calls.at(-1),query=new URL(request.url).searchParams;
 assert.equal(query.get('IncludeItemTypes'),'Series,Season,Episode');assert.equal(query.get('StudioIds'),'2'.repeat(32));
 assert.equal(query.get('Tags'),'Service: Acme+');assert.equal(query.get('ParentId'),'3'.repeat(32));assert.equal(query.get('StartIndex'),'60');
});
test('Vega broad search has server music artists, channels and programmes',async()=>{
 const {s,calls}=fixture();await s.signIn('viewer','password');
 for(const lane of ['library','artists','programmes','channels'])await s.search('ocean',{scope:'all'},0,lane);
 assert(calls.some(c=>c.path==='/Artists'));assert(calls.some(c=>c.path==='/LiveTv/Programs'));assert(calls.some(c=>c.path==='/LiveTv/Channels'));
 assert(!new URL(calls.find(c=>c.path==='/Items').url).searchParams.has('IncludeItemTypes'));
});
test('Vega Home uses only 404 to fall back to existing account recommendations',async()=>{
 for(const status of [200,404,403]){
  const {s,calls}=fixture({'/Bluefin/Home/Recommendations':()=>response(status===200?{recommendations:[]}: {},status),
   '/Movies/Recommendations':()=>response([])});
  await s.signIn('v','p');await s.home();
  assert.equal(calls.filter(c=>c.path==='/Movies/Recommendations').length,status===404?1:0);
 }
});
test('Vega catalogue discards results completed after logout',async()=>{
 let deliver,started;const ready=new Promise(resolve=>started=resolve);
 const {s}=fixture({'/Items':()=>{started();return new Promise(resolve=>deliver=resolve);}});
 await s.signIn('v','p');const pending=s.search('old',{scope:'movies'});await ready;await s.logout();
 deliver(response({Items:[{Id:id,Name:'private title'}]}));await assert.rejects(pending,/account|cancelled/i);
});

test('Vega cursor consumes source rows while merging duplicate IDs and stops on empty pages',()=>{
 let page=mergeSearchPage({lane:'library',items:[],next:0},{Items:[{Id:id},{Id:id}],TotalRecordCount:5});
 assert.equal(page.items.length,1);assert.equal(page.next,2);
 page=mergeSearchPage(page,{Items:[{Id:id},{Id:'2'.repeat(32)}],TotalRecordCount:5});
 assert.equal(page.items.length,2);assert.equal(page.next,4);
 page=mergeSearchPage(page,{Items:[],TotalRecordCount:5});assert.equal(page.next,page.total);
});
test('Vega partial facet aggregation cannot outlive its account',async()=>{
 let deliver,started;const ready=new Promise(resolve=>started=resolve);
 const {s}=fixture({'/Items/Filters2':()=>{started();return new Promise(resolve=>deliver=resolve);}});
 await s.signIn('v','p');const pending=s.searchFacets();await ready;await s.logout();deliver(response({Tags:['private group']}));
 await assert.rejects(pending,/account|refinements/i);
});

test('Vega artist navigation retains credited-media scope on following pages',async()=>{
 const {s,calls}=fixture();await s.signIn('viewer','synthetic password');
 for(const start of [0,40])await s.children(id,undefined,'MusicArtist',start);
 const requested=calls.map(call=>new URL(call.url)).filter(url=>url.searchParams.get('ArtistIds')===id);
 assert.equal(requested.length,2);assert.equal(requested[1].searchParams.get('StartIndex'),'40');
 for(const url of requested){assert.equal(url.searchParams.has('ParentId'),false);assert.equal(url.searchParams.get('IncludeItemTypes'),'MusicAlbum,Audio,MusicVideo');}
});
