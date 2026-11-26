const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const source=fs.readFileSync(path.join(__dirname,'../web/app.js'),'utf8');
const html=fs.readFileSync(path.join(__dirname,'../web/index.html'),'utf8');
class Element {
 constructor(tag){this.tagName=tag.toUpperCase();this.children=[];this.attributes={};this.dataset={};this.style={};this.className='';this.value='';this.text='';this.classList={add(){},remove(){},toggle(){}};}
 appendChild(child){if(child.parentNode)child.remove();this.children.push(child);child.parentNode=this;return child;}
 get firstChild(){return this.children[0];} get lastChild(){return this.children.at(-1);}
 set textContent(value){this.text=String(value);this.children=[];}get textContent(){return this.text+this.children.map(x=>x.textContent||'').join(' ');}
 setAttribute(k,v){this.attributes[k]=String(v);}addEventListener(){}focus(){}scrollIntoView(){}
 querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
 querySelectorAll(selector){return this.children.flatMap(child=>[...(selector[0]==='.'?child.className.split(' ').includes(selector.slice(1)):selector.split(',').map(x=>x.toUpperCase()).includes(child.tagName))?[child]:[],...child.querySelectorAll(selector)]);}
 contains(node){return this===node||this.children.some(child=>child.contains(node));}
 remove(){if(this.parentNode)this.parentNode.children=this.parentNode.children.filter(x=>x!==this);this.parentNode=null;}
}
function setup(reply){
 const nodes=new Map(),requests=[],user='11111111111141118111111111111111';
 const node=id=>{if(!nodes.has(id))nodes.set(id,new Element('div'));return nodes.get(id);};
 const document={getElementById:node,createElement:tag=>new Element(tag),createDocumentFragment:()=>new Element('fragment'),
  body:new Element('body'),activeElement:new Element('button'),querySelectorAll:()=>[],querySelector:()=>new Element('button'),addEventListener(){}};
 const transport=async(url,options)=>{const u=new URL(url);requests.push({url:u,options});const result=await reply?.(u,options);
  return result||new Response(JSON.stringify(u.pathname==='/Bluefin/TV/v1'?{version:1,mediaLease:true}:u.pathname==='/Bluefin/Home/Recommendations'?{recommendations:[]}:u.pathname==='/Movies/Recommendations'?[]:{Items:[],TotalRecordCount:0}),{headers:{'Content-Type':'application/json'}});};
 const context=vm.createContext({document,location:new URL('https://bluefin.test'),URL,fetch:transport,Response,AbortController,Uint8Array,Blob,Map,Set,
  setTimeout,clearTimeout,setInterval(){},localStorage:{getItem(){throw Error('Unavailable');}},getComputedStyle:()=>({visibility:'visible'})});
 context.window=context;context.addEventListener=()=>{};context.scrollTo=()=>{};
 context.BluefinPolicy={load:()=>Promise.resolve(),apply:()=>({})};context.BluefinPlatform={name:'Test TV',init(){},exit(){},position:()=>({}),stop(){}};
 vm.runInContext(source.replace(/\}\(\)\);\s*$/,"globalThis.contract={browse,show,search,searchMore,searchCall,homeRecommendations,acceptLogin,setAccount:function(t,u){token=t;user=u;server='https://bluefin.test';},state:function(){return {epoch:epoch,viewName:viewName,searchState:searchState};}};}());"),context);
 context.contract.setAccount('fixture-token',user);
 return {ui:context.contract,requests,content:node('content'),nodes,user};
}
const json=(value,status=200)=>new Response(JSON.stringify(value),{status,headers:{'Content-Type':'application/json'}});
test('smart TV primary navigation has the six requested categories with separate utilities',()=>{
 const nav=html.match(/<nav id="nav"[\s\S]*?<\/nav>/)[0];
 assert.deepEqual([...nav.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].map(match=>match[1]),['Home','Sports','TV Shows','Movies','Live TV','Music']);
 assert.match(html,/id="utilities"/);assert.match(html,/data-view="collections"/);
});
test('smart TV refined search queries every server page and counts duplicate source rows',async()=>{
 const a={Id:'22222222222242228222222222222222',Name:'A',Type:'Movie'},b={Id:'33333333333343338333333333333333',Name:'B',Type:'Movie'};
 const t=setup(u=>u.searchParams.get('SearchTerm')==='sea'?json({Items:u.searchParams.get('StartIndex')==='0'?[a,a]:[b],TotalRecordCount:3}):undefined);
 await t.ui.show('search');await t.ui.search('sea',{scope:'movies',studioID:'44444444444444448444444444444444',tag:'Service: Acme+',collectionID:'55555555555545558555555555555555'});
 await t.ui.searchMore('library');
 const requests=t.requests.filter(r=>r.url.searchParams.get('SearchTerm')==='sea');
 assert.equal(requests.length,2);assert.equal(requests[1].url.searchParams.get('StartIndex'),'2');
 for(const {url}of requests){assert.equal(url.searchParams.get('IncludeItemTypes'),'Movie');assert.equal(url.searchParams.get('StudioIds'),'44444444444444448444444444444444');assert.equal(url.searchParams.get('Tags'),'Service: Acme+');assert.equal(url.searchParams.get('ParentId'),'55555555555545558555555555555555');}
 assert.deepEqual(Array.from(t.ui.state().searchState.pages[0].items,x=>x.Name),['A','B']);
});
test('smart TV broad search includes artists and live programme endpoints',async()=>{
 const t=setup();await t.ui.show('search');await t.ui.search('sea',{scope:'all'});
 assert(t.requests.some(r=>r.url.pathname==='/Artists'));assert(t.requests.some(r=>r.url.pathname==='/LiveTv/Programs'));assert(t.requests.some(r=>r.url.pathname==='/LiveTv/Channels'));
});
test('smart TV home never changes a denied recommendation request into legacy access',async()=>{
 for(const status of [404,403]){
  const t=setup(u=>u.pathname==='/Bluefin/Home/Recommendations'?json({},status):u.pathname==='/Movies/Recommendations'?json([]):undefined);
  await t.ui.show('home');
  assert.equal(t.requests.filter(r=>r.url.pathname==='/Movies/Recommendations').length,status===404?1:0);
 }
});

test('smart TV metadata controls survive initial search and changed search generations',async()=>{
 let finish;
 const t=setup(async u=>{
  if(u.pathname==='/Studios'){await new Promise(resolve=>finish=resolve);return json({Items:[{Id:'44444444444444448444444444444444',Name:'Studio from server'}],TotalRecordCount:1});}
  if(u.pathname==='/Items/Filters2')return json({Tags:['Service: Acme+']});
 });
 await t.ui.show('search');await t.ui.search('later',{scope:'movies'});
 finish();for(let n=0;n<20;n++)await new Promise(setImmediate);
 assert.match(t.content.textContent,/Studio from server/);assert.match(t.content.textContent,/Service: Acme\+/);
});

test('smart TV artist selection pages through credited media instead of a folder',async()=>{
 const t=setup(),artist='77777777777747778777777777777777';
 for(const start of [0,40])await t.ui.browse({Id:artist,Name:'Account artist',Type:'MusicArtist'},start);
 const requests=t.requests.filter(request=>request.url.searchParams.get('ArtistIds')===artist);
 assert.equal(requests.length,2);assert.equal(requests[1].url.searchParams.get('StartIndex'),'40');
 for(const {url}of requests){assert.equal(url.searchParams.has('ParentId'),false);assert.equal(url.searchParams.get('IncludeItemTypes'),'MusicAlbum,Audio,MusicVideo');}
});
