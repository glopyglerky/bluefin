import {test} from 'node:test';
import assert from 'node:assert/strict';
import {Session} from './session.mjs';
const id = '1'.repeat(32), g = 'a'.repeat(64);
const lease = {version:1,mediaUrl:`/Bluefin/TV/v1/media/${g}/0`,releaseUrl:`/Bluefin/TV/v1/leases/${g}`,playSessionId:'synthetic-session'};
const response = (body, status=200) => new Response(status===204 ? null : JSON.stringify(body),{status});
function fixture(overrides = {}) {
  const calls = [];
  const s = new Session('https://fixture.invalid','synthetic_device_0001', async (url,options) => {
    const path = new URL(url).pathname; calls.push({path,...options});
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
