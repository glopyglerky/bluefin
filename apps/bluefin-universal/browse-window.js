// Bounded absolute ranges for server-backed browsing. Offset paging is a
// transport detail; unloaded ranges can be fetched again in either direction.
(() => {
'use strict';
function createWindow(load,{size=60,maxPages=4,current=()=>true,changed=()=>{}}={}){
 const model={size,maxPages,pages:new Map(),pending:new Map(),errors:new Map(),total:0,known:false,next:0,revision:0,desired:[0,size],stopped:false};
 const live=()=>!model.stopped&&current();
 model.items=()=>{const seen=new Set();return [...model.pages].sort((a,b)=>a[0]-b[0]).flatMap(([,page])=>page.items).filter(item=>{const key=item?.Id||item?.id;if(!key||seen.has(key))return false;seen.add(key);return true;});};
 model.at=index=>{for(const [start,page]of model.pages)if(index>=start&&index<start+page.raw)return page.items[index-start];};
 model.contains=index=>[...model.pages].some(([start,page])=>index>=start&&index<start+page.raw);
 model.count=()=>model.known?model.total:Math.max(model.size,model.next+model.size);
 model.request=start=>{
  if(!live()||start<0||model.known&&start>=model.total)return Promise.resolve();
  if(model.pending.has(start))return model.pending.get(start);
  const request=Promise.resolve().then(async()=>{
   try{
    const result=await load(start,model.size);if(!live())return;
    const rows=Array.isArray(result)?result:result.Items||[],raw=rows.length;
    if(raw>model.size)throw Error('The server returned more results than requested.');
    const total=Number(result?.TotalRecordCount);
    if(Number.isSafeInteger(total)&&total>=0){model.known=true;model.total=Math.max(total,start+raw);}
    else if(raw<model.size){model.known=true;model.total=start+raw;}
    else model.total=Math.max(model.total,start+raw);
    if(raw===0){model.known=true;model.total=Math.min(model.total,start);}
    model.pages.set(start,{items:rows,raw});model.next=Math.max(model.next,start+raw);model.errors.delete(start);model.revision++;
    while(model.pages.size>model.maxPages){
     const [first,last]=model.desired;
     const distance=key=>key+model.pages.get(key).raw<first?first-key:key>last?key-last:0;
     const choices=[...model.pages.keys()].filter(key=>key!==start).sort((a,b)=>distance(b)-distance(a));
     model.pages.delete(choices[0]);
    }
   }catch(error){if(live()&&error.name!=='AbortError')model.errors.set(start,String(error.message||error));}
   finally{model.pending.delete(start);if(live())changed(model);}
  });
  model.pending.set(start,request);return request;
 };
 model.ensure=(first,last)=>{
  if(!live())return;
  first=Math.max(0,first);last=Math.min(model.count(),Math.max(first,last));model.desired=[first,last];
  if(model.pending.size)return;
  for(let index=first;index<last;index++){
   if(model.contains(index))continue;
   let start=Math.floor(index/model.size)*model.size;
   // A server may clamp Limit. Continue after its short range without gaps.
   for(const [at,page]of model.pages)if(at<=index&&at+page.raw<=index)start=Math.max(start,at+page.raw);
   if(!model.errors.has(start))model.request(start);return;
  }
 };
 model.retry=()=>{const starts=[...model.errors.keys()];model.errors.clear();if(starts.length)return model.request(starts[0]);};
 model.stop=()=>{model.stopped=true;model.pages.clear();model.errors.clear();};
 return model;
}
function title(item){
 if(item.Type==='Episode')return item.SeriesName?.trim()||'TV episode';
 return item.SeriesName?.trim()||item.Name||'Untitled';
}
function subtitle(item){
 if(item.Type==='Episode'){
  const numbers=[Number.isInteger(item.ParentIndexNumber)?'S'+item.ParentIndexNumber:'',Number.isInteger(item.IndexNumber)?'E'+item.IndexNumber:''].filter(Boolean).join(' ');
  return [numbers,item.Name].filter(Boolean).join(' · ');
 }
 if(['Program','LiveTvProgram'].includes(item.Type))return item.EpisodeTitle||'';
 return [item.AlbumArtist||item.Artists?.join(', ')||item.ProductionYear,item.Album].filter(Boolean).join(' · ');
}
function artwork(item,landscape=false){
 const valid=value=>typeof value==='string'&&/^(?:[a-f0-9]{32}|[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12})$/i.test(value)&&!/^0+$/.test(value.replaceAll('-',''));
 // Episode stills are deliberately excluded. Missing series art gets a
 // neutral placeholder while playback continues to use the episode's ID.
 const series=item.Type==='Episode',owner=series?(valid(item.SeriesId)?item.SeriesId:valid(item.ParentThumbItemId)?item.ParentThumbItemId:null):item.Id;
 if(!valid(owner))return [];
 const key=owner.replaceAll('-','').toLowerCase();
 return (landscape?['Thumb','Backdrop','Primary']:['Primary','Thumb','Backdrop']).map(type=>({id:key,type}));
}
globalThis.BluefinBrowse={createWindow,title,subtitle,artwork};
if(typeof module!=='undefined'&&module.exports)module.exports=globalThis.BluefinBrowse;
})();