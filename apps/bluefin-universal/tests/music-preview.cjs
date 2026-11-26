// Test-only visual fixture. The production packager never includes tests/.
// Usage: node tests/music-preview.cjs /tmp/bluefin-music-preview.html
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'..');
const read=name=>fs.readFileSync(path.join(root,name),'utf8');
const scriptSafe=value=>value.replace(/<\/script/gi,'<\\/script');
const css=read('bluefin.css');
const native=read('native-view.js').replace(/\}\)\(\);\s*$/,'globalThis.previewContract={state,render,play,navigate};})();');
const darkBlocks=[...css.matchAll(/@media\(prefers-color-scheme:dark\)\{:root\{([^}]+)\}\}/g)].map(match=>match[1]).join(';');
const lightBlocks=[css.match(/:root\{([^}]+)\}/)[1],css.match(/:root\{(--chrome:[^}]+)\}/)[1]].join(';');
function installPreview(lightVariables,darkVariables){
 const query=new URLSearchParams(location.search),previewMode=query.get('theme')||'dark';
 document.documentElement.style.cssText=previewMode==='light'?lightVariables:darkVariables;
 document.documentElement.style.colorScheme=previewMode;
 const uid='11111111111141118111111111111111';
 const songNames=['Harbor Lights','The Quiet Tide','A Little Closer','Northern Skies','Driftwood','Open Water'];
 const artistNames=['The Lanterns','The Lanterns','Aster Vale','North Coast','Aster Vale','North Coast'];
 const albumNames=['After the Rain','Still Water','Soft Landing','Above the Harbor','Soft Landing','Wide Awake'];
 const songs=songNames.map((name,index)=>({Id:(index+2).toString(16).repeat(32),Name:name,Type:'Audio',Artists:[artistNames[index]],Album:albumNames[index],RunTimeTicks:1200000000,ImageTags:{Primary:'original-preview-'+index},UserData:{PlaybackPositionTicks:0}}));
 const albums=songs.map((song,index)=>({...song,Id:(index+8).toString(16).repeat(32),Name:albumNames[index],Type:'MusicAlbum',AlbumArtist:artistNames[index]}));
 const lyricText=['A lantern glows beside the water','The harbor keeps a little light','We leave the noise behind us','And walk into the quiet night','The tide is turning slowly','Our footsteps find their way','A little room for wonder','A little more to say','The morning waits beyond us','The sky begins to clear','We carry home the daylight','And keep the music here'];
 const lyrics=lyricText.map((Text,index)=>{
  const start=index*8+1;let position=0;
  const Cues=Text.split(' ').map((word,n)=>{const from=position;position+=word.length+1;return {Position:from,EndPosition:from+word.length,Start:Math.round((start+n*.72)*1e7),End:Math.round((start+(n+1)*.72)*1e7)};});
  return {Text,Start:start*1e7,Cues};
 });
 const media=document.getElementById('media');let paused=true,position=18;
 Object.defineProperties(media,{paused:{get:()=>paused},currentTime:{get:()=>position,set:value=>{position=Math.max(0,Math.min(120,Number(value)||0));media.dispatchEvent(new Event('timeupdate'));}},duration:{get:()=>120},ended:{get:()=>position>=120}});
 media.pause=()=>{paused=true;media.dispatchEvent(new Event('pause'));};
 media.play=()=>{paused=false;media.dispatchEvent(new Event('play'));return Promise.resolve();};
 media.load=()=>{};media.canPlayType=()=> 'probably';
 setInterval(()=>{if(!paused){position=Math.min(120,position+.1);media.dispatchEvent(new Event('timeupdate'));if(position>=120){paused=true;media.dispatchEvent(new Event('ended'));}}},100);
 const artwork=new Map();
 async function coverArt(id){
  if(artwork.has(id))return artwork.get(id);
  const n=parseInt(id[0],16)%6,canvas=document.createElement('canvas');canvas.width=canvas.height=600;
  const ctx=canvas.getContext('2d'),colors=[['#243d4f','#b4c3c7'],['#315662','#d3d6cc'],['#746077','#ead7cd'],['#223853','#87a1b5'],['#74644d','#ddd3bb'],['#365650','#a5b9af']][n];
  const gradient=ctx.createLinearGradient(0,0,600,600);gradient.addColorStop(0,colors[0]);gradient.addColorStop(1,colors[1]);ctx.fillStyle=gradient;ctx.fillRect(0,0,600,600);
  ctx.fillStyle='rgba(240,240,227,.78)';ctx.beginPath();ctx.arc(370,210,105,0,Math.PI*2);ctx.fill();
  ctx.fillStyle=colors[0];ctx.beginPath();ctx.moveTo(0,340);ctx.bezierCurveTo(190,270,400,440,600,300);ctx.lineTo(600,600);ctx.lineTo(0,600);ctx.closePath();ctx.fill();
  ctx.fillStyle='rgba(255,255,255,.92)';ctx.font='600 38px system-ui';ctx.fillText(albumNames[n].toUpperCase(),40,510);ctx.font='22px system-ui';ctx.fillText(artistNames[n].toUpperCase(),42,548);
  const blob=await new Promise(resolve=>canvas.toBlob(resolve,'image/png'));artwork.set(id,blob);return blob;
 }
 const json=value=>new Response(JSON.stringify(value),{headers:{'Content-Type':'application/json'}});
 window.fetch=async(raw,options={})=>{
  const url=new URL(raw,location.href),p=url.pathname;
  if(options.signal?.aborted)throw new DOMException('Aborted','AbortError');
  if(p.includes('/Images/'))return new Response(await coverArt(p.split('/')[2]||songs[0].Id),{headers:{'Content-Type':'image/png'}});
  if(p.endsWith('/Lyrics'))return json({Metadata:{Offset:0,IsSynced:true},Lyrics:lyrics});
  if(p==='/Bluefin/Web/v1')return json({version:1,cookieMediaLease:true,expiresInSeconds:300,maxSessionSeconds:7200,idleSeconds:90});
  if(p==='/Bluefin/Web/v1/leases')return json({version:1,mediaUrl:'/Bluefin/Web/v1/media/'+'a'.repeat(64)+'/0',releaseUrl:'/Bluefin/Web/v1/leases/'+'a'.repeat(64),renewUrl:'/Bluefin/Web/v1/leases/'+'a'.repeat(64),expiresInSeconds:300,idleSeconds:90,playSessionId:'33333333333343338333333333333333'});
  if(p.includes('/Sessions/')||p.includes('/Bluefin/Web/v1/leases/'))return json({});
  if(p==='/Bluefin/Capabilities')return json({protocolVersion:1,viewer:{userId:uid}});
  if(p==='/Bluefin/Home/Recommendations')return new Response('{}',{status:404});
  if(p==='/Movies/Recommendations')return json([]);
  if(p.endsWith('/Latest'))return json(albums);
  if(p.endsWith('/Resume')||p==='/Shows/NextUp')return json({Items:[]});
  if(p.endsWith('/Views'))return json({Items:[{Id:'eeeeeeeeeeee4eee8eeeeeeeeeeeeeee',Name:'Music',Type:'CollectionFolder',CollectionType:'music'}]});
  if(p.includes('/Search/Hints'))return json({SearchHints:[],TotalRecordCount:0});
  if(p.endsWith('/Items')){
   const type=url.searchParams.get('IncludeItemTypes')||'MusicAlbum',term=(url.searchParams.get('SearchTerm')||'').toLowerCase();
   let items=type==='Audio'?songs:type==='MusicArtist'?songs.map((s,i)=>({...s,Id:(i+10).toString(16).repeat(32),Name:s.Artists[0],Type:'MusicArtist'})):type==='Playlist'?albums.map(a=>({...a,Type:'Playlist'})):albums;
   if(term)items=items.filter(item=>[item.Name,item.Album,...item.Artists].join(' ').toLowerCase().includes(term));
   if(url.searchParams.get('IsFavorite'))items=items.slice(0,3);
   return json({Items:items,TotalRecordCount:items.length});
  }
  return json({Items:[]});
 };
 window.BluefinRuntime={ready:()=>null,engine:async()=>{throw Error('Optional mesh is not part of this visual fixture.');}};
 window.previewFixture={query,previewMode,uid,songs,media};
}
async function bootPreview(){
 try{
  const {query,previewMode,uid,songs,media}=previewFixture;
  const ui=previewContract;Object.assign(ui.state,{phase:'library',base:new URL('/',location.href).href,session:{token:'synthetic-preview-only',user:uid,name:'Preview'},mode:'https',tab:'Music',route:{kind:'music'}});
  await ui.navigate('Music');await ui.play(songs[0],songs,0);media.pause();media.currentTime=18;
  const screen=query.get('screen')||'music';ui.state.expanded=screen!=='music';ui.state.nowPanel=screen==='lyrics'?'lyrics':screen==='queue'?'queue':'artwork';ui.render();
  if(query.get('playing')==='1')await media.play();
  const controls=document.getElementById('preview-tools');if(query.get('controls')==='0')controls.hidden=true;
  document.getElementById('preview-theme').onclick=()=>{query.set('theme',previewMode==='dark'?'light':'dark');location.search=query;};
  document.getElementById('preview-clock').onclick=()=>{media.currentTime=(media.currentTime+8)%96;};
 }catch(error){document.getElementById('preview-tools').textContent='Preview failed: '+error.message;}
}
const mock='('+installPreview.toString()+')('+JSON.stringify(lightBlocks)+','+JSON.stringify(darkBlocks)+');';
const boot='('+bootPreview.toString()+')();';
const html=[
 '<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover"><title>Bluefin music presentation preview</title><style>',
 css,
 '#preview-tools{position:fixed;right:12px;top:8px;z-index:60;display:flex;align-items:center;gap:6px;font:11px system-ui;padding:5px 8px;border-radius:16px;background:#eff2f6;color:#102435;box-shadow:0 2px 10px #0003}#preview-tools button{font:11px system-ui;min-height:28px;padding:4px 8px;border:1px solid #bcc5cf;color:#102435;background:#fff;border-radius:10px}#preview-tools[hidden]{display:none}',
 '</style></head><body><div id="preview-tools"><span>Preview · synthetic media</span><button id="preview-theme">Light / Dark</button><button id="preview-clock">Advance song</button></div><div id="startup-status"></div><div id="app"></div><section id="media-host"><div id="media" hidden></div><p id="media-status"></p></section>',
 '<script>'+scriptSafe(mock)+'</script>',
 '<script>'+scriptSafe(read('local-music.js'))+'</script>',
 '<script>'+scriptSafe(native)+'</script>',
 '<script>'+scriptSafe(boot)+'</script>',
 '</body></html>'
].join('\n');
const output=process.argv[2];if(!output)throw Error('Pass an output HTML path outside the release directory.');
fs.writeFileSync(output,html);process.stdout.write(output+'\n');
