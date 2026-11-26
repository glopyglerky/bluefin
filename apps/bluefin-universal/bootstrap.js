// Capability gate only. The HTML view is useful before any optional Rust/Wasm
// module loads; unavailable features never change an existing session's transport.
(() => {
  'use strict';
  let loaded, pending, unavailable = '';
  function bounded(promise, milliseconds, message) {
    let timer;
    return Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(message)),milliseconds);})])
      .finally(()=>clearTimeout(timer));
  }
  function browserCapability(message) {
    return Object.assign(Error(message), {bluefinBrowserCapability:true});
  }
  async function engine() {
    if (loaded) return loaded;
    if (pending) return pending;
    pending = (async()=>{
      if (!globalThis.isSecureContext || location.protocol !== 'https:') throw Error('Open Bluefin at its trusted HTTPS address.');
      if (typeof WebAssembly !== 'object' || typeof WebAssembly.compile !== 'function')
        throw browserCapability('Encrypted device connections are unavailable in this browser. The HTTPS library remains available.');
      // Do not infer Lockdown Mode from a device name or user-agent.
      try {
        await bounded(WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])),1500,
          'The browser capability check did not respond.');
      } catch (_) {
        throw browserCapability('This browser did not enable encrypted device connections. The HTTPS library remains available.');
      }
      const module = await bounded(import('./bluefin_universal.js'),8000,
        'The encrypted connection module took too long to load. Your current library view is still available.');
      await bounded(module.default(),12000,
        'The encrypted connection module did not start in time. Your current library view is still available.');
      for (const name of ['webMeshBegin','webMeshResume','webMeshProceed','webMeshRequest','webMeshPlay','webMeshLogout'])
        if (typeof module[name] !== 'function') throw Error('This web bundle is incomplete. Reload after the installation is updated.');
      loaded=module;
      return module;
    })().catch(error=>{unavailable=String(error.message||error);throw error;}).finally(()=>{pending=null;});
    return pending;
  }
  window.BluefinRuntime=Object.freeze({
    engine,
    ready:()=>loaded,
    unavailable:()=>unavailable,
    bounded,
    async mediaWorker() {
      if (!navigator.serviceWorker) throw browserCapability('This browser cannot play through the encrypted mesh media bridge.');
      const expected=new URL('./media-worker.js',document.baseURI).href;
      if (navigator.serviceWorker.controller?.scriptURL===expected) return;
      await bounded(navigator.serviceWorker.register(expected),5000,'The encrypted media bridge could not start.');
      await bounded(new Promise(resolve=>{
        const changed=()=>{if(navigator.serviceWorker.controller?.scriptURL===expected){
          navigator.serviceWorker.removeEventListener('controllerchange',changed);resolve();
        }};
        navigator.serviceWorker.addEventListener('controllerchange',changed);changed();
        setTimeout(()=>navigator.serviceWorker.removeEventListener('controllerchange',changed),5000);
      }),5000,'The encrypted media bridge did not activate. Reload and resume this connection.');
    }
  });
})();
