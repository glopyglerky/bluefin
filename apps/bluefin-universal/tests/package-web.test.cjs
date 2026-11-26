'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs'),os=require('node:os'),path=require('node:path'),crypto=require('node:crypto');
const {packageWeb,MAX_ASSET_BYTES}=require('../../../scripts/package-bluefin-web.cjs');
function fixture(t){
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'bluefin-package-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const source=path.join(root,'source'),app=path.join(source,'apps/bluefin-universal'),engine=path.join(root,'engine'),output=path.join(root,'bundle');
 fs.mkdirSync(path.join(app,'vendor'),{recursive:true});fs.mkdirSync(path.join(engine,'snippets/bridge'),{recursive:true});
 fs.writeFileSync(path.join(app,'index.html'),'<link href="./bluefin.css"><script src="./bootstrap.js"></script><script src="./native-view.js"></script>');
 fs.writeFileSync(path.join(app,'bluefin.css'),'body{color:#006e91}');
 fs.writeFileSync(path.join(app,'bootstrap.js'),"import('./bluefin_universal.js');new URL('./media-worker.js',document.baseURI);");
 fs.writeFileSync(path.join(app,'native-view.js'),"new URL('./vendor/hls.light.min.js',document.baseURI);");
 fs.writeFileSync(path.join(app,'media-worker.js'),'self.addEventListener("fetch",()=>{});');
 fs.writeFileSync(path.join(app,'mesh-policy.json'),'{"version":1,"enabled":false,"relays":[]}');
 fs.writeFileSync(path.join(app,'vendor/hls.light.min.js'),'globalThis.Hls={};');
 fs.writeFileSync(path.join(app,'vendor/LICENSE'),'fixture decoder license');
 fs.writeFileSync(path.join(source,'LICENSE'),'fixture app license');
 fs.writeFileSync(path.join(engine,'bluefin_universal.js'),"import './snippets/bridge/mesh-platform.js';");
 fs.writeFileSync(path.join(engine,'bluefin_universal_bg.wasm'),Buffer.from([0,97,115,109,1,0,0,0]));
 fs.writeFileSync(path.join(engine,'snippets/bridge/mesh-platform.js'),"new URL('./media-worker.js',document.baseURI);");
 fs.writeFileSync(path.join(engine,'snippets/bridge/web-player.js'),"new URL('./media-worker.js',document.baseURI);");
 return {source,app,engine,output};
}
test('packaging pins the complete engine and every worker consumer while retaining document scope',t=>{
 const f=fixture(t),manifest=packageWeb(f.engine,f.output,f.source);
 const index=fs.readFileSync(path.join(f.output,'index.html'),'utf8');
 assert(!index.includes('<base'));assert(!index.includes('./native-view.js'));assert.match(index,/native-view-[a-f0-9]{64}\.js/);
 const bootstrap=manifest.assets.find(file=>/^bootstrap-/.test(file.path));
 const boot=fs.readFileSync(path.join(f.output,bootstrap.path),'utf8');
 assert(boot.includes('./'+manifest.engine+'/bluefin_universal.js'));
 const reference='./media-worker.js?release='+manifest.workerSha256;
 assert(boot.includes(reference));
 for(const name of ['mesh-platform.js','web-player.js']){
  assert(fs.readFileSync(path.join(f.output,manifest.engine,'snippets/bridge',name),'utf8').includes(reference));
 }
 assert(!fs.existsSync(path.join(f.output,'bluefin_universal.js')));
 for(const entry of manifest.assets){
  const bytes=fs.readFileSync(path.join(f.output,entry.path));
  assert.equal(bytes.length,entry.bytes);assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'),entry.sha256);
 }
 const second=path.join(path.dirname(f.output),'second');
 assert.deepEqual(packageWeb(f.engine,second,f.source),manifest);
});
test('a bridge-only source change gets a different engine address even with the same Wasm',t=>{
 const f=fixture(t),first=packageWeb(f.engine,f.output,f.source);
 fs.appendFileSync(path.join(f.engine,'snippets/bridge/mesh-platform.js'),'\n// changed bridge\n');
 const second=packageWeb(f.engine,path.join(path.dirname(f.output),'second'),f.source);
 assert.equal(first.wasmSha256,second.wasmSha256);assert.notEqual(first.engine,second.engine);
});
test('packaging rejects oversized assets and filesystem links',t=>{
 const f=fixture(t);fs.writeFileSync(path.join(f.engine,'oversized.bin'),Buffer.alloc(MAX_ASSET_BYTES+1));
 assert.throws(()=>packageWeb(f.engine,f.output,f.source),/exceeds 25 MiB/);
 const g=fixture(t);fs.symlinkSync(path.join(g.source,'LICENSE'),path.join(g.engine,'linked-license'));
 assert.throws(()=>packageWeb(g.engine,g.output,g.source),/symbolic links/);
});
