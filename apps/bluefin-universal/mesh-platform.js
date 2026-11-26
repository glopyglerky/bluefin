// Browser APIs only. Invitation validation, PQ transport, retries, account
// authority and media routes are implemented in Rust.
const journalLimit = 131072;
let databasePromise;
const request = value => new Promise((resolve,reject) => {
  value.onsuccess = () => resolve(value.result);
  value.onerror = () => reject(Error('Protected connection storage unavailable'));
});
function database() {
  if (!globalThis.isSecureContext || location.protocol !== 'https:') throw Error('Use a trusted HTTPS address');
  return databasePromise ||= new Promise((resolve,reject) => {
    const open = indexedDB.open('bluefin-mesh-v1', 1);
    open.onupgradeneeded = () => open.result.createObjectStore('vault');
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(Error('Protected connection storage unavailable'));
    open.onblocked = () => reject(Error('Close older Bluefin tabs and retry'));
  });
}
async function key(db) {
  const found = await request(db.transaction('vault').objectStore('vault').get('key'));
  if (found) return found;
  const generated = await crypto.subtle.generateKey({name:'AES-GCM',length:256},false,['encrypt','decrypt']);
  await new Promise(resolve => {
    const transaction = db.transaction('vault','readwrite');
    transaction.objectStore('vault').add(generated,'key');
    transaction.oncomplete = resolve;
    // Another tab may have installed the same origin's key first.
    transaction.onabort = resolve;
    transaction.onerror = event => event.preventDefault();
  });
  const stored = await request(db.transaction('vault').objectStore('vault').get('key'));
  if (!stored) throw Error('Protected connection key unavailable');
  return stored;
}
export async function loadMeshVault() {
  const db = await database(), secret = await key(db);
  const record = await request(db.transaction('vault').objectStore('vault').get('record'));
  if (!record) return JSON.stringify({revision:0,value:null});
  if (!Number.isSafeInteger(record.revision) || record.revision < 1) throw Error('Invalid connection journal');
  if (record.bytes === null) return JSON.stringify({revision:record.revision,value:null});
  if (!(record.bytes instanceof ArrayBuffer) || record.bytes.byteLength > journalLimit + 16
      || !(record.iv instanceof Uint8Array) || record.iv.byteLength !== 12) throw Error('Invalid connection journal');
  const bytes = new Uint8Array(await crypto.subtle.decrypt(
    {name:'AES-GCM',iv:record.iv,additionalData:new TextEncoder().encode('bluefin-mesh-v1')},secret,record.bytes));
  try {return JSON.stringify({revision:record.revision,value:new TextDecoder('utf-8',{fatal:true}).decode(bytes)});}
  finally {bytes.fill(0);}
}
export async function saveMeshVault(expected, serialized) {
  if (!Number.isSafeInteger(expected) || expected < 0 || expected >= Number.MAX_SAFE_INTEGER) throw Error('Invalid connection revision');
  const db = await database(), secret = await key(db);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  let bytes = null;
  if (serialized !== null) {
    const plain = new TextEncoder().encode(serialized);
    if (!plain.byteLength || plain.byteLength > journalLimit) {plain.fill(0);throw Error('Connection journal limit');}
    try {bytes = await crypto.subtle.encrypt(
      {name:'AES-GCM',iv,additionalData:new TextEncoder().encode('bluefin-mesh-v1')},secret,plain);}
    finally {plain.fill(0);}
  }
  return new Promise((resolve,reject) => {
    const transaction = db.transaction('vault','readwrite'), store = transaction.objectStore('vault');
    const old = store.get('record');
    old.onsuccess = () => {
      if ((old.result?.revision || 0) !== expected) {transaction.abort();return;}
      store.put({revision:expected+1,iv,bytes},'record');
    };
    transaction.oncomplete = () => {lifecycleChannel?.postMessage({type:"journal"});resolve();};
    transaction.onabort = () => reject(Error('Connection changed in another tab. Resume the saved connection.'));
    transaction.onerror = () => reject(Error('Connection could not be saved'));
  });
}
export function takeMeshLaunch() {
  return globalThis.takeBluefinLaunch?.() || '';
}
export async function readMeshPolicy() {
  if (!globalThis.isSecureContext || location.protocol !== 'https:') throw Error('Use a trusted HTTPS address');
  const controller = new AbortController(), timer = setTimeout(() => controller.abort(),10000);
  try {
    const response = await fetch(new URL('./mesh-policy.json', document.baseURI),{
      credentials:'omit',redirect:'error',cache:'no-store',referrerPolicy:'no-referrer',signal:controller.signal});
    if (!response.ok || !response.body) throw Error('Browser mesh policy unavailable');
    const reader = response.body.getReader(), chunks=[];let total=0;
    try {for(;;){const {done,value}=await reader.read();if(done)break;total+=value.length;if(total>8192)throw Error('Policy limit');chunks.push(value);}}
    catch(error){await reader.cancel();throw error;}
    const bytes = new Uint8Array(total);let at=0;for(const chunk of chunks){bytes.set(chunk,at);at+=chunk.length;}
    return new TextDecoder('utf-8',{fatal:true}).decode(bytes);
  } finally {clearTimeout(timer);}
}
function decode(value) {
  if (typeof value !== 'string' || value.length > 65536) throw Error('Invalid passkey options');
  return Uint8Array.from(atob(value.replace(/-/g,'+').replace(/_/g,'/')),c=>c.charCodeAt(0));
}
function encode(value) {
  return btoa(String.fromCharCode(...new Uint8Array(value))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');
}
export async function meshPasskey(serialized) {
  if (!globalThis.isSecureContext || location.protocol !== 'https:' || !window.PublicKeyCredential) throw Error('Passkeys unavailable');
  const value = JSON.parse(serialized);
  if (value.origin !== location.origin || !['authentication','registration'].includes(value.kind)) throw Error('Use the paired server passkey page');
  const enroll = value.kind === 'registration', options = value.publicKey;
  const parser = enroll ? PublicKeyCredential.parseCreationOptionsFromJSON : PublicKeyCredential.parseRequestOptionsFromJSON;
  let publicKey;
  if (parser) publicKey = parser.call(PublicKeyCredential,options);
  else {
    publicKey = {...options,challenge:decode(options.challenge)};
    if (enroll) publicKey.user={...options.user,id:decode(options.user.id)};
    for (const name of ['allowCredentials','excludeCredentials']) if (options[name])
      publicKey[name]=options[name].map(item=>({...item,id:decode(item.id)}));
  }
  const credential = enroll ? await navigator.credentials.create({publicKey}) : await navigator.credentials.get({publicKey});
  if (!credential) throw Error('No passkey selected');
  if (credential.toJSON) return JSON.stringify(credential.toJSON());
  const response=credential.response,clientDataJSON=encode(response.clientDataJSON);
  return JSON.stringify({id:credential.id,rawId:encode(credential.rawId),type:credential.type,
    authenticatorAttachment:credential.authenticatorAttachment,
    clientExtensionResults:credential.getClientExtensionResults(),
    response:enroll ? {clientDataJSON,attestationObject:encode(response.attestationObject),transports:response.getTransports?.()}
      : {clientDataJSON,authenticatorData:encode(response.authenticatorData),signature:encode(response.signature),userHandle:response.userHandle ? encode(response.userHandle) : null}});
}


let meshMediaHandler;
const mediaStreams = new Set();
export function setMeshMediaHandler(handler) { meshMediaHandler = handler; }
navigator.serviceWorker?.addEventListener('message', async event => {
  if (event.data?.op !== 'bluefin-mesh-read') return;
  const port = event.ports?.[0];
  if (!port) return;
  const expected = new URL('./media-worker.js',document.baseURI).href;
  if (!meshMediaHandler || event.source !== navigator.serviceWorker.controller
      || event.source?.scriptURL !== expected || mediaStreams.size >= 4) {
    port.postMessage({type:'error'});port.close();return;
  }
  const message = event.data;
  if (typeof message.handle !== 'string' || !/^bluefin-mesh-v1:[a-f0-9]{64}$/.test(message.handle)
      || !['GET','HEAD'].includes(message.method) || typeof message.url !== 'string'
      || typeof message.range !== 'string') {
    port.postMessage({type:'error'});port.close();return;
  }
  const operation = {};
  mediaStreams.add(operation);
  let stream,reading=false,closed=false,idle;
  const finish = () => {
    clearTimeout(idle);closed=true;stream?.cancel();
    if (!reading && stream) {stream.free();stream=undefined;}
    mediaStreams.delete(operation);port.close();
  };
  const arm = () => {clearTimeout(idle);idle=setTimeout(finish,30000);};
  port.onmessage = async input => {
    if (input.data?.op === 'cancel') {finish();return;}
    if (input.data?.op !== 'pull' || reading || closed || !stream) {
      port.postMessage({type:'error'});finish();return;
    }
    reading=true;clearTimeout(idle);
    try {
      const bytes=await stream.next();
      if (closed) return;
      if (bytes === null) {port.postMessage({type:'end'});finish();}
      else if (bytes instanceof Uint8Array && bytes.byteLength > 0 && bytes.byteLength <= 8192) {
        port.postMessage({type:'chunk',bytes},[bytes.buffer]);arm();
      } else {port.postMessage({type:'error'});finish();}
    } catch (_) {if(!closed)port.postMessage({type:'error'});finish();}
    finally {
      reading=false;
      if (closed && stream) {stream.free();stream=undefined;}
    }
  };
  arm();
  try {
    stream=await meshMediaHandler(message.handle,message.method,message.url,message.range);
    if (closed) {stream.cancel();stream.free();stream=undefined;return;}
    const head=JSON.parse(stream.head());
    port.postMessage({type:'head',status:head.status,headers:head.headers});arm();
  } catch (_) {if(!closed)port.postMessage({type:'error'});finish();}
});

let lifecycleHandler;
const lifecycleChannel = typeof BroadcastChannel === 'function' ? new BroadcastChannel('bluefin-mesh-lifecycle-v1') : null;
function stopOutput(handle) {dispatchEvent(new CustomEvent('bluefin-mesh-retired',{detail:handle}));}
export function setMeshLifecycleHandler(handler) { lifecycleHandler=handler; }
export function notifyMeshRetiring(handle) {
  stopOutput(handle);
  lifecycleChannel?.postMessage({type:'retire',handle});
}
export function stopMeshOutput(handle) {stopOutput(handle);}
lifecycleChannel?.addEventListener('message', event => {
  const value=event.data;
  if (value?.type === 'retire' && typeof value.handle === 'string'
      && /^bluefin-mesh-v1:[a-f0-9]{64}$/.test(value.handle)) {
    stopOutput(value.handle);lifecycleHandler?.(value.handle).catch(()=>{});
  } else if (value?.type === 'journal') {lifecycleHandler?.('').catch(()=>{});}
});
document.addEventListener('visibilitychange',()=>{
  if (document.visibilityState === 'visible') lifecycleHandler?.('').catch(()=>{});
});

export async function copyMeshCode(value) {
  if (typeof value !== 'string' || value.length > 3072) throw Error('Invalid connection code');
  await navigator.clipboard.writeText(value);
}
