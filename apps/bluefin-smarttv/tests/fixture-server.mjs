// Original synthetic fixture only. Binds loopback; never proxies external media.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
const root=path.resolve(process.argv[2]||'.'),port=Number(process.argv[3]||8742);
const id='00000000000000000000000000000001', grant='1'.repeat(64);
const item={Id:id,Name:'Bluefin original tone',Type:'Audio',Overview:'An original synthetic signal for local playback checks.',RunTimeTicks:1200000000,MediaStreams:[{Type:'Audio',Index:0,DisplayTitle:'Stereo fixture'}],ExternalUrls:[{Url:'https://www.youtube.com/watch?v=abcdefghijk'}]};
const events=[];let released=false;
function json(res,body,status=200){res.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(body));}
function wave(){const rate=8000,seconds=120;const data=Buffer.alloc(44+rate*seconds*2);data.write('RIFF');data.writeUInt32LE(data.length-8,4);data.write('WAVEfmt ',8);data.writeUInt32LE(16,16);data.writeUInt16LE(1,20);data.writeUInt16LE(1,22);data.writeUInt32LE(rate,24);data.writeUInt32LE(rate*2,28);data.writeUInt16LE(2,32);data.writeUInt16LE(16,34);data.write('data',36);data.writeUInt32LE(data.length-44,40);for(let n=0;n<rate*seconds;n++)data.writeInt16LE(Math.round(Math.sin(n*2*Math.PI*220/rate)*800),44+n*2);return data;}
const audio=wave();
const app=http.createServer(async(req,res)=>{
 const url=new URL(req.url,'http://localhost');let body='';for await(const chunk of req){body+=chunk;if(body.length>4096){res.writeHead(413).end();return;}}
 if(url.pathname==='/fixture/events'){json(res,events);return;}
 if(url.pathname==='/Users/AuthenticateByName'){const request=JSON.parse(body);if(request.Username!=='fixture'||request.Pw!=='synthetic'){json(res,{},401);return;}json(res,{AccessToken:'fixture-session',User:{Id:id}});return;}
 if(url.pathname.startsWith('/Bluefin/TV/v1/media/')){if(released){res.writeHead(404).end();return;}events.push({method:req.method,path:url.pathname,credentialInUrl:url.search.length>0});let start=0,end=audio.length-1,status=200;const range=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');if(range){start=Number(range[1]);end=range[2]?Math.min(Number(range[2]),end):end;status=206;if(start>end){res.writeHead(416,{'Content-Range':'bytes */'+audio.length}).end();return;}}const headers={'Content-Type':'audio/wav','Content-Length':end-start+1,'Accept-Ranges':'bytes','Cache-Control':'no-store'};if(status===206)headers['Content-Range']='bytes '+start+'-'+end+'/'+audio.length;res.writeHead(status,headers);res.end(req.method==='HEAD'?undefined:audio.subarray(start,end+1));return;}
 if(url.pathname.startsWith('/Bluefin/TV/v1/leases/')&&req.method==='DELETE'){released=true;events.push({method:'DELETE',path:'lease-release'});res.writeHead(204).end();return;}
 if(url.pathname.startsWith('/Items')||url.pathname.startsWith('/Bluefin')||url.pathname.startsWith('/Sessions')||url.pathname.startsWith('/LiveTv')){
  if(!req.headers.authorization?.includes('Token="fixture-session"')){json(res,{},401);return;}
  if(url.pathname==='/Bluefin/TV/v1'){json(res,{version:1,mediaLease:true});return;}
  if(url.pathname==='/Bluefin/TV/v1/leases'){released=false;events.push({method:'POST',path:'lease-admission',request:JSON.parse(body)});json(res,{version:1,mediaUrl:'/Bluefin/TV/v1/media/'+grant+'/0',releaseUrl:'/Bluefin/TV/v1/leases/'+grant,playSessionId:'fixture-play',expiresInSeconds:7200,idleSeconds:90},201);return;}
  if(url.pathname==='/Items'){json(res,{Items:[item],TotalRecordCount:1});return;}
  if(url.pathname.startsWith('/Items/')){json(res,item);return;}
  if(url.pathname==='/LiveTv/Channels'){json(res,{Items:[{Id:id,Name:'Fixture channel',Number:'1',Type:'TvChannel'}],TotalRecordCount:1});return;}
  if(url.pathname==='/LiveTv/Programs'){const now=Date.now();json(res,{Items:[{Id:id,Name:'Bluefin original guide',ChannelId:id,ChannelName:'Fixture channel',StartDate:new Date(now-60000).toISOString(),EndDate:new Date(now+3600000).toISOString()}],TotalRecordCount:1});return;}
  if(url.pathname==='/Bluefin/Music/Queue'){json(res,{protocolVersion:1,revision:1,queue:{itemIds:[id],cursor:0,shuffle:false,repeat:'off'}});return;}
  if(url.pathname.startsWith('/Sessions')){events.push({method:req.method,path:url.pathname});res.writeHead(204).end();return;}
 }
 const relative=decodeURIComponent(url.pathname==='/'?'/index.html':url.pathname);const file=path.resolve(root,'.'+relative);
 if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}
 try{const data=fs.readFileSync(file);const type={'.html':'text/html','.js':'text/javascript','.css':'text/css','.wasm':'application/wasm','.png':'image/png'}[path.extname(file)]||'application/octet-stream';res.writeHead(200,{'Content-Type':type,'Cache-Control':'no-store'});res.end(data);}catch{res.writeHead(404).end();}
});
app.listen(port,'127.0.0.1',()=>console.log('Synthetic TV fixture listening on loopback port '+port));
