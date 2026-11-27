const {test}=require('node:test');
const assert=require('node:assert/strict');
const {createWindow,title,subtitle,artwork}=require('../browse-window.js');
const series='11111111111141118111111111111111',episode='22222222222242228222222222222222';
test('episode presentation uses the series and retains exact episode identity',()=>{
 const item={Id:episode,Type:'Episode',Name:'The Big Quiet',SeriesName:'The Adventures of Pete & Pete',SeriesId:series,ParentIndexNumber:2,IndexNumber:4};
 assert.equal(title(item),'The Adventures of Pete & Pete');assert.equal(subtitle(item),'S2 E4 · The Big Quiet');
 assert.deepEqual(artwork(item,true),[{id:series,type:'Thumb'},{id:series,type:'Backdrop'},{id:series,type:'Primary'}]);
 assert.equal(item.Id,episode);
 assert.deepEqual(artwork({...item,SeriesId:undefined}),[]);
});
test('movies keep their own title and art; guide episodes stay secondary',()=>{
 const movie={Id:episode,Type:'Movie',Name:'Ocean Crossing',ProductionYear:2026};
 assert.equal(title(movie),'Ocean Crossing');assert.equal(artwork(movie,true)[0].id,episode);
 assert.equal(title({Type:'LiveTvProgram',Name:'Chicago P.D.',EpisodeTitle:'Closure'}),'Chicago P.D.');
 assert.equal(subtitle({Type:'LiveTvProgram',Name:'Chicago P.D.',EpisodeTitle:'Closure'}),'Closure');
});
test('long browsing retains four ranges and fetches evicted ranges again',async()=>{
 const calls=[],window=createWindow(async(start,size)=>{calls.push(start);return {Items:Array.from({length:size},(_,i)=>({Id:String(start+i+1)})),TotalRecordCount:1200};},{size:20,maxPages:4});
 for(let start=0;start<1200;start+=20){window.desired=[start,start+20];await window.request(start);assert(window.pages.size<=4);assert(window.items().length<=80);}
 assert.equal(window.at(0),undefined);window.ensure(0,20);await Promise.all(window.pending.values());
 assert.equal(window.at(0).Id,'1');assert.equal(calls.filter(start=>start===0).length,2);
 assert.equal(window.count(),1200);
});
test('late replies cannot repopulate a retired account/query',async()=>{
 let release,current=true,changes=0;
 const window=createWindow(()=>new Promise(resolve=>release=resolve),{current:()=>current,changed:()=>changes++});
 const pending=window.request(0);await Promise.resolve();current=false;
 release({Items:[{Id:'old'}],TotalRecordCount:1});await pending;
 assert.equal(window.pages.size,0);assert.equal(changes,0);
});
test('in-flight ranges deduplicate and transport errors retry without skipping',async()=>{
 let fail=true,calls=0;
 const window=createWindow(async()=>{calls++;if(fail)throw Error('offline');return {Items:[{Id:'one'}],TotalRecordCount:1};});
 const first=window.request(0),same=window.request(0);assert.equal(first,same);await first;
 assert.equal(calls,1);assert.equal(window.next,0);assert.equal(window.errors.size,1);
 fail=false;await window.retry();assert.equal(window.at(0).Id,'one');assert.equal(window.errors.size,0);
});
test('a server-clamped range resumes at the consumed offset',async()=>{
 const calls=[],window=createWindow(async start=>{calls.push(start);return {Items:[{Id:String(start+1)},{Id:String(start+2)}],TotalRecordCount:10};},{size:60});
 await window.request(0);window.ensure(0,6);await Promise.all(window.pending.values());
 assert.deepEqual(calls,[0,2]);assert.equal(window.next,4);
});
test('empty tail stops loading and stopped windows release resident rows',async()=>{
 let calls=0;
 const window=createWindow(async()=>{calls++;return {Items:[],TotalRecordCount:100};});
 await window.request(0);window.ensure(0,60);assert.equal(window.count(),0);assert.equal(calls,1);
 window.pages.set(0,{items:[{Id:'private'}],raw:1});window.stop();assert.equal(window.items().length,0);
});
