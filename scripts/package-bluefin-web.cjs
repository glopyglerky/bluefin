#!/usr/bin/env node
'use strict';
// Build-only packaging. No network, Git lookup, application execution or
// credential access. The caller supplies admitted source and bindgen output.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const MAX_ASSET_BYTES=25*1024*1024;
const sha=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
function files(root,relative=''){
 const names=[];
 for(const entry of fs.readdirSync(path.join(root,relative),{withFileTypes:true}).sort((a,b)=>a.name.localeCompare(b.name,'en'))){
  const name=path.posix.join(relative,entry.name);
  if(entry.isSymbolicLink())throw Error('Static bundles cannot contain symbolic links.');
  if(entry.isDirectory())names.push(...files(root,name));
  else if(entry.isFile())names.push(name);
  else throw Error('Static bundles must contain only regular files and directories.');
 }
 return names;
}
function read(root,name){return fs.readFileSync(path.join(root,name));}
function write(root,name,value){
 const target=path.join(root,name);fs.mkdirSync(path.dirname(target),{recursive:true,mode:0o755});
 fs.writeFileSync(target,value,{mode:0o644});
}
function replace(source,from,to,label){
 if(!source.includes(from))throw Error('Missing expected '+label+' reference.');
 return source.split(from).join(to);
}
function hashed(name,bytes){
 const dot=name.lastIndexOf('.');return name.slice(0,dot)+'-'+sha(bytes)+name.slice(dot);
}
function inventory(output,details={}){
 const entries=files(output).filter(name=>name!=='asset-manifest.json').map(name=>{
  const bytes=read(output,name);
  if(bytes.length>MAX_ASSET_BYTES)throw Error('Static asset exceeds 25 MiB: '+name);
  return {path:name,bytes:bytes.length,sha256:sha(bytes)};
 });
 const value={version:1,entrypoint:'index.html',...details,assets:entries};
 write(output,'asset-manifest.json',JSON.stringify(value,null,2)+'\n');
 return value;
}
function packageWeb(engineInput,output,sourceRoot){
 const app=path.join(sourceRoot,'apps/bluefin-universal');
 if(fs.existsSync(output))throw Error('The bundle output must be a new directory.');
 const engineFiles=files(engineInput);
 if(!engineFiles.includes('bluefin_universal.js')||!engineFiles.includes('bluefin_universal_bg.wasm'))throw Error('The admitted bindgen output is incomplete.');
 const wasm=read(engineInput,'bluefin_universal_bg.wasm');
 if(wasm.length<8||!wasm.subarray(0,4).equals(Buffer.from([0,97,115,109])))throw Error('The admitted Wasm artifact is invalid.');
 fs.mkdirSync(output,{recursive:false,mode:0o755});
 const worker=read(app,'media-worker.js'),workerHash=sha(worker);
 const workerReference='./media-worker.js?release='+workerHash;
 write(output,'media-worker.js',worker);
 const prepared=new Map();let workerConsumers=0;
 for(const name of engineFiles){
  let bytes=read(engineInput,name);
  if(name.endsWith('.js')){
   const text=bytes.toString('utf8');
   if(text.includes('./media-worker.js')){
    workerConsumers++;bytes=Buffer.from(text.split('./media-worker.js').join(workerReference));
   }
  }
  prepared.set(name,bytes);
 }
 if(workerConsumers===0)throw Error('The Rust bundle does not contain its media bridge.');
 const digest=crypto.createHash('sha256');
 for(const [name,bytes]of prepared){digest.update(name);digest.update('\0');digest.update(bytes);digest.update('\0');}
 const engine='engine-'+digest.digest('hex');
 for(const [name,bytes]of prepared)write(output,path.posix.join(engine,name),bytes);
 const vendor=path.join(app,'vendor'),hlsBytes=read(vendor,'hls.light.min.js');
 const hlsName='hls.light-'+sha(hlsBytes)+'.min.js';
 for(const name of files(vendor))write(output,'vendor/'+(name==='hls.light.min.js'?hlsName:name),read(vendor,name));
 let index=read(app,'index.html').toString('utf8');
 if(/<base(?:\s|>)/i.test(index))throw Error('The document base must remain at its deployment root.');
 const css=read(app,'bluefin.css'),cssName=hashed('bluefin.css',css);write(output,cssName,css);
 index=replace(index,'./bluefin.css','./'+cssName,'stylesheet');
 const scripts=['bootstrap.js','native-view.js'];
 if(fs.existsSync(path.join(app,'local-music.js')))scripts.push('local-music.js');
 for(const name of scripts){
  let text=read(app,name).toString('utf8');
  if(name==='bootstrap.js'){
   text=replace(text,'./bluefin_universal.js','./'+engine+'/bluefin_universal.js','Rust engine');
   text=replace(text,'./media-worker.js',workerReference,'media worker');
  }
  if(name==='native-view.js')text=replace(text,'./vendor/hls.light.min.js','./vendor/'+hlsName,'HLS decoder');
  const bytes=Buffer.from(text),versioned=hashed(name,bytes);
  write(output,versioned,bytes);
  index=replace(index,'./'+name,'./'+versioned,name);
 }
 write(output,'index.html',index);
 write(output,'mesh-policy.json',read(app,'mesh-policy.json'));
 write(output,'LICENSE',read(sourceRoot,'LICENSE'));
 // Deployment provenance and the exact corresponding-source archive are
 // supplied by the release owner from the admission receipt, never inferred.
 write(output,'SOURCE.txt','Bluefin corresponding source\n\nSource archive: ./bluefin-corresponding-source.tar.gz\nLicense: ./LICENSE\nAsset checksums: ./asset-manifest.json\n\nThe release provenance accompanying the archive identifies the admitted source revision.\n');
 return inventory(output,{engine,wasmSha256:sha(wasm),workerSha256:workerHash});
}
module.exports={packageWeb,inventory,MAX_ASSET_BYTES};
if(require.main===module){
 const [engine,output,sourceRoot,...extra]=process.argv.slice(2);
 if(extra.length||!engine||!output||!sourceRoot)throw Error('Usage: node scripts/package-bluefin-web.cjs <bindgen output> <new bundle output> <source root>');
 const value=packageWeb(path.resolve(engine),path.resolve(output),path.resolve(sourceRoot));
 process.stdout.write(JSON.stringify({engine:value.engine,wasmSha256:value.wasmSha256,
  assets:value.assets.length,maximumAssetBytes:Math.max(...value.assets.map(file=>file.bytes))})+'\n');
}
